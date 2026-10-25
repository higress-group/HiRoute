#!/usr/bin/env python3
"""Shared native product-fixture mechanics; no native Agent or product acceptance claim."""
import os
import json
import io
from copy import deepcopy
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import urllib.request
from http.server import ThreadingHTTPServer
from types import SimpleNamespace
from types import MethodType
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'crates/daemon/tests/support'))
from agent_product_support import apply_settings, run_native_command, expose_native_installation
from publication_product import Product
from publication_process import AgentSettingsPreviewBlocked
import standalone_headless_product as headless
from dsh_native_context import DshModelSettings, read_patch, encode_patch
from decision_services_product import DecisionEndpoint
from additional_native_history_product import (
    assert_assessed_execution, assert_assessed_stage, freeze_completed_stage,
    native_decision_receipt,
)


class DecisionEndpointContractTests(unittest.TestCase):
    def reply(self, option, assessment=False, target=None):
        endpoint = type('ContractEndpoint', (DecisionEndpoint,),
                        {'requests': [], 'native_history_assessment': assessment})
        server = ThreadingHTTPServer(('127.0.0.1', 0), endpoint)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            body = {'route': {'kind': 'branches', 'categories': [option], 'default_category': option['id']}}
            if target is not None:
                body['preceding_stage'] = target
            request = urllib.request.Request(f'http://127.0.0.1:{server.server_port}/decision',
                data=json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, timeout=5) as response:
                result = json.load(response)
            self.assertEqual(endpoint.requests[0][1], body)
            return result
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)

    def test_single_group_category_retains_the_declared_category(self):
        result = self.reply({'id': 'native', 'condition': 'Native task', 'groups': ['regular']})
        self.assertEqual(result, {'category': 'native', 'group': 'regular'})

    def test_dual_group_category_returns_a_declared_group_without_inventing_probabilities(self):
        option = {'id': 'native', 'condition': 'Native task', 'groups': ['regular', 'primary']}
        result = self.reply(option)
        self.assertEqual(result['category'], 'native')
        self.assertIn(result['group'], option['groups'])
        self.assertEqual(set(result), {'category', 'group'})

    def test_native_endpoint_returns_quality_only_for_a_captured_target(self):
        option = {'id': 'native', 'criterion': 'Native task'}
        self.assertNotIn('assessment', self.reply(option, assessment=True))
        self.assertEqual(self.reply(option, assessment=True, target={'from': 0})['assessment'],
                         {'score': 0.75, 'partial': False})


class NativeHistoryOracleTests(unittest.TestCase):
    def body(self):
        return {'preceding_stage': {'from': 1},
                'visible_conversation': ['OLD', 'ACTUAL_A', 'ACCEPTED_A_OUTPUT'],
                'latest_user': 'ACTUAL_B'}

    def test_actual_preceding_execution_is_required(self):
        assert_assessed_execution([self.body()], 'ACTUAL_A', 'ACCEPTED_A_OUTPUT', 'ACTUAL_B')

    def test_empty_history_missing_output_wrong_target_and_current_input_are_rejected(self):
        for mutate in [
                lambda b: b.update(preceding_stage=None),
                lambda b: b.update(visible_conversation=[]),
                lambda b: b.update(preceding_stage={'from': True}),
                lambda b: b.update(preceding_stage={'from': 2}),
                lambda b: b.update(visible_conversation=['OLD', 'ACTUAL_A']),
                lambda b: b['visible_conversation'].append('ACTUAL_B'),
                lambda b: b.update(latest_user='OLDER_INPUT')]:
            body = self.body()
            mutate(body)
            with self.assertRaises(AssertionError):
                assert_assessed_execution([body], 'ACTUAL_A', 'ACCEPTED_A_OUTPUT', 'ACTUAL_B')


class NativeStageOracleTests(unittest.TestCase):
    def finished(self, requests, ordinal, revision):
        return {
            'kind': 'agent_turn_finished', 'agent_turn_id': f'turn-{ordinal}',
            'segment_id': 'same-stage', 'ordinal': ordinal, 'plan_id': 'plan', 'plan_revision': 1,
            'selected_branch_id': 'native', 'executed_branch_id': 'native',
            'model_configuration_id': 'actual-model', 'profile_digest': 'actual-profile',
            'branch_execution': {'policy': {'criteria_digest': 'rubric'}, 'group': 'regular'},
            'attribution': 'single', 'status': 'completed', 'history_partial': False,
            'opening_cause': 'no_comparable_history' if ordinal == 1 else 'user_message',
            'history_membership': {
                'version': 1, 'branch_id': 'history-branch', 'revision': revision,
                'request_ids': requests, 'source_unverified': True, 'association_unverified': False,
                'relation': 'root' if ordinal == 1 else 'exact_prefix',
                'capture_partial': False, 'clipped': False,
            },
        }

    def receipt(self, request, finished=None):
        facts = [{'kind': 'attempt_started', 'model_configuration_id': 'actual-model',
                  'profile_digest': 'actual-profile'}]
        if finished is not None:
            facts.append(finished)
        return {'request_id': request, 'outcome': 'accepted', 'trust': {'agent_plan_id': 'plan'},
                'ordered_facts': [{'fact': fact, 'occurred_at_unix_nanos': 1} for fact in facts]}

    def setUp(self):
        self.receipts = {'A': self.receipt('A', self.finished(['A'], 2, 3)),
                         'tool-call': self.receipt('tool-call'),
                         'tool-result': self.receipt('tool-result', self.finished(['tool-call', 'tool-result'], 1, 2))}
        self.grade = {
            'kind': 'branch_assessment_recorded', 'segment_id': 'same-stage', 'plan_id': 'plan',
            'plan_revision': 1, 'model_configuration_id': 'actual-model', 'profile_digest': 'actual-profile',
            'trigger_request_id': 'B', 'target_from_turn_id': 'turn-1', 'target_through_turn_id': 'turn-2',
            'target_from_ordinal': 1, 'target_through_ordinal': 2, 'score': .75, 'partial': False,
            'history_membership': {**self.finished([], 2, 3)['history_membership'],
                                   'request_ids': ['tool-call', 'tool-result', 'A']},
        }

    def test_complete_stage_spans_tool_requests_and_new_input(self):
        frozen = freeze_completed_stage(self.receipts, 'A')
        self.assertEqual(set(frozen['request_ids']), {'tool-call', 'tool-result', 'A'})
        assert_assessed_stage(frozen, self.grade, 'B')
        # Freeze is detached from later collection and does not invent human origin.
        self.receipts['A']['ordered_facts'][-1]['fact']['branch_execution']['group'] = 'later'
        self.assertEqual(frozen['identity']['branch_execution']['group'], 'regular')
        self.assertTrue(frozen['source_unverified'])

    def test_initial_root_and_singleton_stage_are_valid(self):
        first = self.receipt('A', self.finished(['A'], 1, 1))
        frozen = freeze_completed_stage({'A': first}, 'A')
        self.assertEqual(frozen['request_ids'], ['A'])
        self.assertEqual(first['ordered_facts'][-1]['fact']['opening_cause'], 'no_comparable_history')

    def test_categorical_route_without_optional_quality_policy_is_valid(self):
        first = self.receipt('A', self.finished(['A'], 1, 1))
        del first['ordered_facts'][-1]['fact']['branch_execution']
        frozen = freeze_completed_stage({'A': first}, 'A')
        self.assertIsNone(frozen['identity']['branch_execution'])

    def test_other_branch_segment_and_future_capture_do_not_enter_frozen_stage(self):
        for key, value in [('segment_id', 'foreign-stage'), ('revision', 4), ('branch_id', 'other-branch')]:
            with self.subTest(key=key):
                receipts = deepcopy(self.receipts)
                foreign = self.finished(['foreign'], 3, 3)
                if key == 'segment_id':
                    foreign[key] = value
                else:
                    foreign['history_membership'][key] = value
                receipts['foreign'] = self.receipt('foreign', foreign)
                self.assertEqual(freeze_completed_stage(receipts, 'A')['request_ids'],
                                 ['A', 'tool-call', 'tool-result'])

    def test_missing_partial_and_foreign_completed_execution_are_rejected(self):
        mutations = [
            lambda r: r.pop('tool-call'),
            lambda r: r['tool-call'].update(outcome='failed'),
            lambda r: r['tool-call']['ordered_facts'][0]['fact'].update(profile_digest='foreign'),
        ]
        for key, value in [('status', 'failed'), ('history_partial', True),
                           ('model_configuration_id', 'foreign'), ('plan_revision', 2),
                           ('branch_execution', {'policy': {'criteria_digest': 'foreign'}})]:
            mutations.append(lambda r, k=key, v=value: r['tool-result']['ordered_facts'][-1]['fact'].update({k: v}))
        for key, value in [('capture_partial', True), ('clipped', True), ('request_ids', ['tool-call', 'tool-call'])]:
            mutations.append(lambda r, k=key, v=value: r['tool-result']['ordered_facts'][-1]['fact']['history_membership'].update({k: v}))
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                receipts = deepcopy(self.receipts)
                mutate(receipts)
                with self.assertRaises(AssertionError):
                    freeze_completed_stage(receipts, 'A')

    def test_wrong_extra_missing_duplicate_current_members_and_uncertainty_are_rejected(self):
        frozen = freeze_completed_stage(self.receipts, 'A')
        mutations = [lambda g: g.update(trigger_request_id='different'),
                     lambda g: g.update(partial=True), lambda g: g.update(score=None)]
        for value in [['A'], ['tool-call', 'tool-result'], ['tool-call', 'tool-result', 'A', 'B'],
                      ['tool-call', 'tool-result', 'A', 'foreign'], ['tool-call', 'tool-result', 'A', 'A']]:
            mutations.append(lambda g, v=value: g['history_membership'].update(request_ids=v))
        for key, value in [('branch_id', 'foreign'), ('revision', 4), ('clipped', True),
                           ('capture_partial', True), ('source_unverified', False)]:
            mutations.append(lambda g, k=key, v=value: g['history_membership'].update({k: v}))
        for key, value in [('model_configuration_id', 'foreign'), ('profile_digest', 'foreign'),
                           ('plan_revision', 2), ('segment_id', 'foreign'),
                           ('target_from_turn_id', 'turn-2'), ('target_through_ordinal', 3)]:
            mutations.append(lambda g, k=key, v=value: g.update({k: v}))
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                grade = deepcopy(self.grade)
                mutate(grade)
                with self.assertRaises(AssertionError):
                    assert_assessed_stage(frozen, grade, 'B')

    def test_public_receipt_collector_checks_the_previously_frozen_stage(self):
        expected = freeze_completed_stage(self.receipts, 'A')
        current = self.receipt('B', self.finished(['B'], 3, 4))
        current['ordered_facts'] += [
            {'occurred_at_unix_nanos': 2, 'fact': self.grade},
            {'occurred_at_unix_nanos': 2, 'fact': {'kind': 'route_decision', 'complexity': {
                'decision_source': 'external_classifier', 'fallback_used': False, 'branch_id': 'native'}}},
        ]
        self.receipts['B'] = current

        def public_cli(command):
            if command.startswith('sessions list'):
                data = {'sessions': [{'session_id': 'session'}]}
            elif command.startswith('sessions show'):
                data = {'turns': [{'receipt_ids': list(self.receipts)}]}
            else:
                data = self.receipts[command.removeprefix('sessions receipt ')]
            return None, {'data': data}

        product = SimpleNamespace(plan_id='plan', public_cli=public_cli)
        result = native_decision_receipt(product, 2, expected)
        self.assertEqual(result['request_id'], 'B')
        self.assertEqual(set(result['completed_stage']['request_ids']), {'tool-call', 'tool-result', 'A', 'B'})
        self.assertNotIn('B', expected['request_ids'])
        current['ordered_facts'][-2]['fact']['history_membership']['request_ids'] = ['A']
        with self.assertRaisesRegex(AssertionError, 'stage members'):
            native_decision_receipt(product, 2, expected)


class DshRestorationEvidenceTests(unittest.TestCase):
    def test_restore_checks_rows_hidden_by_the_model_projection_and_the_adapter_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary)
            (config / 'profiles/web').mkdir(parents=True)
            settings = DshModelSettings(config, 'http://127.0.0.1:9/v1')
            settings.assert_restored()
            rows = read_patch(settings.path)
            rows.append({'insert':[{'id':'hiroute-routing','name':'file:///fixture/hiroute-routing.mjs'}]})
            settings.path.write_text(encode_patch(rows))
            self.assertEqual(settings.read(), settings.baseline)
            with self.assertRaisesRegex(AssertionError, 'native patch row'):
                settings.assert_restored()
            settings.write(settings.baseline)
            settings.path.with_name('hiroute-routing.mjs').write_text('owned adapter')
            with self.assertRaisesRegex(AssertionError, 'adapter file'):
                settings.assert_restored()


class ObservationReadRetryTests(unittest.TestCase):
    def setUp(self):
        self.product = headless.InstalledProduct.__new__(headless.InstalledProduct)
        self.product.cli_path = Path('/fixture/bin/hiroute')
        self.product.project = Path('/fixture/project')
        self.product.env = {}
        self.product.outputs = []
        self.product.daemon = None
        self.now = 0
        self.calls = []
        for target, replacement in (
            ('monotonic', lambda: self.now), ('sleep', self.sleep),
        ):
            mock = patch.object(headless.time, target, replacement)
            mock.start()
            self.addCleanup(mock.stop)

    def sleep(self, seconds):
        self.now += seconds

    def response(self, envelope, code=0):
        return subprocess.CompletedProcess([], code, json.dumps(envelope).encode(), b'')

    def busy(self):
        return {'status': 'unavailable', 'error': {
            'code': 'OBSERVATION_UNAVAILABLE', 'retryable': True,
            'details_schema': 'hiroute.observation.read-failure/v1',
            'observation_read': {'reason': 'reader_busy', 'retry_after_ms': 100, 'action': 'retry_read'},
        }}

    def test_busy_then_success_reuses_exact_scope_cursor_and_body(self):
        busy = self.response(self.busy(), 6)
        success = self.response({'status': 'succeeded', 'data': {'costs': [7]}})
        payload = {'analysis_scope': 'frozen-scope', 'cursor': 'same-page', 'limit': 1}
        with patch.object(headless.subprocess, 'run', side_effect=[busy, success]) as execute:
            observed = self.product.cli('value', 'show', '--request-stdin', payload=payload,
                                        retry_reader_busy=True)
        self.assertEqual(observed['data'], {'costs': [7]})
        first, second = execute.call_args_list
        self.assertEqual(first.args, second.args)
        self.assertEqual(first.kwargs['input'], second.kwargs['input'])
        self.assertEqual(json.loads(second.kwargs['input']), payload)
        self.assertAlmostEqual(self.now, .1)
        self.assertLessEqual(second.kwargs['timeout'], first.kwargs['timeout'])
        self.assertEqual(self.product.outputs, [busy.stdout, b'', success.stdout, b''])

    def test_persistent_busy_fails_with_last_envelope_within_retry_budget(self):
        envelope = self.busy()
        with patch.object(headless.subprocess, 'run', return_value=self.response(envelope, 6)) as execute:
            with self.assertRaises(AssertionError) as failure:
                self.product.cli('value', 'show', retry_reader_busy=True)
        self.assertEqual(failure.exception.args[0][3], envelope)
        self.assertGreater(execute.call_count, 1)
        self.assertLessEqual(execute.call_count, 51)
        self.assertLessEqual(self.now, 5)

    def test_other_failures_are_not_retried(self):
        cases = []
        for key, value in (('retryable', False), ('code', 'CAPABILITY_DENIED'),
                           ('details_schema', 'unknown')):
            envelope = self.busy()
            envelope['error'][key] = value
            cases.append((6, envelope))
        for key, value in (('reason', 'timeout'), ('reason', 'index_unavailable'),
                           ('action', 'refresh_scope'), ('retry_after_ms', 0),
                           ('retry_after_ms', True), ('retry_after_ms', 6000)):
            envelope = self.busy()
            envelope['error']['observation_read'][key] = value
            cases.append((6, envelope))
        cases.append((2, self.busy()))
        for code, envelope in cases:
            with self.subTest(code=code, envelope=envelope):
                with patch.object(headless.subprocess, 'run', return_value=self.response(envelope, code)) as execute:
                    with self.assertRaises(AssertionError):
                        self.product.cli('value', 'show', retry_reader_busy=True)
                self.assertEqual(execute.call_count, 1)
        self.assertEqual(self.now, 0)

    def test_mutations_and_negative_assertions_cannot_opt_in(self):
        for arguments, options in (
            (('prices', 'override', 'apply'), {}),
            (('sessions', 'delete'), {}),
            (('value', 'show'), {'expected': 6}),
            (('value', 'show'), {'pass_fds': (3,)}),
        ):
            with self.subTest(arguments=arguments, options=options):
                with patch.object(headless.subprocess, 'run') as execute:
                    with self.assertRaisesRegex(AssertionError, 'requires an observation read'):
                        self.product.cli(*arguments, **options, retry_reader_busy=True)
                execute.assert_not_called()

    def test_default_and_expected_error_calls_still_run_once(self):
        for arguments, expected in ((('value', 'show'), 0), (('prices', 'override', 'apply'), 6)):
            with self.subTest(arguments=arguments):
                with patch.object(headless.subprocess, 'run', return_value=self.response(self.busy(), 6)) as execute:
                    if expected == 0:
                        with self.assertRaises(AssertionError):
                            self.product.cli(*arguments)
                    else:
                        self.assertEqual(self.product.cli(*arguments, expected=6), self.busy())
                self.assertEqual(execute.call_count, 1)

    def test_invalid_json_is_not_retried(self):
        response = subprocess.CompletedProcess([], 6, b'not-json', b'')
        with patch.object(headless.subprocess, 'run', return_value=response) as execute:
            with self.assertRaises(AssertionError):
                self.product.cli('value', 'show', retry_reader_busy=True)
        execute.assert_called_once()


class NativeInstallationTests(unittest.TestCase):
    def test_dsh_uses_the_explicit_native_runtime_instead_of_an_ambient_older_node(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'bin').mkdir()
            cli = root / 'selected-cli'; cli.write_text('fixture')
            node = root / 'selected-node'; node.write_text('fixture')
            with patch.dict(os.environ, {'HIROUTE_WORKER_NODE': str(node)}), patch('shutil.which', return_value=None):
                expose_native_installation(SimpleNamespace(root=root), 'dsh', cli)
            self.assertEqual((root / 'bin/node').resolve(), node)
            self.assertEqual((root / 'bin/dsh').resolve(), cli)

    def test_opencode_gets_only_the_native_ripgrep_dependency_in_its_private_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'bin').mkdir()
            cli = root / 'selected-cli'; cli.write_text('fixture'); cli.chmod(0o700)
            rg = root / 'selected-rg'; rg.write_text('fixture'); rg.chmod(0o700)
            with patch('shutil.which', return_value=str(rg)):
                expose_native_installation(SimpleNamespace(root=root), 'opencode', cli)
            self.assertEqual((root / 'bin/rg').resolve(), rg)
            self.assertEqual({p.name for p in (root / 'bin').iterdir()}, {'rg', 'opencode'})

    def test_selected_cli_and_required_runtime_share_the_private_fixture_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'bin').mkdir()
            cli = root / 'selected-cli'; cli.write_text('fixture'); cli.chmod(0o700)
            node = root / 'selected-node'; node.write_text('fixture'); node.chmod(0o700)
            product = SimpleNamespace(root=root)
            with self.assertRaisesRegex(AssertionError, 'requires its selected Node'):
                expose_native_installation(product, 'pi', cli)
            self.assertFalse((root / 'bin/pi').exists())
            expose_native_installation(product, 'pi', cli, node)
            self.assertEqual(Path(shutil.which('pi', path=str(root / 'bin'))).resolve(), cli)
            self.assertEqual(Path(shutil.which('node', path=str(root / 'bin'))).resolve(), node)


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


class SettingsBlockerEvidenceTests(unittest.TestCase):
    def test_capability_failure_keeps_exact_typed_cause_without_private_fields(self):
        for capability in ('effective_configuration', 'atomic_managed_replace', 'ingress_authentication'):
            error = AgentSettingsPreviewBlocked([{'facet': 'model', 'reason': 'capability_unavailable',
                'capabilities': [{'capability': capability, 'reason': 'stale', 'private': 'credential'}],
                'model_ids': ['private-model'], 'configuration': 'private-config'}])
            self.assertEqual(error.diagnostic_blockers, [{'facet': 'model', 'reason': 'capability_unavailable',
                'capabilities': [{'capability': capability, 'reason': 'stale'}]}])
            self.assertEqual(error.reason_codes, ['capability_unavailable'])

    def test_unrecognized_values_and_malformed_fields_are_not_echoed(self):
        error = AgentSettingsPreviewBlocked([None, {'reason': 'private_secret'},
            {'reason': [], 'facet': {}}, {'reason': 'capability_unavailable', 'facet': 'private-config',
             'capabilities': [{'capability': 'ingress_authentication', 'reason': 'private-error'},
                              {'capability': [], 'reason': 'unknown'}, 'private-string']}])
        self.assertEqual(error.diagnostic_blockers, [{'reason': 'capability_unavailable', 'capabilities': []}])
        self.assertEqual(AgentSettingsPreviewBlocked(None).diagnostic_blockers, [])

    def test_blocker_and_capability_collections_are_bounded(self):
        row = {'facet': 'model', 'reason': 'capability_unavailable', 'capabilities':
            [{'capability': 'ingress_authentication', 'reason': 'unknown'}] * 30}
        error = AgentSettingsPreviewBlocked([row] * 30)
        self.assertEqual(len(error.diagnostic_blockers), 16)
        self.assertTrue(all(len(row['capabilities']) == 7 for row in error.diagnostic_blockers))


class ProductFailureEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.product = Product.__new__(Product)
        self.product.root = Path(self.temporary.name)
        self.product.outputs = [b'actual stdout\n', b'actual stderr\x00\n', b'']
        self.product.secrets = set()
        self.product.temporary = None
        self.product.listener_port_lease = SimpleNamespace(close=lambda: None)
        self.product.stop = lambda **kwargs: None

    def test_explicit_root_retains_original_output_frames_on_early_failure(self):
        try:
            raise AssertionError('preflight failed')
        except AssertionError:
            self.product.close()
        blob = (self.product.root / 'harness-failure.bin').read_bytes()
        rows = json.loads((self.product.root / 'harness-failure.json').read_text())['frames']
        self.assertEqual([blob[row['offset']:row['offset'] + row['length']] for row in rows], self.product.outputs)
        self.assertEqual((self.product.root / 'harness-failure.bin').stat().st_mode & 0o777, 0o600)

    def test_shutdown_and_export_errors_do_not_replace_the_business_failure(self):
        def broken_stop(**kwargs):
            raise RuntimeError('cleanup failed')
        self.product.stop = broken_stop
        self.product.preserve_outputs = lambda: (_ for _ in ()).throw(OSError('evidence unavailable'))
        with self.assertRaisesRegex(AssertionError, '^original failure$'):
            try:
                raise AssertionError('original failure')
            finally:
                self.product.close()

    def test_retained_root_exports_after_driver_catches_its_business_exception(self):
        try:
            raise AssertionError('driver records this failure in its report')
        except AssertionError:
            pass
        self.product.close()
        blob = (self.product.root / 'harness-outputs.bin').read_bytes()
        rows = json.loads((self.product.root / 'harness-outputs.json').read_text())['frames']
        self.assertEqual([blob[row['offset']:row['offset'] + row['length']] for row in rows], self.product.outputs)

    def test_temporary_root_survives_a_caught_business_failure(self):
        self.product.temporary = self.temporary
        try:
            raise AssertionError('driver records failure before cleanup')
        except AssertionError:
            pass
        self.product.close(failure=True)
        self.assertTrue(self.product.root.is_dir())
        blob = (self.product.root / 'harness-failure.bin').read_bytes()
        rows = json.loads((self.product.root / 'harness-failure.json').read_text())['frames']
        self.assertEqual([blob[row['offset']:row['offset'] + row['length']] for row in rows], self.product.outputs)
        self.assertFalse(self.temporary._finalizer.alive)

    def test_cleanup_failure_without_prior_error_stays_red_and_retains_output(self):
        self.product.stop = lambda **kwargs: (_ for _ in ()).throw(RuntimeError('cleanup failed'))
        with self.assertRaisesRegex(RuntimeError, '^cleanup failed$'):
            self.product.close()
        self.assertTrue((self.product.root / 'harness-failure.bin').is_file())

    def test_existing_evidence_is_never_overwritten(self):
        self.product.preserve_outputs()
        self.product.outputs = [b'later output']
        with self.assertRaises(FileExistsError):
            self.product.preserve_outputs()
        self.assertEqual((self.product.root / 'harness-failure.bin').read_bytes(),
                         b'actual stdout\n\nactual stderr\x00\n\n')

    def test_shutdown_timeout_retains_daemon_stderr_after_owned_stop(self):
        from unittest.mock import Mock
        process = SimpleNamespace(returncode=None, stderr=io.BytesIO(b'original daemon stderr'))
        process.wait = Mock(side_effect=[subprocess.TimeoutExpired('owned daemon', 40), 0])
        process.poll = lambda: process.returncode
        process.kill = lambda: setattr(process, 'returncode', -9)
        self.product.process = process
        for name, fd in zip(('shutdown_r', 'shutdown_w', 'cap_r', 'cap_w', 'cap_ack_r', 'cap_ack_w'),
                            (*os.pipe(), *os.pipe(), *os.pipe())):
            setattr(self.product, name, fd)
        self.product.stop = MethodType(Product.stop, self.product)
        with self.assertRaises(subprocess.TimeoutExpired):
            self.product.close()
        self.assertIsNone(self.product.process)
        self.assertEqual(self.product.outputs[-1], b'original daemon stderr')
        self.assertIn(b'original daemon stderr', (self.product.root / 'harness-failure.bin').read_bytes())


class RemovedRouteWitnessTests(unittest.TestCase):
    def test_messages_revocation_uses_messages_and_refuses_missing_http_endpoint_as_proof(self):
        from unittest.mock import patch
        import json
        import additional_model_product as journey
        source = SimpleNamespace(request_count=lambda: 0)
        calls = []
        response = SimpleNamespace(status=422, read=lambda: b'{"code":"AGENT_PROTOCOL_UNSUPPORTED"}')
        connection = SimpleNamespace(request=lambda *args, **kwargs: calls.append((args, kwargs)),
            getresponse=lambda: response, close=lambda: None)
        with patch.object(journey.http.client, 'HTTPConnection', return_value=connection):
            journey.reject_gateway_request(SimpleNamespace(port=1), 'synthetic', 'removed', 'messages',
                [source], ('AGENT_PROTOCOL_UNSUPPORTED',))
            self.assertEqual(calls[0][0], ('POST', journey.TAIL + '/messages'))
            self.assertEqual(json.loads(calls[0][1]['body'])['messages'][0]['role'], 'user')
            response.status = 404
            response.read = lambda: b'{"code":"INGRESS_PROTOCOL_NOT_AVAILABLE"}'
            with self.assertRaises(AssertionError):
                journey.reject_gateway_request(SimpleNamespace(port=1), 'synthetic', 'removed', 'messages',
                    [source], ('AGENT_PROTOCOL_UNSUPPORTED',))


if __name__ == '__main__':
    unittest.main()
