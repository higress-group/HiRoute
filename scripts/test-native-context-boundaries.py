#!/usr/bin/env python3
"""Focused oracle/owned-shell checks, not real Agent acceptance."""
import http.client
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / 'crates/daemon/tests/support'))
import native_context_boundaries as boundary
from native_context_fixture import prepare


class NativeBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        home, project = self.root / 'home', self.root / 'project'
        home.mkdir()
        project.mkdir()
        self.fixture = prepare(home, home / '.codex', project, 'codex')

    def body(self, fixture, complete=False):
        state = fixture['boundary']
        body = {'tools': [{'name': 'exec_command'}], 'input': [
            {'role': 'user', 'content': state['marker'] + ' '.join(
                skill['discovery'] for skill in fixture['skills'])}]}
        if complete:
            body['input'].append({'type': 'function_call_output', 'call_id': 'native_context_execute',
                                  'output': ' '.join(skill[key] for skill in fixture['skills']
                                                     for key in ('contents', 'executed'))})
        return body

    def test_rejected_wrong_route_attempt_cannot_hide_behind_accepted_ledger(self):
        from native_context_fixture import NativeContextUpstream
        for path, token, model in [('/v1/responses', 'wrong', 'gpt-5.4'),
                                   ('/wrong', NativeContextUpstream.token, 'gpt-5.4'),
                                   ('/v1/responses', NativeContextUpstream.token, 'wrong')]:
            with self.subTest(path=path, model=model):
                source = NativeContextUpstream(self.root)
                try:
                    ledger = self.root / 'native-context-events.jsonl'
                    ledger.write_text(json.dumps({'state':'green', 'model':source.model}) + '\n')
                    boundary.assert_source_routing(source)
                    connection = http.client.HTTPConnection(*source.server.server_address)
                    connection.request('POST', path, json.dumps({'model':model}),
                                       {'Authorization':'Bearer ' + token})
                    response = connection.getresponse()
                    self.assertGreaterEqual(response.status, 400)
                    response.read()
                    connection.close()
                    self.assertEqual(len(boundary.source_events(source)), 1)
                    with self.assertRaisesRegex(AssertionError, 'rejected model request'):
                        boundary.assert_source_routing(source)
                finally:
                    source.close()

    def test_cross_routed_request_fails_before_tool_execution(self):
        target = boundary.prepare_heartbeat(self.fixture, 'target')
        body = self.body(target)
        body['input'][0]['content'] = body['input'][0]['content'].replace('WORKER-target', 'WORKER-neighbor')
        with self.assertRaisesRegex(AssertionError, 'other task source'):
            boundary.boundary_decision(target, body)

    def test_both_public_submissions_enter_before_either_returns(self):
        fixtures = [boundary.prepare_heartbeat(self.fixture, role) for role in ('target', 'neighbor')]
        arrived = threading.Barrier(2)

        def accepted(product, command, prompt):
            arrived.wait(timeout=1)
            plan = command.split('--plan ', 1)[1].split()[0]
            self.assertIn('WORKER-' + plan, prompt)
            return 0, {'data': {'task_id': 'task-' + plan, 'run_id': 'run-' + plan}}

        with patch.object(boundary, 'worker_cli', side_effect=accepted):
            runs = boundary.submit_concurrent_workers(SimpleNamespace(project=self.root), fixtures,
                                                      ('target', 'neighbor'))
        self.assertEqual([run['task_id'] for run in runs], ['task-target', 'task-neighbor'])

    def test_concurrent_submission_does_not_hide_one_rejection(self):
        fixtures = [boundary.prepare_heartbeat(self.fixture, role) for role in ('target', 'neighbor')]

        def accepted(product, command, prompt):
            if 'WORKER-neighbor' in prompt:
                raise AssertionError('neighbor admission rejected')
            return 0, {'data': {'task_id': 'target', 'run_id': 'run'}}

        with patch.object(boundary, 'worker_cli', side_effect=accepted):
            with self.assertRaisesRegex(AssertionError, 'neighbor admission rejected'):
                boundary.submit_concurrent_workers(SimpleNamespace(project=self.root), fixtures,
                                                   ('target', 'neighbor'))

    def test_heartbeat_requires_completed_native_skill_receipts(self):
        target = boundary.prepare_heartbeat(self.fixture, 'target')
        action = boundary.boundary_decision(target, self.body(target))
        self.assertEqual(action['id'], 'native_context_execute')
        action = boundary.boundary_decision(target, self.body(target, True))
        self.assertEqual(action['id'], 'native_context_heartbeat_target')
        self.assertIn(target['boundary']['script'], action['arguments']['cmd'])

    def test_tool_result_does_not_substitute_for_actual_child_receipt(self):
        target = boundary.prepare_heartbeat(self.fixture, 'target')
        body = self.body(target, True)
        body['input'].append({'type': 'function_call_output', 'call_id': 'native_context_heartbeat_target',
                              'output': target['receipt']})
        with patch.object(boundary, 'wait_until', side_effect=AssertionError('child not finished')):
            with self.assertRaisesRegex(AssertionError, 'child not finished'):
                boundary.boundary_decision(target, body)

    def test_owned_shell_stop_has_no_effect_on_neighbor_heartbeat(self):
        target = boundary.prepare_heartbeat(self.fixture, 'target')
        neighbor = boundary.prepare_heartbeat(self.fixture, 'neighbor')
        processes = []
        try:
            for fixture in (target, neighbor):
                processes.append(subprocess.Popen(['/bin/sh', fixture['boundary']['script']],
                                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                                  start_new_session=True))
            boundary.wait_until(lambda: all(boundary.heartbeat_size(item) > 0 for item in (target, neighbor)),
                                'fixture heartbeat absent', 5)
            # Signal only the child group created and still held by this test, never
            # the PID read by the product oracle. No native Agent is launched here.
            os.killpg(processes[0].pid, signal.SIGTERM)
            processes[0].wait(timeout=5)
            boundary.assert_cancelled_child_and_live_neighbor(target, neighbor)
            Path(neighbor['boundary']['release']).touch()
            processes[1].wait(timeout=5)
            self.assertEqual(Path(neighbor['boundary']['done']).read_text().strip(), neighbor['receipt'])
        finally:
            for fixture in (target, neighbor):
                Path(fixture['boundary']['release']).touch()
            for process in processes:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=5)

    def history(self):
        root = Path(self.fixture['config']) / 'sessions'
        root.mkdir()
        path = root / 'native-task.jsonl'
        path.write_text(json.dumps({'type': 'session_meta', 'payload': {'id': 'original-session'}})
                        + '\n' + json.dumps({'text': self.fixture['receipt']}) + '\n')
        return path

    def test_missing_history_rejection_restores_only_its_owned_transcript(self):
        history = self.history()
        original = history.read_bytes()
        with patch.object(boundary, 'worker_cli', return_value=(6, {
                'error': {'code': 'CAPABILITY_UNAVAILABLE'}})):
            result = boundary.missing_history_refuses_new_session(
                object(), self.fixture, {'task_id': 'task', 'run_id': 'run'}, [])
        self.assertEqual(result['native_session_id'], 'original-session')
        self.assertEqual(history.read_bytes(), original)

    def test_accepted_missing_history_must_fail_without_a_new_native_session(self):
        history = self.history()
        original = history.read_bytes()
        with patch.object(boundary, 'worker_cli', return_value=(0, {'data': {'run_id': 'failed-load'}})), \
                patch.object(boundary, 'wait_for_worker_result', return_value={'result': None}) as wait:
            boundary.missing_history_refuses_new_session(
                object(), self.fixture, {'task_id': 'task', 'run_id': 'run'}, [])
        self.assertEqual(wait.call_args.kwargs['expected'], 'failed')
        self.assertEqual(history.read_bytes(), original)

    def test_missing_history_rejects_new_attempts_even_when_the_source_rejects_them(self):
        history = self.history()
        original = history.read_bytes()
        task = {'task_id': 'task', 'run_id': 'run'}
        for protocol in ('responses', 'messages'):
            for rejection, status in (('credential', 401), ('endpoint', 404), ('model', 400)):
                source = boundary.NativeContextUpstream(self.root)
                try:
                    def rejected_request():
                        connection = http.client.HTTPConnection(*source.server.server_address, timeout=3)
                        try:
                            connection.request('POST', '/v1/' + ('wrong' if rejection == 'endpoint' else protocol),
                                json.dumps({'model': 'wrong' if rejection == 'model' else source.model}),
                                {'Authorization': 'Bearer ' + ('wrong' if rejection == 'credential' else source.token)})
                            response = connection.getresponse()
                            response.read()
                            self.assertEqual(response.status, status)
                        finally:
                            connection.close()

                    rejected_request()  # A pre-existing attempt is not a new prompt from Continue.
                    for outcome in ((6, {'error': {'code': 'CAPABILITY_UNAVAILABLE'}}),
                                    (0, {'data': {'run_id': 'failed-load'}})):
                        with self.subTest(protocol=protocol, rejection=rejection, accepted=outcome[0] == 0), \
                                patch.object(boundary, 'wait_for_worker_result', return_value={'result': None}):
                            with patch.object(boundary, 'worker_cli', return_value=outcome):
                                boundary.missing_history_refuses_new_session(object(), self.fixture, task, [source])
                            before = source.request_count()
                            def leaked_prompt(*args, **kwargs):
                                rejected_request()
                                return outcome
                            with patch.object(boundary, 'worker_cli', side_effect=leaked_prompt):
                                with self.assertRaisesRegex(AssertionError, 'missing history sent a model prompt'):
                                    boundary.missing_history_refuses_new_session(object(), self.fixture, task, [source])
                            self.assertEqual(source.request_count(), before + 1)
                            self.assertEqual(boundary.source_events(source), [])
                            self.assertEqual(history.read_bytes(), original)
                            self.assertFalse(history.with_suffix('.native-context-held').exists())
                finally:
                    source.close()

    def test_unrelated_rejection_does_not_satisfy_missing_history(self):
        history = self.history()
        original = history.read_bytes()
        with patch.object(boundary, 'worker_cli', return_value=(1, {
                'error': {'code': 'PERMISSION_DENIED'}})):
            with self.assertRaises(AssertionError):
                boundary.missing_history_refuses_new_session(
                    object(), self.fixture, {'task_id': 'task', 'run_id': 'run'}, [])
        self.assertEqual(history.read_bytes(), original)

    def test_missing_history_cannot_silently_create_a_replacement(self):
        history = self.history()

        def recreated(*args, **kwargs):
            history.write_text('replacement native history')
            return 6, {'error': {'code': 'CAPABILITY_UNAVAILABLE'}}

        with patch.object(boundary, 'worker_cli', side_effect=recreated):
            with self.assertRaisesRegex(AssertionError, 'unexpected replacement transcript'):
                boundary.missing_history_refuses_new_session(
                    object(), self.fixture, {'task_id': 'task', 'run_id': 'run'}, [])
        self.assertEqual(history.read_text(), 'replacement native history')
        self.assertTrue(history.with_suffix('.native-context-held').exists())


if __name__ == '__main__':
    unittest.main()
