#!/usr/bin/env python3
"""Build one native release candidate; never publish or modify an installation."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
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
    "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
)
STATIC_CRT_TARGET = "x86_64-pc-windows-msvc"


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / f"{name}.py")
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def host_target():
    arch = {"arm64": "aarch64", "AMD64": "x86_64"}.get(platform.machine(), platform.machine())
    suffix = {"Darwin": "apple-darwin", "Linux": "unknown-linux-gnu",
              "Windows": "pc-windows-msvc"}.get(platform.system())
    return f"{arch}-{suffix}"


def inspect_elf(path, target):
    with path.open("rb") as stream:
        header = stream.read(20)
    machine = {"x86_64-unknown-linux-gnu": 62, "aarch64-unknown-linux-gnu": 183}[target]
    if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
            or struct.unpack_from("<H", header, 18)[0] != machine):
        raise ValueError(f"binary architecture differs from {target}: {path.name}")


@contextmanager
def windows_build_lock():
    import msvcrt
    path = REPO / "target/release-candidate/windows-build.lock"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as handle:
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        try:
            yield
        finally:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)


def windows_probe_environment(directory):
    import ctypes
    buffer = ctypes.create_unicode_buffer(32768)
    size = ctypes.windll.kernel32.GetWindowsDirectoryW(buffer, len(buffer))
    if not 0 < size < len(buffer):
        raise OSError("Cannot resolve native Windows directory")
    root = Path(buffer.value).resolve(strict=True)
    return {"SYSTEMROOT": str(root), "WINDIR": str(root), "PATH": str(root / "System32"),
            "TEMP": directory, "TMP": directory, "USERPROFILE": directory}


def windows_release_environment(target, inherited):
    if target != STATIC_CRT_TARGET:
        return inherited
    conflicts = [name for name, value in inherited.items() if value and (
        name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "HOST_CMAKE_TOOLCHAIN_FILE")
        or "CMAKE_TOOLCHAIN_FILE" in name or name.endswith("_RUSTFLAGS")
        or name.startswith(("CFLAGS", "CXXFLAGS", "ASMFLAGS", "HOST_CFLAGS", "HOST_CXXFLAGS",
                            "HOST_ASMFLAGS", "TARGET_CFLAGS", "TARGET_CXXFLAGS", "TARGET_ASMFLAGS")))]
    if conflicts:
        raise ValueError("Windows Release compiler overrides must be absent: " + ", ".join(sorted(conflicts)))
    toolchain = (REPO / "scripts/windows-release-toolchain.cmake").resolve(strict=True)
    return dict(inherited, CARGO_ENCODED_RUSTFLAGS="-C\x1ftarget-feature=+crt-static",
                CMAKE_TOOLCHAIN_FILE=toolchain.as_posix())


class Candidate:
    def __init__(self, target, source_repo, windows_desktop=False):
        self.target = target
        self.windows_desktop = windows_desktop
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
                                HIROUTE_BUILD_SOURCE_SHA=self.revision, LC_ALL="C")

    def run(self, *arguments, env=None, timeout=None, cwd=None):
        started = time.monotonic()
        print(f"Running {Path(str(arguments[0])).name} {' '.join(map(str, arguments[1:3]))}", file=sys.stderr, flush=True)
        result = subprocess.run(list(map(str, arguments)), cwd=cwd or REPO, env=env or self.environment,
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
        if self.windows_desktop and self.target != STATIC_CRT_TARGET:
            raise ValueError("Windows Desktop supports only x86_64-pc-windows-msvc")
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
        if self.target.endswith("linux-gnu"):
            baseline = module("linux-release-abi").baseline()
            if (tuple(platform.libc_ver()) != ("glibc", baseline["minimum_glibc"])
                    or os.environ.get("HIROUTE_LINUX_BASELINE_IMAGE") != baseline["build_image"]):
                raise ValueError("Linux release must run inside the pinned glibc 2.31 build image")
            self.result["linux_baseline"] = baseline
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
        windows = self.target.endswith("-pc-windows-msvc")
        frontend = REPO / "apps/desktop"
        npm = "npm.cmd" if windows else "npm"
        self.run(npm, "ci", "--prefix", frontend)
        self.run(npm, "run", "build", "--prefix", frontend)
        cpa = self.output / ("cliproxyapi.exe" if windows else "cliproxyapi")
        provenance = json.loads(self.script("build-cpa", "--source-repo", self.source_repo,
                                           "--target", self.target, "--output", cpa))
        pin = module("build-cpa").pinned_source()[0]
        # The helper version probe has no service side effects and no user config.
        with tempfile.TemporaryDirectory(prefix="hiroute-release-probe-") as directory:
            environment = windows_probe_environment(directory) if windows else {
                "HOME": directory, "PATH": "/usr/bin:/bin", "TMPDIR": directory}
            probe = self.run(cpa, "--help", env=environment, timeout=60)
        match = re.search(r"CLIProxyAPI Version: ([^,]+), Commit: ([0-9a-f]{7,40}), BuiltAt: ([^\r\n]+)", probe)
        if not match or match[1] != pin["version"] or not pin["commit"].startswith(match[2]) or match[3] != pin["built_at"]:
            raise ValueError("CPA release version/commit/build date mismatch")
        build_environment = windows_release_environment(self.target, self.environment)
        if self.target == STATIC_CRT_TARGET:
            toolchain = Path(build_environment["CMAKE_TOOLCHAIN_FILE"])
            self.result["windows_crt_build"] = {
                "selection": "static", "target_scope": self.target, "profile": "release",
                "rust_flags": ["-C", "target-feature=+crt-static"],
                "cmake_runtime": "MultiThreaded",
                "toolchain_sha256": hashlib.sha256(toolchain.read_bytes()).hexdigest(),
                "cmake_version": self.run("cmake", "--version").splitlines()[0],
                "generator": self.environment.get("CMAKE_GENERATOR", "cmake-crate-native-MSVC-default"),
                "generator_toolset": self.environment.get("CMAKE_GENERATOR_TOOLSET"),
                "runtime_acceptance": "not_executed"}
        self.run("cargo", "build", "--locked", "--release", "--target", self.target,
                 "-p", "hiroute-cli", "-p", "hiroute-daemon", "--bin", "hiroute", "--bin", "hirouted",
                 env=build_environment)
        binaries = REPO / "target" / self.target / "release"
        suffix = ".exe" if windows else ""
        for binary in (binaries / ("hiroute" + suffix), binaries / ("hirouted" + suffix), cpa):
            if windows:
                module("package-standalone").verify_windows_image(binary, self.target)
            else:
                inspect_elf(binary, self.target)
        notices = self.output / "notices"
        self.script("collect-third-party-licenses", "--cargo-target", self.target,
                    "--cpa-source-repo", self.source_repo,
                    "--npm-root", frontend, "--output", notices)
        packaged = json.loads(self.script(
            "package-standalone", "build", "--version", self.version, "--revision", self.revision,
            "--target", self.target, "--hiroute", binaries / ("hiroute" + suffix), "--hirouted", binaries / ("hirouted" + suffix),
            "--cpa-binary", cpa, "--cpa-version", pin["version"], "--cpa-license", cpa.with_suffix(".LICENSE"),
            "--notices", notices, "--console-assets", frontend / "dist", "--output", self.output))
        archive, manifest = Path(packaged["archive"]), Path(packaged["manifest"])
        packager = module("package-standalone")
        verified = packager.verify(manifest, archive)
        if any(verified[key] != expected for key, expected in (
                ("revision", self.revision), ("version", self.version), ("target", self.target))):
            raise ValueError("standalone package differs from candidate identity")
        if self.target == STATIC_CRT_TARGET:
            inspector = module("windows-native-runtime-audit")
            images = [inspector.image_record(payload, name, "verified-release-package")
                      for name, (payload, _) in packager.safe_members(archive, verified["files"]).items()
                      if name.lower().endswith((".exe", ".dll"))]
            if (len(images) != 3 or any(image["imports"]["debug_crt"] or
                                      image["imports"]["release_vc_runtime"] for image in images)):
                raise ValueError("Complete Windows x64 Release package has an unexpected CRT dependency")
            self.result["windows_crt_build"]["images"] = images
            self.result["windows_crt_build"]["dependency_gate"] = "green"
        if not windows:
            abi = module("linux-release-abi").archive(manifest, archive, packager, self.run)
            (self.output / "linux-abi.json").write_text(json.dumps(abi, indent=2) + "\n")
            self.result["linux_abi"] = abi
        self.result["cpa_source"] = provenance
        artifact = {"kind": "standalone", "platform": "Windows" if windows else "Linux", "architecture": self.target.split("-")[0],
                    "target": self.target, "format": "tar.gz", "distribution": "unsigned",
                    **verified["archive"], "manifest_filename": manifest.name,
                    "manifest_sha256": packager.digest(manifest), "manifest_size": manifest.stat().st_size}
        if windows:
            self.result["internal_payload"] = {"target": self.target, "archive": archive.name, "manifest": manifest.name}
            artifact = None
        return artifact, [archive, manifest]

    def desktop_windows(self):
        _, files = self.standalone()
        archive, manifest = files
        shell = REPO / "apps/shell"
        packager = module("package-windows-shell")
        package = packager.stage(manifest, archive, shell / "src-tauri/resources/standalone")
        environment = windows_release_environment(self.target, self.environment)
        self.run("npm.cmd", "ci", cwd=shell)
        self.run("npm.cmd", "run", "tauri", "--", "build", "--ci", "--target", self.target,
                 "--features", "shell-runtime", "--no-bundle", "--", "--locked", cwd=shell, env=environment)
        self.run("npm.cmd", "run", "tauri", "--", "bundle", "--ci", "--target", self.target,
                 "--features", "shell-runtime", "--bundles", "nsis", "--no-sign", cwd=shell, env=environment)
        # Tauri CLI 2.11.4 uses product_version_arch-setup.exe. Select that
        # exact candidate path; retained installers from other versions are irrelevant.
        produced = REPO / "target" / self.target / "release/bundle/nsis" / f"HiRoute_{self.version}_x64-setup.exe"
        if not produced.is_file() or produced.is_symlink():
            raise ValueError("Windows Desktop bundle did not produce its exact installer")
        name = f"HiRoute-{self.version}-{self.revision[:12]}-windows-x86_64-setup.exe"
        installer = self.output / name
        shutil.copyfile(produced, installer)
        # The bundler's NSIS input can differ from the workspace executable.
        # Measure and audit the immutable artifact that users actually install.
        image = packager.shell_image(installer, self.run)
        identity = {"schema": packager.SCHEMA, "version": self.version, "revision": self.revision,
                    "architecture": "x86_64", "distribution": "unsigned", "installer_sha256": packager.digest(installer),
                    "installer_size": installer.stat().st_size, "integrity": "green", "embedded_components": "awaiting_runtime",
                    "native_runtime": "not_executed", "shell_image": image, "standalone_revision": package["revision"],
                    "standalone_manifest_sha256": packager.digest(manifest)}
        identity_path = self.output / (name + ".identity.json")
        identity_path.write_text(json.dumps(identity, indent=2) + "\n")
        self.result["desktop_result"] = identity
        artifact = {"kind": "desktop", "platform": "Windows", "architecture": "x86_64", "format": "nsis", "distribution": "unsigned",
                    "filename": name, "sha256": identity["installer_sha256"], "size": identity["installer_size"]}
        return artifact, [installer, identity_path]

    def build(self):
        try:
            self.preflight()
            if self.target.endswith("apple-darwin"):
                artifact, files = self.desktop()  # The Desktop packager owns its existing lock.
            else:
                module("package-desktop").ensure_sccache_server()
                lock = windows_build_lock() if self.target.endswith("-pc-windows-msvc") else module("local-rust").Store().locked(REPO)
                with lock:
                    artifact, files = self.desktop_windows() if self.windows_desktop else self.standalone()
            assets = self.output / ("internal-payloads" if artifact is None else "assets")
            assets.mkdir()
            for path in files:
                shutil.copyfile(path, assets / path.name)
            status = "completed" if self.target.endswith("apple-darwin") else "awaiting_runtime"
            self.result.update(status=status, package_integrity="green")
            if artifact is None:
                self.result.update(publication="internal-only", internal_payloads=str(assets))
            else:
                self.result.update(website_artifact=artifact, assets=str(assets))
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
    parser.add_argument("--windows-desktop", action="store_true", help="Build the x64 NSIS Desktop installer instead of the headless archive")
    args = parser.parse_args()
    try:
        if args.target.endswith("linux-gnu") and not os.environ.get("HIROUTE_LINUX_BASELINE_IMAGE"):
            result = module("linux-release-container").run(args.target, args.cpa_source_repo)
        else:
            if args.target.endswith("linux-gnu"):
                Path.home().mkdir(parents=True, exist_ok=True)
            candidate = Candidate(args.target, args.cpa_source_repo, args.windows_desktop)
            result = candidate.build()
            if args.target.endswith("linux-gnu"):
                Path(os.environ["HIROUTE_RELEASE_RESULT"]).write_text(json.dumps({
                    "result_path": str(candidate.output / "result.json")}))
        print(json.dumps(result, indent=2))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
