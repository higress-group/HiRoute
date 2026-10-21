"""Observed content selection survives reminders without creating human provenance."""
import copy
import json
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch

from test_hermes_plugin import configuration, plugin
from test_hermes_turn_input import context, registered


def messages(text="原始任务\nexact input"):
    return {"messages": [{"role": "user", "content": text},
                         {"role": "assistant", "tool_calls": [{"id": "call"}]},
                         {"role": "tool", "content": "earlier actual result", "tool_call_id": "call"},
                         {"role": "user", "content": "later native reminder"}]}


class ObservedInputTests(unittest.TestCase):
    def test_registered_hook_binds_original_observed_input_after_reminder_and_session_rotation(self):
        native = registered()
        text = "原始任务\nexact input"
        native.hooks["pre_llm_call"](user_message=text, **context())
        payload = messages(text)
        original = copy.deepcopy(payload)
        result = native.middleware["llm_request"](payload, **context(session_id="compression-child"))
        headers = result["request"]["extra_headers"]
        hint = json.loads(headers["X-Hermes-Observed-Input"])
        self.assertEqual(hint, {"version": 1, "kind": "observed_turn_input",
                                "reverse_user_index": 1, "text_sha256": plugin.sha256(text.encode()).hexdigest()})
        self.assertEqual(payload, original)
        self.assertIs(result["request"]["messages"], payload["messages"])
        self.assertIs(json.loads(headers["X-HiRoute-Routing-Context"])["continuation_requires_existing_turn"], True)
        self.assertNotIn(text, json.dumps(headers))

    def test_ambiguous_or_unmatched_text_and_missing_hook_keep_routing_without_a_hint(self):
        for variant in ("no-hook", "duplicate", "mismatch", "conflicting-hook", "unsupported", "oversized"):
            with self.subTest(variant=variant):
                native = registered()
                if variant != "no-hook":
                    native.hooks["pre_llm_call"](user_message="actual input", **context())
                payload = messages("actual input")
                if variant == "duplicate":
                    payload["messages"].insert(0, {"role": "user", "content": "actual input"})
                elif variant == "mismatch":
                    payload = messages("actual input\n")
                elif variant == "conflicting-hook":
                    native.hooks["pre_llm_call"](user_message="another input", **context())
                    native.hooks["pre_llm_call"](user_message="actual input", **context())
                elif variant == "unsupported":
                    payload["messages"][0]["content"] = [{"type": "image_url", "image_url": "local"}]
                elif variant == "oversized":
                    payload = messages("x" * (256 * 1024 + 1))
                result = native.middleware["llm_request"](payload, **context())
                headers = result["request"]["extra_headers"]
                self.assertNotIn("X-Hermes-Observed-Input", headers)
                self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
                self.assertIn("X-HiRoute-Routing-Context", headers)

    def test_ttl_capacity_conflicts_lock_pressure_and_hook_failures_are_recoverable(self):
        now = [0]
        cache = plugin._ObservedInputs(clock=lambda: now[0])
        payload = messages("actual input")
        remember = lambda **kw: cache.remember(user_message="actual input", **context(**kw))
        remember()
        self.assertIsNotNone(cache.hint(payload, **context()))
        cache.lock.acquire()
        try:
            self.assertIsNone(cache.hint(payload, **context()))
            cache.remember(user_message="unrelated", **context())
        finally:
            cache.lock.release()
        self.assertIsNotNone(cache.hint(payload, **context()))
        now[0] = 3601
        self.assertIsNone(cache.hint(payload, **context()))
        remember()
        for index in range(128):
            remember(turn_id="turn-" + str(index))
        self.assertEqual(len(cache.rows), 128)
        self.assertIsNone(cache.hint(payload, **context()))
        remember()
        self.assertIsNotNone(cache.hint(payload, **context()))
        adapter = plugin._adapter(configuration("chat_completions"), cache)
        with patch.object(cache, "hint", side_effect=RuntimeError("synthetic optional failure")):
            headers = adapter(payload, **context())["request"]["extra_headers"]
        self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
        self.assertNotIn("X-Hermes-Observed-Input", headers)
        self.assertIn("X-Hermes-Observed-Input", adapter(payload, **context())["request"]["extra_headers"])

    def test_concurrent_turns_remain_bound_to_their_own_input(self):
        cache = plugin._ObservedInputs()
        def invoke(index):
            identity = context(task_id="task-" + str(index), turn_id="turn-" + str(index))
            text = "observed input " + str(index)
            cache.remember(user_message=text, **identity)
            hint = json.loads(cache.hint(messages(text), **identity))
            self.assertEqual(hint["text_sha256"], plugin.sha256(text.encode()).hexdigest())
        with ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(invoke, range(32)))


if __name__ == "__main__":
    unittest.main()
