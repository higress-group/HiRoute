#!/usr/bin/env python3
"""Qoder fixture ownership and product oracles; no Agent or production verdict."""
import http.client
from contextlib import redirect_stdout
import io
import json
import os
import sqlite3
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / 'crates/daemon/tests/support'))
import native_context_fixture as oracle
import qoder_native_context as qoder
import collaboration_product as collaboration
from collaboration_fixture import MainAgentOracle, worker_decision, output_envelope
from native_compaction_product import CompactionOracle, PRESSURE_TOKENS, assert_native_compaction
import additional_model_fixture as model_fixture
import additional_model_product as model_product


class QoderFixtureContext:
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / 'borrowed-home'
        self.config = self.home / '.qoder'
        self.config.mkdir(parents=True)
        self.settings = self.config / 'settings.json'
        self.settings.write_text('{"model":"user-choice"}\n')
        self.settings.chmod(0o640)
        self.project = self.root / 'private-project'
        self.project.mkdir()
        self.product = SimpleNamespace(env={}, project=self.project, storage=self.root / 'storage')

    def context(self):
        with patch.dict(os.environ, {'HIROUTE_QODER_CONTEXT_HOME': str(self.home),
                                     'HIROUTE_QODER_CONFIG_DIR': str(self.config)}):
            qoder.select_context(self.product)
        return qoder.prepare_context(self.product)

    def body(self, fixture, results=None, prompt='Use the requested native skills'):
        return {'model': 'gpt-5.4', 'tools': [{'name': 'Skill'}, {'name': 'Bash'}], 'input': [
            {'role': 'user', 'content': prompt + ' '.join(item['discovery'] for item in fixture['skills'])},
            *[{'type': 'function_call_output', 'call_id': key, 'output': value}
              for key, value in (results or {}).items()],
            *[{'role': 'user', 'content': 'Base directory for this skill: ' +
               str(Path(item['skill']).parent) + '\n\n' + item['contents']}
              for item in fixture['skills']
              if 'native_context_skill_' + item['scope'] in (results or {})]]}



class QoderFixtureTests(QoderFixtureContext, unittest.TestCase):
    def test_small_context_requires_the_exact_output_budget_on_real_requests(self):
        fixture = self.context()
        fixture['expected_max_output_tokens'] = 4096
        body = self.body(fixture)
        for maximum in (None, 32768):
            if maximum is not None:
                body['max_output_tokens'] = maximum
            with self.assertRaisesRegex(AssertionError, 'frozen source output budget'):
                oracle.decision(fixture, body)
        body['max_output_tokens'] = 4096
        self.assertEqual(oracle.decision(fixture, body)['name'], 'Skill')

    def test_ordinary_skill_journey_rejects_native_compaction_instead_of_reissuing_tools(self):
        fixture = self.context()
        body = self.body(fixture, prompt='Create a detailed summary for summarization.')
        with self.assertRaisesRegex(AssertionError, 'unexpected Qoder compaction'):
            oracle.decision(fixture, body)

    def test_cancelled_history_uses_exact_task_identity_without_a_continue_binding(self):
        fixture = self.context()
        fixture['task_id'] = 'task/cancelled'
        owned = self.product.storage / 'delegation-workers/sessions/owned-task'
        owned.mkdir(parents=True)
        (owned / '.hiroute-native-root-v1.json').write_text(json.dumps({
            'task_id': fixture['task_id'], 'harness': 'qoder_cli'}))
        (owned / 'borrowed-native-context.json').write_text(json.dumps({
            'version': 1, 'harness': 'qoder_cli', 'context': {
                'home': fixture['home'], 'config_root': fixture['config'], 'workspace': fixture['project']}}))
        native_id, run_id = 'exact-cancelled-native-id', 'run/cancelled'
        task = {'task_id': fixture['task_id'], 'latest_run_id': run_id, 'plan': {'harness': 'qoder_cli'},
                'session': {'native_session_id': native_id, 'acp_session_id': native_id}}
        run = {'task_id': fixture['task_id'], 'run_id': run_id,
               'progress': {'state': 'cancelled', 'cleanup': 'complete'}}
        database = self.product.storage / 'live/runtime.db'
        database.parent.mkdir()
        with sqlite3.connect(database) as connection:
            connection.execute('CREATE TABLE delegation_tasks (workspace_id TEXT, task_id TEXT, record_json TEXT)')
            connection.execute('CREATE TABLE delegation_runs (workspace_id TEXT, run_id TEXT, record_json TEXT)')
            connection.execute('INSERT INTO delegation_tasks VALUES (?,?,?)', ('personal/default', fixture['task_id'], json.dumps(task)))
            connection.execute('INSERT INTO delegation_runs VALUES (?,?,?)', ('personal/default', run_id, json.dumps(run)))
        history = qoder.history_directory(fixture) / (native_id + '.jsonl')
        history.parent.mkdir(parents=True)
        history.write_text(fixture['receipt'])
        self.assertEqual(qoder.cancelled_history(fixture, run_id), (str(history), native_id))
        with self.assertRaises(FileNotFoundError):
            qoder.task_binding(fixture)
        with sqlite3.connect(database) as connection:
            run['task_id'] = 'task/other'
            connection.execute('UPDATE delegation_runs SET record_json=?', (json.dumps(run),))
        with self.assertRaisesRegex(AssertionError, 'cancelled task identity'):
            qoder.cancelled_history(fixture, run_id)

    def test_missing_explicit_context_does_not_fall_back_to_daily_home(self):
        with patch.dict(os.environ, {'HOME': str(self.home), 'QODER_CONFIG_DIR': str(self.config)}, clear=True):
            with self.assertRaisesRegex(AssertionError, 'explicitly select'):
                qoder.select_context(self.product)
        self.assertEqual(self.product.env, {})

    def test_borrowed_settings_and_neighbor_survive_owned_receipt_cleanup(self):
        neighbor = self.config / 'user-file'
        neighbor.write_text('keep user file')
        before = qoder.settings_snapshot(self.config)
        fixture = self.context()
        self.assertEqual(self.product.env['QODER_CONFIG_DIR'], str(self.config))
        qoder.cleanup_context(self.product)
        self.assertEqual(qoder.settings_snapshot(self.config), before)
        self.assertEqual(neighbor.read_text(), 'keep user file')
        self.assertFalse(Path(fixture['skills'][0]['skill']).exists())

    def test_partial_setup_failure_cleans_only_recorded_new_user_files(self):
        original = oracle.write_new
        def fail_project(path, text, executable=False):
            if path.is_relative_to(self.project):
                raise OSError('synthetic project write failure')
            original(path, text, executable)
        with patch.object(oracle, 'write_new', side_effect=fail_project):
            with self.assertRaisesRegex(OSError, 'project write failure'):
                self.context()
        qoder.cleanup_context(self.product)
        self.assertEqual(list((self.config / 'skills').iterdir()), [])
        self.assertEqual(self.settings.read_text(), '{"model":"user-choice"}\n')

    def test_new_collaboration_leaf_preserves_permissive_user_parent(self):
        self.context()
        parent = self.config / 'skills'
        parent.chmod(0o775)
        skill = qoder.prepare_collaboration_target(self.product)
        self.assertEqual(skill.parent.stat().st_mode & 0o777, 0o700)
        self.assertFalse(skill.exists(), 'fixture must not manufacture the collaboration Skill')
        qoder.cleanup_context(self.product)
        self.assertFalse(skill.parent.exists())
        self.assertEqual(parent.stat().st_mode & 0o777, 0o775)

    def test_existing_collaboration_leaf_and_skill_are_never_fixture_owned(self):
        self.context()
        directory = self.config / 'skills/hiroute-collaboration'
        directory.mkdir(mode=0o775)
        directory.chmod(0o775)
        skill = directory / 'SKILL.md'
        skill.write_text('existing user Skill')
        before = qoder.directory_identity(directory)
        self.assertEqual(qoder.prepare_collaboration_target(self.product), skill)
        self.assertFalse(hasattr(self.product, 'qoder_owned_collaboration_directory'))
        qoder.cleanup_context(self.product)
        self.assertEqual(qoder.directory_identity(directory), before)
        self.assertEqual(skill.read_text(), 'existing user Skill')

    def test_owned_collaboration_leaf_cleanup_refuses_new_contents_or_permissions(self):
        self.context()
        skill = qoder.prepare_collaboration_target(self.product)
        skill.write_text('material not restored by the product')
        with self.assertRaisesRegex(AssertionError, 'changed or nonempty collaboration'):
            qoder.cleanup_context(self.product)
        self.assertEqual(skill.read_text(), 'material not restored by the product')
        # Receipt cleanup has finished; this second attempt only checks the leaf.
        self.product.qoder_owned_files = {}
        skill.unlink()
        skill.parent.chmod(0o775)
        with self.assertRaisesRegex(AssertionError, 'changed or nonempty collaboration'):
            qoder.cleanup_context(self.product)
        self.assertEqual(skill.parent.stat().st_mode & 0o777, 0o775)

    def test_foreign_project_route_observation_is_part_of_the_product_verdict(self):
        fixture = self.context()
        upstream = oracle.NativeContextUpstream(self.root)
        self.addCleanup(upstream.close)
        qoder.install_project_conflict(fixture, upstream)
        oracle.assert_preserved(fixture)
        connection = http.client.HTTPConnection(*upstream.proxy_trap.server.server_address, timeout=3)
        self.addCleanup(connection.close)
        connection.request('POST', '/v1/responses', '{}')
        response = connection.getresponse()
        response.read()
        with self.assertRaisesRegex(AssertionError, 'foreign project model route'):
            oracle.assert_preserved(fixture)

    def test_cleanup_refuses_a_user_edit_to_owned_receipt(self):
        fixture = self.context()
        skill = Path(fixture['skills'][0]['skill'])
        skill.write_text('user edit after acceptance')
        with self.assertRaisesRegex(AssertionError, 'replaced Qoder fixture material'):
            qoder.cleanup_context(self.product)
        self.assertEqual(skill.read_text(), 'user edit after acceptance')

    def test_responses_require_both_native_skill_calls_and_shell_receipts(self):
        fixture = self.context()
        results = {}
        for item in fixture['skills']:
            action = oracle.decision(fixture, self.body(fixture, results))
            self.assertEqual((action['name'], action['arguments']), ('Skill', {'skill': item['name']}))
            results[action['id']] = 'Launching skill: ' + item['name']
        action = oracle.decision(fixture, self.body(fixture, results))
        self.assertEqual(action['name'], 'Bash')
        with self.assertRaisesRegex(AssertionError, 'receipt mismatch'):
            oracle.decision(fixture, self.body(fixture, {'native_context_execute': 'done'}))
        receipts = ' '.join(item[key] for item in fixture['skills'] for key in ('contents', 'executed'))
        with self.assertRaisesRegex(AssertionError, 'prior tool result'):
            oracle.decision(fixture, self.body(fixture, {'wrong': receipts}, oracle.CONTINUE_PROMPT + receipts))
        answer = oracle.decision(fixture, self.body(fixture, {'native_context_execute': receipts}, oracle.CONTINUE_PROMPT))
        self.assertEqual(answer['text'], 'continued-' + fixture['receipt'])

    def test_same_named_shadow_skill_cannot_prove_user_discovery(self):
        fixture = self.context()
        body = self.body(fixture, {'native_context_skill_user': 'Launching skill'})
        body['input'][-1]['content'] = ('Base directory for this skill: ' +
            str(Path(fixture['skills'][0]['skill']).parent) + '-shadow\n\n' + fixture['skills'][0]['contents'])
        with self.assertRaisesRegex(AssertionError, 'expected native Skill directory'):
            oracle.decision(fixture, body)


class QoderDelegationOracleTests(QoderFixtureContext, unittest.TestCase):
    def main_oracle(self):
        fixture = self.context()
        fixture.update(main_marker='private-main-marker', plan_id='published-plan',
                       artifact={'path': str(self.project / 'artifact'), 'receipt': 'worker-only-nonce'})
        skill = self.config / 'skills/hiroute-collaboration/SKILL.md'
        oracle.write_new(skill, '---\nname: hiroute-collaboration\n---\nUse hiroute worker plans before work.\n')
        events = [{'state': 'green', 'call_id': 'delegated_artifact'}]
        main = MainAgentOracle(fixture, skill, Path('/bin/sh'), lambda: events)
        return fixture, main

    def main_body(self, fixture, main, results=None, actual_directory=True):
        text = ('Base directory for this skill: ' + str(main.skill.parent) + '\n\n' + main.skill_body
                if actual_directory else 'Base directory for this skill: ' + str(main.skill.parent) + '-shadow\n\n' + main.skill_body)
        return {'tools': [{'name': 'Skill'}, {'name': 'Bash'}], 'input': [
            {'role': 'user', 'content': [{'type': 'input_text', 'text': fixture['main_marker'] + ' hiroute-collaboration'}]},
            *[{'type': 'function_call_output', 'call_id': key, 'output': value}
              for key, value in (results or {}).items()],
            {'role': 'user', 'content': [{'type': 'input_text', 'text': text}]}]}

    @staticmethod
    def envelope(data):
        return json.dumps({'schema_version': {'major': 2, 'minor': 0}, 'status': 'succeeded', 'data': data})

    def test_main_oracle_requires_actual_user_skill_before_listing_plans(self):
        fixture, main = self.main_oracle()
        action = main.reply({}, self.main_body(fixture, main))
        self.assertEqual((action['name'], action['arguments']), ('Skill', {'skill': 'hiroute-collaboration'}))
        results = {'main_skill': 'Launching skill: hiroute-collaboration'}
        with self.assertRaisesRegex(AssertionError, 'actual installed user Skill'):
            main.reply({}, self.main_body(fixture, main, results, actual_directory=False))
        action = main.reply({}, self.main_body(fixture, main, results))
        self.assertEqual(action['name'], 'Bash')
        self.assertIn('worker plans --output json', action['arguments']['command'])

    def test_public_result_and_prompt_echo_cannot_replace_worker_disk_artifact(self):
        fixture, main = self.main_oracle()
        results = {'main_skill': 'Launching skill', 'main_plans': self.envelope({'plans': [
            {'agent_plan_id': fixture['plan_id'], 'harness': 'qoder_cli', 'availability': 'ready'}]}),
            'main_exec': self.envelope({'task_id': 'real-task', 'run_id': 'real-run', 'run_state': 'succeeded',
                                       'result': fixture['artifact']['receipt']})}
        with self.assertRaises(FileNotFoundError):
            main.reply({}, self.main_body(fixture, main, results))
        self.assertFalse(main.completed)
        Path(fixture['artifact']['path']).write_text('wrong artifact')
        with self.assertRaisesRegex(AssertionError, 'disk artifact missing'):
            main.reply({}, self.main_body(fixture, main, results))
        Path(fixture['artifact']['path']).write_text(fixture['artifact']['receipt'])
        reply = main.reply({}, self.main_body(fixture, main, results))
        self.assertTrue(main.completed)
        self.assertIn(fixture['artifact']['receipt'], reply['text'])

    def test_wrong_role_endpoint_rejects_before_issuing_a_tool(self):
        fixture, main = self.main_oracle()
        body = self.main_body(fixture, main)
        body['input'][0]['content'][0]['text'] = 'a delegated Worker prompt'
        with self.assertRaisesRegex(AssertionError, 'Worker request reached the main'):
            main.reply({}, body)

    def test_accepted_worker_is_waited_publicly_then_result_requires_the_disk_artifact(self):
        fixture, main = self.main_oracle()
        accepted = {'task_id': 'real-task', 'run_id': 'real-run', 'run_state': 'accepted'}
        pending = json.dumps({'schema_version': {'major': 2, 'minor': 0}, 'status': 'succeeded',
            'data': accepted, 'next_actions': [{'command_id': 'worker.wait',
                                              'input': {'run_id': 'real-run', 'after_revision': 7}}]})
        results = {'main_skill': 'Launching skill', 'main_plans': self.envelope({'plans': [
            {'agent_plan_id': fixture['plan_id'], 'harness': 'qoder_cli', 'availability': 'ready'}]}),
                   'main_exec': pending}
        waiting = main.reply({}, self.main_body(fixture, main, results))
        self.assertEqual(waiting['id'], 'main_wait_1')
        self.assertIn('worker wait --run real-run --after-revision 7', waiting['arguments']['command'])
        self.assertFalse(main.completed)
        results['main_wait_1'] = self.envelope(dict(accepted, run_state='succeeded'))
        reading = main.reply({}, self.main_body(fixture, main, results))
        self.assertEqual(reading['id'], 'main_result')
        self.assertIn('worker result --run real-run', reading['arguments']['command'])
        results['main_result'] = self.envelope(dict(accepted, run_state='succeeded', result=fixture['artifact']['receipt']))
        with self.assertRaises(FileNotFoundError):
            main.reply({}, self.main_body(fixture, main, results))
        Path(fixture['artifact']['path']).write_text(fixture['artifact']['receipt'])
        main.reply({}, self.main_body(fixture, main, results))
        self.assertTrue(main.completed)
        results['main_wait_1'] = self.envelope(dict(accepted, task_id='different-task', run_state='succeeded'))
        with self.assertRaisesRegex(AssertionError, 'switched the accepted Worker identity'):
            main.reply({}, self.main_body(fixture, main, results))

    def test_worker_output_requires_correlated_tool_and_real_artifact(self):
        fixture, _ = self.main_oracle()
        body = self.body(fixture, {'delegated_artifact': fixture['artifact']['receipt']})
        with self.assertRaisesRegex(AssertionError, 'independent disk artifact'):
            worker_decision(fixture, body)
        Path(fixture['artifact']['path']).write_text(fixture['artifact']['receipt'])
        self.assertEqual(worker_decision(fixture, body)['text'], fixture['artifact']['receipt'])
        body['input'][1]['call_id'] = 'wrong-call-id'
        self.assertEqual(worker_decision(fixture, body)['kind'], 'tool')

    def test_failed_or_ambiguous_public_envelopes_never_prove_completion(self):
        good = self.envelope({'task_id': 'task'})
        for output in (good + good, good.replace('succeeded', 'internal_error'), 'claimed success'):
            with self.assertRaisesRegex(AssertionError, 'one successful public CLI envelope'):
                output_envelope(output)


class QoderCollaborationCleanupTests(unittest.TestCase):
    def test_disable_rejects_drift_and_retained_selection_or_restore_reference(self):
        with tempfile.TemporaryDirectory() as root:
            skill = Path(root) / 'SKILL.md'
            for status in ({'state': 'drift'},
                           {'state': 'restored', 'current_selection': {'trigger_mode': 'explicit'}},
                           {'state': 'restored', 'restore_point_ref': 'remaining-owned-reference'}):
                with self.subTest(status=status), patch.object(collaboration, 'apply_collaboration', return_value=status):
                    with self.assertRaisesRegex(AssertionError, 'did not restore and release'):
                        collaboration.restore_skill(None, 'context', 'restore-ref', None, skill)
            for status in ({'state': 'restored'},
                           {'state': 'restored', 'current_selection': None, 'restore_point_ref': None}):
                with patch.object(collaboration, 'apply_collaboration', return_value=status):
                    collaboration.restore_skill(None, 'context', 'restore-ref', None, skill)

    def setup_cleanup(self):
        calls = []
        product = SimpleNamespace(process=object(), close=lambda: calls.append('close_product'))
        source = SimpleNamespace(close=lambda: calls.append('close_source'))
        report = {'candidate': 'fixture-only', 'state': 'red'}
        self.addCleanup(patch.stopall)
        patch.object(collaboration, 'report_failure',
                     side_effect=lambda *_: calls.append('report_and_stop')).start()
        patch.object(qoder, 'cleanup_context',
                     side_effect=lambda *_: calls.append('cleanup_receipts')).start()
        return calls, product, source, report

    def test_restore_precedes_failure_report_stop_and_owned_cleanup(self):
        calls, product, source, report = self.setup_cleanup()
        collaboration.finish_collaboration(product, [source], lambda: calls.append('restore'),
                                           report, 'worker_failed', True)
        self.assertEqual(calls, ['restore', 'report_and_stop', 'close_product', 'cleanup_receipts', 'close_source'])
        self.assertEqual(report['restore_after_failure'], {'state': 'succeeded'})

    def test_restore_error_is_reported_without_masking_original_worker_failure(self):
        calls, product, source, report = self.setup_cleanup()
        def restore():
            calls.append('restore')
            raise ValueError('secondary restore status failure')
        output = io.StringIO()
        with redirect_stdout(output), self.assertRaisesRegex(RuntimeError, 'original Worker failure'):
            try:
                raise RuntimeError('original Worker failure')
            finally:
                collaboration.finish_collaboration(product, [source], restore, report, 'worker_failed', True)
        recorded = json.loads(output.getvalue())
        self.assertTrue(recorded['primary_failure_preserved'])
        self.assertEqual(recorded['failures'], [{'step': 'restore_collaboration', 'error_type': 'ValueError'}])
        self.assertEqual(calls[-3:], ['close_product', 'cleanup_receipts', 'close_source'])

    def test_owned_cleanup_failure_prevents_success_and_still_closes_sources(self):
        calls, product, source, report = self.setup_cleanup()
        with patch.object(qoder, 'cleanup_context', side_effect=AssertionError('changed user material')):
            with redirect_stdout(io.StringIO()), self.assertRaisesRegex(AssertionError, 'cleanup failed'):
                collaboration.finish_collaboration(product, [source], None, report, 'completed', False)
        self.assertEqual(calls[-1], 'close_source')


class QoderCompactionOracleTests(unittest.TestCase):
    def setUp(self):
        self.fixture = {'compaction_prompt': 'public-worker-marker', 'receipt': 'final-worker-receipt',
                        'compaction_script': '/private/owned/read-only-receipt.sh'}
        self.oracle = CompactionOracle(self.fixture, 'frozen-upstream-model')

    def body(self, text, output=None, model='frozen-upstream-model'):
        return {'model': model, 'input': [
            *([{'type': 'function_call_output', 'call_id': 'native_compaction_tool', 'output': output}]
              if output is not None else []),
            {'role': 'user', 'content': [{'type': 'input_text', 'text': text}]}]}

    def begin(self):
        first = self.oracle.reply({}, self.body(self.fixture['compaction_prompt']))
        self.assertEqual((first['request_kind'], first['input_tokens']), ('main', PRESSURE_TOKENS))
        summary = self.oracle.reply({}, self.body('Please summarize this tool output: ' + self.oracle.tool_receipt))
        self.assertEqual(summary['request_kind'], 'tool_summary')
        return summary['text']

    def test_auxiliary_requests_require_independent_summaries_and_correlated_tool_history(self):
        summary = self.begin()
        compact = self.oracle.reply({}, self.body('Create a detailed summary for summarization.', summary))
        self.assertEqual(compact['request_kind'], 'compact')
        final = self.oracle.reply({}, self.body(compact['text']))
        self.assertEqual(final['request_kind'], 'main')
        self.assertIn(self.fixture['receipt'], final['text'])
        self.assertEqual((self.oracle.main_requests, self.oracle.tool_summary_requests, self.oracle.compact_requests), (2, 1, 1))

    def test_ordinary_main_success_does_not_prove_missing_auxiliary_requests(self):
        self.oracle.reply({}, self.body(self.fixture['compaction_prompt']))
        with self.assertRaisesRegex(AssertionError, 'consume the actual native summary'):
            self.oracle.reply({}, self.body('Continue successfully.'))
        self.assertEqual((self.oracle.tool_summary_requests, self.oracle.compact_requests), (0, 0))

    def test_wrong_auxiliary_model_and_uncorrelated_summary_fail_closed(self):
        summary = self.begin()
        for text in ('Create a detailed summary for summarization.', 'summarize tool output: ' + self.oracle.tool_receipt):
            with self.assertRaisesRegex(AssertionError, 'frozen upstream model'):
                self.oracle.reply({}, self.body(text, summary, model='wrong-auxiliary-alias'))
        with self.assertRaisesRegex(AssertionError, 'correlated summarized tool receipt'):
            self.oracle.reply({}, self.body('Create a detailed summary for summarization. ' + summary))
        self.assertEqual(self.oracle.compact_requests, 0)

    def test_native_boundary_and_persisted_summary_are_both_required(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'exact-session.jsonl'
            summary = {'isCompactSummary': True, 'message': self.oracle.summary_receipt}
            path.write_text(json.dumps(summary) + '\n')
            with self.assertRaisesRegex(AssertionError, 'automatic compaction boundary'):
                assert_native_compaction(path, self.oracle.summary_receipt)
            boundary = {'subtype': 'compact_boundary', 'compactMetadata': {
                'trigger': 'auto', 'preTokens': PRESSURE_TOKENS + 12, 'postTokens': 200}}
            path.write_text(json.dumps(boundary) + '\n')
            with self.assertRaisesRegex(AssertionError, 'persist the independently issued summary'):
                assert_native_compaction(path, self.oracle.summary_receipt)
            path.write_text(json.dumps(boundary) + '\n' + json.dumps(summary) + '\n')
            self.assertEqual(assert_native_compaction(path, self.oracle.summary_receipt)[0]['trigger'], 'auto')

    def test_pressure_usage_is_present_on_the_real_responses_wire(self):
        with tempfile.TemporaryDirectory() as root:
            controls = Path(root)
            (controls / 'native-context.json').write_text(json.dumps(self.fixture))
            server = oracle.NativeContextUpstream(controls)
            server.model = 'frozen-upstream-model'
            server.reply = self.oracle.reply
            self.addCleanup(server.close)
            connection = http.client.HTTPConnection(*server.server.server_address, timeout=3)
            self.addCleanup(connection.close)
            connection.request('POST', '/v1/responses', json.dumps(self.body(self.fixture['compaction_prompt'])),
                               {'Authorization': 'Bearer ' + server.token})
            response = connection.getresponse()
            frames = [json.loads(line.removeprefix('data: ')) for line in response.read().decode().splitlines()
                      if line.startswith('data: ')]
            self.assertEqual(response.status, 200)
            self.assertEqual(frames[-1]['type'], 'response.completed')
            self.assertEqual(frames[-1]['response']['usage']['input_tokens'], PRESSURE_TOKENS)
            self.assertEqual(frames[-1]['response']['output'][0]['type'], 'function_call')


class PersistedModelFixtureTests(unittest.TestCase):
    def test_model_settings_do_not_require_native_source_discovery_and_restore_uses_model_state(self):
        calls = []
        state = {'state': 'configured', 'restore_point_ref': 'model-restore',
                 'current_selection': {'mode': 'qoder_additional', 'allowed_plan_ids': ['plan/current']}}

        def preview(command, body):
            calls.append(command)
            self.assertIn(command, ('agents connect preview', 'agents restore preview', 'agents connect status'))
            if command == 'agents connect status':
                return state
            return {'applicable': True, 'spec': body['spec'], 'accept_digest': 'accepted',
                    'dependency_digest': 'bound', 'expected_revisions': {}}

        def cli(command, body):
            calls.append(command)
            self.assertIn(command, ('agents connect apply', 'agents restore apply'))
            return 0, {'data': {'state': 'succeeded'}}

        product = SimpleNamespace(preview=preview, cli=cli)
        model_product.apply_models(product, model_product.model_spec('context', ['plan/current']), 'configure')
        self.assertEqual(calls, ['agents connect preview', 'agents connect apply', 'agents connect status'])
        with self.assertRaisesRegex(AssertionError, 'release its selection/reference'):
            model_product.restore_models(product, 'context')
        state.update(state='not_configured', current_selection=None, restore_point_ref=None)
        with patch.object(model_product, 'settings_status', return_value={'restore_point_ref': 'model-restore'}):
            with patch.object(model_product, 'apply_models', return_value=({}, state)):
                model_product.restore_models(product, 'context')
                for remaining in (
                    {'state': 'restored'},  # Collaboration's terminal state is not a model contract.
                    {'state': 'drift'},
                    {'current_selection': {'mode': 'qoder_additional', 'allowed_plan_ids': ['plan/current']}},
                    {'restore_point_ref': 'remaining-owned-model-reference'},
                ):
                    with self.subTest(remaining=remaining):
                        restored = dict(state, **remaining)
                        with patch.object(model_product, 'apply_models', return_value=({}, restored)):
                            with self.assertRaisesRegex(AssertionError, 'release its selection/reference'):
                                model_product.restore_models(product, 'context')

    def test_source_attempt_counter_includes_rejected_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            source = oracle.NativeContextUpstream(Path(directory))
            self.addCleanup(source.close)
            host, port = source.server.server_address
            connection = http.client.HTTPConnection(host, port, timeout=2)
            try:
                connection.request('POST', '/v1/responses', '{}', {'Authorization': 'Bearer wrong'})
                response = connection.getresponse()
                response.read()
                self.assertEqual(response.status, 401)
            finally:
                connection.close()
            self.assertEqual(source.request_count(), 1)
            self.assertFalse((Path(directory) / 'native-context-events.jsonl').exists())

    def test_collaboration_check_does_not_require_a_worker_plan(self):
        revisions = {'fixture-current-revision': 7}
        captured = []
        product = SimpleNamespace(
            control=lambda operation, payload: {'data': {'revisions': revisions}},
            grant=lambda operation, consent, key: captured.append(consent) or 'bounded-capability',
            cli=lambda command, capability: (0, {'data': {
                'skill_loading': 'proven', 'trusted_cli_execution': 'proven'}}))
        collaboration.check_collaboration(product, 'no-worker-plan')
        self.assertEqual(captured[0]['expected_revisions'], revisions)

    def test_model_context_requires_its_own_explicit_roots(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            config = home / '.qoder'
            config.mkdir()
            product = SimpleNamespace(env={})
            borrowed = {'HIROUTE_QODER_CONTEXT_HOME': str(home), 'HIROUTE_QODER_CONFIG_DIR': str(config)}
            with patch.dict(os.environ, borrowed, clear=True):
                with self.assertRaisesRegex(AssertionError, 'MODEL_CONTEXT_HOME'):
                    model_fixture.select_model_context(product)
            self.assertEqual(product.env, {})
            dedicated = {'HIROUTE_QODER_MODEL_CONTEXT_HOME': str(home),
                         'HIROUTE_QODER_MODEL_CONFIG_DIR': str(config)}
            with patch.dict(os.environ, dedicated), patch.object(Path, 'home', return_value=home):
                with self.assertRaisesRegex(AssertionError, 'daily HOME'):
                    model_fixture.select_model_context(product)
            with patch.dict(os.environ, dedicated):
                self.assertEqual(model_fixture.select_model_context(product), config)

    def test_ordinary_native_command_does_not_hide_persisted_settings(self):
        product = SimpleNamespace(project=Path('/fixture/project'), env={'QODER_CONFIG_DIR': '/fixture/.qoder'})
        command = model_fixture.native_command(product, '/installed/qodercli', 'managed/raw-alias', 'input-nonce')
        self.assertEqual(command[command.index('--model') + 1], 'managed/raw-alias')
        self.assertNotIn('--settings', command)
        self.assertNotIn('--setting-sources', command)
        self.assertFalse(any('TOKEN' in argument or 'apiKey' in argument for argument in command))

    def test_preservation_rejects_unknown_or_default_changes_and_owned_cleanup_refuses_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            settings = model_fixture.OwnedModelSettings(Path(directory), 'http://127.0.0.1:1/v1')
            baseline = settings.read()
            managed = 'hiroute-main-test'
            configured = json.loads(json.dumps(baseline))
            configured['providers'][managed] = {'apiKey': 'synthetic-secret'}
            settings.write(configured)
            settings.assert_preserved(managed)
            for mutate in (
                lambda value: value['model'].update(name='changed-default'),
                lambda value: value['providers']['fixture-native'].pop('fixtureUnknownProviderKey'),
            ):
                changed = json.loads(json.dumps(configured))
                mutate(changed)
                settings.write(changed)
                with self.assertRaisesRegex(AssertionError, 'native/default/unknown'):
                    settings.assert_preserved(managed)
            settings.write(configured)
            with self.assertRaisesRegex(AssertionError, 'managed/drifted'):
                settings.close()
            self.assertTrue(settings.path.exists())
            settings.write(baseline)
            settings.close()
            self.assertFalse(settings.path.exists())

    def test_simulated_user_edits_do_not_reserialize_the_managed_provider(self):
        with tempfile.TemporaryDirectory() as directory:
            settings = model_fixture.OwnedModelSettings(Path(directory), 'http://127.0.0.1:1/v1')
            configured = settings.read()
            provider = {'apiKey': 'synthetic-secret', 'models': [{'model': 'selected'}]}
            configured['providers']['managed'] = provider
            settings.path.write_text(json.dumps(configured, separators=(',', ':')))
            exact_provider = json.dumps(provider, separators=(',', ':'))
            settings.add_user_edit('managed')
            settings.select_default(settings.native_default, 'managed/selected')
            self.assertIn(exact_provider, settings.path.read_text())
            settings.select_default('managed/selected', settings.native_default)
            settings.assert_preserved('managed')

    def test_native_result_must_be_success_without_error_and_independent_receipt(self):
        result = {'type': 'result', 'subtype': 'success', 'is_error': False,
                  'result': 'independent-output', 'session_id': 'native-session'}
        self.assertEqual(model_fixture.successful_native_result(json.dumps(result).encode(), 'independent-output'),
                         'native-session')
        for update in ({'is_error': True}, {'result': 'input-echo'}, {'session_id': ''}):
            with self.assertRaises(AssertionError):
                model_fixture.successful_native_result(json.dumps(dict(result, **update)).encode(), 'independent-output')
        with self.assertRaisesRegex(AssertionError, 'one native'):
            model_fixture.successful_native_result((json.dumps(result) + '\n' + json.dumps(result)).encode(), 'independent-output')

    def test_source_oracle_rejects_another_route_and_uses_a_separate_output_nonce(self):
        expected = model_fixture.PersistedRouteOracle('actual-source-model')
        prompt = expected.arm('native-request')
        body = {'model': 'actual-source-model', 'input': prompt}
        reply = expected.reply({}, body)
        self.assertNotEqual(reply['text'], prompt)
        self.assertEqual(expected.calls, ['native-request'])
        with self.assertRaisesRegex(AssertionError, 'wrong source'):
            expected.reply({}, dict(body, model='other-source-model'))
        with self.assertRaisesRegex(AssertionError, 'unplanned'):
            expected.reply({}, dict(body, input='another task'))


if __name__ == '__main__':
    unittest.main()
