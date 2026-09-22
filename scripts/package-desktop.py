#!/usr/bin/env python3
"""Build a macOS HiRoute installation candidate from a clean committed checkout."""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import tarfile
import tempfile
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True

REPO = Path(__file__).resolve().parent.parent
NATIVE = REPO / "apps/desktop/src-tauri"
MINIMUM_MACOS = "15.0"
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}
CPA_SOURCE = json.loads((REPO / "vendor/cpa/source.json").read_text())
CPA_VERSION = CPA_SOURCE["version"]
CPA_COMMIT = CPA_SOURCE["commit"]
BUILD_LOG = None
BUILD_TIMINGS = []

BINARIES = ("hiroute-desktop", "hirouted", "hiroute", "cliproxyapi")


def run(*args, cwd=REPO, env=None, timeout=None):
    print(f"Running {Path(args[0]).name} {' '.join(args[1:3])}", file=sys.stderr, flush=True)
    started = time.monotonic()
    result = subprocess.run(args, cwd=cwd, env=env, text=True, timeout=timeout,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    BUILD_TIMINGS.append({"command": list(args[:3]), "seconds": round(time.monotonic() - started, 3),
                          "exit_code": result.returncode})
    if BUILD_LOG is not None:
        with BUILD_LOG.open("a") as log:
            log.write(f"$ {Path(args[0]).name} (exit {result.returncode})\n{result.stdout}{result.stderr}\n")
    result.check_returncode()
    return result.stdout.strip()


def ensure_sccache_server():
    try:
        run("sccache", "--start-server")
    except subprocess.CalledProcessError:
        # A previous build may already own the server socket. Only proceed if
        # the cache is actually reachable before acquiring the shared lock.
        run("sccache", "--show-stats")


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, REPO / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def acquire_cpa(architecture, source_repo, output, macos, resources):
    upstream = module("build_cpa", "build-cpa.py").build(
        source_repo, TARGETS[architecture], macos / "cliproxyapi")
    license_path = resources / "Licenses/CLIProxyAPI-LICENSE"
    license_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(macos / "cliproxyapi.LICENSE", license_path)
    shutil.move(macos / "cliproxyapi.provenance.json", output / "cpa-source-provenance.json")
    return upstream


def sign(path, identity):
    args = ["/usr/bin/codesign", "--force", "--sign", identity]
    if identity != "-":
        args += ["--options", "runtime", "--timestamp"]
    run(*args, str(path))
    run("/usr/bin/codesign", "--verify", "--strict", str(path))


def signature(path):
    result = subprocess.run(["/usr/bin/codesign", "-dv", "--verbose=4", str(path)],
                            check=True, capture_output=True, text=True)
    return result.stdout + result.stderr


def version_tuple(value):
    parts = tuple(map(int, value.split(".")))
    return parts + (0,) * (3 - len(parts))


def inspect_binary(path, architecture="arm64"):
    if architecture not in TARGETS:
        raise ValueError("unsupported macOS architecture")
    if path.is_symlink() or not path.is_file() or path.stat().st_mode & 0o022:
        raise ValueError(f"untrusted binary: {path.name}")
    if not path.stat().st_mode & 0o111:
        raise ValueError(f"not executable: {path.name}")
    if run("/usr/bin/lipo", "-archs", str(path)) != architecture:
        raise ValueError(f"binary architecture differs from {architecture}: {path.name}")
    dependencies = [line.strip().split(" (", 1)[0] for line in
                    run("/usr/bin/otool", "-L", str(path)).splitlines()[1:] if line.strip()]
    if not dependencies or any(not dep.startswith(("/usr/lib/", "/System/Library/")) for dep in dependencies):
        raise ValueError(f"non-system dynamic dependency: {path.name}")
    load_commands = run("/usr/bin/otool", "-l", str(path)).splitlines()
    minimums = [line.split()[1] for line in load_commands if line.strip().startswith("minos ")]
    if not minimums:
        minimums = [load_commands[index + 1].split()[1] for index, line in enumerate(load_commands[:-1])
                    if line.strip().startswith("cmdsize ") and index > 0
                    and "LC_VERSION_MIN_MACOSX" in load_commands[index - 1]]
    if len(minimums) != 1 or version_tuple(minimums[0]) > version_tuple(MINIMUM_MACOS):
        raise ValueError(f"binary requires macOS newer than {MINIMUM_MACOS}: {path.name}")
    return {"sha256": digest(path), "size": path.stat().st_size,
            "architecture": architecture, "minimum_macos": minimums[0], "dynamic_dependencies": dependencies}


def component_identity(path, architecture):
    measured = inspect_binary(path, architecture)
    if path.name == "hiroute-desktop":
        # App signing seals the resource inventory into this executable. Its own
        # signed digest cannot be embedded in that inventory (a circular hash).
        del measured["sha256"]
        del measured["size"]
    return measured


def inventory_files(app):
    return [path for path in sorted(app.rglob("*")) if path.is_file()
            and "_CodeSignature" not in path.parts
            and path != app / "Contents/MacOS/hiroute-desktop"
            and path != app / "Contents/Resources/installation.json"]


def verify_contents(app, manifest):
    """Check the closed component set and final signed bytes, including resources."""
    actual = {str(path.relative_to(app)) for path in inventory_files(app)}
    expected = set(manifest["files"])
    if actual != expected:
        raise ValueError("bundle file inventory mismatch")
    for relative, expected_digest in manifest["files"].items():
        path = app / relative
        if path.is_symlink() or digest(path) != expected_digest:
            raise ValueError(f"bundle digest mismatch: {relative}")
    cpa = app / "Contents/MacOS/cliproxyapi"
    artifact = manifest["cpa"]["artifacts"][0]
    if digest(cpa) != artifact["sha256"] or cpa.stat().st_size != artifact["size"]:
        raise ValueError("final CPA bytes differ from compiled manifest")


def verify(app):
    manifest = json.loads((app / "Contents/Resources/installation.json").read_text())
    verify_contents(app, manifest)
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    if info["CFBundleIdentifier"] != "ai.hiroute.desktop" or info["CFBundleShortVersionString"] != manifest["version"]:
        raise ValueError("bundle identity mismatch")
    if info["LSMinimumSystemVersion"] != MINIMUM_MACOS or TARGETS.get(manifest["architecture"]) != manifest["target"]:
        raise ValueError("bundle platform mismatch")
    for name in BINARIES:
        run("/usr/bin/codesign", "--verify", "--strict", str(app / "Contents/MacOS" / name))
        measured = component_identity(app / "Contents/MacOS" / name, manifest["architecture"])
        if measured != manifest["binaries"][name]:
            raise ValueError(f"binary identity mismatch: {name}")
    run("/usr/bin/codesign", "--verify", "--strict", str(app))
    if manifest["distribution"] == "developer-id":
        run("/usr/bin/xcrun", "stapler", "validate", str(app))
        run("/usr/sbin/spctl", "--assess", "--type", "execute", str(app))
    return manifest


def verify_dmg(path):
    """Verify actual mounted bytes, always detaching our own read-only mount."""
    run("/usr/bin/hdiutil", "verify", str(path))
    with tempfile.TemporaryDirectory(prefix="hiroute-dmg-check-") as directory:
        mount = Path(directory) / "volume"
        run("/usr/bin/hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint",
            str(mount), str(path))
        try:
            visible = {item.name for item in mount.iterdir() if not item.name.startswith(".")}
            if visible != {"HiRoute.app", "Applications"}:
                raise ValueError("DMG installation entries mismatch")
            link = mount / "Applications"
            if not link.is_symlink() or os.readlink(link) != "/Applications":
                raise ValueError("DMG Applications link mismatch")
            app = mount / "HiRoute.app"
            if app.is_symlink() or not app.is_dir():
                raise ValueError("DMG App must be a directory")
            manifest = verify(app)
        finally:
            run("/usr/bin/hdiutil", "detach", str(mount))
    return {"dmg_sha256": digest(path), "revision": manifest["revision"],
            "version": manifest["version"], "architecture": manifest["architecture"],
            "distribution": manifest["distribution"], "integrity": "green",
            "mounted_components": "green", "detach": "green"}


def create_dmg(app, destination):
    with tempfile.TemporaryDirectory(prefix="dmg-stage-", dir=destination.parent) as directory:
        stage = Path(directory)
        run("/usr/bin/ditto", str(app), str(stage / "HiRoute.app"))
        (stage / "Applications").symlink_to("/Applications")
        run("/usr/bin/hdiutil", "create", "-volname", "HiRoute", "-srcfolder", str(stage),
            "-fs", "HFS+", "-format", "UDZO", str(destination))


def candidate_output(revision, architecture):
    parent = REPO / "target/desktop-package" / revision / architecture
    parent.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent))


def artifact_name(version, revision, architecture, trial):
    return f"HiRoute-{version}-{revision[:12]}-macos-{architecture}-{'trial' if trial else 'developer-id'}"


def sign_and_probe_cpa(path, identity):
    # Recent macOS releases reject Go's linker-only ad-hoc signature through
    # AMFI. Apply the requested explicit signature before executing the local
    # version probe. Public distribution eligibility still depends on the
    # complete app and DMG notarization below.
    sign(path, identity)
    with tempfile.TemporaryDirectory(prefix="hiroute-release-probe-") as home:
        return run(str(path), "--help", cwd=home,
                   env={"HOME": home, "PATH": "/usr/bin:/bin", "TMPDIR": home}, timeout=60)


def build(args):
    global BUILD_LOG
    if sys.platform != "darwin":
        raise ValueError("build requires macOS")
    architecture = args.arch or run("/usr/bin/uname", "-m")
    target = TARGETS[architecture]
    if run("git", "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("build requires a clean committed candidate")
    if any(key in os.environ for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR")):
        raise ValueError("external Cargo targets are forbidden")
    if args.identity != "-" and not args.notary_profile:
        raise ValueError("Developer ID distribution requires a notary profile")
    revision = run("git", "rev-parse", "HEAD")
    output = candidate_output(revision, architecture)
    BUILD_LOG = output / "build.log"
    app = output / "HiRoute.app"
    macos = app / "Contents/MacOS"
    resources = app / "Contents/Resources"
    macos.mkdir(parents=True)
    resources.mkdir()
    config = json.loads((NATIVE / "tauri.conf.json").read_text())
    info = {"CFBundleIdentifier": config["identifier"], "CFBundleName": "HiRoute",
            "CFBundleDisplayName": "HiRoute", "CFBundleExecutable": "hiroute-desktop",
            "CFBundlePackageType": "APPL", "CFBundleIconFile": "HiRoute.icns",
            "CFBundleShortVersionString": config["version"], "CFBundleVersion": config["version"],
            "LSMinimumSystemVersion": MINIMUM_MACOS, "NSHighResolutionCapable": True}
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info, sort_keys=True))
    upstream = acquire_cpa(architecture, args.cpa_source_repo, output, macos, resources)
    text = sign_and_probe_cpa(macos / "cliproxyapi", args.identity)
    match = re.search(r"CLIProxyAPI Version: ([^,]+), Commit: ([0-9a-f]{7,40}), BuiltAt: ([^\r\n]+)", text)
    if not match or match[1] != CPA_VERSION or not CPA_COMMIT.startswith(match[2]):
        raise ValueError("CPA release version/commit mismatch")
    measured = inspect_binary(macos / "cliproxyapi", architecture)
    text = signature(macos / "cliproxyapi")
    teams = [line.split("=", 1)[1] for line in text.splitlines() if line.startswith("TeamIdentifier=")]
    if args.identity != "-" and (len(teams) != 1 or teams[0] == "not set"):
        raise ValueError("Developer ID team missing")
    artifact = {"target": target, "os": "macos", "arch": target.split("-")[0],
                "binary_name": "cliproxyapi", "version": CPA_VERSION, "commit": CPA_COMMIT,
                "built_at": match[3], "sha256": measured["sha256"], "size": measured["size"],
                "file_description": run("/usr/bin/file", "-b", str(macos / "cliproxyapi")),
                "dynamic_dependencies": measured["dynamic_dependencies"],
                "signature": {"kind": "adhoc" if args.identity == "-" else "developer-id",
                              "team_identifier": None if args.identity == "-" else teams[0]}}
    manifest = {"schema": "hiroute.desktop.cpa-artifacts/v1", "development_only": False,
                "artifacts": [artifact]}
    manifest_path = output / "cpa-artifacts.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    environment = os.environ.copy()
    environment["RUSTC_WRAPPER"] = "sccache"
    environment["CARGO_INCREMENTAL"] = "0"
    environment["MACOSX_DEPLOYMENT_TARGET"] = MINIMUM_MACOS
    environment["HIROUTE_CPA_MANIFEST"] = str(manifest_path)
    environment["TAURI_CONFIG"] = json.dumps({"build": {"frontendDist": str(REPO / "apps/desktop/dist")}})
    # Frontend is embedded by Tauri; no toolchain is needed on the installation machine.
    ui = REPO / "apps/desktop"
    run("npm", "ci", cwd=ui)
    run("npm", "run", "build", cwd=ui)
    run("cargo", "build", "--locked", "--release", "--target", target, "-p", "hiroute-desktop", "-p", "hiroute-daemon",
        "-p", "hiroute-cli", "--features", "hiroute-desktop/desktop-runtime", "--bin", "hiroute-desktop",
        "--bin", "hirouted", "--bin", "hiroute", env=environment)
    notices = output / "third-party-notices"
    collector = module("third_party_licenses", "collect-third-party-licenses.py")
    collector.collect(argparse.Namespace(
        output=notices,
        cpa_source_repo=args.cpa_source_repo,
        cargo_target=target,
        npm_root=ui,
    ))
    for name in BINARIES[:-1]:
        shutil.copy2(REPO / "target" / target / "release" / name, macos / name)
        if name != "hiroute-desktop":
            sign(macos / name, args.identity)
    # Convert the existing icon instead of introducing a second visual identity.
    iconset = output / "HiRoute.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            filename = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
            run("/usr/bin/sips", "-z", str(size * scale), str(size * scale),
                str(NATIVE / "icons/icon.png"), "--out", str(iconset / filename))
    run("/usr/bin/iconutil", "-c", "icns", str(iconset), "-o", str(resources / "HiRoute.icns"))
    shutil.copy2(REPO / "docs/macos-installation.md", resources / "INSTALLATION.md")
    shutil.copy2(NATIVE / "DISTRIBUTION-NOTICE.md", resources / "DISTRIBUTION-NOTICE.md")
    shutil.copy2(REPO / "LICENSE", resources / "Licenses/HiRoute-LICENSE")
    shutil.copytree(notices, resources / "Licenses", dirs_exist_ok=True)
    # Inventory resolved dependencies for attribution review; it is not a substitute for licenses.
    metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1"))
    packages = [{"name": item["name"], "version": item["version"], "license": item["license"]}
                for item in metadata["packages"]]
    npm = json.loads((ui / "package-lock.json").read_text())
    packages += [{"name": name, "version": item.get("version"), "license": item.get("license")}
                 for name, item in npm["packages"].items() if not item.get("dev")]
    (resources / "dependency-inventory.json").write_text(json.dumps(packages, indent=2) + "\n")
    result = {"version": config["version"], "revision": revision, "target": target, "architecture": architecture,
              "distribution": "controlled-trial" if args.identity == "-" else "developer-id",
              "notices_supplied": True, "cpa_release": upstream, "cpa": manifest,
              "binaries": {name: component_identity(macos / name, architecture) for name in BINARIES},
              "files": {str(path.relative_to(app)): digest(path) for path in inventory_files(app)}}
    (resources / "installation.json").write_text(json.dumps(result, indent=2) + "\n")
    # Never --deep sign: CPA's final byte identity is already compiled into Desktop.
    sign(app, args.identity)
    verify_contents(app, result)
    name = artifact_name(config["version"], revision, architecture, args.identity == "-")
    archive = output / f"{name}.zip"
    run("/usr/bin/ditto", "-c", "-k", "--keepParent", str(app), str(archive))
    if args.identity != "-":
        receipt = run("/usr/bin/xcrun", "notarytool", "submit", str(archive), "--keychain-profile",
                      args.notary_profile, "--wait", "--output-format", "json")
        (output / "notarization.json").write_text(receipt + "\n")
        if json.loads(receipt).get("status") != "Accepted":
            raise ValueError("notarization not accepted")
        run("/usr/bin/xcrun", "stapler", "staple", str(app))
        archive.unlink()
        run("/usr/bin/ditto", "-c", "-k", "--keepParent", str(app), str(archive))
    verify(app)
    dmg = output / f"{name}.dmg"
    create_dmg(app, dmg)
    if args.identity != "-":
        sign(dmg, args.identity)
        receipt = run("/usr/bin/xcrun", "notarytool", "submit", str(dmg), "--keychain-profile",
                      args.notary_profile, "--wait", "--output-format", "json")
        (output / "dmg-notarization.json").write_text(receipt + "\n")
        if json.loads(receipt).get("status") != "Accepted":
            raise ValueError("DMG notarization not accepted")
        run("/usr/bin/xcrun", "stapler", "staple", str(dmg))
        run("/usr/bin/xcrun", "stapler", "validate", str(dmg))
    checked = verify_dmg(dmg)
    if any(checked[key] != result[key] for key in ("revision", "version", "architecture", "distribution")):
        raise ValueError("DMG candidate identity mismatch")
    return {"revision": revision, "version": config["version"], "architecture": architecture,
            "dmg": str(dmg), "dmg_sha256": checked["dmg_sha256"], "dmg_checks": checked,
            "app": str(app), "archive": str(archive),
            "archive_sha256": digest(archive), "distribution": result["distribution"],
            "binary_sha256": {name: digest(macos / name) for name in BINARIES},
            "timings": BUILD_TIMINGS, "installation_scenarios": "not_executed"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("build")
    create.add_argument("--arch", choices=tuple(TARGETS), help="defaults to host architecture")
    create.add_argument("--cpa-source-repo", type=Path, required=True, help="checkout containing the exact pinned CPA source commit")
    create.add_argument("--identity", default="-", help="Developer ID identity; '-' is controlled trial only")
    create.add_argument("--notary-profile")
    check = commands.add_parser("verify")
    check.add_argument("app", type=Path)
    image_check = commands.add_parser("verify-dmg")
    image_check.add_argument("dmg", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "build":
            # Share the existing host lock with all managed Desktop builds and cleanup.
            # Start the cache server before children inherit the shared lock FDs.
            ensure_sccache_server()
            local = module("desktop_package_local_rust", "local-rust.py")
            with local.Store().locked(REPO):
                result = build(args)
                (Path(result["app"]).parent / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        elif args.command == "verify-dmg":
            result = verify_dmg(args.dmg.resolve())
        else:
            result = verify(args.app)
        print(json.dumps(result, indent=2))
        return 0
    except subprocess.CalledProcessError as error:
        print(f"{Path(error.cmd[0]).name} exited {error.returncode}: {error.stderr or ''}", file=sys.stderr)
        return 1
    except (OSError, ValueError, tarfile.TarError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
