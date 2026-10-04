#!/usr/bin/env python3
"""Shared native product-fixture mechanics; no native Agent or product acceptance claim."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'crates/daemon/tests/support'))
from agent_product_support import apply_settings, run_native_command


class SettingsJourneyTests(unittest.TestCase):
    def test_each_facet_applies_the_accepted_preview_and_observes_current_state(self):
        for facet in ('model', 'collaboration'):
            for intent in ('configure', 'restore'):
                with self.subTest(facet=facet, intent=intent):
                    calls = []
                    spec = {'context_id': 'context', facet: {'intent': intent}}
                    canonical = dict(spec, access_token={'intent': 'keep'})
                    state = {'state': 'configured', 'collaboration': {'state': 'restored'}}
                    command = 'agents restore' if intent == 'restore' else 'agents connect'

                    def preview(name, body):
                        calls.append(name)
                        if name == 'agents connect status':
                            self.assertEqual(body['context_id'], 'context')
                            return state
                        self.assertEqual(body, {'spec': spec})
                        return {'applicable': True, 'spec': canonical, 'accept_digest': 'accepted',
                                'dependency_digest': 'dependencies', 'expected_revisions': {'settings': 7},
                                'resident_service': {'login_item_required': True}}

                    def apply(name, body):
                        calls.append(name)
                        self.assertEqual(body, {'spec': canonical, 'accept_digest': 'accepted',
                            'dependency_digest': 'dependencies', 'expected_revisions': {'settings': 7},
                            'idempotency_key': 'one-operation',
                            'login_item': {'before': 'not_registered', 'after': 'enabled', 'created': True}})
                        return 0, {'data': {'state': 'succeeded'}}

                    _, observed = apply_settings(SimpleNamespace(preview=preview, cli=apply), spec, 'one-operation', facet)
                    self.assertIs(observed, state)
                    self.assertEqual(calls, [command + ' preview', command + ' apply', 'agents connect status'])

    def test_blocked_or_unfinished_operations_cannot_be_reported_as_configured(self):
        for applicable in (False, True):
            calls = []
            def preview(command, body):
                calls.append(command)
                self.assertNotEqual(command, 'agents connect status')
                return dict(applicable=applicable, spec=body['spec'], accept_digest='a',
                            dependency_digest='d', expected_revisions={})
            def apply(command, body):
                calls.append(command)
                return 0, {'data': {'state': 'awaiting_confirmation'}}
            with self.assertRaises(AssertionError):
                apply_settings(SimpleNamespace(preview=preview, cli=apply),
                               {'context_id': 'c', 'model': {'intent': 'configure'}}, 'key', 'model')
            self.assertEqual(len(calls), 2 if applicable else 1)


class NativeProcessTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.product = SimpleNamespace(project=Path(self.root.name), env=dict(os.environ), outputs=[])

    def test_output_is_retained_privately_and_native_verdict_stays_with_the_caller(self):
        code = "import os,sys; print(os.environ['FIXTURE_VALUE']); print('private-stderr',file=sys.stderr)"
        output = run_native_command(self.product, [sys.executable, '-c', code], timeout=5,
                                    label='fixture', env=dict(self.product.env, FIXTURE_VALUE='receipt'))
        self.assertEqual(output, b'receipt\n')
        self.assertEqual(self.product.outputs, [b'receipt\n', b'private-stderr\n'])
        with self.assertRaisesRegex(AssertionError, '^fixture failed; inspect private diagnostics$'):
            run_native_command(self.product, [sys.executable, '-c', "print('private-failure');exit(2)"],
                               timeout=5, label='fixture')
        self.assertIn(b'private-failure\n', self.product.outputs)

    def test_timeout_reaps_its_process_and_preserves_an_independent_neighbor(self):
        neighbor = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(30)'], start_new_session=True)
        try:
            code = 'import os,signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);print(os.getpid(),flush=True);time.sleep(30)'
            with self.assertRaisesRegex(AssertionError, '^fixture exceeded its deadline$'):
                run_native_command(self.product, [sys.executable, '-c', code], timeout=1, label='fixture')
            pid = int(self.product.outputs[0])
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
            self.assertIsNone(neighbor.poll())
        finally:
            neighbor.terminate()
            neighbor.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
