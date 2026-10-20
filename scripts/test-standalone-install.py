#!/usr/bin/env python3
"""Standalone package/installer ownership and integrity regressions."""

import argparse
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import re
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package = load("package_standalone", "package-standalone.py")
installer = load("install_standalone", "install-standalone.py")


class PreflightTransportTests(unittest.TestCase):
    def test_provisional_helper_rejects_malformed_or_unknown_success_reports(self):
        good = {"schema":"hiroute.upgrade-preflight/v1", "history_cutover_required":False, "required_bytes":0, "available_bytes":1000, "recovery":None}
        with tempfile.TemporaryDirectory() as directory:
            helper = Path(directory) / "preflight"
            for value in [[], {}, dict(good, schema="future"), dict(good, required_bytes=-1), dict(good, history_cutover_required="false")]:
                helper.write_text("#!" + sys.executable + "\nprint(" + repr(json.dumps(value)) + ")\n")
                helper.chmod(0o700)
                with self.assertRaisesRegex(ValueError, "invalid"):
                    installer.candidate_preflight(helper, Path(directory), Path(directory))
            helper.write_text("#!" + sys.executable + "\nprint(" + repr(json.dumps(good)) + ")\n")
            self.assertEqual(installer.candidate_preflight(helper, Path(directory), Path(directory)), good)


class HTTPSResponse(io.BytesIO):
    def __init__(self, body, status=200, headers=None, url="https://downloads.example/package"):
        super().__init__(body)
        self.status = status
        self.headers = {"Content-Length": str(len(body)), **(headers or {})}
        self.url = url

    def geturl(self):
        return self.url


class StandaloneFixture(unittest.TestCase):
    def setUp(self):
        preflight = patch.object(installer, "candidate_preflight", return_value={"schema": "hiroute.upgrade-preflight/v1", "history_cutover_required": False, "required_bytes": 0, "available_bytes": 1000000000, "recovery": None})
        self.preflight = preflight.start()
        self.addCleanup(preflight.stop)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        # Installer paths are canonicalized; Linux workbenches may alias /tmp.
        self.root = Path(self.temporary.name).resolve()
        self.repo = self.root / "repo"
        self.home = self.root / "home"
        self.output = self.root / "output"
        self.home.mkdir(mode=0o700)
        skill = self.repo / "assets/skills/hiroute-management/SKILL.md"
        docs = self.repo / "docs/standalone-cli.md"
        skill.parent.mkdir(parents=True)
        docs.parent.mkdir(parents=True)
        project_license = self.repo / "LICENSE"
        project_license.write_text("HiRoute license\n")
        project_license.chmod(0o644)
        skill.write_text("---\nname: hiroute-management\ndescription: test\n---\n<!-- Managed by HiRoute standalone installer -->\n")
        docs.write_text("test documentation\n")
        windows_docs = docs.with_name("standalone-windows.md")
        windows_docs.write_text("test Windows documentation\n")
        skill.chmod(0o644)
        docs.chmod(0o644)
        windows_docs.chmod(0o644)
        self.skill = skill
        self.sources = {}
        for name in ("hiroute", "hirouted", "cliproxyapi"):
            path = self.root / name
            path.write_bytes((name + "\n").encode())
            path.chmod(0o755)
            self.sources[name] = path
        self.license = self.root / "CPA-LICENSE"
        self.license.write_text("license\n")
        self.license.chmod(0o644)
        self.notices = self.root / "notices"
        self.notices.mkdir()
        (self.notices / "THIRD-PARTY-LICENSES.txt").write_text("third-party licenses\n")
        (self.notices / "third-party-licenses.json").write_text(json.dumps({
            "schema": "hiroute.third-party-licenses/v1",
            "inputs": {"fixture": "locked"},
            "packages": [{"id": "fixture:dependency@1"}],
            "documents": [{"sha256": "0" * 64}],
        }) + "\n")
        for path in self.notices.iterdir():
            path.chmod(0o644)

    def build(self, version="0.1.0", console_assets=None):
        args = argparse.Namespace(
            target=installer.host_target(),
            version=version,
            revision="a" * 40,
            hiroute=self.sources["hiroute"],
            hirouted=self.sources["hirouted"],
            cpa_binary=self.sources["cliproxyapi"],
            cpa_version="fixture-1",
            cpa_license=self.license,
            notices=self.notices,
            output=self.output,
            console_assets=console_assets,
        )
        with patch.object(package, "REPO", self.repo), patch("builtins.print"):
            package.build(args)
        archive = next(self.output.glob(f"hiroute-{version}-*.tar.gz"))
        return archive.with_suffix(archive.suffix + ".json"), archive

    def install_args(self, manifest, archive):
        return argparse.Namespace(
            manifest=manifest,
            archive=archive,
            manifest_url=None,
        )

    def environment(self):
        return patch.dict(
            os.environ,
            {
                "HOME": str(self.home),
                "XDG_STATE_HOME": str(self.home / ".state"),
                "XDG_RUNTIME_DIR": str(self.home / ".runtime"),
            },
            clear=False,
        )


class ConsoleInstallationTests(StandaloneFixture):
    @patch.object(installer.platform, "system", return_value="Linux")
    def test_console_configuration_is_private_preserved_and_transactionally_rolled_back(self, _system):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        args.console_mode = "network"
        args.console_listen = "0.0.0.0"
        args.console_advertise_address = "192.0.2.10"
        args.console_port = 8123
        args.allow_plaintext_network = True
        with self.environment(), patch.object(installer, "desktop_conflict", return_value=False), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(args)
            config = self.home / ".state/hiroute/console-listener.json"
            original = config.read_bytes()
            value = json.loads(original)
            self.assertEqual(value["desired"]["mode"], "network")
            self.assertEqual(value["desired"]["port"], 8123)
            self.assertEqual(value["desired"]["advertise_address"], "192.0.2.10")
            self.assertEqual(config.stat().st_mode & 0o777, 0o600)
            installer.install(self.install_args(manifest, archive))
            self.assertEqual(config.read_bytes(), original, "an upgrade without options must preserve the exact configuration")
            args.console_port = 8124
            replace = installer.os.replace
            failed = False
            def fail_marker(source, destination):
                nonlocal failed
                if Path(destination).name == "standalone.json" and not failed:
                    failed = True
                    raise OSError("injected commit failure")
                return replace(source, destination)
            with patch.object(installer.os, "replace", side_effect=fail_marker):
                with self.assertRaisesRegex(OSError, "injected"):
                    installer.install(args)
            self.assertTrue(failed)
            self.assertEqual(config.read_bytes(), original, "configuration participates in the install rollback")

    @patch.object(installer.platform, "system", return_value="Linux")
    def test_invalid_console_options_fail_before_install_writes(self, _system):
        cases = [
            ["--console-mode", "network", "--console-listen", "192.0.2.1"],
            ["--console-mode", "network", "--console-listen", "0.0.0.0", "--allow-plaintext-network"],
            ["--console-listen", "::", "--console-advertise-address", "192.0.2.1", "--allow-plaintext-network"],
            ["--console-mode", "loopback", "--console-listen", "192.0.2.1"],
            ["--console-port", "0"], ["--console-port", "65536"],
        ]
        for options in cases:
            args = installer.parser().parse_args(["install", *options])
            with self.environment(), self.assertRaises(ValueError), patch.object(installer, "acquire") as acquire:
                installer.install(args)
            acquire.assert_not_called()
            self.assertFalse((self.home / ".local").exists())

    @patch.object(installer.platform, "system", return_value="Linux")
    def test_explicit_configuration_refuses_foreign_link_without_modifying_it(self, _system):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        args.console_mode = "loopback"
        state = self.home / ".state/hiroute"
        state.mkdir(parents=True, mode=0o700)
        foreign = self.root / "foreign.json"
        foreign.write_text("foreign")
        config = state / "console-listener.json"
        config.symlink_to(foreign)
        with self.environment(), patch.object(installer, "desktop_conflict", return_value=False), self.assertRaises(OSError):
            installer.install(args)
        self.assertEqual(foreign.read_text(), "foreign")
        self.assertTrue(config.is_symlink())
        self.assertFalse((self.home / ".local/share/hiroute/standalone.json").exists())


class PackageTests(StandaloneFixture):
    def test_console_assets_are_verified_and_installed_with_the_existing_package(self):
        assets = self.root / "console"
        (assets / "assets").mkdir(parents=True)
        (assets / "index.html").write_text('<script src="/assets/app.js"></script>')
        (assets / "assets/app.js").write_text('console.info("test")')
        for path in assets.rglob("*"):
            if path.is_file():
                path.chmod(0o644)
        manifest, archive = self.build(console_assets=assets)
        verified = package.verify(manifest, archive)
        self.assertEqual(verified["files"]["console/index.html"]["sha256"], package.digest(assets / "index.html"))
        with self.environment(), patch.object(installer, "desktop_conflict", return_value=False), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
            marker = json.loads((self.home / ".local/share/hiroute/standalone.json").read_text())
            self.assertEqual((Path(marker["install_root"]) / "console/assets/app.js").read_bytes(), (assets / "assets/app.js").read_bytes())
        for name in ("console/../escape", "console/.hidden/../escape", "console//bad", "console/bad\\name", "unexpected.txt"):
            self.assertFalse(package.valid_inventory(package.expected_files(installer.host_target()) | {"console/index.html", name}, installer.host_target()))
        (assets / "unsafe.js").symlink_to(assets / "index.html")
        with self.assertRaisesRegex(ValueError, "links"):
            package.console_payload(assets)

    def test_archive_is_deterministic_and_tampering_is_rejected(self):
        manifest, archive = self.build()
        first = archive.read_bytes()
        archive.unlink()
        manifest.unlink()
        manifest, archive = self.build()
        self.assertEqual(first, archive.read_bytes())
        package.verify(manifest, archive)
        archive.write_bytes(archive.read_bytes() + b"tamper")
        with self.assertRaisesRegex(ValueError, "digest"):
            package.verify(manifest, archive)

    def test_symlink_source_is_rejected(self):
        link = self.root / "linked-hiroute"
        link.symlink_to(self.sources["hiroute"])
        with self.assertRaisesRegex(ValueError, "regular file"):
            package.checked_source(link, True)

    def test_missing_systemctl_is_an_optional_integration(self):
        with (
            patch.object(installer.platform, "system", return_value="Linux"),
            patch.object(installer.shutil, "which", return_value=None),
            patch.object(installer.subprocess, "run") as process,
        ):
            self.assertIsNone(installer.run_systemctl("daemon-reload"))
            process.assert_not_called()


@unittest.skipUnless(os.name == "posix" and os.uname().sysname == "Darwin", "macOS layout test")
class MacInstallerTests(StandaloneFixture):
    def test_install_reinstall_and_uninstall_use_owned_launchd_layout(self):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        # This fixture owns only its HOME; an unrelated real /Applications App may exist.
        with (
            self.environment(),
            patch.object(installer, "desktop_conflict", return_value=False),
            patch.object(installer.subprocess, "run"),
            patch("builtins.print"),
        ):
            installer.install(args)
            marker_path = self.home / ".local/share/hiroute/standalone.json"
            marker = json.loads(marker_path.read_text())
            service = self.home / ".local/share/hiroute/service/ai.hiroute.cli.plist"
            self.assertEqual(marker["service_definition"], str(service))
            plist = plistlib.loads(service.read_bytes())
            self.assertEqual(plist["Label"], "ai.hiroute.cli")
            self.assertEqual(plist["EnvironmentVariables"]["HOME"], str(self.home))
            self.assertEqual(
                plist["ProgramArguments"][0],
                str(self.home / ".local/lib/hiroute/0.1.0/hirouted"),
            )
            self.assertFalse(plist["RunAtLoad"])
            self.assertTrue((self.home / ".local/bin/hiroute").is_symlink())
            for directory in (".agents", ".claude"):
                self.assertEqual(
                    (self.home / directory / "skills/hiroute-management/SKILL.md").read_bytes(),
                    self.skill.read_bytes(),
                )
            installer.install(args)
            self.assertEqual(json.loads(marker_path.read_text()), marker)
            state = self.home / "Library/Application Support/ai.hiroute.cli/storage/keep"
            state.parent.mkdir(parents=True)
            state.write_text("business data")
            installer.uninstall(argparse.Namespace())
            self.assertFalse(marker_path.exists())
            self.assertEqual(state.read_text(), "business data")


@unittest.skipUnless(os.name == "posix", "Unix installation recovery")
class RecoveryRetentionTests(StandaloneFixture):
    def setUp(self):
        super().setUp()
        conflict = patch.object(installer, "desktop_conflict", return_value=False)
        conflict.start()
        self.addCleanup(conflict.stop)

    def install_original(self):
        manifest, archive = self.build("0.1.0")
        args = self.install_args(manifest, archive)
        installer.install(args)
        return args, installer.layout(self.home)

    def test_current_format_repair_does_not_copy_packages_or_replace_existing_handoff(self):
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            args, paths = self.install_original()
            retained = paths["state"] / "upgrade-packages/prior-batch"
            retained.mkdir(parents=True)
            (retained / "keep").write_bytes(b"existing recovery material")
            handoff = paths["state"] / "upgrade-package.json"
            original = json.dumps({"package_root": str(retained)}).encode()
            handoff.write_bytes(original)
            data = paths["state"] / "storage/keep"
            data.parent.mkdir()
            data.write_bytes(b"business data")
            with patch.object(installer.shutil, "disk_usage", return_value=SimpleNamespace(free=200 * 1024 * 1024)):
                for _ in range(2):
                    installer.install(args)
            self.assertEqual(list(retained.parent.iterdir()), [retained])
            self.assertEqual(handoff.read_bytes(), original)
            self.assertEqual((retained / "keep").read_bytes(), b"existing recovery material")
            self.assertEqual(data.read_bytes(), b"business data")

    def test_refused_cutover_discards_only_this_unpublished_copy(self):
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            args, paths = self.install_original()
            marker = paths["marker"].read_bytes()
            retained = paths["state"] / "upgrade-packages/prior-batch"
            retained.mkdir(parents=True)
            (retained / "keep").write_bytes(b"retained original")
            handoff = paths["state"] / "upgrade-package.json"
            handoff.write_bytes(b"prior handoff")
            for _ in range(2):
                self.preflight.side_effect = [{"history_cutover_required": True}, ValueError("final preflight refused")]
                with self.assertRaisesRegex(ValueError, "final preflight refused"):
                    installer.install(args)
            self.assertEqual(list(retained.parent.iterdir()), [retained])
            self.assertEqual(handoff.read_bytes(), b"prior handoff")
            self.assertEqual(paths["marker"].read_bytes(), marker)
            self.assertEqual((retained / "keep").read_bytes(), b"retained original")

    def test_copy_failure_discards_the_new_private_archive(self):
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            args, paths = self.install_original()
            self.preflight.return_value = {"history_cutover_required": True}
            with patch.object(installer.os, "read", side_effect=OSError("copy interrupted")):
                with self.assertRaisesRegex(OSError, "copy interrupted"):
                    installer.install(args)
            self.assertEqual(list((paths["state"] / "upgrade-packages").iterdir()), [])
            self.assertFalse((paths["state"] / "upgrade-package.json").exists())

    def test_published_handoff_copy_survives_later_installation_rollback(self):
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            args, paths = self.install_original()
            marker = paths["marker"].read_bytes()
            handoff = paths["state"] / "upgrade-package.json"
            handoff.write_bytes(b"prior handoff")
            self.preflight.return_value = {"history_cutover_required": True}
            replace = installer.os.replace
            def fail_marker(source, destination):
                if Path(destination) == paths["marker"] and Path(source).name.startswith("." + paths["marker"].name + "."):
                    raise OSError("publication interrupted")
                return replace(source, destination)
            with patch.object(installer.os, "replace", side_effect=fail_marker):
                with self.assertRaisesRegex(OSError, "publication interrupted"):
                    installer.install(args)
            self.assertEqual(paths["marker"].read_bytes(), marker)
            self.assertEqual(handoff.read_bytes(), b"prior handoff")
            copies = list((paths["state"] / "upgrade-packages").iterdir())
            self.assertEqual(len(copies), 1)
            self.assertEqual((copies[0] / "program/hirouted").read_bytes(), self.sources["hirouted"].read_bytes())


@unittest.skipUnless(os.name == "posix" and os.uname().sysname == "Linux", "Linux layout test")
class InstallerTests(StandaloneFixture):
    def test_upgrade_retains_the_complete_original_package_and_restoration_entries(self):
        old_manifest, old_archive = self.build("0.1.0")
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(old_manifest, old_archive))
            paths = installer.layout(self.home)
            old_program, old_resources, service = installer.validate_marker(paths, installer.read_marker(paths))
            original_program = (old_program / "hirouted").read_bytes()
            original_service = service.read_bytes()
            self.sources["hirouted"].write_bytes(b"new verified daemon")
            manifest, archive = self.build("0.2.0")
            self.preflight.return_value = {"history_cutover_required": True}
            installer.install(self.install_args(manifest, archive))
            handoff = json.loads((paths["state"] / "upgrade-package.json").read_text())
            retained = Path(handoff["package_root"])
            self.assertEqual((retained / "program/hirouted").read_bytes(), original_program)
            self.assertEqual((retained / "installation/service-definition").read_bytes(), original_service)
            self.assertFalse(old_program.exists())
            self.assertFalse(old_resources.exists())
            self.assertEqual(handoff["target_daemon_sha256"], installer.digest(paths["lib"] / "0.2.0/hirouted"))
            self.assertEqual(handoff["storage_root"], str(paths["state"] / "storage"))
            locations = json.loads((retained / "restore-locations.json").read_text())
            self.assertEqual(locations["program"], str(old_program))
            self.assertEqual(locations["links"], {str(paths["bin"] / name): str(old_program / name) for name in ("hiroute", "hirouted")})
            for name, expected in handoff["files"].items():
                self.assertEqual(installer.digest(retained / name), expected)
            self.assertNotIn(b"HIROUTE_RECOVERY_PACKAGE", service.read_bytes())

    def test_preflight_refusal_preserves_the_original_installation_and_service(self):
        old_manifest, old_archive = self.build("0.1.0")
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(old_manifest, old_archive))
            paths = installer.layout(self.home)
            old_program, old_resources, service = installer.validate_marker(paths, installer.read_marker(paths))
            before = {path: path.read_bytes() for path in [paths["marker"], old_program / "hirouted", service]}
            original_link = (paths["bin"] / "hiroute").readlink()
            self.sources["hirouted"].write_bytes(b"changed candidate daemon")
            manifest, archive = self.build("0.2.0")
            with patch.object(installer, "candidate_preflight", side_effect=ValueError("insufficient_space")):
                with self.assertRaisesRegex(ValueError, "insufficient_space"):
                    installer.install(self.install_args(manifest, archive))
            for path, content in before.items():
                self.assertEqual(path.read_bytes(), content)
            self.assertEqual((paths["bin"] / "hiroute").readlink(), original_link)
            self.assertTrue(old_resources.is_dir())
            self.assertFalse((paths["lib"] / "0.2.0").exists())
            self.assertFalse((paths["state"] / "upgrade-package.json").exists())

    def test_accessible_service_home_preserves_permissions(self):
        manifest, archive = self.build()
        self.home.chmod(0o775)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        self.assertEqual(self.home.stat().st_mode & 0o777, 0o775)
        self.assertTrue((self.home / ".local").is_dir())

    def test_service_parents_are_private_even_with_group_writable_umask(self):
        manifest, archive = self.build()
        previous = os.umask(0o002)
        try:
            with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
                installer.install(self.install_args(manifest, archive))
            for relative in (".local", ".local/share", ".local/share/hiroute", ".local/share/hiroute/service"):
                self.assertEqual((self.home / relative).stat().st_mode & 0o777, 0o700)
        finally:
            os.umask(previous)

    def test_service_parent_rejects_symlinks_and_preserves_existing_permissions(self):
        local = self.home / ".local"
        local.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "service parent is unsafe"):
            installer.prepare_service_directory(self.home)
        local.unlink()
        local.mkdir()
        local.chmod(0o775)
        installer.prepare_service_directory(self.home)
        self.assertEqual(local.stat().st_mode & 0o777, 0o775)

    def test_default_state_parents_are_private_with_group_writable_umask(self):
        manifest, archive = self.build()
        previous = os.umask(0o002)
        try:
            with patch.dict(os.environ, {"HOME": str(self.home)}, clear=True), patch.object(
                installer.subprocess, "run"
            ), patch("builtins.print"):
                installer.install(self.install_args(manifest, archive))
            for relative in (".local/state", ".local/state/hiroute", ".local/state/hiroute/run"):
                self.assertEqual((self.home / relative).stat().st_mode & 0o777, 0o700)
        finally:
            os.umask(previous)

    def test_nested_xdg_state_parents_are_private_with_group_writable_umask(self):
        manifest, archive = self.build()
        previous = os.umask(0o002)
        try:
            environment = {"HOME": str(self.home), "XDG_STATE_HOME": str(self.home / "custom/nested/state")}
            with patch.dict(os.environ, environment, clear=True), patch.object(
                installer.subprocess, "run"
            ), patch("builtins.print"):
                installer.install(self.install_args(manifest, archive))
            for relative in ("custom", "custom/nested", "custom/nested/state", "custom/nested/state/hiroute"):
                self.assertEqual((self.home / relative).stat().st_mode & 0o777, 0o700)
        finally:
            os.umask(previous)

    def test_install_rejects_symlink_state_parent(self):
        manifest, archive = self.build()
        local = self.home / ".local"
        local.mkdir(mode=0o700)
        shared = self.root / "shared-state"
        shared.mkdir(mode=0o700)
        state = local / "state"
        state.symlink_to(shared)
        with patch.dict(os.environ, {"HOME": str(self.home)}, clear=True), patch.object(
            installer.subprocess, "run"
        ), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "state parent is unsafe"):
                installer.install(self.install_args(manifest, archive))
        self.assertTrue(state.is_symlink())
        self.assertEqual(list(shared.iterdir()), [])
        self.assertFalse((local / "bin/hiroute").exists())

    def test_install_accepts_group_writable_state_parent_without_chmod(self):
        manifest, archive = self.build()
        state = self.home / ".local/state"
        state.parent.mkdir(mode=0o700)
        state.mkdir(mode=0o775)
        state.chmod(0o775)
        with patch.dict(os.environ, {"HOME": str(self.home)}, clear=True), patch.object(
            installer.subprocess, "run"
        ), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        self.assertEqual(state.stat().st_mode & 0o777, 0o775)
        self.assertTrue((self.home / ".local/bin/hiroute").exists())

    def test_safe_existing_state_parent_permissions_are_preserved(self):
        manifest, archive = self.build()
        state = self.home / ".local/state"
        state.parent.mkdir(mode=0o700)
        state.mkdir(mode=0o755)
        state.chmod(0o755)
        with patch.dict(os.environ, {"HOME": str(self.home)}, clear=True), patch.object(
            installer.subprocess, "run"
        ), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        self.assertEqual(state.stat().st_mode & 0o777, 0o755)
        self.assertEqual((state / "hiroute").stat().st_mode & 0o777, 0o700)

    def test_install_lock_rejects_concurrent_mutation(self):
        with self.environment():
            paths = installer.layout(self.home)
            with installer.installation_lock(paths):
                with self.assertRaisesRegex(ValueError, "another standalone install"):
                    with installer.installation_lock(paths):
                        self.fail("concurrent installer unexpectedly acquired the lock")

    def test_install_reinstall_and_owned_uninstall_preserve_state(self):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(args)
            marker_path = self.home / ".local/share/hiroute/standalone.json"
            first_marker = json.loads(marker_path.read_text())
            self.assertEqual(first_marker["schema_version"], installer.MARKER_SCHEMA)
            self.assertEqual(
                first_marker["installed_skills"],
                [
                    str(self.home / ".agents/skills/hiroute-management"),
                    str(self.home / ".claude/skills/hiroute-management"),
                ],
            )
            self.assertTrue((self.home / ".local/bin/hiroute").is_symlink())
            self.assertEqual(
                (self.home / ".agents/skills/hiroute-management/SKILL.md").read_bytes(),
                self.skill.read_bytes(),
            )
            self.assertEqual(
                (self.home / ".claude/skills/hiroute-management/SKILL.md").read_bytes(),
                self.skill.read_bytes(),
            )
            for parent in (
                self.home / ".agents",
                self.home / ".agents/skills",
                self.home / ".claude",
                self.home / ".claude/skills",
            ):
                self.assertEqual(parent.stat().st_mode & 0o777, 0o700)
            self.assertIn(
                installer.MANAGED_TEXT,
                (self.home / ".config/systemd/user/ai.hiroute.cli.service").read_text(),
            )
            state = self.home / ".state/hiroute/storage/keep"
            state.parent.mkdir(parents=True)
            state.write_text("business data")
            installer.install(args)
            self.assertEqual(json.loads(marker_path.read_text()), first_marker)
            installer.uninstall(argparse.Namespace())
            self.assertEqual(state.read_text(), "business data")
            self.assertFalse(marker_path.exists())
            self.assertFalse((self.home / ".local/bin/hiroute").exists())
            self.assertFalse((self.home / ".agents/skills/hiroute-management").exists())
            self.assertFalse((self.home / ".claude/skills/hiroute-management").exists())

    def test_desktop_fails_before_writes_and_fixed_component_leaves_are_replaced(self):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        app = self.home / "Applications/HiRoute.app"
        app.mkdir(parents=True)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "Desktop"):
                installer.install(args)
        self.assertFalse((self.home / ".local/lib/hiroute").exists())
        app.rmdir()

        entry = self.home / ".local/bin/hiroute"
        entry.parent.mkdir(parents=True)
        entry.write_text("unrelated")
        service = self.home / ".config/systemd/user/ai.hiroute.cli.service"
        service.parent.mkdir(parents=True)
        service.mkdir()
        (service / "old").write_text("stale")
        agents_skill = self.home / ".agents/skills/hiroute-management"
        claude_skill = self.home / ".claude/skills/hiroute-management"
        for destination in (agents_skill, claude_skill):
            destination.mkdir(parents=True)
            (destination / "SKILL.md").write_text("custom\n")
            (destination / "extra.txt").write_text("remove with component leaf\n")
        for parent in (
            self.home / ".agents",
            self.home / ".agents/skills",
            self.home / ".claude",
            self.home / ".claude/skills",
        ):
            parent.chmod(0o700)
        sibling = self.home / ".agents/skills/user-owned"
        sibling.mkdir()
        (sibling / "SKILL.md").write_text("preserve\n")
        version_root = self.home / ".local/lib/hiroute/0.1.0"
        resource_root = self.home / ".local/share/hiroute/0.1.0"
        version_root.mkdir(parents=True)
        resource_root.mkdir(parents=True)
        for relative in (".local", ".local/share", ".local/share/hiroute"):
            (self.home / relative).chmod(0o700)
        (version_root / "stale").write_text("old\n")
        (resource_root / "stale").write_text("old\n")
        marker_path = self.home / ".local/share/hiroute/standalone.json"
        marker_path.write_text("[]\n")
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(args)
        self.assertTrue(entry.is_symlink())
        self.assertEqual((version_root / "hiroute").read_bytes(), b"hiroute\n")
        self.assertFalse((version_root / "stale").exists())
        self.assertFalse((resource_root / "stale").exists())
        self.assertEqual(json.loads(marker_path.read_text())["version"], "0.1.0")
        self.assertTrue(service.is_file())
        self.assertIn(installer.MANAGED_TEXT, service.read_text())
        self.assertEqual((agents_skill / "SKILL.md").read_bytes(), self.skill.read_bytes())
        self.assertEqual((claude_skill / "SKILL.md").read_bytes(), self.skill.read_bytes())
        self.assertFalse((agents_skill / "extra.txt").exists())
        self.assertFalse((claude_skill / "extra.txt").exists())
        self.assertEqual((sibling / "SKILL.md").read_text(), "preserve\n")

    def test_install_accepts_group_writable_agent_skill_parent(self):
        manifest, archive = self.build()
        parent = self.home / ".agents"
        parent.mkdir(mode=0o700)
        parent.chmod(0o770)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        self.assertTrue((self.home / ".local/bin/hiroute").exists())
        self.assertEqual(parent.stat().st_mode & 0o777, 0o770)

    def test_checksum_failure_makes_no_installation_writes(self):
        manifest, archive = self.build()
        value = json.loads(manifest.read_text())
        value["archive"]["sha256"] = "0" * 64
        manifest.write_text(json.dumps(value))
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "checksum"):
                installer.install(self.install_args(manifest, archive))
        self.assertFalse((self.home / ".local").exists())

    def https_package(self, manifest, archive, mode):
        payload = archive.read_bytes()
        calls = []

        def response(request, **_options):
            if request.full_url.endswith(manifest.name):
                return HTTPSResponse(manifest.read_bytes())
            self.assertTrue(request.full_url.endswith(archive.name))
            value = request.get_header("Range")
            calls.append((value, request.get_header("If-match")))
            if not value or mode == "ignore-ranges":
                body = payload[:len(payload) // 2] if mode in ("short-once", "always-short") else payload
                return HTTPSResponse(body, headers={"Content-Length": str(len(payload))})
            match = re.fullmatch(r"bytes=(\d+)-(\d+)", value)
            self.assertIsNotNone(match)
            start, end = map(int, match.groups())
            body = payload[start:end + 1]
            headers = {"Content-Range": f"bytes {start}-{end}/{len(payload)}", "ETag": '"fixture-object"'}
            if mode == "short-once" and len(calls) == 1 or mode == "always-short":
                headers["Content-Length"] = str(len(body))
                body = body[:len(body) // 2]
            if mode == "changed-object" and start > 0:
                headers["ETag"] = '"different-object"'
            if mode == "invalid-range" and start > 0:
                headers["Content-Range"] = f"bytes {start}-{end}/{len(payload) + 1}"
            if mode == "weak-etag":
                headers["ETag"] = 'W/"fixture-object"'
            url = "http://downloads.example/package" if mode == "unsafe-redirect" else "https://downloads.example/package"
            return HTTPSResponse(body, 206, headers, url)

        args = argparse.Namespace(manifest=None, archive=None,
                                  manifest_url="https://downloads.example/" + manifest.name)
        return args, response, calls

    def test_https_partial_transfer_recovers_through_verified_install(self):
        manifest, archive = self.build()
        args, response, calls = self.https_package(manifest, archive, "short-once")
        with self.environment(), patch.object(installer, "DOWNLOAD_CHUNK_SIZE", 128, create=True), \
                patch.object(installer.urllib.request, "urlopen", side_effect=response), \
                patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(args)
        marker = json.loads((self.home / ".local/share/hiroute/standalone.json").read_text())
        self.assertEqual(marker["version"], json.loads(manifest.read_text())["version"])
        self.assertEqual((Path(marker["install_root"]) / "hiroute").read_bytes(), self.sources["hiroute"].read_bytes())
        self.assertGreater(len(calls), 2)
        self.assertEqual(calls[0][0], calls[1][0])
        self.assertTrue(all(identity == '"fixture-object"' for _, identity in calls[2:]))

    def test_https_changed_object_or_range_preserves_installed_state(self):
        manifest, archive = self.build()
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        marker = self.home / ".local/share/hiroute/standalone.json"
        original = marker.read_bytes()
        user_data = self.home / ".local/share/hiroute/user-data"
        user_data.write_bytes(b"preserve user data")
        manifest, archive = self.build("0.2.0")
        for mode, message in [("changed-object", "object identity"), ("invalid-range", "byte range")]:
            args, response, _ = self.https_package(manifest, archive, mode)
            with self.subTest(mode=mode), self.environment(), \
                    patch.object(installer, "DOWNLOAD_CHUNK_SIZE", 128, create=True), \
                    patch.object(installer.urllib.request, "urlopen", side_effect=response), \
                    patch.object(installer.subprocess, "run"), patch("builtins.print"):
                with self.assertRaisesRegex(ValueError, message):
                    installer.install(args)
            self.assertEqual(marker.read_bytes(), original)
            self.assertEqual(user_data.read_bytes(), b"preserve user data")
            self.assertFalse((self.home / ".local/lib/hiroute/0.2.0").exists())

    def test_https_repeated_short_transfer_is_bounded_and_makes_no_install_writes(self):
        manifest, archive = self.build()
        args, response, calls = self.https_package(manifest, archive, "always-short")
        with self.environment(), patch.object(installer, "DOWNLOAD_CHUNK_SIZE", 128, create=True), \
                patch.object(installer.urllib.request, "urlopen", side_effect=response), \
                patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(OSError, "ended"):
                installer.install(args)
        self.assertEqual(len(calls), 3)
        self.assertFalse((self.home / ".local").exists())

    def test_https_whole_file_fallback_retains_verified_install(self):
        manifest, archive = self.build()
        for mode in ("ignore-ranges", "weak-etag"):
            args, response, calls = self.https_package(manifest, archive, mode)
            with self.subTest(mode=mode), self.environment(), \
                    patch.object(installer, "DOWNLOAD_CHUNK_SIZE", 128, create=True), \
                    patch.object(installer.urllib.request, "urlopen", side_effect=response), \
                    patch.object(installer.subprocess, "run"), patch("builtins.print"):
                installer.install(args)
            self.assertIsNone(calls[-1][0])
            self.assertEqual((self.home / ".local/bin/hiroute").read_bytes(), self.sources["hiroute"].read_bytes())

    def test_https_archive_redirect_cannot_leave_https(self):
        manifest, archive = self.build()
        args, response, _ = self.https_package(manifest, archive, "unsafe-redirect")
        with self.environment(), patch.object(installer, "DOWNLOAD_CHUNK_SIZE", 128, create=True), \
                patch.object(installer.urllib.request, "urlopen", side_effect=response), \
                patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "left HTTPS"):
                installer.install(args)
        self.assertFalse((self.home / ".local").exists())

    def test_manifest_path_escape_is_rejected_and_cross_version_update_converges(self):
        manifest, archive = self.build()
        value = json.loads(manifest.read_text())
        value["version"] = "../../escape"
        manifest.write_text(json.dumps(value))
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "manifest"):
                installer.install(self.install_args(manifest, archive))
        self.assertFalse((self.root / "escape").exists())
        self.assertFalse((self.home / ".local").exists())

        manifest, archive = self.build()
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))
        state = self.home / ".state/hiroute/storage/keep"
        state.parent.mkdir(parents=True)
        state.write_text("business data")
        self.sources["hiroute"].write_bytes(b"hiroute-v2\n")
        self.sources["hirouted"].write_bytes(b"hirouted-v2\n")
        self.sources["cliproxyapi"].write_bytes(b"cliproxyapi-v2\n")
        self.skill.write_text(
            "---\nname: hiroute-management\ndescription: updated\n---\n"
            "<!-- Managed by HiRoute standalone installer -->\n"
        )
        manifest2, archive2 = self.build("0.2.0")
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest2, archive2))
        marker = json.loads(
            (self.home / ".local/share/hiroute/standalone.json").read_text()
        )
        self.assertEqual(marker["version"], "0.2.0")
        self.assertFalse((self.home / ".local/lib/hiroute/0.1.0").exists())
        self.assertFalse((self.home / ".local/share/hiroute/0.1.0").exists())
        self.assertEqual(
            (self.home / ".local/lib/hiroute/0.2.0/hiroute").read_bytes(),
            b"hiroute-v2\n",
        )
        self.assertEqual(
            (self.home / ".agents/skills/hiroute-management/SKILL.md").read_bytes(),
            self.skill.read_bytes(),
        )
        self.assertIn(
            "/0.2.0/hirouted",
            (self.home / ".config/systemd/user/ai.hiroute.cli.service").read_text(),
        )
        self.assertEqual(state.read_text(), "business data")

        entry = self.home / ".local/bin/hiroute"
        entry.unlink()
        entry.write_text("drift\n")
        (self.home / ".local/lib/hiroute/0.2.0/hiroute").write_text("drift\n")
        (self.home / ".agents/skills/hiroute-management/SKILL.md").write_text("drift\n")
        (self.home / ".config/systemd/user/ai.hiroute.cli.service").write_text("drift\n")
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest2, archive2))
        self.assertTrue(entry.is_symlink())
        self.assertEqual(
            (self.home / ".local/lib/hiroute/0.2.0/hiroute").read_bytes(),
            b"hiroute-v2\n",
        )
        self.assertEqual(
            (self.home / ".agents/skills/hiroute-management/SKILL.md").read_bytes(),
            self.skill.read_bytes(),
        )
        self.assertIn(
            "/0.2.0/hirouted",
            (self.home / ".config/systemd/user/ai.hiroute.cli.service").read_text(),
        )

    def test_update_failure_restores_previous_components_and_retry_succeeds(self):
        manifest, archive = self.build()
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest, archive))

        self.sources["hiroute"].write_bytes(b"hiroute-v2\n")
        self.skill.write_text(
            "---\nname: hiroute-management\ndescription: updated\n---\n"
            "<!-- Managed by HiRoute standalone installer -->\n"
        )
        manifest2, archive2 = self.build("0.2.0")
        original_replace = installer.os.replace
        failed = False
        agents_skill = self.home / ".agents/skills/hiroute-management"

        def fail_once(source, destination):
            nonlocal failed
            if Path(destination) == agents_skill and not failed:
                failed = True
                raise OSError("injected replacement failure")
            return original_replace(source, destination)

        with self.environment(), patch.object(installer.subprocess, "run"), patch(
            "builtins.print"
        ), patch.object(installer.os, "replace", side_effect=fail_once):
            with self.assertRaisesRegex(OSError, "injected replacement failure"):
                installer.install(self.install_args(manifest2, archive2))

        marker_path = self.home / ".local/share/hiroute/standalone.json"
        self.assertEqual(json.loads(marker_path.read_text())["version"], "0.1.0")
        self.assertTrue((self.home / ".local/lib/hiroute/0.1.0").is_dir())
        self.assertFalse((self.home / ".local/lib/hiroute/0.2.0").exists())
        self.assertEqual(
            (agents_skill / "SKILL.md").read_text(),
            "---\nname: hiroute-management\ndescription: test\n---\n"
            "<!-- Managed by HiRoute standalone installer -->\n",
        )
        self.assertIn(
            "/0.1.0/hirouted",
            (self.home / ".config/systemd/user/ai.hiroute.cli.service").read_text(),
        )

        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(self.install_args(manifest2, archive2))
        self.assertEqual(json.loads(marker_path.read_text())["version"], "0.2.0")
        self.assertEqual((agents_skill / "SKILL.md").read_bytes(), self.skill.read_bytes())

    def test_uninstall_validates_all_owned_paths_before_stopping_or_deleting(self):
        manifest, archive = self.build()
        args = self.install_args(manifest, archive)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            installer.install(args)
        service = self.home / ".config/systemd/user/ai.hiroute.cli.service"
        service.write_text("externally replaced\n")
        with self.environment(), patch.object(installer.subprocess, "run") as process, patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "service definition changed"):
                installer.uninstall(argparse.Namespace())
            process.assert_not_called()
        self.assertTrue((self.home / ".local/bin/hiroute").is_symlink())
        self.assertTrue((self.home / ".local/share/hiroute/standalone.json").is_file())


if __name__ == "__main__":
    unittest.main()
