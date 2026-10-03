#!/usr/bin/env python3
"""Standalone package/installer ownership and integrity regressions."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import sys
import tempfile
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


class StandaloneFixture(unittest.TestCase):
    def setUp(self):
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
        skill.chmod(0o644)
        docs.chmod(0o644)
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

    def build(self, version="0.1.0"):
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


class PackageTests(StandaloneFixture):
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


@unittest.skipUnless(os.name == "posix" and os.uname().sysname == "Linux", "Linux layout test")
class InstallerTests(StandaloneFixture):
    def test_unsafe_service_home_is_rejected_without_permission_changes(self):
        manifest, archive = self.build()
        self.home.chmod(0o775)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "service HOME is unsafe"):
                installer.install(self.install_args(manifest, archive))
        self.assertEqual(self.home.stat().st_mode & 0o777, 0o775)
        self.assertFalse((self.home / ".local").exists())

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
        with self.assertRaisesRegex(ValueError, "service parent is unsafe"):
            installer.prepare_service_directory(self.home)
        self.assertEqual(local.stat().st_mode & 0o777, 0o775)

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

    def test_install_rejects_group_writable_agent_skill_parent(self):
        manifest, archive = self.build()
        parent = self.home / ".agents"
        parent.mkdir(mode=0o700)
        parent.chmod(0o770)
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "Skill parent is unsafe"):
                installer.install(self.install_args(manifest, archive))
        self.assertFalse((self.home / ".local/bin/hiroute").exists())

    def test_checksum_failure_makes_no_installation_writes(self):
        manifest, archive = self.build()
        value = json.loads(manifest.read_text())
        value["archive"]["sha256"] = "0" * 64
        manifest.write_text(json.dumps(value))
        with self.environment(), patch.object(installer.subprocess, "run"), patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "checksum"):
                installer.install(self.install_args(manifest, archive))
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
