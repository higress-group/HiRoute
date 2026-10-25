#!/usr/bin/env python3
"""Check the product harness's drain protocol, not Gateway/native acceptance."""
import contextlib
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'crates/daemon/tests/support'))
import upgrade_requests


def status(calls=0, tasks=0, paused=True):
    return {'paused': paused, 'active_calls': calls, 'active_tasks': tasks}


class UpgradeDrainProtocolTests(unittest.TestCase):
    def check(self, states, *, clock=(0, 0, 0, 0), timeout=1):
        with patch.object(upgrade_requests, 'upgrade', side_effect=states) as call, \
                patch.object(upgrade_requests.time, 'sleep') as sleep, \
                patch.object(upgrade_requests.time, 'monotonic', side_effect=clock), \
                contextlib.redirect_stdout(io.StringIO()):
            result = upgrade_requests.wait_for_drain(object(), timeout=timeout)
            return result, [args.args[1] for args in call.call_args_list], sleep.call_count

    def test_response_tail_and_tasks_must_both_finish_before_replacement(self):
        result, actions, waits = self.check([status(calls=1), status(tasks=1), status()])
        self.assertEqual(result, status())
        self.assertEqual(actions, ['prepare', 'status', 'status'])
        self.assertEqual(waits, 2)

    def test_already_drained_needs_no_delay_or_repeated_prepare(self):
        _, actions, waits = self.check([status()])
        self.assertEqual(actions, ['prepare'])
        self.assertEqual(waits, 0)

    def test_a_permanently_active_call_is_a_bounded_failure_with_actual_counts(self):
        with self.assertRaisesRegex(AssertionError, "did not drain.*'active_calls': 1"):
            self.check([status(calls=1), status(calls=1)], clock=(0, 0, 2))

    def test_an_unexpected_resume_cannot_pass_even_when_counts_are_zero(self):
        with self.assertRaisesRegex(AssertionError, 'unexpectedly resumed'):
            self.check([status(calls=1), status(paused=False)])


if __name__ == '__main__':
    unittest.main()
