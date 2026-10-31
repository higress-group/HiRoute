#!/usr/bin/env python3
"""Product version lifecycle and automation ownership regressions."""
import importlib.util
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("product_version", Path(__file__).with_name("product-version.py"))
product = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(product)


def fixture(root, version="1.2.3"):
    def write(path, value):
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)
    for path, name in product.CARGO_PACKAGES.items():
        write(path, f'[package]\nname = "{name}"\nversion = "{version}"\n\n[dependencies]\nexample = "1.2.3"\n')
    for path in product.JSON_MANIFESTS:
        write(path, json.dumps({"name": "fixture", "version": version, "scripts": {"test": "unchanged"}}, indent=2) + "\n")
    for path in product.NPM_LOCKS:
        write(path, json.dumps({"name": "fixture", "version": version, "packages": {
            "": {"name": "fixture", "version": version},
            "node_modules/example": {"version": "1.2.3"}}}, indent=2) + "\n")
    lock = '\n\n'.join(f'[[package]]\nname = "{name}"\nversion = "{version}"' for name in product.CARGO_PACKAGES.values())
    write("Cargo.lock", lock + '\n\n[[package]]\nname = "example"\nversion = "1.2.3"\n')
    hello = json.dumps({"properties": {"client_version": {"const": version}}}, indent=2) + "\n"
    write("contracts/cli/local-control-hello.v2.schema.json", hello)
    write("contracts/cli/contract-set.v1.json", json.dumps({"files": {
        "local-control-hello.v2.schema.json": "sha256:" + hashlib.sha256(hello.encode()).hexdigest(),
        "unrelated.json": "sha256:" + "a" * 64}}, indent=2) + "\n")
    write("contracts/releases/index.v1.json", json.dumps({"schema": "hiroute.release-contract-index/v1",
          "latest_stable": "v1.2.3", "releases": [{"tag": "v1.2.3", "channel": "stable", "revision": "a" * 40}]}))


class ProductVersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        fixture(self.root)

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True, stderr=subprocess.PIPE).strip()

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def history(self):
        self.git("init", "-q")
        return self.commit()

    def test_post_release_development_advances_every_component_and_lock(self):
        with self.assertRaisesRegex(ValueError, "must be later"):
            product.check(self.root)
        result = product.advance(self.root)
        self.assertEqual(result["version"], "1.2.4-dev.1")
        self.assertEqual(set(result["changed"]), set(product.VERSION_PATHS))
        self.assertEqual(set(product.version_values(product.sources_at(self.root)).values()), {"1.2.4-dev.1"})
        self.assertEqual(product.check(self.root)["result"], "green")

    def test_equal_or_older_core_cannot_be_disguised_by_a_suffix(self):
        for version in ("1.2.2", "1.2.3", "1.2.3-dev.9", "1.2.3+git.abcdef"):
            with self.subTest(version=version):
                product.set_version(self.root, version)
                with self.assertRaisesRegex(ValueError, "must be later"):
                    product.check(self.root)

    def test_advance_is_idempotent_and_works_without_git_history(self):
        product.advance(self.root)
        before = product.sources_at(self.root)
        self.assertEqual(product.advance(self.root)["changed"], [])
        self.assertEqual(product.sources_at(self.root), before)
        self.assertEqual(product.check(self.root)["version"], "1.2.4-dev.1")

    def test_version_change_updates_local_control_schema_and_its_digest(self):
        product.advance(self.root)
        path = self.root / "contracts/cli/local-control-hello.v2.schema.json"
        self.assertEqual(json.loads(path.read_text())["properties"]["client_version"]["const"], "1.2.4-dev.1")
        contracts = json.loads((self.root / "contracts/cli/contract-set.v1.json").read_text())
        self.assertEqual(contracts["files"][path.name], "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest())
        self.assertEqual(contracts["files"]["unrelated.json"], "sha256:" + "a" * 64)

    def test_check_rejects_stale_local_control_schema_and_digest(self):
        product.advance(self.root)
        hello = self.root / "contracts/cli/local-control-hello.v2.schema.json"
        original = hello.read_text()
        hello.write_text(original.replace('"const": "1.2.4-dev.1"', '"const": "1.2.3"'))
        with self.assertRaisesRegex(ValueError, "release version mismatch.*local-control-hello"):
            product.check(self.root)
        hello.write_text(original)
        index = self.root / "contracts/cli/contract-set.v1.json"
        value = json.loads(index.read_text())
        value["files"][hello.name] = "sha256:" + "0" * 64
        index.write_text(json.dumps(value, indent=2) + "\n")
        with self.assertRaisesRegex(ValueError, "Local Control schema digest"):
            product.check(self.root)

    def test_next_release_can_skip_directly_to_a_new_minor(self):
        fixture(self.root, "0.3.1")
        index = self.root / "contracts/releases/index.v1.json"
        index.write_text(index.read_text().replace("1.2.3", "0.3.1"))
        self.assertEqual(product.advance(self.root)["version"], "0.3.2-dev.1")
        product.set_version(self.root, "0.5.0")
        self.assertEqual(product.check(self.root)["version"], "0.5.0")
        index.write_text(index.read_text().replace("0.3.1", "0.5.0"))
        self.assertEqual(product.advance(self.root)["version"], "0.5.1-dev.1")
        self.assertEqual(product.advance(self.root)["changed"], [])
        self.assertEqual(product.check(self.root)["latest_stable"], "0.5.0")

    def test_newer_planned_minor_or_release_candidate_is_preserved(self):
        for version in ("1.3.0-dev.7", "1.2.4-rc.1", "2.0.0"):
            product.set_version(self.root, version)
            self.assertEqual(product.advance(self.root)["changed"], [])
            self.assertEqual(product.check(self.root)["version"], version)

    def test_set_preserves_unrelated_dependencies_and_exact_format(self):
        before = product.sources_at(self.root)
        product.set_version(self.root, "1.2.4-dev.1")
        for path in product.NPM_LOCKS:
            lock = json.loads((self.root / path).read_text())
            self.assertEqual(lock["packages"]["node_modules/example"]["version"], "1.2.3")
        self.assertIn('name = "example"\nversion = "1.2.3"', (self.root / "Cargo.lock").read_text())
        product.set_version(self.root, "1.2.3")
        self.assertEqual(product.sources_at(self.root), before)

    def test_invalid_version_is_rejected_before_writing(self):
        before = product.sources_at(self.root)
        for version in ("", "1.2", "01.2.3", "1.2.3-dev.01", "1.2.3+", "1.2.3/evil"):
            with self.assertRaises(ValueError):
                product.set_version(self.root, version)
        self.assertEqual(product.sources_at(self.root), before)

    def test_inconsistent_manifest_and_npm_root_versions_are_rejected(self):
        path = self.root / product.NPM_LOCKS[0]
        value = json.loads(path.read_text())
        value["packages"][""]["version"] = "0.0.1"
        path.write_text(json.dumps(value, indent=2) + "\n")
        with self.assertRaisesRegex(ValueError, "packages/root"):
            product.advance(self.root)

    def test_missing_cargo_lock_entry_does_not_partially_update_sources(self):
        lock = self.root / "Cargo.lock"
        lock.write_text(lock.read_text().replace('name = "hiroute-cli"', 'name = "unrelated"'))
        before = product.sources_at(self.root)
        with self.assertRaisesRegex(ValueError, "lock entry"):
            product.set_version(self.root, "1.2.4")
        self.assertEqual(product.sources_at(self.root), before)

    def test_preview_cannot_be_the_stable_baseline(self):
        path = self.root / "contracts/releases/index.v1.json"
        value = json.loads(path.read_text())
        value["releases"][0]["channel"] = "preview"
        path.write_text(json.dumps(value))
        before = product.sources_at(self.root)
        with self.assertRaisesRegex(ValueError, "not recorded"):
            product.advance(self.root)
        self.assertEqual(product.sources_at(self.root), before)

    def test_automation_scope_allows_versions_and_recorded_contracts(self):
        base = self.history()
        product.advance(self.root)
        (self.root / "contracts/releases/new.json").write_text('{}\n')
        self.commit()
        self.assertEqual(product.scope(self.root, base), {"result": "green"})

    def test_automation_scope_rejects_other_changes_inside_a_version_file(self):
        base = self.history()
        product.advance(self.root)
        path = self.root / product.JSON_MANIFESTS[0]
        path.write_text(path.read_text().replace('"test": "unchanged"', '"test": "unrelated command"'))
        self.commit()
        with self.assertRaisesRegex(ValueError, "more than product version"):
            product.scope(self.root, base)

    def test_automation_scope_rejects_unrelated_generated_contract_changes(self):
        base = self.history()
        product.advance(self.root)
        path = self.root / "contracts/cli/local-control-hello.v2.schema.json"
        value = json.loads(path.read_text())
        value["required"] = ["unrelated-field"]
        path.write_text(json.dumps(value, indent=2) + "\n")
        product.set_version(self.root, "1.2.4-dev.1")
        self.commit()
        with self.assertRaisesRegex(ValueError, "more than product version"):
            product.scope(self.root, base)

    def test_automation_scope_rejects_unrelated_paths(self):
        base = self.history()
        (self.root / "foreign.txt").write_text("another change")
        self.commit()
        with self.assertRaisesRegex(ValueError, "unrelated changes"):
            product.scope(self.root, base)


if __name__ == "__main__":
    unittest.main()
