#!/usr/bin/env python3
"""Bundle identity regressions; does not claim macOS installation acceptance."""
import importlib.util
from contextlib import contextmanager
import struct
import subprocess
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

import sys
sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("package_desktop", Path(__file__).with_name("package-desktop.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class BundleIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.app = Path(self.temp.name) / "HiRoute.app"
        self.cpa = self.app / "Contents/MacOS/cliproxyapi"
        self.cpa.parent.mkdir(parents=True)
        self.cpa.write_bytes(b"final signed bytes")
        self.manifest = {"files": {str(self.cpa.relative_to(self.app)): package.digest(self.cpa)},
                         "cpa": {"artifacts": [{"sha256": package.digest(self.cpa), "size": self.cpa.stat().st_size}]}}

    def test_pinned_source_patch_is_required(self):
        builder = package.module("build_cpa", "build-cpa.py")
        pin, patch_path = builder.pinned_source()
        self.assertEqual(pin["commit"], package.CPA_COMMIT)
        self.assertEqual(pin["version"], package.CPA_VERSION)
        with patch.object(builder.hashlib, "sha256") as sha:
            sha.return_value.hexdigest.return_value = "0" * 64
            with self.assertRaisesRegex(ValueError, "patch checksum mismatch"):
                builder.pinned_source()

    def test_build_children_do_not_inherit_validation_lock_descriptors(self):
        completed = package.subprocess.CompletedProcess(
            ["fixture"], 0, stdout="ok\n", stderr=""
        )
        with patch.object(package.subprocess, "run", return_value=completed) as invoked:
            self.assertEqual(package.run("fixture"), "ok")
        self.assertNotIn("pass_fds", invoked.call_args.kwargs)

    def test_desktop_icon_matches_the_website_brand_and_has_a_retina_source(self):
        desktop_svg = package.NATIVE / "icons/icon.svg"
        website_svg = package.REPO / "apps/website/public/brand/app-icon.svg"
        self.assertEqual(desktop_svg.read_text(), website_svg.read_text())
        png = (package.NATIVE / "icons/icon.png").read_bytes()
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", png[16:24]), (1024, 1024))

    def test_final_signed_bytes_verified_and_resigning_rejected(self):
        package.verify_contents(self.app, self.manifest)
        self.cpa.write_bytes(b"signed again")
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            package.verify_contents(self.app, self.manifest)

    def test_cpa_is_explicitly_signed_before_release_probe(self):
        events = []
        release = ("CLIProxyAPI Version: 7.2.140-hiroute.2, Commit: "
                   "c76dfd4e0edabab9000628b1560ab8ab379eadb8, "
                   "BuiltAt: 2026-09-18T00:00:00Z")
        with patch.object(package, "sign", side_effect=lambda *_: events.append("sign")), \
                patch.object(package, "run", side_effect=lambda *_args, **_kwargs:
                             events.append("probe") or release):
            self.assertEqual(package.sign_and_probe_cpa(self.cpa, "-"), release)
        self.assertEqual(events, ["sign", "probe"])

    def test_missing_or_extra_component_rejected(self):
        extra = self.cpa.with_name("unexpected")
        extra.write_text("extra")
        with self.assertRaisesRegex(ValueError, "inventory mismatch"):
            package.verify_contents(self.app, self.manifest)
        extra.unlink()
        self.cpa.unlink()
        with self.assertRaisesRegex(ValueError, "inventory mismatch"):
            package.verify_contents(self.app, self.manifest)

    def test_compiled_cpa_pin_cannot_be_replaced_by_new_inventory(self):
        self.cpa.write_bytes(b"replacement")
        self.manifest["files"][str(self.cpa.relative_to(self.app))] = package.digest(self.cpa)
        with self.assertRaisesRegex(ValueError, "compiled manifest"):
            package.verify_contents(self.app, self.manifest)

    def test_symlink_rejected_even_with_same_bytes(self):
        original = self.cpa.with_name("original")
        self.cpa.rename(original)
        self.cpa.symlink_to(original)
        self.manifest["files"][str(original.relative_to(self.app))] = package.digest(original)
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            package.verify_contents(self.app, self.manifest)

    def test_binary_newer_than_declared_minimum_is_rejected(self):
        self.cpa.chmod(0o755)
        with patch.object(package, "run", side_effect=["arm64", "binary:\n /usr/lib/libSystem.B.dylib (version)", "    minos 16.0"]):
            with self.assertRaisesRegex(ValueError, "newer than 15.0"):
                package.inspect_binary(self.cpa)

    def test_intel_and_equivalent_minimum_version(self):
        self.cpa.chmod(0o755)
        with patch.object(package, "run", side_effect=["x86_64", "binary:\n /usr/lib/libSystem.B.dylib (version)", "    minos 15.0.0", ""]):
            measured = package.inspect_binary(self.cpa, "x86_64")
        self.assertEqual(measured["architecture"], "x86_64")
        with patch.object(package, "run", return_value="arm64"):
            with self.assertRaisesRegex(ValueError, "architecture differs"):
                package.inspect_binary(self.cpa, "x86_64")

    def test_non_system_dynamic_dependency_rejected(self):
        self.cpa.chmod(0o755)
        with patch.object(package, "run", side_effect=["arm64", "binary:\n /opt/homebrew/lib/libbad.dylib (version)"]):
            with self.assertRaisesRegex(ValueError, "non-system dynamic dependency"):
                package.inspect_binary(self.cpa)


class DmgTests(unittest.TestCase):
    def test_cache_server_starts_before_shared_build_lock(self):
        events = []

        class Store:
            @contextmanager
            def locked(self, _repo):
                events.append("lock")
                yield ()

        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "HiRoute.app"
            app.mkdir()
            with patch.object(package, "run", side_effect=lambda *args: events.append(args[1])), \
                    patch.object(package, "module", return_value=type("Local", (), {"Store": Store})), \
                    patch.object(package, "build", side_effect=lambda _args: events.append("build") or {"app": str(app)}), \
                    patch.object(sys, "argv", ["package-desktop.py", "build", "--cpa-source-repo", directory]):
                self.assertEqual(package.main(), 0)
        self.assertEqual(events, ["--start-server", "lock", "build"])

    def test_existing_cache_server_is_verified_before_shared_build_lock(self):
        events = []

        class Store:
            @contextmanager
            def locked(self, _repo):
                events.append("lock")
                yield ()

        def cache_run(*args):
            events.append(args[1])
            if args[1] == "--start-server":
                raise subprocess.CalledProcessError(2, args, stderr="Address in use")

        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "HiRoute.app"
            app.mkdir()
            with patch.object(package, "run", side_effect=cache_run), \
                    patch.object(package, "module", return_value=type("Local", (), {"Store": Store})), \
                    patch.object(package, "build", side_effect=lambda _args: events.append("build") or {"app": str(app)}), \
                    patch.object(sys, "argv", ["package-desktop.py", "build", "--cpa-source-repo", directory]):
                self.assertEqual(package.main(), 0)
        self.assertEqual(events, ["--start-server", "--show-stats", "lock", "build"])

    def test_names_and_repeated_output_preserve_previous_round(self):
        for arch in package.TARGETS:
            name = package.artifact_name("0.1.0", "a" * 40, arch, True)
            self.assertEqual(name, f"HiRoute-0.1.0-{'a' * 12}-macos-{arch}-trial")
        with tempfile.TemporaryDirectory() as directory, patch.object(package, "REPO", Path(directory)):
            first = package.candidate_output("a" * 40, "arm64")
            (first / "evidence").write_text("retain")
            second = package.candidate_output("a" * 40, "arm64")
            self.assertNotEqual(first, second)
            self.assertEqual((first / "evidence").read_text(), "retain")

    def test_mount_checks_and_detach_on_component_failure(self):
        for failure in (None, "components", "link", "extra"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                dmg = Path(directory) / "test.dmg"
                dmg.write_bytes(b"image")
                calls = []
                def run(*args):
                    calls.append(args)
                    if args[1] == "attach":
                        mount = Path(args[args.index("-mountpoint") + 1])
                        mount.mkdir()
                        (mount / "HiRoute.app").mkdir()
                        (mount / "Applications").symlink_to("/tmp" if failure == "link" else "/Applications")
                        if failure == "extra":
                            (mount / "unexpected").touch()
                    return ""
                manifest = dict(revision="a" * 40, version="0.1.0", architecture="arm64", distribution="controlled-trial")
                with patch.object(package, "run", side_effect=run), patch.object(package, "verify", return_value=manifest,
                        side_effect=ValueError("bad components") if failure == "components" else None):
                    if failure:
                        with self.assertRaises(ValueError):
                            package.verify_dmg(dmg)
                    else:
                        result = package.verify_dmg(dmg)
                        self.assertEqual(result["dmg_sha256"], package.digest(dmg))
                        self.assertEqual(result["detach"], "green")
                self.assertEqual(calls[0][1], "verify")
                self.assertIn("-readonly", calls[1])
                self.assertEqual(calls[-1][1], "detach")

    def test_bad_image_never_mounts(self):
        with patch.object(package, "run", side_effect=ValueError("bad image")) as run:
            with self.assertRaisesRegex(ValueError, "bad image"):
                package.verify_dmg(Path("bad.dmg"))
            self.assertEqual(run.call_count, 1)

    def test_staging_contains_app_and_absolute_applications_link(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "HiRoute.app"
            app.mkdir()
            destination = Path(directory) / "trial.dmg"
            def run(*args):
                if args[1] == "create":
                    stage = Path(args[args.index("-srcfolder") + 1])
                    self.assertEqual((stage / "Applications").readlink(), Path("/Applications"))
                    self.assertIn("UDZO", args)
                else:
                    self.assertEqual(args[0], "/usr/bin/ditto")
                    self.assertEqual(args[1], str(app))
            with patch.object(package, "run", side_effect=run):
                package.create_dmg(app, destination)
            self.assertEqual(list(Path(directory).iterdir()), [app])


if __name__ == "__main__":
    unittest.main()
