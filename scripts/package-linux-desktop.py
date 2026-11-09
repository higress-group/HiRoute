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


def verify_member(tar, package_path, name, expected_sha256, expected_size):
    digest = hashlib.sha256()
    size = 0
    with subprocess.Popen([tar, "-xOf", str(package_path), name], stdout=subprocess.PIPE) as reader:
        for chunk in iter(lambda: reader.stdout.read(1024 * 1024), b""):
            size += len(chunk)
            if size > expected_size:
                reader.kill()
                raise ValueError(f"Omarchy package member exceeds its expected size: {name}")
            digest.update(chunk)
        if reader.wait() != 0 or size != expected_size or digest.hexdigest() != expected_sha256:
            raise ValueError(f"Omarchy package member differs from the qualified runtime: {name}")


def verify_desktop_architecture(tar, package_path, name, arch):
    machine = {"aarch64": 183, "x86_64": 62}[arch]
    with subprocess.Popen([tar, "-xOf", str(package_path), name], stdout=subprocess.PIPE) as reader:
        header = reader.stdout.read(20)
        if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
                or int.from_bytes(header[18:20], "little") != machine):
            reader.kill()
            raise ValueError("Omarchy desktop executable does not match the package architecture")
        while reader.stdout.read(1024 * 1024):
            pass
        if reader.wait() != 0:
            raise ValueError("Omarchy desktop executable is unreadable")


def verify_package(args):
    package_path = args.verify_package.resolve()
    signature = package_path.with_name(package_path.name + ".sig")
    with tempfile.TemporaryDirectory(prefix="hiroute-omarchy-verify-") as directory:
        keyring = Path(directory) / "trusted.gpg"
        subprocess.run(["gpg", "--no-options", "--homedir", directory, "--batch", "--dearmor",
                        "--output", str(keyring), str(args.public_key.resolve())], check=True)
        status = output("gpgv", "--homedir", directory, "--keyring", str(keyring), "--status-fd", "1",
                        str(signature), str(package_path))
    signers = [line.split()[-1] for line in status.splitlines() if line.startswith("[GNUPG:] VALIDSIG ")]
    if len(signers) != 1 or not re.fullmatch(r"[0-9A-F]{40}", signers[0]):
        raise ValueError("Omarchy package requires one trusted OpenPGP signature")

    standalone = runpy.run_path(str(ROOT / "scripts/package-standalone.py"))
    manifest = standalone["verify"](args.manifest, args.archive)
    arch = manifest["target"].split("-", 1)[0]
    if arch not in SUPPORTED or manifest["target"] != f"{arch}-unknown-linux-gnu":
        raise ValueError("Omarchy package requires a supported Linux standalone runtime")
    version = manifest["version"]
    revision = manifest["revision"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Omarchy package requires an exact source revision")
    prefix = "usr/share/hiroute-desktop/standalone/"
    runtime_name = f"{prefix}HiRoute-{version}-linux-{arch}.tar.gz"
    manifest_name = f"{prefix}manifest.json"
    desktop = "usr/bin/hiroute-desktop"
    source_files = {
        "usr/share/applications/ai.hiroute.desktop.desktop": "apps/desktop/arch/ai.hiroute.desktop.desktop",
        "usr/share/icons/hicolor/scalable/apps/ai.hiroute.desktop.svg": "apps/desktop/src-tauri/icons/icon.svg",
        "usr/share/licenses/hiroute-desktop/LICENSE": "LICENSE",
        f"{prefix}install-standalone.py": "scripts/install-standalone.py",
    }
    required = {
        ".PKGINFO", ".BUILDINFO", ".MTREE", desktop, *source_files, manifest_name, runtime_name,
    }
    tar = shutil.which("bsdtar") or "tar"
    files = [name for name in output(tar, "-tf", str(package_path)).splitlines() if not name.endswith("/")]
    if len(files) != len(required) or set(files) != required:
        raise ValueError("Omarchy package has a different file inventory")
    info = {}
    for line in output(tar, "-xOf", str(package_path), ".PKGINFO").splitlines():
        field, _, value = line.partition(" = ")
        if field in {"pkgname", "arch", "pkgver"}:
            if field in info:
                raise ValueError(f"Omarchy package metadata repeats {field}")
            info[field] = value
    if (info.get("pkgname") != "hiroute-desktop" or info.get("arch") != arch
            or not re.fullmatch(re.escape(version.replace("-", ".")) + r"-[0-9]+(?:\.[0-9]+)?", info.get("pkgver", ""))):
        raise ValueError("Omarchy package metadata differs from the qualified runtime")
    verify_desktop_architecture(tar, package_path, desktop, arch)
    verify_member(tar, package_path, manifest_name, standalone["digest"](args.manifest), args.manifest.stat().st_size)
    verify_member(tar, package_path, runtime_name, manifest["archive"]["sha256"], manifest["archive"]["size"])
    for name, path in source_files.items():
        source = subprocess.check_output(["git", "show", f"{revision}:{path}"], cwd=ROOT)
        verify_member(tar, package_path, name, hashlib.sha256(source).hexdigest(), len(source))
    return {"architecture": arch, "revision": revision, "version": version,
            "signing_fingerprint": signers[0], "sha256": standalone["digest"](package_path)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--binary", type=Path, help="use a prebuilt shell with the same compiled identity")
    parser.add_argument("--output", type=Path, help="destination for a new Arch package")
    parser.add_argument("--verify-package", type=Path, help="verify a signed Arch package and its bundled runtime")
    parser.add_argument("--public-key", type=Path, help="trusted OpenPGP public key for verification")
    args = parser.parse_args()
    if args.verify_package:
        if not args.public_key or args.output or args.binary:
            parser.error("--verify-package requires --public-key and does not build a package")
    elif not args.output or args.public_key:
        parser.error("--output is required to build a package")
    try:
        result = verify_package(args) if args.verify_package else package(args)
        print(json.dumps(result, sort_keys=True))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Linux Desktop packaging failed: {error}\n")


if __name__ == "__main__":
    main()
