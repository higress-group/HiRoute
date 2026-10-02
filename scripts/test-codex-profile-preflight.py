#!/usr/bin/env python3
"""Harness outcome checks; these do not establish product/client success."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'preflight', Path(__file__).with_name('codex-profile-preflight.py'))
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


def stream(*events):
    return b'\n'.join(json.dumps(event).encode() for event in events)


ANSWER = {'type': 'item.completed', 'item': {'type': 'agent_message', 'text': 'PROFILE_OK'}}
COMPLETE = {'type': 'turn.completed'}


class OutcomeTests(unittest.TestCase):
    def test_exit_zero_or_partial_answer_is_not_success(self):
        for output in (b'', b'not json', stream(ANSWER), stream(COMPLETE)):
            with self.subTest(output=output):
                self.assertFalse(preflight.codex_completed(output, 0))

    def test_error_after_answer_does_not_pass(self):
        for event in ({'type': 'turn.failed'}, {'type': 'error'}):
            self.assertFalse(preflight.codex_completed(stream(ANSWER, COMPLETE, event), 0))

    def test_completed_real_client_event_shape(self):
        self.assertTrue(preflight.codex_completed(stream(
            {'type': 'thread.started', 'thread_id': 'test'}, ANSWER, COMPLETE), 0))

    def test_nonzero_exit_does_not_pass_even_with_success_events(self):
        self.assertFalse(preflight.codex_completed(stream(ANSWER, COMPLETE), 1))

    def test_tool_output_cannot_impersonate_an_answer(self):
        self.assertFalse(preflight.codex_completed(stream(
            {'type': 'item.completed', 'item': {'type': 'command_execution',
                                               'text': 'PROFILE_OK'}}, COMPLETE), 0))


if __name__ == '__main__':
    unittest.main()
