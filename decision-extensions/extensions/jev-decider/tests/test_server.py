from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from aiohttp import web
from aiohttp.test_utils import TestClient, TestServer

from jev_decider.server import Settings, create_app


BRANCHES = {
    "smart_saving_simple": "Use the economy model group for a clear, well-scoped task.",
    "smart_saving_complex": "Use the primary model group for a complex task.",
}


class SettingsTests(unittest.TestCase):
    def load(self, timeout: str) -> Settings:
        with tempfile.TemporaryDirectory() as directory:
            key_file = os.path.join(directory, "key")
            with open(key_file, "w", encoding="utf-8") as file:
                file.write("test-key")
            with patch.dict(
                os.environ,
                {
                    "OPENROUTER_API_KEY_FILE": key_file,
                    "JEV_REQUEST_TIMEOUT_SECONDS": timeout,
                },
                clear=True,
            ):
                return Settings.from_env()

    def test_configured_timeout_can_exceed_the_default(self) -> None:
        self.assertEqual(self.load("15").request_timeout_seconds, 15.0)

    def test_configured_timeout_remains_bounded(self) -> None:
        with self.assertRaisesRegex(RuntimeError, r"\(0, 3600\]"):
            self.load("3600.1")


def input_body(history: list[dict] | None = None, assessment_from: int | None = None) -> dict:
    return {
        "branches": BRANCHES,
        "latest_user": [{"kind": "text", "text": "Fix the typo."}],
        "visible_conversation": history or [],
        "history_partial": False,
        "assessment_from": assessment_from,
    }


def turn(label: str, status: str = "completed") -> dict:
    return {
        "branch_id": "smart_saving_simple",
        "user": [{"kind": "text", "text": label}],
        "status": status,
        "steps": [[{"kind": "tool_activity", "tool": "apply_patch", "status": "completed"}]],
    }


class DeciderHttpTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.calls: list[dict] = []
        self.authorization_headers: list[str | None] = []
        self.upstream_status = 200
        self.upstream_delay = 0.0
        self.upstream_body: bytes | None = None
        self.active_upstream = 0
        self.max_active_upstream = 0
        self.answer = {
            "answers": {
                "branch": {
                    "type": "choice",
                    "choice": "smart_saving_simple",
                },
                "competence": {"type": "score", "score": 1.2},
            }
        }

        async def upstream(request: web.Request) -> web.Response:
            self.active_upstream += 1
            self.max_active_upstream = max(self.max_active_upstream, self.active_upstream)
            try:
                self.authorization_headers.append(request.headers.get("Authorization"))
                self.calls.append(await request.json())
                if self.upstream_delay:
                    await asyncio.sleep(self.upstream_delay)
                if self.upstream_body is not None:
                    return web.Response(body=self.upstream_body, status=self.upstream_status)
                return web.json_response(self.answer, status=self.upstream_status)
            finally:
                self.active_upstream -= 1

        upstream_app = web.Application()
        upstream_app.router.add_post("/api/alpha/decisions", upstream)
        self.upstream = TestServer(upstream_app)
        await self.upstream.start_server()

    async def asyncTearDown(self) -> None:
        await self.upstream.close()

    def settings(self, mode: str = "auto", **changes: object) -> Settings:
        values = dict(
            mode=mode,
            api_key="secret",
            upstream_url=str(self.upstream.make_url("/api/alpha/decisions")),
            model="typesafe/jev-1.13",
            request_timeout_seconds=1.0,
            max_state_tokens=24_000,
            max_concurrency=8,
            simple_threshold=0.8,
            competence_floor=0.5,
            inbound_header_name=None,
            inbound_header_value=None,
        )
        values.update(changes)
        return Settings(**values)

    async def client(self, settings: Settings) -> TestClient:
        client = TestClient(TestServer(create_app(settings)))
        await client.start_server()
        self.addAsyncCleanup(client.close)
        return client

    async def test_auto_asks_choice_and_score_in_one_upstream_request(self) -> None:
        client = await self.client(self.settings())
        response = await client.post("/v1/decisions", json=input_body([turn("old")], 0))
        self.assertEqual(response.status, 200)
        self.assertEqual(
            await response.json(),
            {
                "branch_id": "smart_saving_simple",
                "assessment": {"score": 0.6, "partial": False},
            },
        )
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(set(self.calls[0]["questions"]), {"branch", "competence"})
        self.assertEqual(self.authorization_headers, ["Bearer secret"])

    async def test_first_turn_omits_score_question_and_assessment(self) -> None:
        client = await self.client(self.settings())
        response = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(response.status, 200)
        self.assertEqual(await response.json(), {"branch_id": "smart_saving_simple"})
        self.assertEqual(set(self.calls[0]["questions"]), {"branch"})

    async def test_logs_explain_rule_override_and_correlate_response(self) -> None:
        self.answer = {"answers": {
            "complexity": {"type": "choice", "choice": "smart_saving_complex",
                           "probabilities": {"smart_saving_simple": 0.1, "smart_saving_complex": 0.9}},
        }}
        client = await self.client(self.settings("rules", simple_threshold=0))
        with self.assertLogs("jev_decider", level="INFO") as captured:
            response = await client.post("/v1/decisions", json=input_body())
            self.assertEqual((await response.json())["branch_id"], "smart_saving_simple")
        events = [json.loads(record.getMessage()) for record in captured.records]
        self.assertEqual([event["event"] for event in events], ["decision_started", "decision_completed"])
        result = events[-1]
        self.assertEqual(result["decision_id"], response.headers["X-Jev-Decision-Id"])
        self.assertEqual(events[0]["decision_id"], result["decision_id"])
        self.assertEqual(result["upstream_choice"], "smart_saving_complex")
        self.assertEqual(result["branch_id"], "smart_saving_simple")
        self.assertEqual(result["simple_threshold"], 0)
        self.assertEqual(result["reason"], "economy_eligible")
        self.assertEqual(result["competence_status"], "missing")
        self.assertGreaterEqual(result["duration_ms"], result["queue_ms"])
        self.assertNotIn("Fix the typo", " ".join(captured.output))
        self.assertNotIn("secret", " ".join(captured.output))

    async def test_logs_guard_trim_and_failures(self) -> None:
        self.answer = {"answers": {
            "complexity": {"type": "choice", "choice": "smart_saving_simple",
                           "probabilities": {"smart_saving_simple": 0.95, "smart_saving_complex": 0.05}},
            "competence": {"type": "score", "score": 0.2},
        }}
        client = await self.client(self.settings("rules", max_state_tokens=900))
        with self.assertLogs("jev_decider", level="INFO") as captured:
            response = await client.post("/v1/decisions", json=input_body([turn("x" * 400), turn("recent")], 0))
            self.assertEqual(response.status, 200)
        result = json.loads(captured.records[-1].getMessage())
        self.assertEqual(result["reason"], "competence_guard")
        self.assertEqual(result["competence"], 0.1)
        self.assertTrue(result["target_trimmed"])
        self.assertLess(result["retained_turns"], result["input_turns"])
        self.upstream_status = 429
        with self.assertLogs("jev_decider", level="INFO") as captured:
            response = await client.post("/v1/decisions", json=input_body())
        result = json.loads(captured.records[-1].getMessage())
        self.assertEqual(response.status, 502)
        self.assertEqual(result["upstream_status"], 429)
        self.assertEqual(result["error"], "upstream_rejected")
        self.assertEqual(result["phase"], "upstream")

    async def test_timeout_logs_terminal_event(self) -> None:
        self.upstream_delay = 0.1
        client = await self.client(self.settings(request_timeout_seconds=0.02))
        with self.assertLogs("jev_decider", level="INFO") as captured:
            response = await client.post("/v1/decisions", json=input_body())
        result = json.loads(captured.records[-1].getMessage())
        self.assertEqual(response.status, 504)
        self.assertEqual(result["error"], "upstream_timeout")
        self.assertEqual(result["event"], "decision_completed")

    async def test_openapi_route_is_served_without_legacy_aliases(self) -> None:
        document = json.loads(
            (Path(__file__).resolve().parents[3] / "api/decision.openapi.json").read_text()
        )
        self.assertEqual(set(document["paths"]), {"/v1/decisions"})
        client = await self.client(self.settings())
        for old_path in ("/classify", "/v1/classify"):
            response = await client.post(old_path, json=input_body())
            self.assertEqual(response.status, 404)
        self.assertEqual(self.calls, [])
        response = await client.post(next(iter(document["paths"])), json=input_body())
        self.assertEqual(response.status, 200)
        self.assertEqual(await response.json(), {"branch_id": "smart_saving_simple"})
        self.assertEqual(len(self.calls), 1)

    async def test_questions_treat_repeat_summary_and_unknown_as_valid_rounds_not_feedback(self) -> None:
        client = await self.client(self.settings())
        for latest in ("same task", "Summary: continue the same task"):
            body = input_body([turn("same task")], 0)
            body["latest_user"] = [{"kind": "text", "text": latest}]
            body["visible_conversation"][0]["status"] = "unknown"
            response = await client.post("/v1/decisions", json=body)
            self.assertEqual(response.status, 200)
        self.assertEqual(len(self.calls), 2)
        questions = self.calls[0]["questions"]
        self.assertIn("repeat prior text", questions["branch"]["instructions"])
        self.assertIn("unknown terminal status", questions["competence"]["instructions"])
        self.assertIn("not by itself praise or a complaint", questions["competence"]["instructions"])

    async def test_latest_user_remains_required_and_non_empty(self) -> None:
        client = await self.client(self.settings())
        for invalid in (None, []):
            body = input_body()
            body["latest_user"] = invalid
            response = await client.post("/v1/decisions", json=body)
            self.assertEqual(response.status, 400)
        missing = input_body()
        del missing["latest_user"]
        response = await client.post("/v1/decisions", json=missing)
        self.assertEqual(response.status, 400)
        self.assertEqual(self.calls, [])

    async def test_outbound_client_honors_standard_proxy_environment(self) -> None:
        proxy_calls: list[str] = []

        async def proxy(request: web.Request) -> web.Response:
            proxy_calls.append(request.raw_path)
            return web.json_response(self.answer)

        proxy_app = web.Application()
        proxy_app.router.add_route("*", "/{tail:.*}", proxy)
        proxy_server = TestServer(proxy_app)
        await proxy_server.start_server()
        self.addAsyncCleanup(proxy_server.close)
        proxy_url = str(proxy_server.make_url("/")).rstrip("/")

        with patch.dict(
            os.environ,
            {
                "HTTP_PROXY": proxy_url,
                "http_proxy": proxy_url,
                "NO_PROXY": "",
                "no_proxy": "",
            },
        ):
            client = await self.client(self.settings())
            response = await client.post("/v1/decisions", json=input_body())

        self.assertEqual(response.status, 200)
        self.assertEqual(await response.json(), {"branch_id": "smart_saving_simple"})
        self.assertEqual(len(proxy_calls), 1)
        self.assertEqual(self.calls, [])

    async def test_rules_uses_complexity_for_opportunity_and_score_only_as_guard(self) -> None:
        self.answer = {
            "answers": {
                "complexity": {
                    "type": "choice",
                    "choice": "smart_saving_simple",
                    "probabilities": {
                        "smart_saving_simple": 0.95,
                        "smart_saving_complex": 0.05,
                    },
                },
                "competence": {"type": "score", "score": 0.2},
            }
        }
        client = await self.client(self.settings("rules"))
        response = await client.post("/v1/decisions", json=input_body([turn("old")], 0))
        body = await response.json()
        self.assertEqual(body["branch_id"], "smart_saving_complex")
        self.assertEqual(body["assessment"]["score"], 0.1)
        self.assertEqual(len(self.calls), 1)

    async def test_invalid_optional_score_is_omitted_without_second_call(self) -> None:
        self.answer["answers"]["competence"] = {"type": "score", "score": 9}
        client = await self.client(self.settings())
        response = await client.post("/v1/decisions", json=input_body([turn("old")], 0))
        self.assertEqual(await response.json(), {"branch_id": "smart_saving_simple"})
        self.assertEqual(len(self.calls), 1)

    async def test_trimming_removes_whole_old_turns_and_marks_target_partial(self) -> None:
        history = [turn("old-" + "x" * 400), turn("recent")]
        client = await self.client(self.settings(max_state_tokens=900))
        response = await client.post("/v1/decisions", json=input_body(history, 0))
        body = await response.json()
        self.assertEqual(response.status, 200, body)
        self.assertTrue(body["assessment"]["partial"])
        state = self.calls[0]["state"]
        self.assertEqual([item["user"][0]["text"] for item in state["visible_conversation"]], ["recent"])
        self.assertEqual(state["latest_user"][0]["text"], "Fix the typo.")
        self.assertEqual(state["assessment_from"], 0)

    async def test_required_choice_failure_is_sanitized_and_not_retried(self) -> None:
        self.answer = {"answers": {"branch": {"type": "choice", "choice": "other", "probabilities": {"other": 1.0}}}}
        client = await self.client(self.settings())
        response = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(response.status, 502)
        self.assertEqual(await response.json(), {"error": "upstream_invalid"})
        self.assertEqual(len(self.calls), 1)

    async def test_upstream_rejections_are_sanitized_and_not_retried(self) -> None:
        client = await self.client(self.settings())
        for status in (401, 402, 429):
            with self.subTest(status=status):
                self.upstream_status = status
                response = await client.post("/v1/decisions", json=input_body())
                self.assertEqual(response.status, 502)
                self.assertEqual(
                    await response.json(),
                    {"error": "upstream_rejected", "upstream_status": status},
                )
        self.assertEqual(len(self.calls), 3)

    async def test_slow_upstream_consumes_single_request_budget_without_retry(self) -> None:
        self.upstream_delay = 0.10
        client = await self.client(self.settings(request_timeout_seconds=0.02))
        response = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(response.status, 504)
        self.assertEqual(await response.json(), {"error": "upstream_timeout"})
        self.assertEqual(len(self.calls), 1)

    async def test_oversized_upstream_response_is_rejected(self) -> None:
        self.upstream_body = json.dumps({"padding": "x" * (64 * 1024)}).encode()
        client = await self.client(self.settings())
        response = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(response.status, 502)
        self.assertEqual(await response.json(), {"error": "upstream_invalid"})
        self.assertEqual(len(self.calls), 1)

    async def test_fixed_state_over_budget_returns_413_without_upstream_call(self) -> None:
        body = input_body()
        body["latest_user"] = [{"kind": "text", "text": "x" * 500}]
        client = await self.client(self.settings(max_state_tokens=100))
        response = await client.post("/v1/decisions", json=body)
        self.assertEqual(response.status, 413)
        self.assertEqual(len(self.calls), 0)

    async def test_concurrency_limit_bounds_simultaneous_upstream_calls(self) -> None:
        self.upstream_delay = 0.05
        client = await self.client(self.settings(max_concurrency=1))
        first, second = await asyncio.gather(
            client.post("/v1/decisions", json=input_body()),
            client.post("/v1/decisions", json=input_body()),
        )
        self.assertEqual((first.status, second.status), (200, 200))
        self.assertEqual(self.max_active_upstream, 1)
        self.assertEqual(len(self.calls), 2)

    async def test_optional_inbound_header_and_strict_request(self) -> None:
        client = await self.client(self.settings(inbound_header_name="X-Decision-Key", inbound_header_value="value"))
        denied = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(denied.status, 401)
        invalid = await client.post(
            "/v1/decisions",
            json={**input_body(), "instructions": "old contract"},
            headers={"X-Decision-Key": "value"},
        )
        self.assertEqual(invalid.status, 400)
        self.assertEqual(len(self.calls), 0)

    async def test_health_does_not_call_upstream(self) -> None:
        client = await self.client(self.settings())
        response = await client.get("/health")
        self.assertEqual(response.status, 200)
        self.assertEqual(await response.json(), {"status": "ok"})
        self.assertEqual(self.calls, [])

    async def test_trimmed_non_target_history_marks_state_partial_not_assessment(self) -> None:
        history = [turn("old-" + "x" * 500), turn("target")]
        client = await self.client(self.settings(max_state_tokens=1000))
        response = await client.post("/v1/decisions", json=input_body(history, 1))
        body = await response.json()
        self.assertEqual(response.status, 200, body)
        self.assertFalse(body["assessment"]["partial"])
        self.assertTrue(self.calls[0]["state"]["history_partial"])
        self.assertEqual(self.calls[0]["state"]["assessment_from"], 0)

    async def test_rules_formula_boundaries_and_missing_score(self) -> None:
        client = await self.client(self.settings("rules"))
        cases = [
            (0.80, None, "smart_saving_simple"),
            (0.799, 2.0, "smart_saving_complex"),
            (0.90, 1.0, "smart_saving_simple"),
            (0.90, 0.999, "smart_saving_complex"),
            (0.90, 0.0, "smart_saving_complex"),
        ]
        for probability, score, expected in cases:
            with self.subTest(probability=probability, score=score):
                answers = {
                    "complexity": {
                        "type": "choice",
                        "choice": (
                            "smart_saving_simple"
                            if probability >= 0.5
                            else "smart_saving_complex"
                        ),
                        "probabilities": {
                            "smart_saving_simple": probability,
                            "smart_saving_complex": 1 - probability,
                        },
                    }
                }
                if score is not None:
                    answers["competence"] = {"type": "score", "score": score}
                self.answer = {"answers": answers}
                response = await client.post(
                    "/v1/decisions", json=input_body([turn("previous")], 0)
                )
                self.assertEqual(response.status, 200)
                self.assertEqual((await response.json())["branch_id"], expected)

    async def test_rules_rejects_non_binary_branches_before_upstream(self) -> None:
        body = input_body()
        body["branches"] = {"a": "A", "b": "B", "c": "C"}
        client = await self.client(self.settings("rules"))
        response = await client.post("/v1/decisions", json=body)
        self.assertEqual(response.status, 400)
        self.assertEqual(len(self.calls), 0)

    async def test_rules_rejects_invalid_probability_distribution(self) -> None:
        self.answer = {
            "answers": {
                "complexity": {
                    "type": "choice",
                    "choice": "smart_saving_simple",
                    "probabilities": {
                        "smart_saving_simple": 0.9,
                        "smart_saving_complex": 0.9,
                    },
                }
            }
        }
        client = await self.client(self.settings("rules"))
        response = await client.post("/v1/decisions", json=input_body())
        self.assertEqual(response.status, 502)
        self.assertEqual(await response.json(), {"error": "upstream_invalid"})
        self.assertEqual(len(self.calls), 1)


if __name__ == "__main__":
    unittest.main()
