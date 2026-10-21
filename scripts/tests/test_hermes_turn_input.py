"""Public SDK hook -> producer negative constraint; no native product execution."""
import copy
import json
import sys
import threading
import types
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch

from test_hermes_plugin import configuration, plugin


def context(**changes):
    return {"provider": "custom", "model": "plan-alias", "base_url": "http://127.0.0.1:5837/v1",
            "api_mode": "chat_completions", "session_id": "parent", "task_id": "task", "turn_id": "turn",
            **changes}


class NativeContext:
    def __init__(self, fail=None, reject=None):
        self.hooks, self.middleware, self.fail, self.reject = {}, {}, fail, reject

    def register_hook(self, name, callback):
        self.hooks[name] = callback
        if name == self.fail:
            raise RuntimeError("optional native hook registration failed")
        if name == self.reject:
            return False

    def register_middleware(self, name, callback):
        self.middleware[name] = callback


def registered(native=None):
    native = native or NativeContext()
    package, config = types.ModuleType("hermes_cli"), types.ModuleType("hermes_cli.config")
    package.__path__ = []
    config.load_config_readonly = lambda: configuration("chat_completions")
    with patch.dict(sys.modules, {"hermes_cli": package, "hermes_cli.config": config}):
        plugin.register(native)
    return native


def observe(native, text, **changes):
    call = {**context(), **changes}
    # Exact public 0.20/current pre_llm_call payload, not middleware-only fields.
    native.hooks["pre_llm_call"](
        task_id=call["task_id"], turn_id=call["turn_id"], session_id=call["session_id"],
        user_message=text, conversation_history=[], is_first_turn=True,
        model=call["model"], platform="cli", parent_session_id="", sender_id="")


def request(native, messages, **changes):
    payload = {"messages": messages, "stream": True, "tools": [{"type": "function"}],
               "extra_headers": {"X-Preserve": "yes"}}
    before = copy.deepcopy(payload)
    with patch.object(plugin, "_lineage_header", return_value="existing-native-lineage"):
        result = native.middleware["llm_request"](payload, **{**context(), **changes})
    assert payload == before
    assert result["request"]["messages"] is messages
    return json.loads(result["request"]["extra_headers"]["X-HiRoute-Routing-Context"]), result


def flag(native, content, **changes):
    metadata, _ = request(native, [{"role": "user", "content": content}], **changes)
    return metadata["continuation_requires_existing_turn"]


class TurnInputTests(unittest.TestCase):
    def test_public_hook_binds_exact_original_without_sending_text_or_changing_native_kwargs(self):
        native = registered()
        self.assertEqual(set(native.hooks), {"pre_llm_call", "post_llm_call"})
        secret = "private real user input stays in native request only"
        observe(native, secret)
        metadata, result = request(native, [{"role": "user", "content": secret}])
        self.assertIs(metadata["continuation_requires_existing_turn"], False)
        self.assertEqual(metadata["turn_id"], "turn")
        headers = result["request"]["extra_headers"]
        self.assertEqual(headers["X-Hermes-Session-Id"], "parent")
        self.assertEqual(headers["X-Hermes-Compression-Lineage"], "existing-native-lineage")
        self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
        self.assertNotIn(secret, json.dumps(headers))
        self.assertIs(result["request"]["stream"], True)

    def test_no_hook_missing_hook_and_partial_registration_keep_the_model_adapter_conservative(self):
        missing = types.SimpleNamespace(middleware={})
        missing.register_middleware = lambda name, callback: missing.middleware.update({name: callback})
        for native in [registered(missing), registered(NativeContext("pre_llm_call")),
                       registered(NativeContext("post_llm_call")),
                       registered(NativeContext(reject="pre_llm_call")),
                       registered(NativeContext(reject="post_llm_call"))]:
            with self.subTest(native=native):
                if hasattr(native, "hooks") and "pre_llm_call" in native.hooks:
                    observe(native, "real input")  # partially registered callbacks cannot enable proof
                self.assertIs(flag(native, "real input"), True)
        native = registered()
        self.assertIs(flag(native, "real input"), True)

    def test_optional_input_read_failure_preserves_auth_identity_and_native_kwargs_for_all_modes(self):
        class BrokenInputs:
            def __init__(self, error):
                self.error, self.calls = error, 0

            def requires_existing_turn(self, request, context):
                self.calls += 1
                raise self.error("optional input read failed")

        for mode in ("chat_completions", "codex_responses", "anthropic_messages"):
            for error in (OSError, RuntimeError):
                with self.subTest(mode=mode, error=error.__name__):
                    inputs = BrokenInputs(error)
                    adapter = plugin._adapter(configuration(mode), inputs)
                    user = "private original user must stay only in the native body"
                    key = "input" if mode == "codex_responses" else "messages"
                    native_body = [{"role": "user", "content": user}]
                    tools = [{"type": "function", "function": {"name": "native_tool"}}]
                    payload = {"model": "plan-alias", key: native_body, "tools": tools,
                               "stream": True, "stream_options": {"include_usage": True},
                               "extra_headers": {"X-Preserve": "yes", "x-hiroute-token": "stale"}}
                    before = copy.deepcopy(payload)
                    with patch.object(plugin, "_lineage_header", return_value="existing-native-lineage"):
                        result = adapter(payload, **context(api_mode=mode))
                    self.assertEqual(inputs.calls, 1)
                    self.assertEqual(result["source"], "hiroute")
                    self.assertEqual(payload, before)
                    self.assertEqual(set(result["request"]), set(payload))
                    for field in (key, "tools", "stream", "stream_options"):
                        self.assertIs(result["request"][field], payload[field])
                    headers = result["request"]["extra_headers"]
                    self.assertEqual(headers["X-Preserve"], "yes")
                    self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
                    self.assertNotIn("x-hiroute-token", headers)
                    self.assertEqual(headers["X-HiRoute-Agent"], "hermes")
                    self.assertEqual(headers["X-Hermes-Session-Id"], "parent")
                    self.assertEqual(headers["X-Hermes-Compression-Lineage"], "existing-native-lineage")
                    metadata = json.loads(headers["X-HiRoute-Routing-Context"])
                    self.assertEqual(metadata, {"version": 1, "agent": "hermes", "session_id": "parent",
                                                "purpose": "main", "turn_id": "turn",
                                                "continuation_requires_existing_turn": True})
                    self.assertNotIn(user, json.dumps(headers))

    def test_main_tool_continuation_and_physical_rotation_keep_the_same_real_turn(self):
        native = registered()
        observe(native, "real input")
        messages = [{"role": "user", "content": "summary from older history"},
                    {"role": "assistant", "content": "previous output"},
                    {"role": "user", "content": "real input"},
                    {"role": "assistant", "tool_calls": [{"id": "tool-1"}]},
                    {"role": "tool", "tool_call_id": "tool-1", "content": "result"}]
        self.assertIs(request(native, messages)[0]["continuation_requires_existing_turn"], False)
        metadata, _ = request(native, messages, session_id="rotated-child")
        self.assertIs(metadata["continuation_requires_existing_turn"], False)
        self.assertEqual(metadata["session_id"], "rotated-child")

    def test_summary_continuation_and_merged_user_require_existing_turn_without_banner_guessing(self):
        native = registered()
        observe(native, "real input")
        for synthetic in ["summary", "continue", "summary\n\nreal input", "real input\n\nsummary"]:
            self.assertIs(flag(native, synthetic), True)
        messages = [{"role": "user", "content": "real input"},
                    {"role": "user", "content": "summary"}]
        self.assertIs(request(native, messages)[0]["continuation_requires_existing_turn"], True)
        # A human may literally type a summary-looking sentence: equality, not a prefix, decides.
        observe(native, "[CONTEXT COMPACTION — REFERENCE ONLY] a real user's words", turn_id="literal")
        self.assertIs(flag(native, "[CONTEXT COMPACTION — REFERENCE ONLY] a real user's words", turn_id="literal"), False)

    def test_only_unambiguous_plain_or_one_text_block_can_match(self):
        native = registered()
        observe(native, "real input")
        self.assertIs(flag(native, [{"type": "text", "text": "real input"}]), False)
        for value in [None, [], [{"type": "image", "source": "native"}],
                      [{"type": "text", "text": "real "}, {"type": "text", "text": "input"}],
                      [{"type": "text", "text": "real input", "unknown": True}],
                      {"text": "real input"}, "", " "]:
            with self.subTest(content=value):
                self.assertIs(flag(native, value), True)
        observe(native, [{"type": "text", "text": "real"}, {"type": "text", "text": " input"}], turn_id="multipart")
        self.assertIs(flag(native, "real input", turn_id="multipart"), True)
        observe(native, [{"type": "text", "text": "real input"}], turn_id="one-block")
        self.assertIs(flag(native, "real input", turn_id="one-block"), False)
        for trailing in ["unknown", {}, {"role": "unknown", "content": "not classified"}]:
            messages = [{"role": "user", "content": "real input"}, trailing]
            self.assertIs(request(native, messages)[0]["continuation_requires_existing_turn"], True)

    def test_exact_text_does_not_normalize_whitespace_or_unicode(self):
        native = registered()
        observe(native, " é ")
        self.assertIs(flag(native, " é "), False)
        for changed in ["é", " e\u0301 ", " É "]:
            self.assertIs(flag(native, changed), True)

    def test_parent_child_task_and_plugin_instances_do_not_share_input(self):
        parent, independent = registered(), registered()
        observe(parent, "parent input")
        observe(parent, "child input", turn_id="child-turn", session_id="child")
        observe(parent, "other task input", task_id="other-task")
        self.assertIs(flag(parent, "parent input"), False)
        self.assertIs(flag(parent, "child input", turn_id="child-turn", session_id="child"), False)
        self.assertIs(flag(parent, "parent input", turn_id="child-turn", session_id="child"), True)
        self.assertIs(flag(parent, "other task input", task_id="other-task"), False)
        self.assertIs(flag(independent, "parent input"), True)

    def test_overlapping_native_tasks_cannot_cross_match_or_remove_other_turns(self):
        native, barrier = registered(), threading.Barrier(16)
        def run(index):
            changes = {"turn_id": "turn-" + str(index), "session_id": "session-" + str(index)}
            text = "native input " + str(index)
            observe(native, text, **changes)
            barrier.wait(timeout=5)
            self.assertIs(flag(native, text, **changes), False)
            self.assertIs(flag(native, "another task's user", **changes), True)
            native.hooks["post_llm_call"](task_id="task", turn_id=changes["turn_id"])
            self.assertIs(flag(native, text, **changes), True)
        with ThreadPoolExecutor(max_workers=16) as pool:
            list(pool.map(run, range(16)))

    def test_completed_turn_clears_only_itself_and_expiration_is_not_refreshed_by_requests(self):
        native = registered()
        with patch.object(plugin.time, "monotonic", return_value=100):
            observe(native, "first input")
            observe(native, "second input", turn_id="next")
        native.hooks["post_llm_call"](task_id="task", turn_id="next")
        with patch.object(plugin.time, "monotonic", return_value=100 + plugin._TURN_INPUT_TTL - 1):
            self.assertIs(flag(native, "first input"), False)
            self.assertIs(flag(native, "second input", turn_id="next"), True)
        with patch.object(plugin.time, "monotonic", return_value=100 + plugin._TURN_INPUT_TTL):
            self.assertIs(flag(native, "first input"), True)

    def test_count_size_invalid_reobservation_and_conflicting_turns_are_bounded_unknowns(self):
        native = registered()
        for index in range(plugin._TURN_INPUT_LIMIT + 1):
            observe(native, "user", turn_id="turn-" + str(index))
        self.assertIs(flag(native, "user", turn_id="turn-0"), True)
        self.assertIs(flag(native, "user", turn_id="turn-" + str(plugin._TURN_INPUT_LIMIT)), False)
        observe(native, "x" * plugin._TURN_INPUT_BYTES)
        self.assertIs(flag(native, "x" * plugin._TURN_INPUT_BYTES), False)
        observe(native, "x" * (plugin._TURN_INPUT_BYTES + 1))
        self.assertIs(flag(native, "x" * plugin._TURN_INPUT_BYTES), True)
        unicode_text = "\U0001f600" * (plugin._TURN_INPUT_BYTES // 4)
        observe(native, unicode_text, turn_id="utf8")
        self.assertIs(flag(native, unicode_text, turn_id="utf8"), False)
        observe(native, unicode_text + "\U0001f600", turn_id="utf8")
        self.assertIs(flag(native, unicode_text, turn_id="utf8"), True)
        observe(native, "first", turn_id="conflict")
        observe(native, "different", turn_id="conflict")
        self.assertIs(flag(native, "first", turn_id="conflict"), True)
        self.assertIs(flag(native, "different", turn_id="conflict"), True)
        for value in ["\ud800", None, {"text": "user"}]:
            observe(native, value, turn_id="unsupported")
            self.assertIs(flag(native, "user", turn_id="unsupported"), True)

    def test_missing_native_keys_input_only_and_unknown_mode_stay_negative_without_mutation(self):
        native = registered()
        for changes in [{"task_id": None}, {"turn_id": ""}, {"task_id": "x" * 129}]:
            observe(native, "user", **changes)
            self.assertIs(flag(native, "user", **changes), True)
        observe(native, "user")
        payload = {"input": [{"role": "user", "content": "user"}], "stream": True}
        with patch.object(plugin, "_lineage_header", return_value=None):
            result = native.middleware["llm_request"](payload, **context())
        metadata = json.loads(result["request"]["extra_headers"]["X-HiRoute-Routing-Context"])
        self.assertIs(metadata["continuation_requires_existing_turn"], True)
        self.assertIs(result["request"]["input"], payload["input"])


if __name__ == "__main__":
    unittest.main()
