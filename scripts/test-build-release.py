#!/usr/bin/env python3
"""Release preparation regressions using real package and publication consumers."""
from contextlib import nullcontext, redirect_stdout
import importlib.util
import io
import hashlib
import json
import os
import re
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("build_release", REPO / "scripts/build-release.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)
MODULES = {name: builder.module(name) for name in ("package-desktop", "package-standalone", "package-windows-shell", "build-cpa", "local-rust", "linux-release-abi", "windows-native-runtime-audit")}
PE_FIXTURES = builder.module("test-windows-native-runtime-audit")


def elf(path, machine):
    path.parent.mkdir(parents=True, exist_ok=True)
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", header, 16, 2)
    struct.pack_into("<H", header, 18, machine)
    path.write_bytes(header)
    path.chmod(0o755)


def pe(path, machine, delayed=()):
    path.parent.mkdir(parents=True, exist_ok=True)
    header = bytearray(PE_FIXTURES.image(delayed=delayed))
    struct.pack_into("<H", header, 0x84, machine)
    path.write_bytes(header)
    path.chmod(0o755)


class CandidateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.source = self.root / "cpa-source"
        self.source.mkdir()
        self.addCleanup(patch.stopall)
        patch.object(builder, "REPO", self.root).start()
        patch.object(builder, "module", side_effect=MODULES.__getitem__).start()
        self.target = "aarch64-unknown-linux-gnu"
        patch.object(builder, "host_target", side_effect=lambda: self.target).start()
        with patch.object(builder.subprocess, "check_output", return_value="a" * 40):
            self.candidate = builder.Candidate(self.target, self.source)
        self.version = json.loads((REPO / "apps/desktop/src-tauri/tauri.conf.json").read_text())["version"]
        config = self.root / "apps/desktop/src-tauri/tauri.conf.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps({"version": self.version}))
        for relative in ("LICENSE", "docs/standalone-cli.md", "docs/standalone-windows.md", "assets/skills/hiroute-management/SKILL.md", "scripts/windows-release-toolchain.cmake"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((REPO / relative).read_bytes())
            path.chmod(0o644)
        self.commands = []
        patch.object(builder.platform, "libc_ver", return_value=("glibc", "2.31")).start()
        patch.dict(os.environ, {"HIROUTE_LINUX_BASELINE_IMAGE": MODULES["linux-release-abi"].baseline()["build_image"]}).start()

    def command(self, *args, **kwargs):
        self.commands.append(args)
        if args[:2] == ("git", "status"):
            return ""
        if args[:2] == ("cargo", "metadata"):
            return json.dumps({"target_directory": str(self.root / "target")})
        if args[0] in ("cargo", "rustc", "node", "npm", "npm.cmd") and args[1] == "--version":
            return "fixture toolchain"
        if args[0] in ("npm", "npm.cmd"):
            if args[1:3] == ("run", "build"):
                assets = self.root / "apps/desktop/dist"
                (assets / "assets").mkdir(parents=True)
                (assets / "index.html").write_text('<script src="/assets/app.js"></script>')
                (assets / "assets/app.js").write_text('console.info("fixture")')
                for path in assets.rglob("*"):
                    if path.is_file():
                        path.chmod(0o644)
            return ""
        if args[:2] == ("cargo", "build"):
            self.assertIn("--locked", args)
            self.assertEqual(args[args.index("--target") + 1], self.target)
            for name in ("hiroute", "hirouted"):
                elf(self.root / "target" / self.target / "release" / name, 183)
            return ""
        if args[-1] == "--help":
            pin = MODULES["build-cpa"].pinned_source()[0]
            return f'CLIProxyAPI Version: {pin["version"]}, Commit: {pin["commit"]}, BuiltAt: {pin["built_at"]}'
        if args[0] == "readelf":
            return "fixture ELF dependencies"
        self.fail(f"unexpected command: {args}")

    def script(self, name, *args):
        if name == "build-cpa":
            output = Path(args[args.index("--output") + 1])
            elf(output, 183)
            output.with_suffix(".LICENSE").write_text("fixture CPA license")
            output.with_suffix(".LICENSE").chmod(0o644)
            return json.dumps({"target": self.target})
        if name == "collect-third-party-licenses":
            notices = Path(args[args.index("--output") + 1])
            notices.mkdir()
            (notices / "THIRD-PARTY-LICENSES.txt").write_text("fixture notices")
            (notices / "third-party-licenses.json").write_text(json.dumps({
                "schema": "hiroute.third-party-licenses/v1", "inputs": {},
                "packages": [{"name": "fixture"}], "documents": [{"name": "fixture"}],
            }))
            for path in notices.iterdir():
                path.chmod(0o644)
            return "{}"
        if name == "package-standalone":
            packager = MODULES["package-standalone"]
            output = io.StringIO()
            with redirect_stdout(output), patch.object(packager, "REPO", self.root):
                packager.build(packager.parser().parse_args(list(map(str, args))))
            return output.getvalue()
        self.fail(f"unexpected script: {name}")

    def test_arm64_candidate_is_consumed_by_existing_release_verifier(self):
        with patch.object(self.candidate, "run", side_effect=self.command), \
                patch.object(self.candidate, "script", side_effect=self.script), \
                patch.object(MODULES["package-desktop"], "ensure_sccache_server"), \
                patch.object(MODULES["local-rust"], "Store") as store:
            store.return_value.locked.return_value = nullcontext()
            result = self.candidate.build()
        self.assertEqual(result["status"], "awaiting_runtime")
        self.assertEqual(result["toolchain"]["node"], "fixture toolchain")
        self.assertEqual(result["toolchain"]["npm"], "fixture toolchain")
        self.assertEqual(set(result["linux_abi"]["files"]), {"bin/hiroute", "bin/hirouted", "libexec/cliproxyapi"})
        self.assertEqual(result["website_artifact"]["architecture"], "aarch64")
        self.assertEqual(sum(args[:3] == ("npm", "run", "build") for args in self.commands), 1)
        manifest = next(Path(result["assets"]).glob("*.json"))
        self.assertIn("console/index.html", json.loads(manifest.read_text())["files"])
        artifacts = list(Path(result["assets"]).iterdir())
        self.assertEqual(len(artifacts), 2)
        catalog = self.root / "releases.json"
        catalog.write_text(json.dumps({"schema": "hiroute.website.releases/v2", "releases": [{
            "version": self.version, "channel": "stable", "published_at": "2026-10-08T00:00:00Z",
            "notes": {"zh": "测试", "en": "Fixture"}, "artifacts": [result["website_artifact"]],
        }]}))
        command = ["node", str(REPO / "apps/website/scripts/release-manifest.mjs"), "verify-assets",
                   str(catalog), "--tag", f"v{self.version}", "--revision", "a" * 40,
                   "--asset-dir", result["assets"]]
        verified = subprocess.run(command, text=True, capture_output=True)
        self.assertEqual(verified.returncode, 0, verified.stderr)
        self.assertIn("2 release asset(s) verified", verified.stdout)
        archive = next(path for path in artifacts if path.name.endswith(".tar.gz"))
        archive.write_bytes(b"tampered")
        self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)

    def test_missing_frontend_tools_fail_before_frontend_cpa_or_rust_build(self):
        for missing in ("node", "npm"):
            self.commands.clear()

            def command(*args, **kwargs):
                if args[0] == missing:
                    self.commands.append(args)
                    raise FileNotFoundError(2, "No such file or directory", missing)
                return self.command(*args, **kwargs)

            with self.subTest(missing=missing), \
                    patch.object(self.candidate, "run", side_effect=command), \
                    patch.object(self.candidate, "script") as script, \
                    patch.object(MODULES["package-desktop"], "ensure_sccache_server") as sccache, \
                    self.assertRaisesRegex(ValueError, "Node.js and npm on PATH; missing: " + missing):
                self.candidate.build()
            script.assert_not_called()
            sccache.assert_not_called()
            self.assertFalse(any(args[0] == "cargo" for args in self.commands))
            self.assertFalse(any(args[:2] == ("npm", "ci") for args in self.commands))
            result = json.loads((self.candidate.output / "result.json").read_text())
            self.assertEqual(result["status"], "failed")
            self.assertIn("missing: " + missing, result["error"])

    def test_archive_with_new_glibc_requirement_cannot_produce_accepted_assets(self):
        def newer(*args, **kwargs):
            if args[0] == "readelf":
                return "Version needs section '.gnu.version_r':\n Name: GLIBC_2.34 Flags: none Version: 2"
            return self.command(*args, **kwargs)
        with patch.object(self.candidate, "run", side_effect=newer), \
                patch.object(self.candidate, "script", side_effect=self.script), \
                patch.object(MODULES["package-desktop"], "ensure_sccache_server"), \
                patch.object(MODULES["local-rust"], "Store") as store:
            store.return_value.locked.return_value = nullcontext()
            with self.assertRaisesRegex(ValueError, "archive bin/hiroute.*GLIBC_2.34"):
                self.candidate.build()
        result = json.loads((self.candidate.output / "result.json").read_text())
        self.assertEqual(result["status"], "failed")
        self.assertNotIn("website_artifact", result)
        self.assertFalse((self.candidate.output / "assets").exists())

    def build_windows_candidate(self, daemon_delay=()):
        self.target = self.candidate.target = "x86_64-pc-windows-msvc"

        def command(*args, **kwargs):
            if args[:2] == ("cargo", "build"):
                self.assertIn("--locked", args)
                self.assertIn("--release", args)
                self.assertEqual(args[args.index("--target") + 1], self.target)
                self.assertEqual(kwargs["env"]["CARGO_ENCODED_RUSTFLAGS"], "-C\x1ftarget-feature=+crt-static")
                self.assertEqual(kwargs["env"]["CMAKE_TOOLCHAIN_FILE"],
                                 (self.root / "scripts/windows-release-toolchain.cmake").as_posix())
                for name in ("hiroute.exe", "hirouted.exe"):
                    pe(self.root / "target" / self.target / "release" / name, 0x8664,
                       daemon_delay if name == "hirouted.exe" else ())
                return ""
            if args[:2] == ("cmake", "--version"):
                return "cmake fixture native toolchain"
            if args[-1] == "--help":
                self.assertEqual(kwargs["env"], {"SYSTEMROOT": "fixture-native-root"})
            return self.command(*args, **kwargs)

        def script(name, *args):
            if name == "build-cpa":
                output = Path(args[args.index("--output") + 1])
                self.assertEqual(output.name, "cliproxyapi.exe")
                pe(output, 0x8664)
                output.with_suffix(".LICENSE").write_text("fixture CPA license")
                output.with_suffix(".LICENSE").chmod(0o644)
                return json.dumps({"target": self.target})
            return self.script(name, *args)

        with patch.object(self.candidate, "run", side_effect=command), \
                patch.object(self.candidate, "script", side_effect=script), \
                patch.object(builder, "windows_probe_environment", return_value={"SYSTEMROOT": "fixture-native-root"}), \
                patch.object(builder, "windows_build_lock", return_value=nullcontext()), \
                patch.object(MODULES["package-desktop"], "ensure_sccache_server"):
            return self.candidate.build()

    def test_windows_candidate_is_verified_as_native_pe_and_consumed_by_release_verifier(self):
        result = self.build_windows_candidate()
        self.assertEqual(result["status"], "awaiting_runtime")
        self.assertIn(("npm.cmd", "--version"), self.commands)
        self.assertEqual(result["toolchain"]["npm"], "fixture toolchain")
        self.assertNotIn("website_artifact", result)
        self.assertNotIn("assets", result)
        self.assertEqual(result["publication"], "internal-only")
        self.assertEqual(result["internal_payload"]["target"], self.target)
        self.assertEqual(len(list(Path(result["internal_payloads"]).iterdir())), 2)
        self.assertNotIn("linux_abi", result)
        crt = result["windows_crt_build"]
        self.assertEqual(crt["dependency_gate"], "green")
        self.assertEqual({image["path"] for image in crt["images"]},
                         {"bin/hiroute.exe", "bin/hirouted.exe", "libexec/cliproxyapi.exe"})

    def test_release_vc_dependency_in_delayed_daemon_import_cannot_publish_assets(self):
        with self.assertRaisesRegex(ValueError, "unexpected CRT dependency"):
            self.build_windows_candidate(daemon_delay=("VCRUNTIME140_1.dll",))
        result = json.loads((self.candidate.output / "result.json").read_text())
        self.assertEqual(result["status"], "failed")
        self.assertNotIn("website_artifact", result)
        self.assertFalse((self.candidate.output / "assets").exists())

    def test_windows_shell_stage_uses_verified_package_bytes_and_preserves_foreign_files(self):
        result = self.build_windows_candidate()
        assets = Path(result["internal_payloads"])
        archive = next(assets.glob("*.tar.gz"))
        manifest = next(assets.glob("*.json"))
        destination = self.root / "shell-resources/standalone"
        packager = MODULES["package-windows-shell"]
        value = packager.stage(manifest, archive, destination)
        self.assertEqual(value["revision"], "a" * 40)
        self.assertTrue((destination / "hiroute.exe").read_bytes().startswith(b"MZ"))
        self.assertEqual((destination / "package-manifest.json").read_bytes(), manifest.read_bytes())
        self.assertTrue((destination / "console/index.html").is_file())
        packager.stage(manifest, archive, destination)
        foreign = destination / "personal.txt"
        foreign.write_text("preserve me")
        with self.assertRaisesRegex(ValueError, "foreign"):
            packager.stage(manifest, archive, destination)
        self.assertEqual(foreign.read_text(), "preserve me")
        (destination / "hiroute.exe").write_bytes(b"changed owned entry")
        with self.assertRaisesRegex(ValueError, "owned standalone stage entry changed"):
            packager.stage(manifest, archive, destination)
        self.assertEqual((destination / "hiroute.exe").read_bytes(), b"changed owned entry")

    def test_windows_shell_identity_measures_actual_nsis_member_and_crt_imports(self):
        result = self.build_windows_candidate()
        payloads = Path(result["internal_payloads"])
        files = [next(payloads.glob("*.tar.gz")), next(payloads.glob("*.json"))]
        executable = self.root / "target" / self.target / "release/hiroute-shell.exe"
        delayed = []

        def command(*args, **kwargs):
            if "--no-bundle" in args:
                pe(executable, 0x8664)
            if "bundle" in args:
                pe(executable, 0x8664)
                executable.write_bytes(executable.read_bytes() + b"workspace PE resources")
                installer = executable.parent / "bundle/nsis" / f"HiRoute_{self.version}_x64-setup.exe"
                installer.parent.mkdir(parents=True, exist_ok=True)
                installer.write_bytes(b"fixture NSIS package")
            if args[0] == "7z-fixture":
                directory = Path(next(a[2:] for a in args if a.startswith("-o")))
                pe(directory / "hiroute-shell.exe", 0x8664, delayed)
                image = directory / "hiroute-shell.exe"
                image.write_bytes(image.read_bytes() + b"actual NSIS member")
            return ""

        with patch.object(self.candidate, "standalone", return_value=(None, files)), \
                patch.object(self.candidate, "run", side_effect=command), \
                patch.object(MODULES["package-windows-shell"].shutil, "which", return_value="7z-fixture"):
            self.candidate.desktop_windows()
            identity = self.candidate.result["desktop_result"]
            self.assertEqual(identity["shell_image"]["sha256"], hashlib.sha256(PE_FIXTURES.image() + b"actual NSIS member").hexdigest())
            self.assertNotEqual(identity["shell_image"]["sha256"], MODULES["package-windows-shell"].digest(executable))
            delayed.append("VCRUNTIME140_1.dll")
            with self.assertRaisesRegex(ValueError, "CRT dependency"):
                self.candidate.desktop_windows()

    def test_nsis_shell_audit_rejects_missing_tool_or_ambiguous_members(self):
        packager = MODULES["package-windows-shell"]
        installer = self.root / "fixture.exe"
        installer.write_bytes(b"fixture NSIS package")
        with patch.object(packager.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "7-Zip is required"):
                packager.shell_image(installer)
        for count in (0, 2):
            def extract(*args):
                directory = Path(next(a[2:] for a in args if a.startswith("-o")))
                for i in range(count):
                    pe(directory / str(i) / "hiroute-shell.exe", 0x8664)
            with patch.object(packager.shutil, "which", return_value="7z-fixture"):
                with self.assertRaisesRegex(ValueError, "exactly one shell image"):
                    packager.shell_image(installer, extract)

    def test_desktop_publication_rejects_partial_or_misbound_native_attestation(self):
        # Verifier-shape regression only; fabricated JSON is never installation evidence.
        packager = MODULES["package-windows-shell"]
        installer = self.root / f"HiRoute-{self.version}-aaaaaaaaaaaa-windows-x86_64-setup.exe"
        installer.write_bytes(b"fixture installer")
        identity = {"schema": packager.SCHEMA, "version": self.version, "revision": "a" * 40,
                    "architecture": "x86_64", "distribution": "unsigned", "integrity": "green",
                    "embedded_components": "green", "native_runtime": "green", "installer_sha256": packager.digest(installer),
                    "installer_size": installer.stat().st_size}
        with self.assertRaisesRegex(ValueError, "Windows 11"):
            packager.verify(installer, identity, "a" * 40)
        required = json.loads((REPO / "scripts/windows-desktop-cases.json").read_text())
        identity["native_evidence"] = {"runs": [{"platform": platform_name, "revision": "a" * 40,
            "installer_sha256": packager.digest(installer), "standard_user": True, "architecture": "AMD64",
            "os_version": "fixture-only", "scenarios": dict.fromkeys(cases, "green")} for platform_name, cases in required.items()]}
        self.assertIs(packager.verify(installer, identity, "a" * 40), identity)
        for key, value in (("revision", "b" * 40), ("standard_user", False), ("architecture", "ARM64")):
            altered = json.loads(json.dumps(identity))
            altered["native_evidence"]["runs"][0][key] = value
            with self.assertRaisesRegex(ValueError, "all standard-user"):
                packager.verify(installer, altered, "a" * 40)
        identity["native_evidence"]["runs"][0]["scenarios"].pop("update.killed-resume")
        with self.assertRaisesRegex(ValueError, "all standard-user"):
            packager.verify(installer, identity, "a" * 40)

    def test_windows_release_rejects_ambient_compiler_overrides_without_affecting_other_contexts(self):
        for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS",
                     "CMAKE_TOOLCHAIN_FILE", "HOST_CMAKE_TOOLCHAIN_FILE", "CXXFLAGS_x86_64_pc_windows_msvc"):
            environment = {name: "caller override", "PATH": "existing path"}
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "compiler overrides"):
                builder.windows_release_environment(builder.STATIC_CRT_TARGET, environment)
            self.assertIs(builder.windows_release_environment("aarch64-pc-windows-msvc", environment), environment)
            self.assertIs(builder.windows_release_environment("x86_64-unknown-linux-gnu", environment), environment)

    def test_elf_architecture_rejects_mislabelled_and_non_elf_binaries(self):
        binary = self.root / "binary"
        for target, machine in ((self.target, 183), ("x86_64-unknown-linux-gnu", 62)):
            elf(binary, machine)
            builder.inspect_elf(binary, target)
            with self.assertRaisesRegex(ValueError, "architecture differs"):
                builder.inspect_elf(binary, "x86_64-unknown-linux-gnu" if machine == 183 else self.target)
        for data in (b"", b"not ELF" * 5, b"\x7fELF\x01\x01" + b"\x00" * 58):
            binary.write_bytes(data)
            with self.assertRaises(ValueError):
                builder.inspect_elf(binary, self.target)

    def test_preflight_rejects_dirty_source_wrong_host_and_external_targets(self):
        with patch.object(self.candidate, "run", side_effect=self.command):
            self.candidate.preflight()
            with patch.object(builder, "host_target", return_value="x86_64-unknown-linux-gnu"):
                with self.assertRaisesRegex(ValueError, "native host"):
                    self.candidate.preflight()
            for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
                with patch.dict(os.environ, {key: "/tmp/external"}), self.assertRaisesRegex(ValueError, "external Cargo"):
                    self.candidate.preflight()
        with patch.object(self.candidate, "run", return_value=" M source.rs"), self.assertRaisesRegex(ValueError, "clean committed"):
            self.candidate.preflight()

    def test_cargo_config_cannot_redirect_the_candidate_target(self):
        def redirected(*args, **kwargs):
            if args[:2] == ("cargo", "metadata"):
                return json.dumps({"target_directory": str(self.root / "external-target")})
            return self.command(*args, **kwargs)
        with patch.object(self.candidate, "run", side_effect=redirected):
            with self.assertRaisesRegex(ValueError, "checkout-local"):
                self.candidate.preflight()

    def test_failed_build_keeps_revision_bound_evidence_without_assets(self):
        with patch.object(self.candidate, "preflight", side_effect=ValueError("fixture failure")):
            with self.assertRaisesRegex(ValueError, "fixture failure"):
                self.candidate.build()
        result = json.loads((self.candidate.output / "result.json").read_text())
        self.assertEqual(result["revision"], "a" * 40)
        self.assertEqual(result["status"], "failed")
        self.assertNotIn("website_artifact", result)
        self.assertFalse((self.candidate.output / "assets").exists())

    def test_intel_desktop_uses_existing_packager_and_preserves_identity(self):
        self.candidate.target = "x86_64-apple-darwin"
        self.candidate.version = self.version
        dmg = self.root / f"HiRoute-{self.version}-aaaaaaaaaaaa-macos-x86_64-trial.dmg"
        dmg.write_bytes(b"fixture mounted and verified DMG")
        packaged = {"revision": "a" * 40, "version": self.version, "architecture": "x86_64",
                    "distribution": "controlled-trial", "dmg": str(dmg),
                    "dmg_sha256": MODULES["package-desktop"].digest(dmg)}
        with patch.object(self.candidate, "script", return_value=json.dumps(packaged)) as run:
            artifact, files = self.candidate.desktop()
        self.assertEqual(files, [dmg])
        self.assertEqual(artifact["architecture"], "x86_64")
        self.assertEqual(artifact["minimum_os"], "15.0")
        self.assertEqual(run.call_args.args, ("package-desktop", "build", "--arch", "x86_64",
                                             "--cpa-source-repo", self.source))
        for field, value in (("revision", "b" * 40), ("architecture", "arm64"), ("version", "99.0.0"),
                             ("dmg_sha256", "0" * 64)):
            with patch.object(self.candidate, "script", return_value=json.dumps({**packaged, field: value})):
                with self.assertRaises(ValueError):
                    self.candidate.desktop()


class NativeCompilerContextTests(unittest.TestCase):
    def run_context(self, overrides=None):
        text = (REPO / 'scripts/windows-headless-ci.ps1').read_text()
        matches = re.findall(r"\$contextScript = @'\n(.*?)\n'@", text, re.S)
        self.assertEqual(len(matches), 1)
        environment = {'PATH': os.environ.get('PATH', ''), 'HIROUTE_FIXTURE_SECRET': 'must-not-be-emitted'}
        environment.update(overrides or {})
        return subprocess.run([sys.executable, '-I', '-c', matches[0]], cwd=REPO,
                              env=environment, capture_output=True, timeout=15)

    def test_native_debug_entry_reuses_canonical_release_linkage_without_environment_leak(self):
        process = self.run_context()
        self.assertEqual(process.returncode, 0, process.stderr)
        value = json.loads(process.stdout)
        self.assertEqual(set(value), {'CARGO_ENCODED_RUSTFLAGS', 'CMAKE_TOOLCHAIN_FILE'})
        self.assertEqual(value['CARGO_ENCODED_RUSTFLAGS'], '-C\x1ftarget-feature=+crt-static')
        self.assertEqual(Path(value['CMAKE_TOOLCHAIN_FILE']), REPO / 'scripts/windows-release-toolchain.cmake')
        self.assertNotIn(b'must-not-be-emitted', process.stdout)

    def test_native_entry_rejects_conflicting_compiler_flags_before_cargo(self):
        process = self.run_context({'RUSTFLAGS': '-C target-feature=-crt-static'})
        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(process.stdout, b'')
        self.assertIn(b'compiler overrides must be absent', process.stderr)


class HostedReleasePolicyTests(unittest.TestCase):
    def test_expensive_windows_assessment_is_release_or_manual_only(self):
        workflow = (REPO / '.github/workflows/windows-headless.yml').read_text()
        triggers = workflow.split('\non:\n', 1)[1].split('\npermissions:', 1)[0]
        events = re.findall(r'^  ([a-z_]+):', triggers, re.M)
        self.assertEqual(set(events), {'workflow_dispatch', 'workflow_call'})
        self.assertNotIn('continue-on-error:', workflow)
        self.assertEqual(workflow.count('ref: ${{ github.sha }}'), 3)
        self.assertEqual(workflow.count(
            "-Phase '${{ inputs.scope != '' && inputs.scope != 'all' && 'focused' || 'final' }}'"), 2)
        self.assertEqual(workflow.count('-ExpectedSha $env:GITHUB_SHA'), 3)

    def test_manual_scope_cannot_narrow_the_reusable_release_gate(self):
        workflow = (REPO / '.github/workflows/windows-headless.yml').read_text()
        triggers = workflow.split('\non:\n', 1)[1].split('\npermissions:', 1)[0]
        # Scope is a dispatch-only choice. Reusable calls reject undeclared
        # inputs, so callers cannot turn the release gate into a partial run.
        self.assertEqual(triggers.split('  workflow_call:', 1)[1].strip(), '')
        self.assertIn('default: all', triggers)
        self.assertIn('options: [all, components, core, product, background, agents]', triggers)
        matrix = ("scope: ${{ fromJSON(inputs.scope != '' && inputs.scope != 'all' && "
                  "format('[\"{0}\"]', inputs.scope) || "
                  "'[\"components\",\"core\",\"product\",\"background\",\"agents\"]') }}")
        self.assertEqual(workflow.count(matrix), 1)
        self.assertIn("windows11-arm-install:", workflow)
        self.assertIn("-Scope install", workflow)

    def test_windows_replay_requires_successful_producers(self):
        workflow = (REPO / '.github/workflows/windows-headless.yml').read_text()
        replay = workflow.split('\n  windows11:\n', 1)[1]
        self.assertRegex(replay, r'(?m)^    needs: native$')
        # Default job success admission skips replay after failed or cancelled
        # producers; always() remains appropriate only for evidence upload steps.
        self.assertIn("if: inputs.scope != 'background'", replay)
        self.assertNotRegex(replay, r'(?m)^    continue-on-error:')

    def test_only_windows_assets_require_windows_validation(self):
        workflow = (REPO / '.github/workflows/build-release.yml').read_text()
        jobs = dict(re.findall(r'^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)',
                               workflow.split('\njobs:\n', 1)[1], re.M | re.S))
        gate = jobs['windows-validation']
        windows = jobs['build-windows']
        unix = jobs['build-unix']
        self.assertIn('uses: ./.github/workflows/windows-headless.yml', gate)
        self.assertNotRegex(gate, r'(?m)^    (if|continue-on-error):')
        self.assertRegex(windows, r'\A    needs: windows-validation\n')
        self.assertNotRegex(windows, r'(?m)^    (if|continue-on-error):')
        self.assertNotRegex(unix, r'(?m)^    (needs|if|continue-on-error):')
        self.assertEqual(re.findall(r'target: ([a-z0-9_-]+)', windows),
                         ['x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc'])
        self.assertEqual(re.findall(r'target: ([a-z0-9_-]+)', unix),
                         ['aarch64-apple-darwin', 'x86_64-apple-darwin',
                          'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'])
        for job in (unix, windows):
            self.assertIn('uses: ./.github/workflows/build-release-platform.yml', job)
            self.assertIn('target: ${{ matrix.target }}', job)
            self.assertIn('runner: ${{ matrix.runner }}', job)
        build = (REPO / '.github/workflows/build-release-platform.yml').read_text()
        self.assertIn('ref: ${{ github.sha }}', build)
        self.assertIn('--expected-sha "$GITHUB_SHA"', build)
        self.assertIn("if ((git rev-parse HEAD) -ne $env:GITHUB_SHA)", build)
        self.assertNotIn('matrix.', build)

    def test_pr_windows_check_is_bounded_cached_and_fails_closed(self):
        workflow = (REPO / '.github/workflows/windows-compile.yml').read_text()
        triggers = workflow.split('\non:\n', 1)[1].split('\npermissions:', 1)[0]
        self.assertEqual(re.findall(r'^  ([a-z_]+):', triggers, re.M), ['workflow_dispatch', 'pull_request'])
        self.assertIn('github.event.pull_request.number || github.run_id', workflow)
        for path in ('crates/**', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
                     '.cargo/**', 'assets/**', 'vendor/**', 'tools/**', 'scripts/**',
                     '.github/workflows/windows-compile.yml'):
            self.assertIn("- '" + path + "'", triggers)
        self.assertEqual(workflow.count('runs-on:'), 1)
        self.assertNotIn('matrix:', workflow)
        self.assertNotIn('continue-on-error:', workflow)
        self.assertIn('cancel-in-progress: true', workflow)
        self.assertIn('mozilla-actions/sccache-action@', workflow)
        self.assertIn("SCCACHE_GHA_ENABLED: 'true'", workflow)
        self.assertIn('actions/cache@', workflow)
        self.assertIn('cargo check --locked --target x86_64-pc-windows-msvc '
                      '-p hiroute-cli -p hiroute-daemon -p hiroute-platform '
                      '-p hiroute-local-storage -p hiroute-integrations --all-targets', workflow)
        self.assertIn("if ($LASTEXITCODE -ne 0) { throw", workflow)
        component_tests = [line.strip() for line in workflow.splitlines()
                           if line.strip().startswith('cargo test ')]
        self.assertEqual(component_tests, [
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-host-runtime --lib standalone_install::',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-host-runtime --lib storage_recovery::',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-platform --lib windows::shell_registration::tests::'
            'inherited_export_is_published_privately_and_changed_material_is_not_overwritten -- --exact',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-platform --lib private_fs::tests::'
            'storage_preparation_reopens_without_adopting_a_file -- --exact',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-local-storage --lib migrations::startup_recovery::tests -- --test-threads=1',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-local-storage --lib control::capability_registration::tests -- --test-threads=1',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-cli --lib service::recovery_text_tests -- --test-threads=1',
            'cargo test --locked --target x86_64-pc-windows-msvc '
            '-p hiroute-shell --features shell-runtime --lib'])
        self.assertIn("throw 'Windows installation component regressions failed'", workflow)
        self.assertIn('component fixtures, not standard-user installation acceptance', workflow)
        self.assertNotIn('--ignored', workflow)
        self.assertNotIn('--test windows_headless', workflow)
        self.assertNotIn('--test windows_desktop', workflow)
        self.assertNotIn('windows-headless-ci.ps1', workflow)


if __name__ == "__main__":
    unittest.main()
