#!/usr/bin/env python3
"""Stage the one pinned development CPA beside an already-built hirouted.

The source path is always explicit. This script neither searches PATH nor downloads artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


MANIFEST = Path(__file__).resolve().parents[1] / "apps/desktop/src-tauri/development-cpa-artifacts.v1.json"
SCHEMA = "hiroute.desktop.cpa-artifacts/v1"
VERSION_LINE = re.compile(
    r"CLIProxyAPI Version: (?P<version>[^,\r\n]+), Commit: (?P<commit>[0-9a-f]{40}), BuiltAt: (?P<built_at>[^\r\n]+)"
)
MAX_PROBE_OUTPUT = 256 * 1024
PROBE_TIMEOUT_SECONDS = 30


class StageError(Exception):
    pass


def fail(code: str) -> None:
    raise StageError(code)


def read_manifest(path: Path = MANIFEST) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        fail("DEVELOPMENT_CPA_MANIFEST_INVALID")
    if manifest.get("schema") != SCHEMA or not isinstance(manifest.get("development_only"), bool):
        fail("DEVELOPMENT_CPA_MANIFEST_INVALID")
    return manifest


def host_identity() -> tuple[str, str]:
    system = platform.system()
    machine = platform.machine().lower()
    os_name = "macos" if system == "Darwin" else system.lower()
    arch = "aarch64" if machine in {"arm64", "aarch64"} else machine
    return os_name, arch


def selected_artifact(manifest: dict[str, Any]) -> dict[str, Any]:
    os_name, arch = host_identity()
    matches = [item for item in manifest.get("artifacts", []) if item.get("os") == os_name and item.get("arch") == arch]
    if len(matches) != 1:
        fail("DEVELOPMENT_CPA_TARGET_UNAVAILABLE")
    artifact = matches[0]
    required = {
        "target", "binary_name", "version", "commit", "built_at", "size", "sha256",
        "file_description", "dynamic_dependencies", "signature",
    }
    if set(artifact) != required | {"os", "arch"}:
        fail("DEVELOPMENT_CPA_MANIFEST_INVALID")
    if artifact["binary_name"] != "cliproxyapi" or not re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"]):
        fail("DEVELOPMENT_CPA_MANIFEST_INVALID")
    current = json.loads((MANIFEST.parents[3] / "vendor/cpa/source.json").read_text())
    if artifact["version"] != current["version"] or artifact["commit"] != current["commit"]:
        fail("DEVELOPMENT_CPA_PIN_STALE: generate a current CPA manifest with package-desktop.py; pass --manifest and build with the same HIROUTE_CPA_MANIFEST")
    return artifact


def regular_owned_file(path: Path, error_code: str) -> os.stat_result:
    try:
        metadata = path.lstat()
    except OSError:
        fail(error_code)
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid():
        fail(error_code)
    return metadata


def run_probe(arguments: list[str], *, env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(
            arguments,
            capture_output=True,
            text=True,
            timeout=PROBE_TIMEOUT_SECONDS,
            env=env,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        fail("DEVELOPMENT_CPA_PROBE_FAILED")
    output = result.stdout + result.stderr
    if result.returncode != 0 or len(output.encode("utf-8")) > MAX_PROBE_OUTPUT:
        fail("DEVELOPMENT_CPA_PROBE_FAILED")
    return output


def inspect_staged(path: Path, artifact: dict[str, Any]) -> None:
    description = run_probe(["/usr/bin/file", "-b", os.fspath(path)]).strip()
    if description != artifact["file_description"]:
        fail("DEVELOPMENT_CPA_ARCHITECTURE_MISMATCH")

    dependency_output = run_probe(["/usr/bin/otool", "-L", os.fspath(path)])
    dependencies = [line.strip().split(" (", 1)[0] for line in dependency_output.splitlines()[1:] if line.strip()]
    if dependencies != artifact["dynamic_dependencies"]:
        fail("DEVELOPMENT_CPA_DEPENDENCIES_MISMATCH")

    run_probe(["/usr/bin/codesign", "--verify", "--strict", os.fspath(path)])
    signature = run_probe(["/usr/bin/codesign", "-dv", "--verbose=4", os.fspath(path)])
    expected_signature = artifact["signature"]
    team = expected_signature["team_identifier"] or "not set"
    if f"Signature={expected_signature['kind']}" not in signature or f"TeamIdentifier={team}" not in signature:
        fail("DEVELOPMENT_CPA_SIGNATURE_MISMATCH")

    with tempfile.TemporaryDirectory(prefix="hiroute-cpa-probe-") as probe_root:
        probe_env = {
            "HOME": probe_root,
            "LANG": "C",
            "LC_ALL": "C",
            "PATH": "/usr/bin:/bin",
            "TMPDIR": probe_root,
        }
        help_output = run_probe([os.fspath(path), "--help"], env=probe_env)
    match = VERSION_LINE.search(help_output)
    if not match or any(match.group(field) != artifact[field] for field in ("version", "commit", "built_at")):
        fail("DEVELOPMENT_CPA_VERSION_MISMATCH")


def copy_and_measure(source: Path, destination_dir: Path, artifact: dict[str, Any]) -> tuple[Path, Path, str]:
    staging_dir = Path(tempfile.mkdtemp(prefix=".hiroute-cpa.", dir=destination_dir))
    staging_dir.chmod(0o700)
    temporary_path = staging_dir / artifact["binary_name"]
    try:
        before = regular_owned_file(source, "DEVELOPMENT_CPA_SOURCE_UNTRUSTED")
        copied = subprocess.run(
            ["/bin/cp", "-p", os.fspath(source.resolve(strict=True)), os.fspath(temporary_path)],
            capture_output=True,
            timeout=PROBE_TIMEOUT_SECONDS,
            check=False,
        )
        if copied.returncode != 0 or copied.stdout or copied.stderr:
            fail("DEVELOPMENT_CPA_COPY_FAILED")
        after = regular_owned_file(source, "DEVELOPMENT_CPA_SOURCE_UNTRUSTED")
        identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        if identity(before) != identity(after):
            fail("DEVELOPMENT_CPA_SOURCE_CHANGED")
        staged = regular_owned_file(temporary_path, "DEVELOPMENT_CPA_COPY_FAILED")
        if staged.st_size != artifact["size"] or staged.st_mode & 0o111 == 0 or staged.st_mode & 0o022:
            fail("DEVELOPMENT_CPA_COPY_FAILED")
        measured = hashlib.sha256(temporary_path.read_bytes()).hexdigest()
        if measured != artifact["sha256"]:
            fail("DEVELOPMENT_CPA_DIGEST_MISMATCH")
        inspect_staged(temporary_path, artifact)
        return staging_dir, temporary_path, measured
    except Exception:
        temporary_path.unlink(missing_ok=True)
        staging_dir.rmdir()
        raise


def stage(source: Path, hirouted: Path, manifest: Path = MANIFEST) -> dict[str, Any]:
    artifact = selected_artifact(read_manifest(manifest))
    source_stat = regular_owned_file(source, "DEVELOPMENT_CPA_SOURCE_UNTRUSTED")
    if source_stat.st_mode & 0o022:
        fail("DEVELOPMENT_CPA_SOURCE_UNTRUSTED")
    regular_owned_file(hirouted, "DAEMON_BINARY_UNAVAILABLE")
    destination_dir = hirouted.resolve(strict=True).parent
    destination = destination_dir / artifact["binary_name"]
    if destination.exists() or destination.is_symlink():
        regular_owned_file(destination, "DEVELOPMENT_CPA_DESTINATION_UNTRUSTED")

    staging_dir, temporary, measured = copy_and_measure(source, destination_dir, artifact)
    try:
        os.replace(temporary, destination)
        directory_fd = os.open(destination_dir, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        temporary.unlink(missing_ok=True)
        staging_dir.rmdir()

    final_stat = regular_owned_file(destination, "DEVELOPMENT_CPA_DESTINATION_UNTRUSTED")
    if final_stat.st_size != artifact["size"] or hashlib.sha256(destination.read_bytes()).hexdigest() != measured:
        fail("DEVELOPMENT_CPA_DESTINATION_MISMATCH")
    return {
        "status": "staged",
        "development_only": True,
        "target": artifact["target"],
        "version": artifact["version"],
        "commit": artifact["commit"],
        "sha256": measured,
        "destination": os.fspath(destination),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpa", required=True, type=Path, help="explicit path to the pinned CPA artifact")
    parser.add_argument("--hirouted", required=True, type=Path, help="already-built hirouted that will own the adjacent CPA")
    parser.add_argument("--manifest", type=Path, default=MANIFEST, help="manifest also selected by HIROUTE_CPA_MANIFEST when building Desktop")
    arguments = parser.parse_args()
    try:
        result = stage(arguments.cpa.expanduser(), arguments.hirouted.expanduser(), arguments.manifest.expanduser())
    except StageError as error:
        print(json.dumps({"status": "error", "code": str(error)}, sort_keys=True), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
