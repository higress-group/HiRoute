#!/usr/bin/env python3
"""Security and evidence regressions for the opt-in real OAuth acceptance harness."""
from datetime import datetime, timezone
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import tempfile
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace


spec = importlib.util.spec_from_file_location(
    'managed_login_live', Path(__file__).with_name('cpa-managed-login-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def saved_source(provider='codex'):
    return {'source_id': 'source/' + provider, 'binding_id': 'binding/' + provider,
            'model_ref': 'model-ref/' + provider, 'model': provider + '-low-cost',
            'plan_id': 'plan/' + provider, 'alias': provider + '_public_alias',
            'connection': 'connection/' + provider, 'context': 'agent-context/' + provider}


def bound_product(source, provider='codex'):
    inventory = {'sources': [{'source_id': source['source_id'], 'state': 'ready', 'models': [{
        'model_ref': source['model_ref'], 'binding_id': source['binding_id'], 'upstream_model_id': source['model']}]}]}
    plan = {'agent_plan_id': source['plan_id'], 'head': {'model_alias': source['alias'],
        'reference': {'plan_id': source['plan_id']}}, 'desired': {'mode': 'fixed_model',
        'strategy': {'mode': 'custom', 'candidates': [{'binding_id': source['binding_id']}]}}}
    selection = ({'mode': 'codex_default', 'default_selection': {'kind': 'plan', 'plan_id': source['plan_id']},
                  'allowed_plan_ids': [source['plan_id']]} if provider == 'codex' else {
        'mode': 'claude_launcher', 'surfaces': ['claude_cli'], 'fixed_models': [], 'preset_mappings': {
            'opus': {'kind': 'plan', 'plan_id': source['plan_id']},
            'sonnet': {'kind': 'preserve_native'}, 'haiku': {'kind': 'preserve_native'}}})
    status = {'state': 'configured', 'context_id': source['context'], 'current_selection': selection}
    product = SimpleNamespace(bin=Path('/retained/target/debug'))
    descriptor = {'status': 'succeeded', 'data': {'connection_id': source['connection'],
        'helper_executable': str(product.bin / 'hiroute'), 'presets': {'opus': source['alias']},
        'helper_argv': ['__internal-agent-grant-v1', source['connection']]}}
    product.control = lambda operation, *args, **kw: ({'data': inventory}
                          if operation == 'ListCompute' else descriptor)
    product.public_cli = lambda command: (0, {'data': plan if command.startswith('routing show') else status})
    product.fixture_descriptor = descriptor
    return product, inventory, plan, status


class ProtectedCallback(unittest.TestCase):
    def test_secret_crosses_only_inherited_fd_and_not_arguments_environment_or_stdin(self):
        secret = 'private-callback-code#private-state'
        consumed = []

        def runner(arguments, **options):
            reader = options['pass_fds'][0]
            consumed.append(os.read(reader, 4097).decode())
            self.assertEqual(os.read(reader, 1), b'')
            self.assertNotIn(secret, json.dumps(arguments))
            self.assertNotIn(secret, json.dumps(options['env']))
            self.assertNotIn('input', options)
            self.assertEqual(arguments[arguments.index('--secret-fd') + 1], str(reader))
            return subprocess.CompletedProcess(arguments, 0, json.dumps({
                'data': {'registered': True}}).encode(), b'')

        live.register_callback({'product_bin': '/isolated/target/debug',
                                'product_root': '/isolated/product'},
                               {'candidate_ref': 'candidate/subscription-login/exact',
                                'candidate_revision': 1}, secret, runner=runner)
        self.assertEqual(consumed, [secret])

    def test_rejected_registration_does_not_embed_provider_secret_in_failure(self):
        secret = 'sensitive-callback'
        with self.assertRaises(live.LiveFailure) as caught:
            live.register_callback({'product_bin': '/isolated/target/debug',
                                    'product_root': '/isolated/product'},
                                   {'candidate_ref': 'candidate/exact'}, secret,
                                   runner=lambda *args, **kw: subprocess.CompletedProcess(
                                       args, 1, secret.encode(), secret.encode()))
        self.assertNotIn(secret, str(caught.exception))
        self.assertEqual(live.safe_code(caught.exception), 'protected_callback_public_output_leak')

    def test_without_controlling_terminal_oauth_is_never_started(self):
        arguments = type('Args', (), {'run_dir': Path('/private/run'),
                                      'provider': 'codex', 'no_browser': True})()
        with patch.object(live.os, 'open', side_effect=OSError('terminal unavailable')), \
                patch.object(live, 'request') as request:
            with self.assertRaises(OSError):
                live.authorize(arguments)
        request.assert_not_called()

    def test_browser_url_is_written_to_private_tty_and_absent_from_return_value(self):
        arguments = type('Args', (), {'run_dir': Path('/private/run'),
                                      'provider': 'claude', 'no_browser': True})()
        url = 'https://provider.invalid/oauth?private-state=do-not-log'
        callback = 'private-code#private-state'
        writes = []
        responses = [
            {'daemon_alive': True, 'public_control_ready': True},
            {'authorization_url': url, 'session': {'callback_input_candidate': {
                'candidate_ref': 'candidate/exact', 'candidate_revision': 1}}},
            {'state': 'green', 'provider': 'claude', 'status': 'authorized'}]
        with patch.object(live.os, 'open', return_value=70), \
                patch.object(live.os, 'isatty', return_value=True), \
                patch.object(live.os, 'close'), \
                patch.object(live.os, 'write', side_effect=lambda fd, raw: writes.append((fd, raw))), \
                patch.object(live, 'private_read', return_value={}), \
                patch.object(live, 'request', side_effect=responses), \
                patch.object(live, 'hidden_tty_input', return_value=callback), \
                patch.object(live, 'register_callback') as register:
            result = live.authorize(arguments)
        self.assertTrue(any(url.encode() in raw for _, raw in writes))
        self.assertTrue(all(fd == 70 for fd, _ in writes))
        self.assertFalse(any(callback.encode() in raw for _, raw in writes))
        self.assertNotIn(url, json.dumps(result))
        self.assertNotIn(callback, json.dumps(result))
        self.assertEqual(register.call_args.args[2], callback)

    def test_unready_authorization_never_starts_or_reads_callback(self):
        args = SimpleNamespace(run_dir=Path('/private/run'), provider='claude', no_browser=True)
        for status in ({'daemon_alive': False, 'public_control_ready': True},
                       {'daemon_alive': True, 'public_control_ready': False}, {'daemon_alive': True}):
            with patch.object(live.os, 'open', return_value=70), patch.object(live.os, 'isatty', return_value=True), \
                    patch.object(live.os, 'close'), patch.object(live.os, 'write') as writes, \
                    patch.object(live, 'private_read', return_value={}), \
                    patch.object(live, 'request', return_value=status) as request, \
                    patch.object(live, 'hidden_tty_input') as hidden, \
                    patch.object(live, 'register_callback') as register:
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.authorize(args)
            request.assert_called_once_with(args.run_dir, {'action': 'status'})
            writes.assert_not_called()
            hidden.assert_not_called()
            register.assert_not_called()

    def test_readiness_checks_public_control_and_rechecks_process_exit(self):
        ok = (0, {'status': 'succeeded', 'data': {'services': []}})
        cases = [([6], ok, (False, False)), ([None, 6], ok, (False, False)),
                 ([None, None], (6, {'status': 'unavailable'}), (True, False)),
                 ([None, None], (0, {'status': 'unavailable'}), (True, False)),
                 ([None, None], (True, {'status': 'succeeded'}), (True, False)),
                 ([None, None], ValueError('private-cli-detail'), (True, False)),
                 ([None, None], ok, (True, True))]
        for polls, response, expected in cases:
            cli = Mock(return_value=response, side_effect=response if isinstance(response, Exception) else None)
            product = SimpleNamespace(process=SimpleNamespace(poll=Mock(side_effect=polls)), public_cli=cli)
            self.assertEqual(live.runtime_support.authorization_readiness(product),
                             dict(zip(('daemon_alive', 'public_control_ready'), expected)))
            if polls[0] is None:
                cli.assert_called_once_with('decision services list', success=False)
            else:
                cli.assert_not_called()

    def test_supervisor_rechecks_control_before_login_start(self):
        supervisor = live.Supervisor.__new__(live.Supervisor)
        supervisor.product = SimpleNamespace(process=SimpleNamespace(poll=lambda: None),
                                             public_cli=Mock(return_value=(6, {'status': 'unavailable'})))
        supervisor.login = Mock()
        with self.assertRaises(live.runtime_support.RuntimeFailure):
            supervisor.action({'action': 'start', 'provider': 'claude'})
        supervisor.login.assert_not_called()


class IsolationAndEvidence(unittest.TestCase):
    def test_live_status_updates_terminal_state_without_losing_original_expiry(self):
        supervisor = live.Supervisor.__new__(live.Supervisor)
        original = {'access_sha256': 'original', 'access_expires_at_unix': 2000}
        supervisor.report = {'sessions': {'claude': {'login_ref': 'login-opaque',
                              'status': 'pending', 'original_credential': original}}}
        supervisor.login = lambda value: [{'login_ref': 'login-opaque', 'status': 'failed'}]
        supervisor.write_report = lambda: None
        updated = supervisor.refresh_session('claude')
        self.assertEqual(updated['status'], 'failed')
        self.assertIs(updated['original_credential'], original)
        self.assertEqual(original['access_expires_at_unix'], 2000)

    def test_ambiguous_callback_ack_records_real_state_without_replaying_submission(self):
        supervisor = live.Supervisor.__new__(live.Supervisor)
        supervisor.report = {'sessions': {'claude': {'login_ref': 'login-opaque',
            'status': 'pending', 'callback_input_candidate': {'candidate_ref': 'candidate/opaque'}}}}
        calls = []

        def login(value):
            calls.append(value['action'])
            if value['action'] == 'callback':
                raise live.LiveFailure('public_cli:DAEMON_UNAVAILABLE')
            return [{'login_ref': 'login-opaque', 'status': 'failed'}]

        supervisor.login, supervisor.write_report = login, lambda: None
        with self.assertRaises(live.LiveFailure) as caught:
            supervisor.action({'action': 'callback', 'provider': 'claude'})
        self.assertEqual(str(caught.exception), 'public_cli:DAEMON_UNAVAILABLE')
        self.assertEqual(calls, ['callback', 'status'])
        self.assertEqual(supervisor.report['sessions']['claude']['status'], 'failed')

    def test_selected_native_cpa_audit_rejects_refresh_and_expiry_without_rewriting_files(self):
        with tempfile.TemporaryDirectory() as name:
            storage = Path(name)
            root = storage / 'cpa'
            root.mkdir(mode=0o700)
            auth = root / 'auth'
            auth.mkdir(mode=0o700)
            (root / 'config.yaml').write_text('auth-dir: "' + str(auth) + '"\n')
            credential = auth / 'hiroute-managed-fixture.json'
            native = SimpleNamespace(forbidden_refresh=lambda value: 'refresh_token' in value)
            product = SimpleNamespace(storage=storage, repo=Path('/repository'))
            with patch.object(live, 'modules', return_value=(None, None, native)):
                for value in ({'access_token': 'fixture-access'},
                              {'access_token': 'fixture-access', 'refresh_token': 'never-copy-native-refresh'},
                              {'access_token': 'fixture-access', 'expired': '2030-01-01'}):
                    credential.write_text(json.dumps(value)); credential.chmod(0o600)
                    before = credential.read_bytes()
                    if len(value) == 1:
                        result = live.access_only_cpa_audit(product, 'codex')
                        self.assertFalse(result['refresh_authority'])
                        self.assertNotIn('fixture-access', json.dumps(result))
                    else:
                        with self.assertRaises(live.LiveFailure):
                            live.access_only_cpa_audit(product, 'codex')
                    self.assertEqual(credential.read_bytes(), before)

    def test_managed_auth_path_follows_real_role_all_cpa_state_parent(self):
        path = live.managed_auth_directory(Path('/private/storage'), 'login-opaque')
        self.assertEqual(path, Path('/private/storage/cpa/managed-logins/login-opaque/auth'))
        with self.assertRaises(live.LiveFailure):
            live.managed_auth_directory(Path('/private/storage'), '../native')

    def test_forget_proof_requires_a_preexisting_authorization_and_exact_removal(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name) / 'auth'
            with self.assertRaises(live.LiveFailure):
                live.prove_forget_removed(directory, {})
            directory.mkdir(mode=0o700)
            credential = directory / 'managed.json'
            credential.write_text('{}')
            credential.chmod(0o600)
            before = {'file_sha256': live.sha256(credential)}
            with self.assertRaises(live.LiveFailure):
                live.prove_forget_removed(directory, before)
            credential.unlink()
            with self.assertRaises(live.LiveFailure):
                live.prove_forget_removed(directory, before)
            directory.rmdir()
            live.prove_forget_removed(directory, before)

    def test_inherited_client_credentials_and_config_roots_are_scrubbed(self):
        inherited = {'HOME': '/native', 'CODEX_HOME': '/native/codex',
                     'CLAUDE_CONFIG_DIR': '/native/claude',
                     'CLAUDE_SECURESTORAGE_CONFIG_DIR': '/native/secure',
                     'CLAUDE_CODE_OAUTH_TOKEN': 'native-refresh-private',
                     'OPENAI_API_KEY': 'native-api-private',
                     'ANTHROPIC_AUTH_TOKEN': 'native-access-private',
                     'HIROUTE_RUNTIME_DIR': '/native/runtime',
                     'AWS_SECRET_ACCESS_KEY': 'native-cloud-private',
                     'LANG': 'C.UTF-8'}
        with patch.dict(os.environ, inherited, clear=True):
            result = live.isolated_environment('/private/isolated')
        serialized = json.dumps(result)
        self.assertNotIn('/native', serialized)
        self.assertNotIn('private', serialized.replace('/private/isolated', 'isolated'))
        for key in ('HOME', 'CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'CLAUDE_SECURESTORAGE_CONFIG_DIR',
                    'HIROUTE_RUNTIME_DIR', 'XDG_CONFIG_HOME'):
            self.assertTrue(result[key].startswith('/private/isolated/'))

    def test_credential_projection_keeps_original_expiry_and_only_digests(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / 'credentials.json'
            value = {'type': 'claude', 'access_token': 'fresh-access-private',
                     'refresh_token': 'fresh-refresh-private',
                     'email': 'private@example.invalid',
                     'account_uuid': 'private-provider-account',
                     'expired': '2030-01-01T00:00:00Z'}
            path.write_text(json.dumps(value))
            path.chmod(0o600)
            before = path.read_bytes()
            result = live.credential_projection(path, 'claude')
            self.assertEqual(path.read_bytes(), before)
            self.assertEqual(result['access_expires_at_unix'], 1893456000)
            serialized = json.dumps(result)
            for secret in (value['access_token'], value['refresh_token'], value['email'],
                           value['account_uuid'], str(path)):
                self.assertNotIn(secret, serialized)

    def test_symlink_or_nonprivate_credential_is_rejected_without_serialization(self):
        with tempfile.TemporaryDirectory() as name:
            source = Path(name) / 'owner.json'
            source.write_text('{"refresh_token":"native-refresh-private"}')
            source.chmod(0o600)
            link = Path(name) / 'link.json'
            link.symlink_to(source)
            with self.assertRaises(OSError):
                live.private_read(link)
            source.chmod(0o644)
            with self.assertRaises(live.LiveFailure) as caught:
                live.private_read(source)
            self.assertNotIn('native-refresh-private', str(caught.exception))

    def test_rotation_before_original_expiry_is_not_automatic_expiry_acceptance(self):
        original = {'access_sha256': 'original', 'access_expires_at_unix': 2000}
        rotated = {'access_sha256': 'rotated', 'access_expires_at_unix': 4000}
        result = live.refresh_verdict(original, rotated, 1999, 0)
        self.assertEqual(result['state'], 'not_executed')
        self.assertFalse(result['original_expiry_crossed'])
        self.assertTrue(result['access_rotated'])

    def test_expired_original_without_rotation_or_with_native_process_is_not_green(self):
        original = {'access_sha256': 'original', 'access_expires_at_unix': 2000}
        for current, native in [(original, 0),
                ({'access_sha256': 'rotated', 'access_expires_at_unix': 4000}, 1)]:
            self.assertEqual(live.refresh_verdict(original, current, 2001, native)['state'], 'not_executed')

    def test_unchanged_original_expiry_crossing_requires_rotated_extended_lease(self):
        original = {'access_sha256': 'original', 'access_expires_at_unix': 2000}
        rotated = {'access_sha256': 'rotated', 'access_expires_at_unix': 4000}
        result = live.refresh_verdict(original, rotated, 2001, 0)
        self.assertEqual(result['state'], 'green')
        self.assertFalse(result['expiry_was_modified'])
        self.assertFalse(result['forced_refresh'])
        self.assertEqual(original['access_expires_at_unix'], 2000)

    def test_unknown_error_text_cannot_enter_report(self):
        self.assertEqual(live.safe_code(RuntimeError('private@example.invalid token-secret')),
                         'RuntimeError')
        self.assertEqual(live.safe_code(live.LiveFailure('https://private.invalid/callback?code=x')),
                         'LiveFailure')


class NativeBusinessEvidence(unittest.TestCase):
    def test_claude_tool_use_and_matching_stream_result_remain_distinct_from_native_dialogue(self):
        product = SimpleNamespace(repo=Path('/repository'))
        source, calls = {}, []

        def gateway(product, provider, facts, stream, label):
            calls.append((stream, label))
            if label == 'claude-initial-inference':
                facts.update(tool_use_id='tool-fixture', tool_history=[{'role': 'assistant'}])
                return {'state': 'green', 'tool_round': 'tool_use', 'multi_turn_verified': False}
            return {'state': 'green', 'tool_round': 'tool_result', 'multi_turn_verified': True}

        native = SimpleNamespace(gateway=gateway)
        with patch.object(live, 'modules', return_value=(None, None, native)):
            first = live.gateway_roundtrip(product, 'claude', source, False, 'claude-managed-nonstream')
            second = live.gateway_roundtrip(product, 'claude', source, True, 'claude-managed-stream')
        self.assertEqual(calls, [(False, 'claude-initial-inference'), (True, 'claude-managed-stream')])
        self.assertEqual(first['scenario'], 'claude-managed-nonstream')
        self.assertEqual(first['tool_round'], 'tool_use')
        self.assertEqual(second['tool_round'], 'tool_result')
        self.assertTrue(second['multi_turn_verified'])
        self.assertNotIn('native', first['scenario'] + second['scenario'])

    def test_plain_answer_or_unverified_tool_result_cannot_satisfy_required_claude_round(self):
        product = SimpleNamespace(repo=Path('/repository'))
        source = {'tool_use_id': 'tool-fixture', 'tool_history': [{'role': 'assistant'}]}
        for result in ({'state': 'green', 'tool_round': None},
                       {'state': 'green', 'tool_round': 'tool_result', 'multi_turn_verified': False}):
            native = SimpleNamespace(gateway=lambda *args: dict(result))
            with patch.object(live, 'modules', return_value=(None, None, native)):
                with self.assertRaises(live.LiveFailure):
                    live.gateway_roundtrip(product, 'claude', source, True, 'claude-managed-stream')

    def test_successful_client_text_without_a_production_send_is_red(self):
        product = SimpleNamespace(root=Path('/private/product'), bin=Path('/retained/target/debug'),
                                  repo=Path('/repository'), env={}, project=Path('/private/project'), outputs=[])
        source = saved_source()
        stdout = b'\n'.join([json.dumps({'type': 'item.completed', 'item': {
            'type': 'agent_message', 'text': 'NATIVE_OK'}}).encode(),
            json.dumps({'type': 'turn.completed', 'usage': {
                'input_tokens': 10, 'output_tokens': 3}}).encode()])
        native = SimpleNamespace(wire_request_count=lambda product: 0,
                                 usage_numbers=lambda usage: usage)
        with patch.object(live, 'modules', return_value=(None, None, native)), \
                patch.object(live.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout, b'')), \
                patch.object(live.time, 'sleep'):
            with self.assertRaises(live.LiveFailure) as caught:
                live.native_roundtrip(product, 'codex', source)
        self.assertEqual(str(caught.exception), 'codex:native_client_no_production_upstream_send')

    def test_claude_dialogue_uses_production_launcher_and_saved_context(self):
        product = SimpleNamespace(root=Path('/private/product'), bin=Path('/retained/target/debug'),
                                  repo=Path('/repository'), env={}, project=Path('/private/project'), outputs=[])
        source = saved_source('claude')
        stdout = json.dumps({'type': 'result', 'is_error': False, 'result': 'NATIVE_OK',
                             'usage': {'input_tokens': 10, 'output_tokens': 3}}).encode()
        sends = iter([0, 1])
        native = SimpleNamespace(wire_request_count=lambda product: next(sends),
                                 usage_numbers=lambda usage: usage)
        with patch.object(live, 'modules', return_value=(None, None, native)), \
                patch.object(live.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout, b'')) as run, \
                patch.object(live.time, 'sleep'):
            result = live.native_roundtrip(product, 'claude', source)
        args = run.call_args.args[0]
        self.assertEqual(args[:4], ['/retained/target/debug/hiroute', 'agent', 'launch', '--agent'])
        self.assertEqual(args[args.index('--context') + 1], source['context'])
        self.assertEqual(args[args.index('--model') + 1], source['alias'])
        self.assertEqual(result['production_upstream_sends'], 1)


class ClosedFailureEvidence(unittest.TestCase):
    def test_public_launch_envelope_preserves_before_spawn_code_and_discards_message(self):
        private = 'callback-private-account-do-not-export'
        for schema in ({'schema': 'hiroute.machine-envelope/v2'},
                       {'schema_version': {'major': 2, 'minor': 0}}):
            raw = json.dumps(schema | {'status': 'denied', 'error': {
                'code': 'CAPABILITY_DENIED', 'message': private, 'details': {'account': private}}}).encode()
            projected = live.evidence.native_projection(raw, b'', 'claude', 4)
            self.assertEqual(projected['public_cli_errors'], [
                {'before_spawn': True, 'status': 'denied', 'code': 'CAPABILITY_DENIED'}])
            self.assertEqual(projected['error_codes'], ['CAPABILITY_DENIED'])
            self.assertEqual(projected['http_statuses'], [])
            self.assertNotIn(private, json.dumps(projected))

    def test_unknown_launch_code_or_wrong_schema_cannot_become_trusted_error(self):
        private = 'private-one-time-code'
        for schema, status in [('hiroute.machine-envelope/v2', 'denied'),
                               ('unknown-private-schema', 'denied'),
                               ('hiroute.machine-envelope/v2', private)]:
            raw = json.dumps({'schema': schema, 'status': status,
                              'error': {'code': private, 'message': private}}).encode()
            projected = live.evidence.native_projection(raw, b'', 'claude', 4)
            self.assertNotIn(private, json.dumps(projected))
            expected = [{'before_spawn': True, 'status': 'denied', 'code': 'other'}]
            self.assertEqual(projected['public_cli_errors'], expected if status == 'denied'
                             and schema == 'hiroute.machine-envelope/v2' else [])

    def test_native_provider_error_retains_explicit_parameter_without_raw_provider_message(self):
        error = {'type': 'error', 'error': {'type': 'invalid_request_error', 'param': 'thinking.type',
            'message': 'Adaptive thinking is not supported. secret-access-private private@example.invalid'}}
        raw = json.dumps({'type': 'result', 'subtype': 'error_during_execution', 'is_error': True,
                          'errors': ['API Error: 400 ' + json.dumps(error)]}).encode()
        projected = live.evidence.native_projection(raw, b'', 'claude', 1)
        self.assertEqual(projected['http_statuses'], [400])
        self.assertIn('invalid_request_error', projected['error_codes'])
        self.assertEqual(projected['explicit_error_parameters'], ['thinking.type'])
        self.assertIn('adaptive_thinking_unsupported', projected['message_categories'])
        self.assertIn('thinking', projected['parameter_mentions'])
        text = json.dumps(projected)
        for private in ('secret-access-private', 'private@example.invalid', error['error']['message']):
            self.assertNotIn(private, text)
        self.assertFalse(projected['raw_output_retained'])

    def test_unknown_error_values_and_successful_prompt_echo_are_never_exported(self):
        private = 'private-code-and-prompt'
        stdout = b'\n'.join((json.dumps({'type': 'assistant', 'message': {
            'content': private + ' max_tokens'}}).encode(), json.dumps({'type': 'result', 'is_error': True,
            'errors': [{'code': private, 'type': private, 'param': private, 'message': private}]}).encode()))
        projected = live.evidence.native_projection(stdout, b'<html>' + private.encode() + b'</html>', 'claude', 1)
        self.assertNotIn(private, json.dumps(projected))
        self.assertNotIn('max_tokens', projected['parameter_mentions'])
        self.assertEqual(projected['explicit_error_parameters'], ['other'])
        self.assertEqual(projected['http_statuses'], [])

    def test_gateway_message_and_unrecognized_stage_do_not_enter_failure_projection(self):
        private = 'private@example.invalid?code=do-not-log'
        product = SimpleNamespace(gateway_failures=[{'scenario': private, 'provider': private,
            'http_status': 400, 'new_upstream_wire_requests': 0,
            'error': {'code': private, 'type': 'invalid_request_error', 'phase': private, 'message': private}}])
        result = live.evidence.failure_projection(product, {'secret': private})
        self.assertNotIn(private, json.dumps(result))
        self.assertEqual(result['stage'], 'unknown')
        self.assertEqual(result['gateway_failures'][0]['scenario'], 'unknown')
        self.assertEqual(result['gateway_failures'][0]['error'], {
            'code': 'other', 'type': 'invalid_request_error', 'phase': 'other'})

    def test_real_client_exit_failure_is_projected_before_the_exception(self):
        stdout = json.dumps({'type': 'result', 'is_error': True, 'subtype': 'error_during_execution',
                             'errors': ['API Error: 400 {"error":{"type":"invalid_request_error"}}']}).encode()
        product = SimpleNamespace(root=Path('/private/product'), bin=Path('/retained/target/debug'),
            repo=Path('/repository'), env={}, project=Path('/private/project'), outputs=[])
        native = SimpleNamespace(wire_request_count=lambda product: 0)
        with patch.object(live, 'modules', return_value=(None, None, native)), \
                patch.object(live.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, stdout, b'')):
            with self.assertRaises(live.LiveFailure):
                live.native_roundtrip(product, 'claude', saved_source('claude'))
        failure = live.evidence.failure_projection(product, 'claude-real-native-client-dialogue')
        self.assertEqual(failure['native_client']['http_statuses'], [400])
        self.assertEqual(failure['stage'], 'claude-real-native-client-dialogue')

    def test_native_success_filters_usage_and_normalizes_codex_cache_without_exporting_answer(self):
        stdout = b'\n'.join((json.dumps({'type': 'item.completed', 'item': {
            'type': 'agent_message', 'text': 'NATIVE_OK'}}).encode(),
            json.dumps({'type': 'turn.completed', 'usage': {'input_tokens': 30, 'output_tokens': 2,
                'cached_input_tokens': 8, 'account': 'private-account', 'secret': 'private-token'}}).encode()))
        facts = live.evidence.native_success(stdout, 'codex', 'NATIVE_OK')
        self.assertTrue(facts['answer_verified'])
        self.assertEqual(facts['usage']['input_tokens_details'], {'cached_tokens': 8})
        self.assertNotIn('NATIVE_OK', json.dumps(facts))
        self.assertNotIn('private', json.dumps(facts))


    def test_client_normalized_error_keeps_unknown_parameter_and_true_terminal_error(self):
        raw = b'\n'.join((json.dumps({'type': 'assistant', 'error': 'unknown-sdk-error', 'message': {
            'content': [{'type': 'text', 'text': 'API Error: 400 Adaptive thinking is not supported.'}]}}).encode(),
            json.dumps({'type': 'result', 'subtype': 'success', 'is_error': True,
                'result': 'Model request failed.'}).encode()))
        projected = live.evidence.native_projection(raw, b'', 'claude', 1)
        self.assertEqual(projected['terminal_subtypes'], ['success'])
        self.assertEqual(projected['terminal_error_flags'], [True])
        self.assertEqual(projected['http_statuses'], [400])
        self.assertEqual(projected['error_codes'], ['other'])
        self.assertEqual(projected['explicit_error_parameters'], [])
        self.assertEqual(projected['message_categories'], ['adaptive_thinking_unsupported'])
        self.assertFalse(live.evidence.native_success(raw, 'claude', 'NATIVE_OK')['answer_verified'])


class IncrementalObservation(unittest.TestCase):
    baseline = {'totals': {'input': 10000, 'output': 600, 'cache_read': 200}, 'pending_requests': 0}
    rows = [{'usage': {'input_tokens': 20, 'output_tokens': 5, 'cache_read_input_tokens': 2}}]

    def test_historical_unparsed_send_stays_in_baseline_and_only_new_usage_is_accepted(self):
        current = {'totals': {'input': 10020, 'output': 605, 'cache_read': 202}, 'pending_requests': 0}
        result = live.evidence.observation_delta(self.baseline, current, self.rows)
        self.assertEqual(result['state'], 'green')
        self.assertEqual(result['delta'], {'input': 20, 'output': 5, 'cache_read': 2})
        self.assertEqual(result['parsed_response_count'], 1)
        self.assertEqual(result['baseline'], self.baseline)
        wrong = {'totals': {'input': 10021, 'output': 605, 'cache_read': 202}, 'pending_requests': 0}
        self.assertEqual(live.evidence.observation_delta(self.baseline, wrong, self.rows)['state'], 'red')

    def test_pending_or_missing_selected_usage_cannot_be_green(self):
        with self.assertRaises(live.evidence.EvidenceFailure):
            live.evidence.observation_delta(self.baseline, self.baseline, [])
        with self.assertRaises(live.evidence.EvidenceFailure):
            live.evidence.observation_delta(self.baseline, self.baseline | {'pending_requests': 1}, self.rows)

    def test_read_observation_uses_production_read_grant_and_exports_only_closed_totals(self):
        calls = []

        def control(operation, payload):
            calls.append(operation)
            self.assertEqual(operation, 'GetClientServiceStatus')
            return {'data': {'revisions': {'opaque': 1}}}

        def grant(operation, payload, label):
            calls.append(operation)
            self.assertEqual(operation, 'GetValueV2')
            self.assertTrue(payload['change_digest'].startswith('sha256:'))
            return 'synthetic-local-read-grant'

        def cli(command, payload, capability):
            calls.append(command)
            self.assertEqual(command, 'value show')
            self.assertEqual(capability, 'synthetic-local-read-grant')
            return 0, {'data': {'usage': [
                *({'metric': key, 'known_sum': value} for key, value in self.baseline['totals'].items()),
                {'metric': 'private-account', 'known_sum': 'private-token'}],
                'pending_requests': 0, 'private_metadata': 'private-token'}}

        result = live.evidence.read_observation(SimpleNamespace(control=control, grant=grant, cli=cli))
        self.assertEqual(calls, ['GetClientServiceStatus', 'GetValueV2', 'value show'])
        self.assertEqual(result, self.baseline)
        self.assertNotIn('private', json.dumps(result))

    def test_native_only_real_entry_adds_increment_gate_and_keeps_historical_failure(self):
        supervisor = live.Supervisor.__new__(live.Supervisor)
        supervisor.run_dir, supervisor.caller_sha = Path('/private/run'), 'exact-caller'
        supervisor.configuration = {'candidate_sha': 'exact-product', 'repository': '/repository'}
        historical = {'scenario': 'live_smoke', 'state': 'red', 'candidate_sha': 'old-product'}
        supervisor.report = {'state': 'red', 'scenarios': [historical],
            'sessions': {'codex': {'status': 'authorized'}}, 'sources': {'codex': saved_source()}}
        supervisor.product = SimpleNamespace()
        supervisor.audit, supervisor.write_report = lambda provider: {}, lambda: None
        supervisor.record = lambda row: supervisor.report['scenarios'].append(row)
        native = SimpleNamespace()
        result = {'scenario': 'codex-real-native-client-dialogue', 'provider': 'codex',
                  'state': 'green', 'usage': self.rows[0]['usage']}
        current = {'totals': {'input': 10020, 'output': 605, 'cache_read': 202}, 'pending_requests': 0}
        with patch.object(live, 'modules', return_value=(None, None, native, None, None, None, None)), \
                patch.object(live, 'native_roundtrip', return_value=result), \
                patch.object(live, 'gateway_roundtrip') as gateway, \
                patch.object(live.evidence, 'read_observation', side_effect=[self.baseline, current]):
            response = supervisor.smoke(['codex'], False, native_only=True)
        gateway.assert_not_called()
        self.assertIs(supervisor.report['scenarios'][0], historical)
        self.assertEqual(response['state'], 'red')
        self.assertEqual(response['current_run_state'], 'green')
        self.assertEqual(supervisor.report['smoke_runs'][0]['observation']['state'], 'green')
        self.assertEqual(supervisor.report['smoke_runs'][0]['scenario_start'], 1)


class OneShotCapabilityAndAttestation(unittest.TestCase):
    def test_apply_fixed_key_rejects_changed_request_but_scoped_key_reuses_only_same_receipt(self):
        receipts, dispatched = {}, []
        class Consumer:
            def control(self, operation, payload, *args, **kwargs):
                accepted = {key: value for key, value in payload.items() if key != 'idempotency_key'}
                index = (operation, payload['idempotency_key'])
                if index in receipts:
                    body, receipt = receipts[index]
                    return receipt if body == accepted else {'error': {'code': 'IDEMPOTENCY_KEY_REUSED'}}
                receipt = {'data': {'state': 'succeeded', 'operation_id': len(receipts) + 1}}
                receipts[index] = (copy.deepcopy(accepted), receipt)
                dispatched.append(copy.deepcopy(payload))
                return receipt
        class Bound(live.runtime_support.StableApplyMixin, Consumer):
            pass
        before = {'spec': {'candidate': 'candidate/first'}, 'accept_digest': 'sha256:first',
                  'expected_revisions': {'source': 1}, 'idempotency_key': 'same-business-label'}
        changed = dict(before, accept_digest='sha256:second')
        consumer = Consumer()
        consumer.control('ApplySubscriptionCheck', before)
        self.assertEqual(consumer.control('ApplySubscriptionCheck', changed)['error']['code'],
                         'IDEMPOTENCY_KEY_REUSED')
        original = copy.deepcopy(changed)
        bound = Bound()
        first = bound.control('ApplySubscriptionCheck', before)
        second = bound.control('ApplySubscriptionCheck', changed)
        self.assertNotEqual(first, second)
        self.assertIs(bound.control('ApplySubscriptionCheck', changed), second)
        self.assertEqual(changed, original)
        self.assertEqual(dispatched[-1]['spec'], changed['spec'])
        self.assertEqual(dispatched[-1]['accept_digest'], changed['accept_digest'])
        saved = bound.control('ApplyComputeSave', changed)
        self.assertNotEqual(saved, second)
        self.assertIs(bound.control('ApplyComputeSave', changed), saved)
        self.assertNotEqual(dispatched[-1]['idempotency_key'], dispatched[-2]['idempotency_key'])

    def test_stable_operation_key_covers_actual_digest_revisions_spec_and_field_order(self):
        payload = {'spec': {'candidate': 'candidate/exact'}, 'accept_digest': 'sha256:accept',
                   'change_digest': 'sha256:change', 'dependency_digest': 'sha256:dependency',
                   'expected_revisions': {'models': 2}, 'idempotency_key': 'original-label'}
        key = live.runtime_support.scoped_operation_key('ApplySubscriptionCheck', payload, 'label')
        reordered = dict(reversed(list(payload.items())))
        self.assertEqual(key, live.runtime_support.scoped_operation_key('ApplySubscriptionCheck', reordered, 'label'))
        for field in ('spec', 'accept_digest', 'change_digest', 'dependency_digest', 'expected_revisions'):
            changed = dict(payload, **{field: {'changed': True} if isinstance(payload[field], dict) else 'sha256:changed'})
            self.assertNotEqual(key, live.runtime_support.scoped_operation_key('ApplySubscriptionCheck', changed, 'label'))
        self.assertNotEqual(key, live.runtime_support.scoped_operation_key('ApplyComputeSave', payload, 'label'))
        self.assertNotEqual(key, live.runtime_support.scoped_operation_key('ApplySubscriptionCheck', payload, 'other-label'))
        self.assertEqual(payload['idempotency_key'], 'original-label')

    def test_native_selected_early_failure_retains_closed_module_projection_and_report(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            product = SimpleNamespace(secrets=set(), outputs=[], diagnostics_root=root, process=None,
                diagnostics_snapshot=lambda: {'state': 'complete'}, stop=lambda **kw: None,
                preserve_diagnostics=lambda path: None, listener_port_lease=SimpleNamespace(close=lambda: None))
            native = SimpleNamespace(remote_source=unittest.mock.Mock(side_effect=live.LiveFailure('borrow_denied')),
                wire_request_count=lambda p: 0, audit_native_sources=lambda *args: None)
            arguments = SimpleNamespace(provider='codex', repository=root, candidate_sha='exact-product',
                codex_host='fixture-host', claude_host='unselected-fixture', codex_cli=root / 'codex',
                claude_cli=root / 'claude', report=root / 'report.json')
            with patch.object(live, 'modules', return_value=(None, None, native, None, None, None, None)), \
                    patch.object(live, 'caller_harness_sha', return_value='exact-caller'), \
                    patch.object(live, 'make_product', return_value=product):
                result = live.native_borrowed_selected(arguments, {'binaries': {}, 'cpa': {}})
            self.assertEqual(result['state'], 'red')
            self.assertEqual(result['paid_inference_requests'], 0)
            report = live.private_read(arguments.report)
            self.assertEqual(report['failure']['code'], 'borrow_denied')
            self.assertEqual(report['failure']['gateway_failures'], [])
            self.assertNotIn('UnboundLocalError', json.dumps(report))

    def test_real_frame_producer_fixed_capability_repeats_but_nonce_preserves_business_key(self):
        Product = live.modules(Path(__file__).resolve().parents[1])[0]
        database = sqlite3.connect(':memory:')
        database.execute('CREATE TABLE grants (digest TEXT PRIMARY KEY, consumed INTEGER)')
        frames = []

        def register(frame):
            digest = hashlib.sha256(frame['capability'].encode()).hexdigest()
            database.execute('INSERT INTO grants VALUES (?, 1)', (digest,))
            frames.append(frame)

        original = Product.__new__(Product)
        original.secrets, original.register_protected_frame = set(), register
        revisions = {'models': 'synthetic-revision'}
        original.desktop_grant('ApplySubscriptionCheck', 'sha256:synthetic', revisions, 'stable-operation')
        with self.assertRaises(sqlite3.IntegrityError):
            original.desktop_grant('ApplySubscriptionCheck', 'sha256:synthetic', revisions, 'stable-operation')
        class Fresh(live.runtime_support.FreshCapabilityMixin, Product):
            pass
        resumed = Fresh.__new__(Fresh)
        resumed.secrets, resumed.register_protected_frame = set(), register
        preview = {'change_digest': 'sha256:synthetic', 'expected_revisions': revisions}
        body = {'idempotency_key': 'stable-operation', 'payload': 'synthetic'}
        for _ in range(2):
            resumed.desktop_grant('ApplySubscriptionCheck', 'sha256:synthetic', revisions,
                                  body['idempotency_key'])
            resumed.grant('GetValueV2', preview, body['idempotency_key'])
        self.assertEqual(len(frames), 5)
        self.assertEqual(len({frame['capability'] for frame in frames}), 5)
        self.assertEqual(body['idempotency_key'], 'stable-operation')
        for frame in frames:
            self.assertEqual(frame['expected_revisions'], revisions)
            self.assertEqual(frame['accepted_digest'], 'sha256:synthetic')
        database.close()

    def test_caller_attests_new_runtime_helper_and_rejects_dirty_untracked_or_symlink(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'scripts').mkdir()
            files = [root / 'scripts' / file for file in live.runtime_support.CALLER_FILES]
            for file in files:
                file.write_text('# synthetic caller\n')
            def git(*arguments):
                return subprocess.check_output(['git', '-C', name, *arguments], stderr=subprocess.DEVNULL)
            git('init', '-q')
            git('add', 'scripts')
            git('-c', 'user.name=Safety Fixture', '-c', 'user.email=fixture@invalid',
                'commit', '-qm', 'synthetic caller')
            expected = git('rev-parse', 'HEAD').decode().strip()
            self.assertEqual(live.runtime_support.caller_harness_sha(root), expected)
            helper = root / 'scripts/cpa-managed-login-runtime.py'
            helper.write_text('# uncommitted helper\n')
            with self.assertRaisesRegex(live.runtime_support.RuntimeFailure, 'caller_harness_uncommitted'):
                live.runtime_support.caller_harness_sha(root)
            git('checkout', '--', str(helper))
            git('rm', '--cached', str(helper))
            with self.assertRaises(live.runtime_support.RuntimeFailure):
                live.runtime_support.caller_harness_sha(root)
            helper.unlink()
            helper.symlink_to(files[0])
            with self.assertRaisesRegex(live.runtime_support.RuntimeFailure, 'caller_harness_file_missing'):
                live.runtime_support.caller_harness_sha(root)


class ManagedRebind(unittest.TestCase):
    def fixture(self, root, mutate=None):
        old = root / 'old/target/debug/hiroute'
        old.parent.mkdir(parents=True)
        old.write_text('synthetic executable')
        old.chmod(0o700)
        source = saved_source('claude')
        selection = {'mode': 'claude_launcher', 'presets': {'haiku': {'kind': 'plan', 'plan_id': source['plan_id']}}}
        state = {'apiKeyHelper': str(old) + ' __internal-agent-grant-v1 ' + source['connection'],
                 'env': {'window': 131072, 'proxy': 'synthetic'}, 'extra': {'preserve': True}}
        before = copy.deepcopy(state)
        calls = []
        product = SimpleNamespace(bin=root / 'new/target/debug', settings=root / 'settings',
                                  env={'HOME': '/isolated'}, project=root / 'project')
        rebound = False
        def control(operation, payload, **options):
            calls.append(operation)
            self.assertEqual(operation, 'GetManagedAgentLaunchDescriptor')
            if not rebound:
                return {'status': 'denied', 'error': {'code': 'CAPABILITY_DENIED'}}
            return {'status': 'succeeded', 'data': {'helper_executable': str(product.bin / 'hiroute'),
                                                    'connection_id': source['connection']}}
        def public(command, payload=None):
            nonlocal rebound
            calls.append(command)
            if command.startswith('agents connect status'):
                return 0, {'data': {'state': 'configured', 'current_selection': copy.deepcopy(selection),
                                    'protected_native_model_ids': []}}
            if command.startswith('agents check'):
                self.assertNotIn('--allow-model-call', command)
                self.assertIn('--scope native-authentication', command)
                return 0, {'data': {'state': 'succeeded'}}
            if command == 'agents connect preview':
                spec = copy.deepcopy(payload['spec'])
                if mutate:
                    mutate(spec, state)
                return 0, {'data': {'applicable': True, 'spec': spec, 'accept_digest': 'synthetic',
                                    'dependency_digest': 'synthetic', 'expected_revisions': {}}}
            if command == 'agents connect apply':
                self.assertEqual(payload['spec']['access_token'], {'intent': 'keep'})
                self.assertEqual(payload['spec']['collaboration'], {'intent': 'keep'})
                expected_key = live.runtime_support.scoped_operation_key(
                    'ApplyAgentConnectionChange', payload, 'managed-helper-' + source['context'])
                self.assertEqual(payload['idempotency_key'], expected_key)
                state['apiKeyHelper'] = str(product.bin / 'hiroute') + ' __internal-agent-grant-v1 ' + source['connection']
                rebound = True
                return 0, {'data': {'state': 'succeeded'}}
            self.fail('unexpected public command')
        product.control, product.public_cli = control, public
        reader = lambda path: copy.deepcopy(state)
        return product, source, old, reader, calls, before, state

    def test_path_change_reseals_only_helper_through_public_v2_and_version_launch(self):
        with tempfile.TemporaryDirectory() as name:
            product, source, old, reader, calls, before, state = self.fixture(Path(name))
            runner = unittest.mock.Mock(return_value=subprocess.CompletedProcess([], 0, b'2.1.295\n', b''))
            result = live.runtime_support.rebind_managed_claude(
                product, {'claude': source}, reader, previous_cli=old, runner=runner)
            self.assertEqual(result['state'], 'green')
            self.assertFalse(result['provider_model_calls_allowed'])
            self.assertEqual(before['env'], state['env'])
            self.assertEqual(before['extra'], state['extra'])
            self.assertEqual(calls.count('agents connect apply'), 1)
            self.assertEqual(runner.call_args.args[0][-2:], ['--', '--version'])
            runner.reset_mock()
            self.assertIsNone(live.runtime_support.rebind_managed_claude(
                product, {'claude': source}, reader, runner=runner))
            runner.assert_not_called()

    def test_rebind_rejects_changed_selection_access_or_owned_settings_without_native_call(self):
        mutations = [lambda spec, state: spec['model']['settings'].update(mode='other'),
                     lambda spec, state: spec['access_token'].update(intent='replace'),
                     lambda spec, state: state.pop('extra')]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)), tempfile.TemporaryDirectory() as name:
                product, source, old, reader, _, _, _ = self.fixture(Path(name), mutation)
                runner = unittest.mock.Mock()
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.rebind_managed_claude(
                        product, {'claude': source}, reader, previous_cli=old, runner=runner)
                runner.assert_not_called()

    def test_rebind_refuses_an_untrusted_previous_helper_or_extra_command(self):
        for replacement in ('/usr/bin/hiroute', 'not-absolute', 'previous-but-not-retargeted'):
            with self.subTest(replacement=replacement), tempfile.TemporaryDirectory() as name:
                product, source, old, reader, calls, _, state = self.fixture(Path(name))
                if replacement == 'previous-but-not-retargeted':
                    expected = Path('/unexpected/target/debug/hiroute')
                else:
                    state['apiKeyHelper'] = replacement + ' __internal-agent-grant-v1 ' + source['connection']
                    expected = old
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.rebind_managed_claude(product, {'claude': source}, reader,
                                                              previous_cli=expected)
                self.assertNotIn('agents connect apply', calls)


class BoundLifecycleContinuation(unittest.TestCase):
    def history_fixture(self, root, supervisor, provider='codex'):
        supervisor.run_dir = root
        if provider == 'codex':
            prior = supervisor.report['smoke_runs'][0]
            prior.pop('source_snapshots')
            prior.pop('scenario_end')
            proof = {'schema': 'hiroute.cpa-managed-disable-closed-cause/v1',
                'first_failure_stage': 'protected_registration_before_ApplySubscriptionCheck',
                'diagnostic_conclusion': 'harness_reused_synthetic_one_shot_capability_rejected_on_insert'}
            purpose, proof_name = 'codex_remaining_lifecycle', 'codex-disable-d44-closed-cause.json'
        else:
            proof = {'schema': 'hiroute.cpa-managed-rebind/v1', 'selection_exactly_preserved': True,
                'all_settings_fields_except_helper_preserved': True,
                'oauth_credential_hashes_and_original_expiry_unchanged': True, 'direct_storage_edits': False}
            purpose, proof_name = 'claude_native_reuse', 'claude-rebind-d44.json'
        proof.update(state='green', candidate_sha='exact-product', caller_harness_sha='old-caller')
        files = {'original_report': ('report-d44-c157-before-caller-repair.json', copy.deepcopy(supervisor.report)),
                 'identity_report': ('report-e303-c012-before-integration.json',
                                     {'sources': copy.deepcopy(supervisor.report['sources'])}),
                 'boundary_proof': (proof_name, proof)}
        manifest = {'schema': 'hiroute.cpa-managed-history/v1', 'provider': provider,
                    'purpose': purpose, 'run_index': 0, 'scenario_end': 4}
        for kind, (file, value) in files.items():
            live.private_write(root / file, value)
            manifest[kind] = {'file': file, 'sha256': live.sha256(root / file)}
        path = root / ('history-' + provider + '-fixture.json')
        live.private_write(path, manifest)
        return path, manifest, files

    def fixture(self):
        source = saved_source()
        usage = {'input_tokens': 10, 'output_tokens': 2, 'cache_read_input_tokens': 1}
        before = {'totals': {'input': 100, 'output': 20, 'cache_read': 10}, 'pending_requests': 0}
        current = {'totals': {'input': 130, 'output': 26, 'cache_read': 13}, 'pending_requests': 0}
        completed = [{'scenario': scenario, 'provider': 'codex', 'state': 'green',
                      'usage': dict(usage), 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller'}
            for scenario in ('codex-managed-nonstream', 'codex-managed-stream', 'codex-real-native-client-dialogue')]
        failure = {'scenario': 'live_smoke', 'state': 'red', 'stage': 'codex-managed-disable'}
        prior = {'selected_providers': ['codex'], 'state': 'red', 'native_only': False,
                 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller',
                 'scenario_start': 0, 'scenario_end': 4, 'failure': failure, 'baseline': before,
                 'source_snapshots': {'codex': live.runtime_support.source_fingerprint(source)}}
        product, inventory, plan, status = bound_product(source)
        supervisor = live.Supervisor.__new__(live.Supervisor)
        supervisor.configuration = {'candidate_sha': 'exact-product', 'repository': '/repository'}
        supervisor.report = {'state': 'red', 'sources': {'codex': source}, 'smoke_runs': [prior],
                              'sessions': {'codex': {'status': 'authorized'}}, 'scenarios': completed + [failure]}
        supervisor.product = product
        supervisor.caller_sha, supervisor.run_dir = 'new-caller', Path('/private/run')
        supervisor.audit, supervisor.write_report = lambda provider: {}, lambda: None
        supervisor.record = lambda row: supervisor.report['scenarios'].append(row)
        return supervisor, current, inventory, plan, status

    def test_continuation_requires_exact_prior_usage_boundary_and_current_saved_identity(self):
        supervisor, current, *_ = self.fixture()
        carried, rows, before = live.runtime_support.continuation(supervisor, ['codex'], 0, current, live.evidence)
        self.assertEqual(carried['prior_observation']['parsed_response_count'], 3)
        self.assertEqual(len(rows), 3)
        self.assertEqual(before['totals']['input'], 100)
        self.assertEqual(supervisor.report['smoke_runs'][0]['state'], 'red')

    def test_candidate_provider_source_connection_or_boundary_changes_fail_before_continuation(self):
        mutations = [lambda s, b, i, p, c: s.configuration.update(candidate_sha='different'),
            lambda s, b, i, p, c: s.report['smoke_runs'][0].update(selected_providers=['claude']),
            lambda s, b, i, p, c: s.report['sources']['codex'].update(binding_id='changed'),
            lambda s, b, i, p, c: i['sources'][0]['models'][0].update(binding_id='changed'),
            lambda s, b, i, p, c: p['head'].update(model_alias='different'),
            lambda s, b, i, p, c: c['current_selection'].update(allowed_plan_ids=[]),
            lambda s, b, i, p, c: s.report['smoke_runs'][0].update(scenario_end=3),
            lambda s, b, i, p, c: b['totals'].update(input=131),
            lambda s, b, i, p, c: b.update(pending_requests=1)]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                supervisor, baseline, inventory, plan, status = self.fixture()
                mutation(supervisor, baseline, inventory, plan, status)
                with self.assertRaises((live.runtime_support.RuntimeFailure, live.evidence.EvidenceFailure)):
                    live.runtime_support.continuation(supervisor, ['codex'], 0, baseline, live.evidence)
                self.assertEqual(len(supervisor.report['smoke_runs']), 1)

    def test_live_continuation_adds_only_one_remaining_send_and_keeps_old_red(self):
        supervisor, baseline, *_ = self.fixture()
        prior = copy.deepcopy(supervisor.report['smoke_runs'][0])
        after = {'totals': {'input': 140, 'output': 28, 'cache_read': 14}, 'pending_requests': 0}
        def lifecycle(s, *args):
            s.record({'scenario': 'codex-managed-after-restart', 'provider': 'codex', 'state': 'green',
                      'usage': {'input_tokens': 10, 'output_tokens': 2, 'cache_read_input_tokens': 1}})
        with patch.object(live, 'modules', return_value=(None, None, None, None, None, None, None)), \
                patch.object(live.evidence, 'read_observation', side_effect=[baseline, after]), \
                patch.object(live.runtime_support, 'lifecycle', side_effect=lifecycle), \
                patch.object(live, 'native_roundtrip') as native, \
                patch.object(live, 'gateway_roundtrip') as gateway:
            result = supervisor.smoke(['codex'], True, continue_run=0)
        native.assert_not_called()
        gateway.assert_not_called()
        self.assertEqual(supervisor.report['smoke_runs'][0], prior)
        self.assertEqual(result['state'], 'red')
        self.assertEqual(result['current_run_state'], 'green')
        current = supervisor.report['smoke_runs'][1]
        self.assertEqual(current['observation']['parsed_response_count'], 1)
        self.assertEqual(current['cumulative_observation']['parsed_response_count'], 4)

    def test_legacy_missing_identity_uses_explicit_digest_manifest_without_backfilling_old_run(self):
        with tempfile.TemporaryDirectory() as name:
            supervisor, baseline, *_ = self.fixture()
            path, _, _ = self.history_fixture(Path(name), supervisor)
            original = copy.deepcopy(supervisor.report)
            carried, rows, _ = live.runtime_support.continuation(
                supervisor, ['codex'], 0, baseline, live.evidence, path, live.private_read)
            self.assertEqual(supervisor.report, original)
            self.assertEqual(len(rows), 3)
            self.assertTrue(carried['reconstructed_from']['historical_fields_were_not_backfilled'])
            self.assertNotIn('source_snapshots', supervisor.report['smoke_runs'][0])
            self.assertNotIn('scenario_end', supervisor.report['smoke_runs'][0])

    def test_history_digest_identity_and_before_apply_proof_are_all_required(self):
        for mutation in ('digest', 'identity', 'boundary', 'scenario'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                supervisor, baseline, *_ = self.fixture()
                path, manifest, files = self.history_fixture(root, supervisor)
                if mutation == 'digest':
                    manifest['original_report']['sha256'] = 'wrong-digest'
                elif mutation == 'scenario':
                    supervisor.report['scenarios'][0]['usage']['input_tokens'] += 1
                else:
                    kind = 'identity_report' if mutation == 'identity' else 'boundary_proof'
                    file, value = files[kind]
                    if mutation == 'identity':
                        value['sources']['codex']['binding_id'] = 'changed'
                    else:
                        value['first_failure_stage'] = 'unknown_after_apply'
                    live.private_write(root / file, value)
                    manifest[kind]['sha256'] = live.sha256(root / file)
                live.private_write(path, manifest)
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.continuation(
                        supervisor, ['codex'], 0, baseline, live.evidence, path, live.private_read)

    def test_history_manifest_cannot_open_a_credential_or_outside_run_path(self):
        supervisor, *_ = self.fixture()
        reader = unittest.mock.Mock()
        for path in ('/private/run/product/storage/cpa/auth/managed.json', '/outside/history-codex-fixture.json'):
            with self.assertRaises(live.runtime_support.RuntimeFailure):
                live.runtime_support.reconstructed_history(supervisor, 'codex', path, reader,
                                                          'codex_remaining_lifecycle')
        reader.assert_not_called()

    def test_native_reuse_is_validated_before_gateway_and_new_usage_excludes_old_dialogue(self):
        supervisor, baseline, *_ = self.fixture()
        source = saved_source('claude')
        supervisor.report['sources'] = {'claude': source}
        supervisor.report['sessions'] = {'claude': {'status': 'authorized'}}
        supervisor.product = bound_product(source, 'claude')[0]
        original_native = {'scenario': 'claude-real-native-client-dialogue', 'provider': 'claude',
            'state': 'green', 'model': source['model'], 'candidate_sha': 'exact-product',
            'caller_harness_sha': 'old-caller', 'source_identity_sha256': live.runtime_support.source_fingerprint(source),
            'answer_verified': True, 'production_upstream_sends': 1, 'client_evidence': {'exit_code': 0},
            'usage': {'input_tokens': 1600, 'output_tokens': 7}}
        supervisor.report['scenarios'].append(original_native)
        original = copy.deepcopy(original_native)
        usage = {'input_tokens': 10, 'output_tokens': 2, 'cache_read_input_tokens': 1}
        def gateway(product, provider, saved, stream, scenario):
            return {'scenario': scenario, 'provider': provider, 'state': 'green', 'usage': dict(usage)}
        def lifecycle(s, *args):
            s.record(gateway(None, 'claude', source, False, 'claude-managed-after-restart'))
        after = {'totals': {'input': 160, 'output': 32, 'cache_read': 16}, 'pending_requests': 0}
        with patch.object(live, 'modules', return_value=(None, None, None, None, None, None, None)), \
                patch.object(live.evidence, 'read_observation', side_effect=[baseline, after]), \
                patch.object(live, 'gateway_roundtrip', side_effect=gateway), \
                patch.object(live.runtime_support, 'lifecycle', side_effect=lifecycle), \
                patch.object(live, 'native_roundtrip') as native:
            result = supervisor.smoke(['claude'], True, reuse_native=True)
        native.assert_not_called()
        self.assertEqual(result['current_run_state'], 'green')
        self.assertEqual(original_native, original)
        self.assertEqual(supervisor.report['smoke_runs'][-1]['observation']['parsed_response_count'], 3)
        original_native['state'] = 'red'
        with patch.object(live, 'modules', return_value=(None, None, None, None, None, None, None)), \
                patch.object(live.evidence, 'read_observation', return_value=after), \
                patch.object(live, 'gateway_roundtrip') as gateway:
            with self.assertRaises(live.runtime_support.RuntimeFailure):
                supervisor.smoke(['claude'], False, reuse_native=True)
        gateway.assert_not_called()

    def test_legacy_native_identity_reuse_records_manifest_and_preserves_original_row(self):
        with tempfile.TemporaryDirectory() as name:
            supervisor, *_ = self.fixture()
            source = saved_source('claude')
            supervisor.report['sources']['claude'] = source
            supervisor.product = bound_product(source, 'claude')[0]
            row = {'scenario': 'claude-real-native-client-dialogue', 'provider': 'claude', 'model': source['model'],
                'state': 'green', 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller',
                'answer_verified': True, 'production_upstream_sends': 1, 'client_evidence': {'exit_code': 0}}
            supervisor.report['scenarios'].append(row)
            path, _, _ = self.history_fixture(Path(name), supervisor, 'claude')
            result = live.runtime_support.reused_native_evidence(supervisor, 'claude', source, path, live.private_read)
            self.assertTrue(result['reconstructed_from']['historical_fields_were_not_backfilled'])
            self.assertNotIn('source_identity_sha256', row)
            row['client_evidence']['exit_code'] = 1
            with self.assertRaises(live.runtime_support.RuntimeFailure):
                live.runtime_support.reused_native_evidence(supervisor, 'claude', source, path, live.private_read)

    def test_native_reuse_preserves_original_revision_without_duplicate_usage(self):
        supervisor, _, *_ = self.fixture()
        source = saved_source('claude')
        supervisor.product = bound_product(source, 'claude')[0]
        native = {'scenario': 'claude-real-native-client-dialogue', 'provider': 'claude',
                  'model': source['model'], 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller',
                  'source_identity_sha256': live.runtime_support.source_fingerprint(source), 'state': 'green',
                  'answer_verified': True, 'production_upstream_sends': 1, 'client_evidence': {'exit_code': 0},
                  'usage': {'input_tokens': 1600, 'output_tokens': 7}}
        supervisor.report['scenarios'].append(native)
        result = live.runtime_support.reused_native_evidence(supervisor, 'claude', source)
        self.assertEqual(result['original_caller_harness_sha'], 'old-caller')
        self.assertFalse(result['parsed_usage_counted_again'])
        self.assertNotIn('usage', result)
        for key, value in [('state', 'red'), ('model', 'changed'), ('candidate_sha', 'old-product'),
                           ('source_identity_sha256', 'different'), ('production_upstream_sends', 0)]:
            with self.subTest(key=key):
                altered = dict(native, **{key: value})
                supervisor.report['scenarios'] = [altered]
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.reused_native_evidence(supervisor, 'claude', source)

    def test_native_reuse_rejects_current_selection_or_plan_drift_with_old_report_unchanged(self):
        source = saved_source('claude')
        mutations = [lambda i, p, s, d: s['current_selection'].update(mode='codex_default'),
            lambda i, p, s, d: s['current_selection']['preset_mappings']['opus'].update(plan_id='plan/other'),
            lambda i, p, s, d: p['desired']['strategy']['candidates'][0].update(binding_id='binding/other'),
            lambda i, p, s, d: i['sources'][0]['models'][0].update(upstream_model_id='other-model'),
            lambda i, p, s, d: s.update(context_id='agent-context/other')]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                supervisor, *_ = self.fixture()
                supervisor.product, inventory, plan, status = bound_product(source, 'claude')
                row = {'scenario': 'claude-real-native-client-dialogue', 'provider': 'claude',
                    'model': source['model'], 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller',
                    'source_identity_sha256': live.runtime_support.source_fingerprint(source), 'state': 'green',
                    'answer_verified': True, 'production_upstream_sends': 1, 'client_evidence': {'exit_code': 0}}
                supervisor.report['scenarios'] = [row]
                before = copy.deepcopy(supervisor.report)
                mutation(inventory, plan, status, supervisor.product.fixture_descriptor)
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.reused_native_evidence(supervisor, 'claude', source)
                self.assertEqual(supervisor.report, before)

    def test_native_reuse_rejects_current_descriptor_denial_connection_helper_or_alias_drift(self):
        source = saved_source('claude')
        mutations = [lambda d: d.update(status='denied', error={'code': 'CAPABILITY_DENIED'}),
            lambda d: d['data'].update(connection_id='connection/other'),
            lambda d: d['data'].update(helper_executable='/old/target/debug/hiroute'),
            lambda d: d['data'].update(helper_argv=['__internal-agent-grant-v1', 'connection/other']),
            lambda d: d['data']['presets'].update(opus='different_alias')]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                supervisor, *_ = self.fixture()
                supervisor.product = bound_product(source, 'claude')[0]
                row = {'scenario': 'claude-real-native-client-dialogue', 'provider': 'claude',
                    'model': source['model'], 'candidate_sha': 'exact-product', 'caller_harness_sha': 'old-caller',
                    'source_identity_sha256': live.runtime_support.source_fingerprint(source), 'state': 'green',
                    'answer_verified': True, 'production_upstream_sends': 1, 'client_evidence': {'exit_code': 0}}
                supervisor.report['scenarios'] = [row]
                before = copy.deepcopy(supervisor.report)
                mutation(supervisor.product.fixture_descriptor)
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    live.runtime_support.reused_native_evidence(supervisor, 'claude', source)
                self.assertEqual(supervisor.report, before)

    def test_status_distinguishes_observed_process_exit_from_inference_failure(self):
        supervisor, _, *_ = self.fixture()
        supervisor.product.process = SimpleNamespace(poll=lambda: 6)
        response = supervisor.action({'action': 'status'})
        self.assertFalse(response['daemon_alive'])
        self.assertEqual(response['daemon_exit_code'], 6)
        self.assertEqual(response['sessions'], {'codex': 'authorized'})


if __name__ == '__main__':
    unittest.main()
