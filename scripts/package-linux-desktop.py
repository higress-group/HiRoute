#!/usr/bin/env python3
"""Build the native Arch Desktop candidate from a verified standalone bundle."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ARCH = ROOT / "apps/desktop/arch"
SUPPORTED = {"aarch64", "x86_64"}


def console_fingerprint(manifest):
    """Match the daemon's sorted, content-addressed console inventory."""
    files = manifest["files"]
    if "console/index.html" not in files:
        raise ValueError("the desktop requires a standalone console")
    digest = hashlib.sha256()
    for name, record in sorted(files.items()):
        if name.startswith("console/"):
            digest.update(f"{name}\0{record['sha256']}\n".encode("ascii"))
    return digest.hexdigest()


def output(*command, cwd=ROOT):
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def package(args):
    architecture = platform.machine()
    target = f"{architecture}-unknown-linux-gnu"
    if platform.system() != "Linux" or architecture not in SUPPORTED:
        raise ValueError("use a native aarch64 or x86_64 Arch build host")
    standalone = runpy.run_path(str(ROOT / "scripts/package-standalone.py"))
    manifest = standalone["verify"](args.manifest, args.archive)
    if manifest["target"] != target:
        raise ValueError("standalone payload does not match the native build host")
    revision = output("git", "rev-parse", "HEAD")
    if manifest["revision"] != revision or output("git", "status", "--porcelain"):
        raise ValueError("build from the clean source revision of the standalone payload")
    version = manifest["version"]
    if not re.fullmatch(r"[0-9][a-zA-Z0-9._-]{0,63}", version):
        raise ValueError("standalone version cannot be represented as an Arch package")
    fingerprint = console_fingerprint(manifest)
    binary = args.binary
    if binary is None:
        subprocess.run(["npm", "ci", "--prefix", "apps/shell"], cwd=ROOT, check=True)
        subprocess.run(["npm", "run", "build", "--prefix", "apps/shell"], cwd=ROOT, check=True)
        env = {**os.environ, "HIROUTE_BUILD_SOURCE_SHA": revision,
               "HIROUTE_LINUX_CONSOLE_ASSETS_SHA256": fingerprint}
        subprocess.run([
            "cargo", "build", "--locked", "--release", "--target", target, "-p", "hiroute-desktop",
            "--features", "desktop-runtime", "--bin", "hiroute-desktop",
        ], cwd=ROOT, env=env, check=True)
        binary = ROOT / "target" / target / "release/hiroute-desktop"
    identity = json.loads(output(binary.resolve(), "--version"))
    if any(identity.get(key) != value for key, value in {
        "component": "linux-desktop", "version": version, "source_revision": revision,
        "os": "linux", "arch": architecture, "console_assets_sha256": fingerprint,
    }.items()):
        raise ValueError("desktop binary does not match the verified standalone and console files")
    epoch = output("git", "show", "-s", "--format=%ct", revision)
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="hiroute-arch-") as directory:
        stage = Path(directory)
        sources = {
            "hiroute-desktop": binary,
            "ai.hiroute.desktop.desktop": ARCH / "ai.hiroute.desktop.desktop",
            "icon.svg": ROOT / "apps/desktop/src-tauri/icons/icon.svg",
            "LICENSE": ROOT / "LICENSE",
            "manifest.json": args.manifest,
            "runtime.tar.gz": args.archive,
            "install-standalone.py": ROOT / "scripts/install-standalone.py",
        }
        checksums = []
        for name, source in sources.items():
            shutil.copyfile(source, stage / name)
            checksums.append("'" + standalone["digest"](stage / name) + "'")
        recipe = (ARCH / "PKGBUILD").read_text().replace(
            "pkgver=0.0.0", "pkgver=" + version.replace("-", "."),
        ).replace("_runtime_version=0.0.0", "_runtime_version=" + version).replace("sha256sums=()", f"sha256sums=({' '.join(checksums)})")
        (stage / "PKGBUILD").write_text(recipe)
        env = {**os.environ, "PKGDEST": str(stage), "SOURCE_DATE_EPOCH": epoch}
        subprocess.run(["makepkg", "--noconfirm", "--nosign"], cwd=stage, env=env, check=True)
        paths = list(stage.glob("hiroute-desktop-*.pkg.tar.*"))
        if len(paths) != 1:
            raise ValueError("expected one Arch Desktop package")
        members = set(output("bsdtar", "-tf", paths[0]).splitlines())
        payload = f"usr/share/hiroute-desktop/standalone/HiRoute-{version}-linux-{architecture}.tar.gz"
        if payload not in members:
            raise ValueError("Arch package omitted the standalone installer's accepted archive name")
        artifact = args.output / paths[0].name
        if artifact.exists() or artifact.is_symlink():
            raise ValueError("output already contains this package")
        shutil.copyfile(paths[0], artifact)
    checksum = standalone["digest"](artifact)
    artifact.with_name(artifact.name + ".sha256").write_text(f"{checksum}  {artifact.name}\n")
    return {"path": str(artifact), "sha256": checksum, "target": target, "revision": revision,
            "version": version, "console_assets_sha256": fingerprint}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--binary", type=Path, help="use a prebuilt shell with the same compiled identity")
    parser.add_argument("--output", type=Path, required=True)
    try:
        print(json.dumps(package(parser.parse_args()), sort_keys=True))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Linux Desktop packaging failed: {error}\n")


if __name__ == "__main__":
    main()
