#!/usr/bin/env python3
"""Regression tests for hosted backend shard planning; no Rust build."""
import copy
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "ci_shards", Path(__file__).with_name("ci-shards.py")
)
shards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shards)


def full_plan():
    return {
        "mode": "full",
        "rust": True,
        "commands": [
            ["cargo", "fmt", "--check"],
            ["cargo", "clippy", "--locked", "--workspace", "--exclude",
             "hiroute-desktop", "--all-targets", "--all-features", "--", "-D", "warnings"],
            shards.FULL_TEST[:],
            ["cargo", "test", "--locked", "-p", "hiroute-gateway-core",
             "--no-default-features", "--test", "transport_loopback"],
            ["python3", "scripts/check-contract-convergence.py"],
        ],
    }


class ShardPlanningTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = shards.cargo_metadata()

    def test_current_workspace_assigns_every_integration_target_once(self):
        names = shards.verify_assignments(self.metadata)
        self.assertEqual(set(names), set(shards.INTEGRATION_SHARDS))
        self.assertEqual(sum(map(len, names.values())), len({name for _, name in shards.integration_targets(self.metadata)}))

    def test_unknown_target_is_a_hard_coverage_failure(self):
        metadata = copy.deepcopy(self.metadata)
        package = next(item for item in metadata["packages"]
                       if item["name"] == "hiroute-product-e2e")
        target = copy.deepcopy(next(item for item in package["targets"] if "test" in item["kind"]))
        target["name"] = "new_unassigned_journey"
        package["targets"].append(target)
        with self.assertRaisesRegex(ValueError, "missing=.*new_unassigned_journey"):
            shards.verify_assignments(metadata)

    def test_same_name_targets_must_share_one_shard(self):
        assignments = copy.deepcopy(shards.INTEGRATION_SHARDS)
        assignments["integration-daemon"].add(
            ("hiroute-daemon", "pre_gateway_compute_routing")
        )
        assignments["integration-foundation"].remove(
            ("hiroute-daemon", "pre_gateway_compute_routing")
        )
        with patch.object(shards, "INTEGRATION_SHARDS", assignments):
            with self.assertRaisesRegex(ValueError, "same-name workspace targets"):
                shards.verify_assignments(self.metadata)

    def test_full_plan_becomes_bounded_shards_without_dropping_gates(self):
        plan = full_plan()
        commands = shards.full_commands(plan, self.metadata)
        self.assertEqual(set(commands), set(shards.FULL_SHARDS))
        self.assertEqual(commands["quality"], plan["commands"][:2])
        self.assertEqual(commands["unit"], [[*shards.FULL_TEST, "--lib", "--bins"]])
        self.assertEqual(commands["gates"][0], [*shards.FULL_TEST, "--doc"])
        self.assertEqual(commands["gates"][1:], plan["commands"][3:])
        for name in shards.INTEGRATION_SHARDS:
            command = commands[name][0]
            self.assertEqual(command[:len(shards.FULL_TEST)], shards.FULL_TEST)
            self.assertIn("--test", command)

    def test_matrix_keeps_affected_plans_linear(self):
        self.assertEqual(
            shards.matrix({"rust": True, "mode": "affected"}),
            {"include": [{"shard": "affected"}]},
        )
        self.assertEqual(
            shards.matrix({"rust": True, "mode": "full"}),
            {"include": [{"shard": name} for name in shards.FULL_SHARDS]},
        )
        self.assertEqual(shards.matrix({"rust": False, "mode": "affected"}), {"include": []})

    def test_matrix_github_output_is_compact_and_machine_readable(self):
        value = shards.matrix({"rust": True, "mode": "affected"})
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "github-output"
            with patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
                with contextlib.redirect_stdout(io.StringIO()):
                    shards.write_matrix(value, github_output=True)
            key, encoded = output.read_text().strip().split("=", 1)
            self.assertEqual(key, "matrix")
            self.assertEqual(json.loads(encoded), value)

    def test_execution_is_github_actions_only(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(ValueError, "restricted to GitHub Actions"):
                shards.run_commands("quality", [["true"]])


if __name__ == "__main__":
    unittest.main()
