#!/usr/bin/env python3
"""Test the native-context oracle and isolated fixture, without launching any Agent."""
import http.client
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / 'crates/daemon/tests/support'))
import native_context_fixture as fixture
from native_context_product import assert_frozen_route, exact_history, publish_replacement_route, report_failure, save_context_source, wait_for_resumable_task
from publication_product import Product


class NativeContextOracleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def context(self, harness):
        root = self.root / harness
        home, project = root / 'home', root / 'project'
        home.mkdir(parents=True)
        project.mkdir()
        config = home / ('.codex' if harness == 'codex' else '.claude')
        return fixture.prepare(home, config, project, harness)

    def body(self, value, results=None, prompt='Use the named native skills'):
        text = prompt + ' ' + ' '.join(skill['discovery'] for skill in value['skills'])
        results = results or {}
        if value['harness'] == 'codex':
            return {'model': 'gpt-5.4', 'tools': [{'name': 'exec_command'}], 'input': [
                {'role': 'user', 'content': text},
                *[{'type': 'function_call_output', 'call_id': key, 'output': output}
                  for key, output in results.items()]]}
        return {'model': 'gpt-5.4', 'tools': [{'name': 'Skill'}, {'name': 'Bash'}], 'messages': [
            {'role': 'user', 'content': text},
            {'role': 'user', 'content': [{'type': 'tool_result', 'tool_use_id': key, 'content': output}
                                       for key, output in results.items()]}]}

    def receipt(self, value):
        return ' '.join(skill[key] for skill in value['skills'] for key in ('contents', 'executed'))

    def test_old_private_context_fails_without_native_discovery(self):
        for harness in ('codex', 'claude'):
            value = self.context(harness)
            body = self.body(value)
            if harness == 'codex':
                body['input'][0]['content'] = 'Use native-context-user and native-context-project'
            else:
                body['messages'][0]['content'] = 'Use native-context-user and native-context-project'
            with self.assertRaisesRegex(AssertionError, 'native skill discovery missing'):
                fixture.decision(value, body)

    def test_discovery_alone_is_not_skill_execution(self):
        value = self.context('codex')
        action = fixture.decision(value, self.body(value))
        self.assertEqual(action['kind'], 'tool')
        self.assertEqual(action['id'], 'native_context_execute')
        self.assertNotIn(value['receipt'], json.dumps(action))

    def test_actual_fixture_scripts_produce_the_independent_receipts(self):
        value = self.context('codex')
        action = fixture.decision(value, self.body(value))
        # Only our two synthetic read-only scripts run; no Agent or product binary.
        result = subprocess.run(['/bin/sh', '-c', action['arguments']['cmd']],
                                check=True, capture_output=True, text=True)
        for skill in value['skills']:
            self.assertIn(skill['contents'], result.stdout)
            self.assertIn(skill['executed'], result.stdout)
        answer = fixture.decision(value, self.body(value, {'native_context_execute': result.stdout}))
        self.assertEqual(answer['text'], value['receipt'])
        fixture.assert_preserved(value)

    def test_wrong_tool_id_or_prompt_echo_cannot_prove_execution(self):
        value = self.context('codex')
        action = fixture.decision(value, self.body(value, {'wrong': self.receipt(value)}, self.receipt(value)))
        self.assertEqual(action['kind'], 'tool')
        with self.assertRaisesRegex(AssertionError, 'receipt mismatch'):
            fixture.decision(value, self.body(value, {'native_context_execute': 'done'}))

    def test_continue_requires_prior_correlated_tool_history(self):
        for harness in ('codex', 'claude'):
            value = self.context(harness)
            for result in ({}, {'wrong-id': self.receipt(value)}):
                with self.assertRaisesRegex(AssertionError, 'prior tool result'):
                    fixture.decision(value, self.body(value, result, fixture.CONTINUE_PROMPT + self.receipt(value)))
            action = fixture.decision(value, self.body(
                value, {'native_context_execute': self.receipt(value)}, fixture.CONTINUE_PROMPT))
            self.assertTrue(action['continued'])

    def test_claude_invokes_both_native_skills_before_shell(self):
        value = self.context('claude')
        results = {}
        for skill in value['skills']:
            action = fixture.decision(value, self.body(value, results))
            self.assertEqual(action['name'], 'Skill')
            self.assertEqual(action['arguments'], {'skill': skill['name']})
            results[action['id']] = skill['contents']
        action = fixture.decision(value, self.body(value, results))
        self.assertEqual(action['name'], 'Bash')

    def test_claude_missing_skill_tool_fails_closed(self):
        value = self.context('claude')
        body = self.body(value)
        body['tools'] = [{'name': 'Bash'}]
        with self.assertRaisesRegex(AssertionError, 'Skill tool unavailable'):
            fixture.decision(value, body)

    def test_failed_claude_tool_cannot_satisfy_continue(self):
        value = self.context('claude')
        body = self.body(value, {'native_context_execute': self.receipt(value)}, fixture.CONTINUE_PROMPT)
        body['messages'][-1]['content'][0]['is_error'] = True
        with self.assertRaisesRegex(AssertionError, 'prior tool result'):
            fixture.decision(value, body)

    def test_setup_refuses_existing_skill_and_preserves_neighbor(self):
        value = self.context('codex')
        with self.assertRaises(FileExistsError):
            fixture.prepare(value['home'], value['config'], value['project'], 'codex')
        fixture.assert_preserved(value)
        Path(value['skills'][0]['skill']).write_text('changed')
        with self.assertRaisesRegex(AssertionError, 'changed native user material'):
            fixture.assert_preserved(value)

    def test_native_session_identity_is_independent_of_task_identity(self):
        value = self.context('codex')
        root = Path(value['config']) / 'sessions'
        root.mkdir()
        history = root / 'rollout.jsonl'
        history.write_text(json.dumps({'type': 'session_meta', 'payload': {'id': 'original-native-session'}})
                           + '\n' + json.dumps({'type': 'assistant', 'text': value['receipt']}) + '\n')
        self.assertEqual(exact_history(value), (str(history), 'original-native-session'))
        (root / 'new-session.jsonl').write_text(history.read_text())
        with self.assertRaisesRegex(AssertionError, 'exactly the task native transcript'):
            exact_history(value)

    def test_protocol_events_contain_correlated_tool_and_terminal_frames(self):
        for harness, protocol in (('codex', 'responses'), ('claude', 'messages')):
            value = self.context(harness)
            body = self.body(value)
            action = fixture.decision(value, body)
            frames = fixture.events(body, action, protocol)
            self.assertIn(action['id'], json.dumps(frames))
            self.assertEqual(frames[-1][0], 'response.completed' if protocol == 'responses' else 'message_stop')

    def assert_native_wire(self, harness, protocol):
        value = self.context(harness)
        controls = self.root / harness
        (controls / 'native-context.json').write_text(json.dumps(value))
        server = fixture.NativeContextUpstream(controls)
        self.addCleanup(server.close)
        connection = http.client.HTTPConnection(*server.server.server_address, timeout=3)
        self.addCleanup(connection.close)
        for results in ({}, {'native_context_execute': self.receipt(value)}):
            connection.request('POST', '/v1/' + protocol, json.dumps(self.body(value, results)),
                               {'Authorization': 'Bearer ' + server.token})
            response = connection.getresponse()
            wire = response.read().decode()
            self.assertEqual(response.status, 200)
            self.assertEqual(response.getheader('Content-Type'), 'text/event-stream')
            frames = []
            for frame in wire.strip().split('\n\n'):
                fields = dict(line.split(': ', 1) for line in frame.splitlines())
                event, payload = fields['event'], json.loads(fields['data'])
                self.assertEqual(payload.get('type'), event, 'native SSE JSON lost its event discriminator')
                if protocol == 'responses':
                    self.assertEqual(payload.get('sequence_number'), len(frames))
                frames.append((event, payload))
            self.assertGreater(len(frames), 2)
            if protocol == 'responses':
                self.assertEqual(frames[-1][0], 'response.completed')
                self.assertEqual(frames[-1][1]['response']['status'], 'completed')
                terminal = frames[-1][1]['response']['output'][0]
                self.assertEqual(terminal['type'], 'message' if results else 'function_call')
                if results:
                    self.assertEqual(terminal['content'][0]['text'], value['receipt'])
            else:
                self.assertEqual(frames[-1][0], 'message_stop')
                self.assertEqual(frames[-2][1]['delta']['stop_reason'], 'end_turn' if results else 'tool_use')

    def test_responses_native_wire_has_typed_ordered_terminal_frames(self):
        self.assert_native_wire('codex', 'responses')

    def test_messages_native_wire_has_typed_terminal_frames(self):
        self.assert_native_wire('claude', 'messages')

    def test_loopback_server_rejects_wrong_route_and_records_old_context_red(self):
        value = self.context('codex')
        (self.root / 'native-context.json').write_text(json.dumps(value))
        server = fixture.NativeContextUpstream(self.root)
        self.addCleanup(server.close)
        connection = http.client.HTTPConnection(*server.server.server_address, timeout=3)
        self.addCleanup(connection.close)
        body = {'model': server.model, 'input': 'old private HOME', 'tools': []}
        connection.request('POST', '/v1/responses', json.dumps(body), {'Authorization': 'Bearer wrong'})
        response = connection.getresponse()
        self.assertEqual(response.status, 401)
        response.read()
        connection.request('POST', '/v1/responses', json.dumps(body), {'Authorization': 'Bearer ' + server.token})
        response = connection.getresponse()
        self.assertEqual(response.status, 400)
        response.read()
        recorded = json.loads((self.root / 'native-context-events.jsonl').read_text())
        self.assertEqual(recorded['state'], 'red')
        self.assertIn('discovery missing', recorded['reason'])

    def test_product_native_roots_do_not_escape_to_parent_configuration(self):
        with patch.dict(os.environ, {'CODEX_HOME': '/daily/codex', 'CLAUDE_CONFIG_DIR': '/daily/claude',
                                    'CLAUDE_CODE_OAUTH_TOKEN': 'synthetic-parent-token',
                                    'CODEX_CONFIG': 'synthetic-parent-config', 'AWS_PROFILE': 'daily',
                                    'QODER_CONFIG_DIR': '/daily/qoder',
                                    'HIROUTE_WORKER_RECEIPT_DIR': '/daily/worker-receipts',
                                    'QODER_PERSONAL_ACCESS_TOKEN': 'synthetic-qoder-secret'}):
            product = Product(REPO, root=self.root / 'product')
        home = Path(product.env['HOME'])
        self.assertEqual(product.env['CODEX_HOME'], str(home / '.codex'))
        self.assertEqual(product.env['CLAUDE_CONFIG_DIR'], str(home / '.claude'))
        self.assertEqual(product.env['QODER_CONFIG_DIR'], str(home / '.qoder'))
        receipt_root = str(product.root / 'worker-receipts')
        self.assertEqual(product.env['HIROUTE_WORKER_RECEIPT_DIR'], receipt_root)
        # Borrowing a normally logged-in native HOME must not borrow HiRoute's
        # client submission journal or collide with earlier acceptance runs.
        product.env['HOME'] = '/explicitly-borrowed/native-home'
        self.assertEqual(product.env['HIROUTE_WORKER_RECEIPT_DIR'], receipt_root)
        neighbor = Product(REPO, root=self.root / 'neighbor-product')
        neighbor.env['HOME'] = product.env['HOME']
        self.assertNotEqual(neighbor.env['HIROUTE_WORKER_RECEIPT_DIR'], receipt_root)
        for key in ('CLAUDE_CODE_OAUTH_TOKEN', 'CODEX_CONFIG', 'AWS_PROFILE', 'QODER_PERSONAL_ACCESS_TOKEN'):
            self.assertNotIn(key, product.env)

    def test_proxy_trap_records_any_method_without_request_content(self):
        trap = fixture.NativeProxyTrap(self.root / 'proxy-attempts.log')
        self.addCleanup(trap.close)
        connection = http.client.HTTPConnection(*trap.server.server_address, timeout=3)
        self.addCleanup(connection.close)
        for method in ('CONNECT', 'POST', 'CUSTOM_METHOD'):
            connection.request(method, '/synthetic-private-path', 'synthetic-private-body',
                               {'Proxy-Authorization': 'synthetic-private-token'})
            response = connection.getresponse()
            self.assertEqual(response.status, 502)
            response.read()
        self.assertEqual(trap.path.read_text(), 'proxy-request\n' * 3)

    def test_claude_proxy_conflict_is_live_and_part_of_preservation_verdict(self):
        value = self.context('claude')
        upstream = fixture.NativeContextUpstream(self.root)
        self.addCleanup(upstream.close)
        environment = fixture.install_proxy_conflict(value, upstream)
        self.assertEqual({environment[name] for name in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY',
                                                         'http_proxy', 'https_proxy', 'all_proxy')},
                         {upstream.proxy_trap.url})
        self.assertEqual(environment['NO_PROXY'], environment['no_proxy'])
        self.assertNotIn('127.0.0.1', environment['NO_PROXY'])
        fixture.assert_preserved(value)
        connection = http.client.HTTPConnection(*upstream.proxy_trap.server.server_address, timeout=3)
        self.addCleanup(connection.close)
        connection.request('CONNECT', 'synthetic.invalid:443')
        response = connection.getresponse()
        response.read()
        with self.assertRaisesRegex(AssertionError, 'user-configured proxy'):
            fixture.assert_preserved(value)

    def test_native_sources_declare_a_budget_the_selected_client_can_admit(self):
        upstream = Mock(token='synthetic-token', model='synthetic-model')
        for harness, protocol, context_tokens in (('codex_cli', 'responses', 32768),
                ('claude_code', 'messages', 200000), ('qoder_cli', 'responses', 100000)):
            product = Mock(worker_work={'harness': harness, 'protocol': protocol})
            with patch('native_context_product.save_native_source', return_value={'binding_id': 'saved'}) as save:
                self.assertEqual(save_context_source(product, upstream, 'neighbor'), {'binding_id': 'saved'})
            self.assertEqual(save.call_args.kwargs['context_tokens'], context_tokens)
            self.assertEqual(save.call_args.kwargs['protocol'], protocol)
            self.assertEqual(save.call_args.kwargs['variant'], 'neighbor')

    def test_plan_replacement_changes_the_actual_route(self):
        product = Mock(root=self.root, worker_work={'protocol': 'responses', 'harness': 'codex_cli'})
        product.preview.return_value = {'preview_id': 'new-route'}
        upstream = Mock(controls=self.root)
        with patch('native_context_product.NativeContextUpstream', return_value=upstream), \
                patch('native_context_product.save_native_source', return_value={'binding_id': 'new-binding'}) as save, \
                patch('native_context_product.plan_change', return_value={'change': 'new-route'}) as change:
            self.assertIs(publish_replacement_route(product, {'harness': 'codex'}), upstream)
        self.assertEqual(upstream.model, 'gpt-5.5')
        self.assertEqual(save.call_args.kwargs['upstream_model_id'], 'gpt-5.5')
        self.assertEqual(change.call_args.kwargs['candidates'], [{'binding_id': 'new-binding'}])
        product.apply.assert_called_once()

    def test_frozen_route_requires_old_source_traffic_and_rejects_new_route_traffic(self):
        replacement = Mock(controls=self.root)
        continued = {'state': 'green', 'continued': True}
        with self.assertRaisesRegex(AssertionError, 'original source'):
            assert_frozen_route([continued], 1, replacement)
        assert_frozen_route([continued], 0, replacement)
        (self.root / 'native-context-events.jsonl').write_text(json.dumps(continued) + '\n')
        with self.assertRaisesRegex(AssertionError, 'newly published route'):
            assert_frozen_route([continued], 0, replacement)

    def test_failed_journey_preserves_diagnostics_even_if_stop_fails(self):
        product = Mock(repo=self.root)
        product.stop.side_effect = RuntimeError('stop failure')
        product.preserve_diagnostics.return_value = {'state': 'complete', 'current_boot': 'this-boot'}
        report = {'scenario': 'native-case', 'worker_harness': 'codex'}
        with patch('sys.stdout', new_callable=io.StringIO) as output:
            report_failure(product, report, 'actual-failure-stage')
        emitted = json.loads(output.getvalue())
        self.assertEqual(emitted['stage'], 'actual-failure-stage')
        self.assertEqual(emitted['state'], 'red')
        self.assertEqual(emitted['daemon_stop_state'], 'failed')
        self.assertEqual(emitted['diagnostics']['current_boot'], 'this-boot')
        product.preserve_diagnostics.assert_called_once()

    def test_diagnostics_copy_error_preserves_red_stage_and_primary_exception(self):
        product = Mock(repo=self.root)
        product.preserve_diagnostics.side_effect = OSError('sensitive exception detail')
        report = {'scenario': 'native-case', 'worker_harness': 'claude'}
        with patch('sys.stdout', new_callable=io.StringIO) as output:
            with self.assertRaisesRegex(ValueError, 'primary failure'):
                try:
                    raise ValueError('primary failure')
                except ValueError:
                    report_failure(product, report, 'native-history')
                    raise
        emitted = json.loads(output.getvalue())
        self.assertEqual(emitted['state'], 'red')
        self.assertEqual(emitted['stage'], 'native-history')
        self.assertEqual(emitted['diagnostics'], {'state': 'unavailable', 'reason': 'diagnostics_io'})
        self.assertNotIn('sensitive', output.getvalue())

    def test_native_history_waits_for_public_cleanup_and_resume_eligibility(self):
        ready = {'task_id': 'task', 'latest_run_id': 'run',
                 'run': {'state': 'succeeded', 'cleanup': 'complete'}, 'resumable_until_ms': 2000}
        responses = [(0, {'data': {'tasks': [dict(ready, resumable_until_ms=None)]}}),
                     (0, {'data': {'tasks': [ready]}})]
        with patch('native_context_product.worker_cli', side_effect=responses) as read, \
                patch('native_context_product.time.sleep'), patch('native_context_product.time.time', return_value=1):
            self.assertEqual(wait_for_resumable_task(object(), 'task', 'run'), ready)
        self.assertEqual(read.call_count, 2)

    def test_missing_resume_eligibility_cannot_pass_just_because_history_exists(self):
        task = {'task_id': 'task', 'latest_run_id': 'run',
                'run': {'state': 'succeeded', 'cleanup': 'complete'}, 'resumable_until_ms': None}
        with patch('native_context_product.worker_cli', return_value=(0, {'data': {'tasks': [task]}})):
            with self.assertRaisesRegex(AssertionError, 'continuation did not become ready'):
                wait_for_resumable_task(object(), 'task', 'run', timeout=0)

    def test_fixture_new_skill_ancestors_are_private_under_group_writable_umask(self):
        previous = os.umask(0o002)
        try:
            value = self.context('codex')
        finally:
            os.umask(previous)
        for base in (Path(value['home']), Path(value['project'])):
            for relative in ('.agents', '.agents/skills'):
                self.assertEqual((base / relative).stat().st_mode & 0o777, 0o700)

    def test_fixture_never_chmods_an_existing_native_parent(self):
        parent = self.root / 'existing-native-root'
        parent.mkdir(mode=0o755)
        original_mode = parent.stat().st_mode
        fixture.write_new(parent / 'owned/subdirectory/file', 'owned')
        self.assertEqual(parent.stat().st_mode, original_mode)
        self.assertEqual((parent / 'owned').stat().st_mode & 0o777, 0o700)

    def test_user_skill_uses_the_selected_claude_root(self):
        home, project = self.root / 'home', self.root / 'workspace'
        home.mkdir()
        project.mkdir()
        selected = home / 'custom-native-configuration'
        value = fixture.prepare(home, selected, project, 'claude')
        self.assertEqual(Path(value['skills'][0]['skill']).parent.parent, selected / 'skills')
        self.assertFalse((home / '.claude').exists())


if __name__ == '__main__':
    unittest.main()
