#!/usr/bin/env python3
"""Build one native release candidate; never publish or modify an installation."""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
TARGETS = (
    "aarch64-apple-darwin", "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
)


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / f"{name}.py")
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def host_target():
    arch = {"arm64": "aarch64", "AMD64": "x86_64"}.get(platform.machine(), platform.machine())
    suffix = {"Darwin": "apple-darwin", "Linux": "unknown-linux-gnu"}.get(platform.system())
    return f"{arch}-{suffix}"


def inspect_elf(path, target):
    with path.open("rb") as stream:
        header = stream.read(20)
    machine = {"x86_64-unknown-linux-gnu": 62, "aarch64-unknown-linux-gnu": 183}[target]
    if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
            or struct.unpack_from("<H", header, 18)[0] != machine):
        raise ValueError(f"binary architecture differs from {target}: {path.name}")


class Candidate:
    def __init__(self, target, source_repo):
        self.target = target
        self.source_repo = source_repo.resolve(strict=True)
        self.revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
        parent = REPO / "target/release-candidate" / self.revision / target
        parent.mkdir(parents=True, exist_ok=True)
        self.output = Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent))
        self.result = {"revision": self.revision, "target": target,
                       "host": {"system": platform.system(), "architecture": platform.machine(),
                                "libc": platform.libc_ver()},
                       "status": "failed", "timings": []}
        self.environment = dict(os.environ, RUSTC_WRAPPER="sccache", CARGO_INCREMENTAL="0",
                                HIROUTE_BUILD_SOURCE_SHA=self.revision)

    def run(self, *arguments, env=None, timeout=None):
        started = time.monotonic()
        print(f"Running {Path(str(arguments[0])).name} {' '.join(map(str, arguments[1:3]))}", file=sys.stderr, flush=True)
        result = subprocess.run(list(map(str, arguments)), cwd=REPO, env=env or self.environment,
                                text=True, capture_output=True, timeout=timeout)
        self.result["timings"].append({"command": list(map(str, arguments[:3])),
                                       "seconds": round(time.monotonic() - started, 3),
                                       "exit_code": result.returncode})
        with (self.output / "build.log").open("a") as log:
            log.write(f"$ {' '.join(map(str, arguments))}\n{result.stdout}{result.stderr}\n")
        result.check_returncode()
        return result.stdout.strip()

    def script(self, name, *args):
        return self.run(sys.executable, REPO / "scripts" / f"{name}.py", *args)

    def preflight(self):
        if self.target not in TARGETS or host_target() != self.target:
            raise ValueError(f"native host required for {self.target}; host is {host_target()}")
        if self.run("git", "status", "--porcelain", "--untracked-files=normal"):
            raise ValueError("build requires a clean committed candidate")
        if any(key in os.environ for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR")):
            raise ValueError("external Cargo targets are forbidden")
        self.version = json.loads((REPO / "apps/desktop/src-tauri/tauri.conf.json").read_text())["version"]
        module("package-desktop").validate_release_versions(self.version)
        metadata = json.loads(self.run("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"))
        if Path(metadata["target_directory"]).resolve() != REPO / "target":
            raise ValueError("Cargo config must use checkout-local target/")
        self.result["version"] = self.version
        self.result["toolchain"] = {name: self.run(name, "--version") for name in ("cargo", "rustc")}

    def desktop(self):
        arch = "arm64" if self.target.startswith("aarch64-") else "x86_64"
        result = json.loads(self.script("package-desktop", "build", "--arch", arch,
                                       "--cpa-source-repo", self.source_repo))
        if (result["revision"] != self.revision or result["version"] != self.version
                or result["architecture"] != arch or result["distribution"] != "controlled-trial"):
            raise ValueError("desktop package differs from candidate identity")
        self.result["desktop_result"] = result
        dmg = Path(result["dmg"])
        artifact = {"kind": "desktop", "platform": "macOS", "architecture": arch,
                    "format": "dmg", "minimum_os": module("package-desktop").MINIMUM_MACOS,
                    "distribution": "self-signed", "filename": dmg.name,
                    "sha256": module("package-desktop").digest(dmg), "size": dmg.stat().st_size}
        if artifact["sha256"] != result["dmg_sha256"]:
            raise ValueError("desktop DMG changed after verification")
        return artifact, [dmg]

    def standalone(self):
        cpa = self.output / "cliproxyapi"
        provenance = json.loads(self.script("build-cpa", "--source-repo", self.source_repo,
                                           "--target", self.target, "--output", cpa))
        pin = module("build-cpa").pinned_source()[0]
        # The helper version probe has no service side effects and no user config.
        with tempfile.TemporaryDirectory(prefix="hiroute-release-probe-") as directory:
            probe = self.run(cpa, "--help", env={"HOME": directory, "PATH": "/usr/bin:/bin",
                                               "TMPDIR": directory}, timeout=60)
        match = re.search(r"CLIProxyAPI Version: ([^,]+), Commit: ([0-9a-f]{7,40}), BuiltAt: ([^\r\n]+)", probe)
        if not match or match[1] != pin["version"] or not pin["commit"].startswith(match[2]) or match[3] != pin["built_at"]:
            raise ValueError("CPA release version/commit/build date mismatch")
        self.run("cargo", "build", "--locked", "--release", "--target", self.target,
                 "-p", "hiroute-cli", "-p", "hiroute-daemon", "--bin", "hiroute", "--bin", "hirouted")
        binaries = REPO / "target" / self.target / "release"
        for binary in (binaries / "hiroute", binaries / "hirouted", cpa):
            inspect_elf(binary, self.target)
        self.result["linux_dependencies"] = {
            name: self.run("readelf", "--dynamic", "--version-info", binaries / name)
            for name in ("hiroute", "hirouted")}
        notices = self.output / "notices"
        self.script("collect-third-party-licenses", "--cargo-target", self.target,
                    "--cpa-source-repo", self.source_repo, "--output", notices)
        packaged = json.loads(self.script(
            "package-standalone", "build", "--version", self.version, "--revision", self.revision,
            "--target", self.target, "--hiroute", binaries / "hiroute", "--hirouted", binaries / "hirouted",
            "--cpa-binary", cpa, "--cpa-version", pin["version"], "--cpa-license", cpa.with_suffix(".LICENSE"),
            "--notices", notices, "--output", self.output))
        archive, manifest = Path(packaged["archive"]), Path(packaged["manifest"])
        packager = module("package-standalone")
        verified = packager.verify(manifest, archive)
        if any(verified[key] != expected for key, expected in (
                ("revision", self.revision), ("version", self.version), ("target", self.target))):
            raise ValueError("standalone package differs from candidate identity")
        self.result["cpa_source"] = provenance
        artifact = {"kind": "standalone", "platform": "Linux", "architecture": self.target.split("-")[0],
                    "target": self.target, "format": "tar.gz", "distribution": "unsigned",
                    **verified["archive"], "manifest_filename": manifest.name,
                    "manifest_sha256": packager.digest(manifest), "manifest_size": manifest.stat().st_size}
        return artifact, [archive, manifest]

    def build(self):
        try:
            self.preflight()
            if self.target.endswith("apple-darwin"):
                artifact, files = self.desktop()  # The Desktop packager owns its existing lock.
            else:
                module("package-desktop").ensure_sccache_server()
                with module("local-rust").Store().locked(REPO):
                    artifact, files = self.standalone()
            assets = self.output / "assets"
            assets.mkdir()
            for path in files:
                shutil.copyfile(path, assets / path.name)
            self.result.update(status="completed", package_integrity="green",
                               website_artifact=artifact, assets=str(assets))
            return self.result
        except Exception as error:
            self.result["error"] = str(error)
            raise
        finally:
            (self.output / "result.json").write_text(json.dumps(self.result, indent=2) + "\n")
            print(f"Candidate evidence: {self.output}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--cpa-source-repo", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(Candidate(args.target, args.cpa_source_repo).build(), indent=2))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
