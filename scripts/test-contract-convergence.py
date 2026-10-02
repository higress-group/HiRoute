#!/usr/bin/env python3
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("check-contract-convergence.py")
SPEC = importlib.util.spec_from_file_location("contract_convergence", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def registry():
    return {
        "schema": "hiroute.contract-compatibility-support/v1",
        "production_contracts": ["hiroute.sample/v2"],
        "subjects": [
            {
                "subject": "sample",
                "audited_prefixes": ["hiroute.sample"],
                "current_contracts": ["hiroute.sample/v2"],
                "legacy_support": [
                    {
                        "contract": "hiroute.sample/v1",
                        "mode": "recovery_read_only",
                        "owner": "sample-owner",
                        "reason": "read an old record",
                        "removal_condition": "remove after the backup window",
                        "allowed_paths": ["legacy.rs"],
                    }
                ],
            }
        ],
        "forbidden_contracts": ["hiroute.forbidden/v1"],
        "rejection_paths": ["negative.rs"],
    }


class ContractConvergenceTests(unittest.TestCase):
    def run_audit(self, files, value=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = {
                "crates/domain/src/schema.rs": (
                    'pub const LATEST_SCHEMA_VERSION: u32 = 17;\n"hiroute.sample/v2"'
                ),
                **files,
            }
            for path, content in files.items():
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content)
            return MODULE.audit(root, value or registry(), files)

    def test_registered_recovery_path_is_green(self):
        self.assertEqual(self.run_audit({"legacy.rs": '"hiroute.sample/v1"'}), [])

    def test_historical_release_inventory_is_not_a_current_reader_or_fixture(self):
        snapshot = json.dumps({"schema": "hiroute.release-contract-snapshot/v1",
                               "contracts": ["hiroute.sample/v1", "hiroute.forbidden/v1"]})
        self.assertEqual(self.run_audit({"legacy.rs": '"hiroute.sample/v1"',
                                        "contracts/releases/v1.0.0.json": snapshot}), [])
        for path in ("contracts/releases/live.rs", "contracts/releases/other.json",
                     "contracts/cli/v1.0.0.json"):
            errors = self.run_audit({"legacy.rs": '"hiroute.sample/v1"', path: snapshot})
            self.assertTrue(any("forbidden contract" in error for error in errors), errors)
        errors = self.run_audit({"legacy.rs": '"hiroute.sample/v1"',
                                "contracts/releases/v1.0.0.json": '{"value":"hiroute.forbidden/v1"}'})
        self.assertTrue(any("forbidden contract" in error for error in errors), errors)
        # A snapshot cannot make an absent real legacy reader look implemented.
        errors = self.run_audit({"contracts/releases/v1.0.0.json": snapshot})
        self.assertTrue(any("stale compatibility registration" in error for error in errors), errors)

    def test_legacy_escape_is_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "live.rs": '"hiroute.sample/v1"',
            }
        )
        self.assertTrue(
            any("escapes registered support" in error for error in errors), errors
        )

    def test_unknown_version_and_forbidden_contract_are_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "live.rs": '"hiroute.sample/v3" "hiroute.forbidden/v1"',
            }
        )
        self.assertTrue(any("unregistered" in error for error in errors), errors)
        self.assertTrue(any("forbidden contract" in error for error in errors), errors)

    def test_unknown_version_is_allowed_only_in_a_named_rejection_path(self):
        self.assertEqual(
            self.run_audit(
                {
                    "legacy.rs": '"hiroute.sample/v1"',
                    "negative.rs": '"hiroute.sample/v999"',
                }
            ),
            [],
        )

    def test_obsolete_local_control_contract_file_is_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "contracts/cli/local-control.v1.schema.json": "{}",
            }
        )
        self.assertTrue(
            any("obsolete live contract file" in error for error in errors), errors
        )

    def test_obsolete_routing_authoring_api_file_is_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/application-api/src/routing_control.rs": "// retired API",
            }
        )
        self.assertTrue(
            any("obsolete live contract file" in error for error in errors), errors
        )

    def test_obsolete_routing_operation_producer_is_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/domain/src/operation/routing.rs": (
                    "pub fn from_routing_planner() {}"
                ),
            }
        )
        self.assertTrue(
            any("obsolete live contract producer" in error for error in errors), errors
        )

    def test_generic_compiled_plan_sealer_is_rejected(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/domain/src/routing/materialized.rs": (
                    "pub fn new(body: CompiledAgentPlanBodyV1) {}"
                ),
            }
        )
        self.assertTrue(
            any("obsolete live contract producer" in error for error in errors), errors
        )

    def test_new_internal_production_family_is_not_hidden_by_audited_prefixes(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/daemon/src/control/bin.rs": (
                    'const NEW_A: &str = "hiroute.new-internal-contract/v1";\n'
                    'const NEW_B: &str = "hiroute.new-internal-contract/v2";\n'
                ),
            }
        )
        self.assertTrue(
            any("undeclared production contract" in error for error in errors), errors
        )

    def test_declaring_tokens_does_not_hide_unclassified_multi_version_family(self):
        value = registry()
        value["production_contracts"] = sorted(
            [
                *value["production_contracts"],
                "hiroute.new-internal-contract/v1",
                "hiroute.new-internal-contract/v2",
            ]
        )
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/daemon/src/control/bin.rs": (
                    'const NEW_A: &str = "hiroute.new-internal-contract/v1";\n'
                    'const NEW_B: &str = "hiroute.new-internal-contract/v2";\n'
                ),
            },
            value,
        )
        self.assertTrue(
            any("unclassified production multi-version family" in error for error in errors),
            errors,
        )

    def test_production_consumer_cannot_accept_a_registered_recovery_version(self):
        value = registry()
        value["production_contracts"] = ["hiroute.sample/v1", "hiroute.sample/v2"]
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/gateway/src/producer.rs": (
                    'const EMITTED: &str = "hiroute.sample/v2";\n'
                ),
                "crates/daemon/src/consumer.rs": (
                    'const ACCEPTED: &str = "hiroute.sample/v1";\n'
                ),
            },
            value,
        )
        self.assertTrue(
            any(
                "consumer.rs: legacy contract escapes registered support" in error
                for error in errors
            ),
            errors,
        )

    def test_cfg_test_rejection_literal_is_ignored_but_following_production_is_scanned(self):
        errors = self.run_audit(
            {
                "legacy.rs": '"hiroute.sample/v1"',
                "crates/daemon/src/control/bin.rs": (
                    "#[cfg(all(test, unix))]\n"
                    "mod rejection {\n"
                    '    const REJECTED: &str = "hiroute.rejection-only/v999";\n'
                    "}\n"
                    'const CURRENT: &str = "hiroute.following-production/v1";\n'
                ),
            }
        )
        self.assertFalse(
            any("hiroute.rejection-only" in error for error in errors), errors
        )
        self.assertTrue(
            any("hiroute.following-production/v1" in error for error in errors), errors
        )

    def test_production_capable_cfg_is_not_hidden(self):
        for cfg in ('not(test)', 'any(test, unix)', 'all(unix, not(test))'):
            with self.subTest(cfg=cfg):
                errors = self.run_audit({
                    "legacy.rs": '"hiroute.sample/v1"',
                    "crates/daemon/src/live.rs": (
                        f"#[cfg({cfg})]\nfn live() {{\n"
                        ' let a = "hiroute.new-internal-contract/v1";\n'
                        ' let b = "hiroute.new-internal-contract/v2";\n}\n'
                    ),
                })
                self.assertTrue(any("undeclared production contract" in e for e in errors), errors)


if __name__ == "__main__":
    unittest.main()
