"""Run the same decision examples without HTTP, credentials or an event loop."""
from __future__ import annotations

from types import SimpleNamespace
import unittest

from jev_decider.decision import decision_response, prepare_decision
from jev_decider.protocol import validate_hiroute_request
from tests.decision_cases import decision_cases


class DecisionContractTests(unittest.TestCase):
    def test_decision_examples_do_not_require_a_service(self) -> None:
        for case in decision_cases():
            with self.subTest(capability=case["name"]):
                settings = SimpleNamespace(
                    mode=case["mode"],
                    model="typesafe/jev-1.13",
                    max_state_tokens=24_000,
                    simple_threshold=0.8,
                    competence_floor=0.5,
                    branch_criteria=case["settings"].get("branch_criteria", {}),
                    policy_configured=bool(case["settings"]),
                )
                prepared = prepare_decision(settings, validate_hiroute_request(case["request"]))
                self.assertEqual(prepared.body, case["expected_upstream"])
                result = decision_response(
                    settings, prepared.body["state"], case["upstream"], prepared.target_trimmed
                )
                self.assertEqual(result, case["expected_response"])
