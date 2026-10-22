"""Clean callback text cannot identify a decorated final Hermes User instance."""
import copy
import json
import types
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch

from test_hermes_plugin import configuration, plugin
from test_hermes_turn_input import context, registered


class ObservedInputTests(unittest.TestCase):
    def test_clean_continue_never_selects_an_old_occurrence_or_changes_final_native_content(self):
        for mode in ("chat_completions", "codex_responses", "anthropic_messages"):
            for reminder in (False, True):
                for content in ("[native memory] new task\ncontinue", [
                    {"type": "text", "text": "[voice/persistence override]"},
                    {"type": "text", "text": "continue"},
                    {"type": "image_url", "image_url": {"url": "controlled-image"}},
                ]):
                    with self.subTest(mode=mode, reminder=reminder, multipart=isinstance(content, list)):
                        messages = [{"role": "user", "content": "continue"},
                                    {"role": "assistant", "content": "old actual answer"},
                                    {"role": "user", "content": content}]
                        if reminder:
                            messages.append({"role": "user", "content": "native trailing reminder"})
                        key = "input" if mode == "codex_responses" else "messages"
                        payload = {key: messages, "extra_headers": {
                            "x-hermes-observed-input": "stale reverse ordinal selecting the old continue",
                            "X-HERMES-OBSERVED-INPUT": "another stale hint"}}
                        before = copy.deepcopy(payload)
                        adapter = plugin._adapter(configuration(mode))
                        with patch.object(plugin, "_lineage_header", return_value=None):
                            result = adapter(payload, **context(api_mode=mode, user_message="continue"))
                        self.assertEqual(payload, before)
                        self.assertIs(result["request"][key], messages)
                        headers = result["request"]["extra_headers"]
                        self.assertFalse(any(k.lower() == "x-hermes-observed-input" for k in headers))
                        metadata = json.loads(headers["X-HiRoute-Routing-Context"])
                        self.assertEqual(metadata["input"], {"kind": "observed_batch", "event_id": "turn"})
                        self.assertNotIn("continue", json.dumps(headers))

    def test_minimal_context_survives_missing_or_throwing_optional_lineage(self):
        adapter = plugin._adapter(configuration("chat_completions"))
        for failure in (None, OSError("controlled missing/locked DB"), RuntimeError("optional SDK failure")):
            with self.subTest(failure=type(failure).__name__):
                with patch.object(plugin, "_lineage_header", side_effect=failure, return_value=None):
                    result = adapter({}, **context(api_request_id="turn:api:3"))
                headers = result["request"]["extra_headers"]
                self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
                self.assertEqual(headers["X-Hermes-Session-Id"], "parent")
                self.assertEqual(json.loads(headers["X-HiRoute-Routing-Context"]), {
                    "version": 2, "agent": "hermes", "purpose": "main", "session_id": "parent",
                    "turn_id": "turn", "request_id": "turn:api:3",
                    "input": {"kind": "observed_batch", "event_id": "turn"}})
                self.assertNotIn("X-Hermes-Compression-Lineage", headers)

    def test_optional_ids_never_promote_task_or_callback_text_to_per_call_identity(self):
        adapter = plugin._adapter(configuration("chat_completions"))
        for invalid in (None, "", "x\nheader", "x" * 129, "\ud800"):
            with self.subTest(invalid=repr(invalid)), patch.object(plugin, "_lineage_header", return_value=None):
                result = adapter({}, **context(turn_id=invalid, api_request_id=invalid,
                                               task_id="not-a-turn", user_message="not-an-instance"))
                metadata = json.loads(result["request"]["extra_headers"]["X-HiRoute-Routing-Context"])
                self.assertEqual(metadata, {"version": 2, "agent": "hermes", "purpose": "main",
                                            "session_id": "parent", "input": {"kind": "observed_batch"}})

    def test_optional_hook_apis_are_unused_on_older_and_throwing_sdks(self):
        native = types.SimpleNamespace(middleware={})
        native.register_middleware = lambda name, callback: native.middleware.update({name: callback})
        native.register_hook = lambda *_: self.fail("optional callback cannot identify the final input")
        self.assertIs(registered(native), native)
        self.assertIn("llm_request", native.middleware)
        self.assertFalse(hasattr(plugin, "_ObservedInputs"))

    def test_concurrent_windows_and_retries_preserve_actual_per_call_ids_without_shared_input_cache(self):
        adapter = registered().middleware["llm_request"]
        def invoke(index):
            identity = context(session_id="window-" + str(index % 2), turn_id="same-native-turn",
                               api_request_id="same-native-turn:api:" + str(index))
            payload = {"messages": [{"role": "user", "content": "continue"}]}
            result = adapter(payload, **identity)
            metadata = json.loads(result["request"]["extra_headers"]["X-HiRoute-Routing-Context"])
            self.assertIs(result["request"]["messages"], payload["messages"])
            return metadata
        with patch.object(plugin, "_lineage_header", return_value=None), ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(invoke, range(32)))
        self.assertEqual(len({row["request_id"] for row in results}), 32)
        self.assertEqual(len({row["session_id"] for row in results}), 2)
        self.assertTrue(all(row["input"]["kind"] == "observed_batch" for row in results))


if __name__ == "__main__":
    unittest.main()
