#!/usr/bin/env python3
"""Stage verified standalone bytes and audit Windows Desktop package identity."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "hiroute.windows-desktop-package/v1"


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / (name + ".py"))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def digest(path):
    sha = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(chunk)
    return sha.hexdigest()


def regular(path):
    for item in (path, *path.parents):
        if item.exists() and (item.is_symlink() or getattr(item.lstat(), "st_file_attributes", 0) & 0x400):
            raise ValueError("package staging contains a link or reparse point")
    if path.exists() and not path.is_file():
        raise ValueError("package staging entry is not a regular file")


def stage(manifest, archive, destination):
    packager = module("package-standalone")
    package = packager.verify(manifest, archive)
    if package["target"] != "x86_64-pc-windows-msvc" or "console/index.html" not in package["files"]:
        raise ValueError("Windows Desktop requires the x64 package and console assets")
    payload = {name.removeprefix("bin/"): data for name, (data, _) in packager.safe_members(archive, package["files"]).items()}
    payload["package-manifest.json"] = manifest.read_bytes()
    record = destination.parent / "standalone-stage.json"
    if record.exists():
        regular(record)
        old = json.loads(record.read_text())
        if old.get("schema") != "hiroute.windows-shell-stage/v1" or not isinstance(old.get("files"), dict):
            raise ValueError("unknown standalone stage ownership")
        for name, expected in old["files"].items():
            if not safe_name(name):
                raise ValueError("unsafe stage ownership entry")
            file = destination / name
            regular(file)
            if not file.is_file() or digest(file) != expected:
                raise ValueError("an owned standalone stage entry changed")
        entries = list(destination.rglob("*"))
        if any(p.is_symlink() or getattr(p.lstat(), "st_file_attributes", 0) & 0x400 for p in entries):
            raise ValueError("standalone stage contains a link or reparse point")
        actual = {p.relative_to(destination).as_posix() for p in entries if p.is_file()}
        if actual != set(old["files"]):
            raise ValueError("foreign standalone stage entries must be preserved")
        for name in old["files"]:
            (destination / name).unlink()
    elif destination.exists() and any(destination.rglob("*")):
        raise ValueError("refuse an unowned standalone staging directory")
    for name, data in payload.items():
        if not safe_name(name):
            raise ValueError("unsafe standalone entry")
        file = destination / name
        regular(file)
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(data)
    record.write_text(json.dumps({"schema": "hiroute.windows-shell-stage/v1", "revision": package["revision"],
                                  "files": {name: hashlib.sha256(data).hexdigest() for name, data in payload.items()}}, indent=2) + "\n")
    return package


def safe_name(name):
    return isinstance(name, str) and not PurePosixPath(name).is_absolute() and "\\" not in name and all(
        part not in ("", ".", "..") and re.fullmatch(r"[A-Za-z0-9_.@-]+", part) for part in name.split("/"))


def shell_image(installer, run=None):
    """Audit the actual NSIS member, without executing the installer."""
    regular(installer)
    extractor = shutil.which("7z") or shutil.which("7zz")
    if not extractor:
        raise ValueError("7-Zip is required to audit the embedded NSIS shell")
    with tempfile.TemporaryDirectory(prefix="nsis-shell-", dir=installer.parent) as temporary:
        args = (extractor, "x", "-y", "-bd", "-r", "-o" + temporary,
                str(installer.resolve()), "hiroute-shell.exe")
        if run:
            run(*args)
        else:
            subprocess.run(args, check=True, capture_output=True, text=True)
        images = list(Path(temporary).rglob("hiroute-shell.exe"))
        if len(images) != 1:
            raise ValueError("NSIS must contain exactly one shell image")
        regular(images[0])
        image = module("windows-native-runtime-audit").image_record(
            images[0].read_bytes(), "hiroute-shell.exe", "windows-desktop-release")
        if (image["pe_machine"] != 0x8664 or image["imports"]["debug_crt"]
                or image["imports"]["release_vc_runtime"]):
            raise ValueError("Windows Desktop has an unexpected architecture or CRT dependency")
        return image


def verify(installer, identity, revision):
    if not isinstance(identity, dict):
        raise ValueError("Windows Desktop identity must be an object")
    regular(installer)
    expected = {"schema": SCHEMA, "revision": revision, "architecture": "x86_64", "distribution": "unsigned",
                "installer_sha256": digest(installer), "installer_size": installer.stat().st_size,
                "integrity": "green", "embedded_components": "green", "native_runtime": "green"}
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or any(identity.get(k) != v for k, v in expected.items()):
        raise ValueError("Windows Desktop identity lacks matching native acceptance")
    if installer.name != f'HiRoute-{identity.get("version")}-{revision[:12]}-windows-x86_64-setup.exe':
        raise ValueError("Windows Desktop installer identity differs from its filename")
    evidence = identity.get("native_evidence", {})
    runs = evidence.get("runs", []) if isinstance(evidence, dict) else []
    required = json.loads((ROOT / "scripts/windows-desktop-cases.json").read_text())
    if (not isinstance(runs, list) or len(runs) != len(required) or any(not isinstance(run, dict) for run in runs)
            or {run.get("platform") for run in runs} != set(required)):
        raise ValueError("native evidence must cover Windows 11 x64 and the Server selection")
    for native in runs:
        scenarios = native.get("scenarios", {})
        if (native.get("revision") != revision or native.get("installer_sha256") != expected["installer_sha256"]
                or native.get("standard_user") is not True or native.get("architecture") != "AMD64"
                or not native.get("os_version") or not isinstance(scenarios, dict)
                or any(scenarios.get(case) != "green" for case in required[native["platform"]])
                or any(value != "green" for value in scenarios.values())):
            raise ValueError("native evidence does not bind all standard-user scenarios to this artifact")
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("stage")
    for name in ("manifest", "archive", "destination"):
        p.add_argument("--" + name, type=Path, required=True)
    p = sub.add_parser("verify")
    p.add_argument("--installer", type=Path, required=True)
    p.add_argument("--identity", type=Path, required=True)
    p.add_argument("--revision", required=True)
    p = sub.add_parser("image")
    p.add_argument("--installer", type=Path, required=True)
    args = parser.parse_args()
    if args.action == "image":
        result = shell_image(args.installer)
    elif args.action == "stage":
        result = stage(args.manifest, args.archive, args.destination)
    else:
        result = verify(args.installer, json.loads(args.identity.read_text()), args.revision)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
