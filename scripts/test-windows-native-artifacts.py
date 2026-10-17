#!/usr/bin/env python3
"""Exercise artifact receipts with valid small PE fixtures; never execute tests."""

from contextlib import redirect_stdout
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("native_artifacts", HERE / "windows-native-artifacts.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
EXPECTED = "a" * 40


def small_pe(machine=0x8664):
    """PE32+ console image: one aligned RX section, entry point returns zero."""
    image = bytearray(1024)
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 0x3C, 0x80)
    image[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", image, 0x84, machine, 1, 0, 0, 0, 240, 0x22)
    optional = 0x98
    struct.pack_into("<HBBIIIIIQII", image, optional, 0x20B, 14, 0, 512, 0, 0,
                     0x1000, 0x1000, 0x140000000, 0x1000, 0x200)
    struct.pack_into("<HHHHHH", image, optional + 40, 6, 0, 0, 0, 6, 0)
    struct.pack_into("<IIIHHQQQQII", image, optional + 56, 0x2000, 0x200, 0, 3, 0,
                     0x100000, 0x1000, 0x100000, 0x1000, 0, 16)
    section = optional + 240
    image[section:section + 8] = b".text\0\0\0"
    struct.pack_into("<IIIIIIHHI", image, section + 8, 3, 0x1000, 512, 512,
                     0, 0, 0, 0, 0x60000020)
    image[512:515] = b"\x31\xc0\xc3"
    return bytes(image)


class NativeArtifactReceipts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.evidence = self.root / "evidence"
        self.head = EXPECTED
        self.host = module.TARGET
        self.calls = []
        for name in module.SOURCE_FILES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((HERE.parent / name).read_bytes())
        (self.root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        self.patchers = [patch.object(module, "ROOT", self.root),
                         patch.object(module, "command", self.command),
                         patch.dict(os.environ, HIROUTE_BUILD_SOURCE_SHA=EXPECTED)]
        for patcher in self.patchers:
            patcher.start()

    def tearDown(self):
        for patcher in reversed(self.patchers):
            patcher.stop()
        self.temporary.cleanup()

    def command(self, *arguments):
        self.calls.append(arguments)
        if arguments == ("git", "rev-parse", "HEAD"):
            return self.head
        if arguments[:2] == ("git", "diff"):
            return ""
        if arguments[:2] == ("git", "ls-files"):
            return "\n".join(arguments[4:])
        if arguments == ("rustc", "-vV"):
            return "rustc 1.91.0 fixture\nhost: " + self.host + "\nrelease: 1.91.0"
        self.fail("unexpected command: " + repr(arguments))

    def inputs(self, name="platform", image=None, kinds=None):
        binary = self.root / "target/debug/deps" / (name + "-0123456789abcdef.exe")
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(small_pe() if image is None else image)
        artifact = {"reason": "compiler-artifact", "profile": {"test": True},
                    "executable": str(binary), "target": {"name": name, "kind": kinds or ["test"]}}
        cargo = self.root / (name + "-cargo.jsonl")
        events = [{"reason": "compiler-artifact", "profile": {"test": False},
                   "executable": None, "target": {"name": "dependency", "kind": ["lib"]}},
                  artifact, {"reason": "build-finished", "success": True}]
        cargo.write_text("\n".join(json.dumps(event) for event in events), encoding="utf-8-sig")
        selected = {"required_cases": ["private_file_identity_is_held_and_reparse_free"],
                    "cargo_arguments": ["-p", "hiroute-platform", "--test", "windows"],
                    "harness_arguments": ["--test-threads=1"], "filter": None}
        recipe = self.root / (name + "-recipe.json")
        recipe.write_text(json.dumps(selected), encoding="utf-8")
        args = SimpleNamespace(expected_sha=EXPECTED, evidence=self.evidence, name=name,
                               cargo_json=cargo, recipe_json=recipe)
        return args, binary, artifact, selected

    def finalized(self):
        args, binary, _, _ = self.inputs()
        module.capture(args)
        final = SimpleNamespace(expected_sha=EXPECTED, evidence=self.evidence,
                                include_product_binaries=False, package_result=None)
        return module.finalize(final), binary, final

    def verify(self, value=None):
        if value is None:
            return module.verify(SimpleNamespace(expected_sha=EXPECTED, evidence=self.evidence))
        return module.verify_manifest(value, self.evidence, EXPECTED)

    def test_capture_binds_one_cargo_executable_and_verify_never_rebuilds_or_runs_it(self):
        args, binary, artifact, selected = self.inputs()
        stale = binary.parent / "stale-ffffffffffffffff.exe"
        stale.write_bytes(small_pe(0xAA64))
        capture = module.capture(args)
        self.assertEqual(capture["recipe"]["target"], artifact["target"])
        self.assertEqual(capture["recipe"]["source_artifact"], binary.relative_to(self.root).as_posix())
        for key, value in selected.items():
            self.assertEqual(capture["recipe"][key], value)
        copied = self.evidence / capture["file"]["path"]
        self.assertEqual(copied.read_bytes(), binary.read_bytes())
        self.assertEqual(capture["file"]["sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
        self.assertEqual(capture["file"]["pe_machine"], 0x8664)
        final = SimpleNamespace(expected_sha=EXPECTED, evidence=self.evidence,
                                include_product_binaries=False, package_result=None)
        value = module.finalize(final)
        self.calls.clear()
        self.assertEqual(self.verify(), value)
        self.assertTrue(all(arguments[0] == "git" for arguments in self.calls))
        self.assertEqual(value["runtime_state"], "not_assessed")

    def test_finalize_merges_recipes_and_is_byte_identical_on_a_second_run(self):
        for name in ("platform", "jobs"):
            args, _, _, _ = self.inputs(name)
            module.capture(args)
        args = SimpleNamespace(expected_sha=EXPECTED, evidence=self.evidence,
                               include_product_binaries=False, package_result=None)
        first = module.finalize(args)
        encoded = (self.evidence / "native-artifacts.json").read_bytes()
        self.assertEqual(module.finalize(args), first)
        self.assertEqual((self.evidence / "native-artifacts.json").read_bytes(), encoded)
        self.assertEqual([recipe["name"] for recipe in first["recipes"]], ["jobs", "platform"])

    def test_zero_multiple_wrong_profile_and_failed_build_artifacts_are_rejected(self):
        args, _, artifact, _ = self.inputs()
        variants = [[], [artifact, artifact], [dict(artifact, profile={"test": False})]]
        for events in variants:
            with self.subTest(events=events):
                args.cargo_json.write_text("\n".join(json.dumps(event) for event in events +
                                                     [{"reason": "build-finished", "success": True}]))
                with self.assertRaisesRegex(ValueError, "exactly one"):
                    module.capture(args)
        args.cargo_json.write_text("\n".join(json.dumps(event) for event in
                                            [artifact, {"reason": "build-finished", "success": False}]))
        with self.assertRaisesRegex(ValueError, "successfully built"):
            module.capture(args)
        self.assertFalse(self.evidence.exists())

    def test_duplicate_json_keys_and_empty_case_selection_are_rejected(self):
        args, _, _, selected = self.inputs()
        selected["required_cases"] = []
        args.recipe_json.write_text(json.dumps(selected))
        with self.assertRaisesRegex(ValueError, "required cases"):
            module.capture(args)
        selected["required_cases"] = ["case"]
        args.recipe_json.write_text(json.dumps(selected))
        args.cargo_json.write_text('{"reason":"build-finished","success":true,"success":false}')
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            module.capture(args)

    def test_wrong_producer_sha_compiler_host_and_embedded_sha_environment_are_rejected(self):
        args, _, _, _ = self.inputs()
        self.head = "b" * 40
        with self.assertRaisesRegex(ValueError, "checkout differs"):
            module.capture(args)
        self.head = EXPECTED
        self.host = "aarch64-pc-windows-msvc"
        with self.assertRaisesRegex(ValueError, "x64 rustc host"):
            module.capture(args)
        self.host = module.TARGET
        with patch.dict(os.environ, HIROUTE_BUILD_SOURCE_SHA="b" * 40):
            with self.assertRaisesRegex(ValueError, "source SHA"):
                module.capture(args)
        self.assertFalse(self.evidence.exists())

    def test_dirty_checkout_is_rejected_before_copying_any_bytes(self):
        args, _, _, _ = self.inputs()

        def dirty(*arguments):
            if arguments[:2] == ("git", "diff"):
                raise subprocess.CalledProcessError(1, arguments)
            return self.command(*arguments)

        with patch.object(module, "command", dirty):
            with self.assertRaises(subprocess.CalledProcessError):
                module.capture(args)
        self.assertFalse(self.evidence.exists())

    def test_cargo_source_outside_checkout_is_rejected_without_a_glob_fallback(self):
        args, _, artifact, _ = self.inputs()
        outside = self.root / "outside.exe"
        outside.write_bytes(small_pe())
        artifact["executable"] = str(outside)
        args.cargo_json.write_text("\n".join(json.dumps(event) for event in
                                            [artifact, {"reason": "build-finished", "success": True}]))
        with self.assertRaisesRegex(ValueError, "checkout-local"):
            module.capture(args)

    def test_same_size_tampering_and_a_foreign_pe_machine_are_rejected(self):
        value, _, _ = self.finalized()
        captured = self.evidence / value["files"][0]["path"]
        changed = bytearray(captured.read_bytes())
        changed[512] ^= 1
        captured.write_bytes(changed)
        with self.assertRaisesRegex(ValueError, "bytes differ"):
            self.verify(value)
        captured.write_bytes(small_pe(0xAA64))
        forged = copy.deepcopy(value)
        forged["files"][0]["sha256"] = hashlib.sha256(captured.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, "architecture"):
            self.verify(forged)

    def test_foreign_manifest_sha_lock_and_installer_bytes_are_rejected(self):
        value, _, _ = self.finalized()
        foreign = copy.deepcopy(value)
        foreign["build"]["source_sha"] = "b" * 40
        with self.assertRaisesRegex(ValueError, "build SHA"):
            self.verify(foreign)
        for name in ("Cargo.lock", "crates/cli/src/service/windows/install.rs", "scripts/package-standalone.py"):
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(original + b"\n# changed\n")
                with self.assertRaisesRegex(ValueError, "lock, scripts"):
                    self.verify(value)
                path.write_bytes(original)

    def test_escape_ads_reserved_and_noncanonical_relative_paths_are_rejected(self):
        value, _, _ = self.finalized()
        for path in ("../outside.exe", "/outside.exe", "C:/outside.exe",
                     "native-binaries\\outside.exe", "native-binaries/outside.exe:stream",
                     "native-binaries/CON/outside.exe", "native-binaries/../outside.exe",
                     "native-binaries//outside.exe", "native-binaries/evil./outside.exe"):
            with self.subTest(path=path):
                bad = copy.deepcopy(value)
                bad["files"][0]["path"] = path
                bad["recipes"][0]["executable"] = path
                with self.assertRaisesRegex(ValueError, "unsafe"):
                    self.verify(bad)

    def test_reparse_ancestor_is_rejected_without_reading_the_redirected_payload(self):
        value, _, _ = self.finalized()
        parent = self.evidence / "native-binaries/platform"
        moved = self.evidence / "outside"
        parent.rename(moved)
        try:
            parent.symlink_to(moved, target_is_directory=True)
        except OSError as error:
            self.skipTest("symlink fixture unavailable: " + str(error))
        before = {path.name: path.read_bytes() for path in moved.iterdir()}
        with self.assertRaisesRegex(ValueError, "reparse"):
            self.verify(value)
        self.assertEqual({path.name: path.read_bytes() for path in moved.iterdir()}, before)

    def test_missing_extra_duplicate_and_empty_recipe_inventories_are_rejected(self):
        value, _, _ = self.finalized()
        extra = self.evidence / "native-binaries/foreign.exe"
        extra.write_bytes(small_pe())
        with self.assertRaisesRegex(ValueError, "actual native file inventory"):
            self.verify(value)
        extra.unlink()
        for change in (lambda bad: bad.update(recipes=[]),
                       lambda bad: bad["recipes"][0].update(required_cases=[]),
                       lambda bad: bad["recipes"].append(copy.deepcopy(bad["recipes"][0])),
                       lambda bad: bad["files"].append(copy.deepcopy(bad["files"][0]))):
            bad = copy.deepcopy(value)
            change(bad)
            with self.assertRaises(ValueError):
                self.verify(bad)
        (self.evidence / value["files"][0]["path"]).unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            self.verify(value)

    def test_case_colliding_manifest_paths_are_rejected_on_all_fixture_hosts(self):
        value, _, _ = self.finalized()
        captured = self.evidence / value["files"][0]["path"]
        collision = captured.with_name(captured.name.upper())
        separate_file = not collision.exists()
        if separate_file:
            collision.write_bytes(captured.read_bytes())
        forged = copy.deepcopy(value)
        record = copy.deepcopy(forged["files"][0])
        record["path"] = collision.relative_to(self.evidence).as_posix()
        forged["files"].append(record)
        with self.assertRaisesRegex(ValueError, "duplicate path"):
            self.verify(forged)
        if separate_file:
            with self.assertRaisesRegex(ValueError, "case collision"):
                self.verify(value)

    def test_product_and_full_verified_package_bytes_use_portable_captured_paths(self):
        previous_umask = os.umask(0o002)
        self.addCleanup(os.umask, previous_umask)
        _, _, args = self.finalized()
        for filename in ("hiroute.exe", "hirouted.exe", "hiroute-cpa-process-fixture.exe"):
            binary = self.root / "target/debug" / filename
            binary.write_bytes(small_pe())
            binary.chmod(0o644)
        for name in ("LICENSE", "CPA-LICENSE", "assets/skills/hiroute-management/SKILL.md",
                     "docs/standalone-windows.md"):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("legal package fixture " + name)
            path.chmod(0o644)
        notices = self.root / "notices"
        notices.mkdir()
        (notices / "THIRD-PARTY-LICENSES.txt").write_text("legal license fixture")
        (notices / "third-party-licenses.json").write_text(json.dumps({
            "schema": "hiroute.third-party-licenses/v1", "inputs": {},
            "packages": [{"name": "fixture", "license": "MIT"}],
            "documents": [{"name": "fixture", "text": "legal license fixture"}]}))
        for path in notices.iterdir():
            path.chmod(0o644)
        package = module.packager()
        package_args = SimpleNamespace(target=module.TARGET, version="0.1.0", revision=EXPECTED,
                                       cpa_version="fixture", cpa_license=self.root / "CPA-LICENSE",
                                       hiroute=self.root / "target/debug/hiroute.exe",
                                       hirouted=self.root / "target/debug/hirouted.exe",
                                       cpa_binary=self.root / "target/debug/hirouted.exe",
                                       notices=notices, output=self.root / "prepared-package/package")
        output = io.StringIO()
        with redirect_stdout(output):
            package.build(package_args)
        result = json.loads(output.getvalue())
        result.update(source_revision=EXPECTED, target=module.TARGET, deterministic_publication="green",
                      archive_sha256=module.digest(Path(result["archive"])),
                      manifest_sha256=module.digest(Path(result["manifest"])))
        producer_evidence = self.root / "prepared-package/package-evidence.json"
        producer_evidence.write_text(json.dumps(result))
        args.package_result = self.root / "package-result.json"
        args.package_result.write_text(json.dumps(result))
        args.include_product_binaries = True
        value = module.finalize(args)
        self.assertEqual(set(value["product_binaries"]), {"hiroute", "hirouted", "cpa_process_fixture"})
        self.assertEqual(len(value["files"]), 7)
        self.assertEqual(set(value["package"]), {"archive", "manifest", "evidence"})
        for path in value["package"].values():
            self.assertFalse(Path(path).is_absolute())
        self.assertEqual((self.evidence / value["package"]["evidence"]).read_bytes(), producer_evidence.read_bytes())
        self.verify(value)


if __name__ == "__main__":
    unittest.main()
