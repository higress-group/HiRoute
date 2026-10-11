#!/usr/bin/env python3
"""Isolated installation, one-shot grants and saved-source lifecycle mechanics.

All configuration changes use the public V2 producer. This helper neither opens
provider credentials nor modifies runtime storage. Report rows keep their original
producer revision when a completed business request is reused.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import stat
import subprocess


class RuntimeFailure(Exception):
    pass


def require(condition, code):
    if not condition:
        raise RuntimeFailure(code)


def authorization_readiness(product):
    """Prove public control responds, without asserting provider readiness."""
    process = product.process
    readiness = {'daemon_alive': process is not None and process.poll() is None,
                 'public_control_ready': False}
    if not readiness['daemon_alive']:
        return readiness
    try:
        exit_code, envelope = product.public_cli('decision services list', success=False)
    except Exception:
        # Public CLI failures may contain private diagnostics. Only closed flags
        # leave this probe; no raw subprocess output or exception is returned.
        exit_code, envelope = None, None
    readiness['daemon_alive'] = process.poll() is None
    readiness['public_control_ready'] = (
        readiness['daemon_alive'] and type(exit_code) is int and exit_code == 0
        and isinstance(envelope, dict) and envelope.get('status') == 'succeeded')
    return readiness


def require_authorization_readiness(status):
    require(isinstance(status, dict) and status.get('daemon_alive') is True,
            'authorization_daemon_unavailable')
    require(status.get('public_control_ready') is True, 'authorization_control_unavailable')
    return status


def fresh_grant_key(key):
    require(isinstance(key, str) and bool(key), 'grant_key_invalid')
    return hashlib.sha256(key.encode()).hexdigest()[:24] + '-' + secrets.token_hex(16)


class FreshCapabilityMixin:
    # The business Operation keeps its caller's idempotency key. Only the one-use
    # launcher capability receives a new nonce, including across process resumes.
    def grant(self, operation, preview, key):
        return super().grant(operation, preview, fresh_grant_key(key))

    def desktop_grant(self, operation, accepted_digest, expected_revisions, key):
        return super().desktop_grant(operation, accepted_digest, expected_revisions,
                                     fresh_grant_key(key))


def scoped_operation_key(operation, payload, key):
    require(isinstance(operation, str) and bool(operation) and isinstance(payload, dict)
            and isinstance(key, str) and bool(key), 'operation_key_input_invalid')
    accepted = {field: value for field, value in payload.items() if field != 'idempotency_key'}
    value = {'schema': 'hiroute.managed-operation-key/v1', 'operation': operation,
             'business_label': key, 'accepted_payload': accepted}
    return 'managed-operation-' + hashlib.sha256(json.dumps(
        value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


class StableApplyMixin:
    # Capability randomness and business idempotence serve different contracts.
    # Bind only the direct subscription helpers to their actual accepted request;
    # an identical request retries its original receipt across process resumes.
    def control(self, operation, payload, *args, **kwargs):
        if operation in ('ApplySubscriptionCheck', 'ApplyComputeSave'):
            require(isinstance(payload, dict), 'operation_payload_invalid')
            key = scoped_operation_key(operation, payload, payload.get('idempotency_key'))
            payload = dict(payload, idempotency_key=key)
        return super().control(operation, payload, *args, **kwargs)


CALLER_FILES = ('cpa-managed-login-live.py', 'cpa-managed-login-evidence.py',
                'cpa-managed-login-runtime.py', 'test-cpa-managed-login-live-safety.py')


def caller_harness_sha(repository):
    files = ['scripts/' + name for name in CALLER_FILES]
    require(all((Path(repository) / path).is_file() and not (Path(repository) / path).is_symlink()
                for path in files), 'caller_harness_file_missing')
    dirty = subprocess.check_output(['git', '-C', str(repository), 'status', '--porcelain',
                                     '--untracked-files=normal', '--', *files])
    require(not dirty, 'caller_harness_uncommitted')
    tracked = subprocess.check_output(['git', '-C', str(repository), 'ls-files', '--', *files],
                                      text=True).splitlines()
    require(set(tracked) == set(files), 'caller_harness_file_untracked')
    return subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'], text=True).strip()


IDENTITY_FIELDS = ('source_id', 'binding_id', 'model_ref', 'model', 'plan_id',
                   'alias', 'connection', 'context')


def source_fingerprint(source):
    require(all(isinstance(source.get(key), str) and 0 < len(source[key]) <= 512
                for key in IDENTITY_FIELDS), 'saved_source_identity_missing')
    value = {key: source[key] for key in IDENTITY_FIELDS}
    return hashlib.sha256(json.dumps(value, sort_keys=True,
                                    separators=(',', ':')).encode()).hexdigest()


def rebind_managed_claude(product, sources, private_read, previous_cli=None,
                          runner=subprocess.run):
    source = sources.get('claude')
    if source is None:
        return None
    source_fingerprint(source)
    expected_cli = product.bin / 'hiroute'
    descriptor = product.control('GetManagedAgentLaunchDescriptor',
                                 {'connection_id': source['connection']}, success=False)
    if descriptor.get('status') == 'succeeded':
        require(descriptor['data'].get('helper_executable') == str(expected_cli),
                'managed_helper_current_path_mismatch')
        return None
    require((descriptor.get('error') or {}).get('code') == 'CAPABILITY_DENIED',
            'managed_descriptor_unavailable')
    status = product.public_cli('agents connect status ' + source['context'])[1]['data']
    require(status.get('state') == 'configured', 'managed_connection_not_configured')
    selection = status.get('current_selection')
    require(isinstance(selection, dict) and selection.get('mode') == 'claude_launcher',
            'managed_connection_selection_invalid')
    settings = private_read(product.settings)
    try:
        helper = shlex.split(settings.get('apiKeyHelper', ''))
    except ValueError:
        raise RuntimeFailure('managed_helper_shape_invalid') from None
    require(len(helper) == 3 and helper[1:] == [
        '__internal-agent-grant-v1', source['connection']], 'managed_helper_shape_invalid')
    old_cli = Path(helper[0])
    require(old_cli.is_absolute() and old_cli != expected_cli and old_cli.name == 'hiroute'
            and old_cli.parent.name == 'debug' and old_cli.parent.parent.name == 'target',
            'managed_helper_previous_path_invalid')
    info = old_cli.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
            and info.st_mode & 0o111, 'managed_helper_previous_executable_invalid')
    if previous_cli is not None:
        require(old_cli == Path(previous_cli), 'managed_helper_previous_path_mismatch')
    # Local native evidence is process-local; this registered check never allows a
    # model call and does not run a client in its original native HOME.
    product.public_cli('agents check agent_claude_default --scope native-authentication --suite quick')
    spec = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': source['context'],
            'model': {'intent': 'configure', 'settings': selection},
            'collaboration': {'intent': 'keep'}, 'access_token': {'intent': 'keep'},
            'protected_native_model_ids': status.get('protected_native_model_ids', [])}
    preview = product.public_cli('agents connect preview', {'spec': spec})[1]['data']
    normalized = preview.get('spec') or {}
    require(preview.get('applicable') is True and normalized.get('context_id') == source['context']
            and normalized.get('model', {}).get('settings') == selection
            and normalized.get('access_token', {}).get('intent') == 'keep'
            and normalized.get('collaboration', {}).get('intent') == 'keep',
            'managed_rebind_preview_changed_selection')
    apply_body = {
        'spec': normalized, 'accept_digest': preview['accept_digest'],
        'dependency_digest': preview['dependency_digest'],
        'expected_revisions': preview['expected_revisions']}
    apply_body['idempotency_key'] = scoped_operation_key(
        'ApplyAgentConnectionChange', apply_body, 'managed-helper-' + source['context'])
    applied = product.public_cli('agents connect apply', apply_body)[1]['data']
    require(applied.get('state') == 'succeeded', 'managed_rebind_not_succeeded')
    current = product.public_cli('agents connect status ' + source['context'])[1]['data']
    after = private_read(product.settings)
    require(current.get('state') == 'configured' and current.get('current_selection') == selection
            and {key: value for key, value in settings.items() if key != 'apiKeyHelper'}
                == {key: value for key, value in after.items() if key != 'apiKeyHelper'},
            'managed_rebind_changed_owned_settings')
    current_descriptor = product.control('GetManagedAgentLaunchDescriptor',
                                        {'connection_id': source['connection']})['data']
    require(current_descriptor.get('helper_executable') == str(expected_cli)
            and current_descriptor.get('connection_id') == source['connection'],
            'managed_rebind_descriptor_mismatch')
    version = runner([str(expected_cli), 'agent', 'launch', '--agent', 'claude-code',
                      '--context', source['context'], '--', '--version'],
                     capture_output=True, timeout=45, env=product.env, cwd=product.project)
    match = re.search(rb'\b\d+\.\d+\.\d+\b', version.stdout)
    require(version.returncode == 0 and match is not None, 'managed_rebind_version_launch_failed')
    return {'scenario': 'claude-managed-helper-rebind', 'provider': 'claude', 'state': 'green',
            'source_identity_sha256': source_fingerprint(source), 'public_v2_apply': True,
            'selection_preserved': True, 'owned_settings_except_helper_preserved': True,
            'access_token_intent': 'keep', 'provider_model_calls_allowed': False,
            'version_only_launch_exit': version.returncode, 'native_version': match.group().decode()}


def lifecycle(supervisor, providers, sources, native, apply_control, prepare, gateway_roundtrip):
    for provider in providers:
        source = sources[provider]
        supervisor.stage = provider + '-managed-disable'
        native.save_enabled(supervisor.product, provider, source, False, apply_control, prepare,
                            provider + '-managed-disable', supervisor.report['scenarios'])
        supervisor.stage = provider + '-managed-disabled-rejection'
        supervisor.record(native.gateway(supervisor.product, provider, source, False,
                          provider + '-managed-disabled-rejection', allowed=False))
        supervisor.stage = provider + '-managed-reenable'
        native.save_enabled(supervisor.product, provider, source, True, apply_control, prepare,
                            provider + '-managed-reenable', supervisor.report['scenarios'])
    supervisor.stage = 'restart'
    supervisor.product.stop()
    supervisor.product.start()
    supervisor.verify_debug()
    for provider in providers:
        supervisor.stage = provider + '-managed-after-restart'
        status = supervisor.login({'action': 'status',
            'login_ref': supervisor.report['sessions'][provider]['login_ref']})[0]
        require(status['status'] == 'authorized', 'saved_login_not_restored')
        supervisor.audit(provider)
        supervisor.record(gateway_roundtrip(supervisor.product, provider, sources[provider], False,
                                            provider + '-managed-after-restart'))


def reconstructed_history(supervisor, provider, manifest_path, private_read, purpose):
    require(manifest_path is not None, 'historical_identity_manifest_required')
    manifest_path = Path(manifest_path)
    require(manifest_path.is_absolute() and manifest_path.parent == supervisor.run_dir
            and re.fullmatch(r'history-(?:codex|claude)-[a-z0-9-]+\.json', manifest_path.name),
            'historical_identity_manifest_path_invalid')
    manifest = private_read(manifest_path)
    require(manifest.get('schema') == 'hiroute.cpa-managed-history/v1'
            and manifest.get('provider') == provider and manifest.get('purpose') == purpose,
            'historical_identity_manifest_invalid')
    names = {'original_report': 'report-d44-c157-before-caller-repair.json',
             'identity_report': 'report-e303-c012-before-integration.json',
             'boundary_proof': ('codex-disable-d44-closed-cause.json' if provider == 'codex'
                                else 'claude-rebind-d44.json')}
    documents, digests = {}, {}
    for kind, name in names.items():
        entry = manifest.get(kind) or {}
        path = supervisor.run_dir / name
        value = private_read(path)
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        require(entry.get('file') == name and entry.get('sha256') == digest,
                'historical_identity_document_digest_mismatch')
        documents[kind], digests[kind] = value, digest
    original = documents['original_report']
    source = supervisor.report['sources'][provider]
    fingerprint = source_fingerprint(source)
    require(source_fingerprint(original['sources'][provider]) == fingerprint
            and source_fingerprint(documents['identity_report']['sources'][provider]) == fingerprint,
            'historical_saved_source_identity_changed')
    proof = documents['boundary_proof']
    require(proof.get('state') == 'green' and proof.get('candidate_sha') == supervisor.configuration['candidate_sha'],
            'historical_boundary_proof_invalid')
    if provider == 'codex':
        require(proof.get('schema') == 'hiroute.cpa-managed-disable-closed-cause/v1'
                and proof.get('first_failure_stage') == 'protected_registration_before_ApplySubscriptionCheck'
                and proof.get('diagnostic_conclusion') == 'harness_reused_synthetic_one_shot_capability_rejected_on_insert',
                'historical_apply_boundary_unknown')
    else:
        require(proof.get('schema') == 'hiroute.cpa-managed-rebind/v1'
                and proof.get('selection_exactly_preserved') is True
                and proof.get('all_settings_fields_except_helper_preserved') is True
                and proof.get('oauth_credential_hashes_and_original_expiry_unchanged') is True
                and proof.get('direct_storage_edits') is False, 'historical_native_rebind_boundary_unknown')
    metadata = {'source_identity_sha256': fingerprint, 'reconstructed_from': {
        **{kind + '_sha256': digest for kind, digest in digests.items()},
        'original_caller_harness_sha': proof['caller_harness_sha'],
        'original_candidate_sha': proof['candidate_sha'],
        'historical_fields_were_not_backfilled': True,
        'source_identity_verified_now': True}}
    return manifest, original, metadata


def validate_current_binding(supervisor, provider, source):
    inventory = supervisor.product.control('ListCompute', {})['data']
    current = next((item for item in inventory['sources']
                    if item.get('source_id') == source['source_id']), None)
    require(current is not None and current.get('state') == 'ready' and len(current['models']) == 1
            and current['models'][0].get('model_ref') == source['model_ref']
            and current['models'][0].get('binding_id') == source['binding_id']
            and current['models'][0].get('upstream_model_id') == source['model'],
            'current_saved_source_mismatch')
    plan = supervisor.product.public_cli('routing show ' + source['plan_id'])[1]['data']
    candidates = plan.get('desired', {}).get('strategy', {}).get('candidates')
    require(plan.get('agent_plan_id') == source['plan_id']
            and plan['head'].get('reference', {}).get('plan_id') == source['plan_id']
            and plan['head']['model_alias'] == source['alias']
            and plan.get('desired', {}).get('mode') == 'fixed_model'
            and isinstance(candidates, list) and len(candidates) == 1
            and candidates[0].get('binding_id') == source['binding_id'],
            'current_saved_plan_mismatch')
    status = supervisor.product.public_cli('agents connect status ' + source['context'])[1]['data']
    selection = status.get('current_selection') or {}
    require(status.get('state') == 'configured' and status.get('context_id') == source['context'],
            'current_saved_context_mismatch')
    if provider == 'codex':
        require(selection.get('mode') == 'codex_default'
                and selection.get('default_selection') == {'kind': 'plan', 'plan_id': source['plan_id']}
                and source['plan_id'] in selection.get('allowed_plan_ids', []),
                'current_saved_connection_mismatch')
        return
    expected = {'mode': 'claude_launcher', 'surfaces': ['claude_cli'], 'fixed_models': [],
                'preset_mappings': {'opus': {'kind': 'plan', 'plan_id': source['plan_id']},
                    'sonnet': {'kind': 'preserve_native'}, 'haiku': {'kind': 'preserve_native'}}}
    require(selection == expected, 'current_saved_connection_mismatch')
    descriptor = supervisor.product.control('GetManagedAgentLaunchDescriptor',
                                           {'connection_id': source['connection']}, success=False)
    data = descriptor.get('data') or {}
    require(descriptor.get('status') == 'succeeded' and descriptor.get('error') is None
            and data.get('connection_id') == source['connection']
            and data.get('helper_executable') == str(supervisor.product.bin / 'hiroute')
            and data.get('helper_argv') == ['__internal-agent-grant-v1', source['connection']]
            and data.get('presets', {}).get('opus') == source['alias'],
            'current_managed_descriptor_mismatch')


def continuation(supervisor, providers, run_index, baseline, evidence, manifest_path=None, private_read=None):
    runs = supervisor.report.get('smoke_runs', [])
    require(type(run_index) is int and 0 <= run_index < len(runs), 'continuation_run_invalid')
    prior = runs[run_index]
    require(providers == ['codex'] and prior.get('selected_providers') == providers
            and prior.get('candidate_sha') == supervisor.configuration['candidate_sha']
            and prior.get('state') == 'red' and prior.get('native_only') is False
            and prior.get('failure', {}).get('stage') == 'codex-managed-disable',
            'continuation_scope_invalid')
    history = None
    if not prior.get('source_snapshots') or prior.get('scenario_end') is None:
        manifest, original, history = reconstructed_history(
            supervisor, 'codex', manifest_path, private_read, 'codex_remaining_lifecycle')
        require(manifest.get('run_index') == run_index
                and original['smoke_runs'][run_index] == prior
                and history['reconstructed_from']['original_caller_harness_sha'] == prior['caller_harness_sha'],
                'continuation_original_run_changed')
        end = manifest.get('scenario_end')
    else:
        end = prior['scenario_end']
    start = prior.get('scenario_start')
    rows = supervisor.report['scenarios']
    require(type(start) is int and type(end) is int and 0 <= start < end <= len(rows),
            'continuation_scenario_boundary_invalid')
    selected = rows[start:end]
    require(len(selected) == 4 and selected[-1] == prior['failure'], 'continuation_failure_boundary_invalid')
    if history is not None:
        require(original['scenarios'][start:end] == selected, 'continuation_original_scenarios_changed')
    expected = {'codex-managed-nonstream', 'codex-managed-stream', 'codex-real-native-client-dialogue'}
    completed = selected[:-1]
    require({row.get('scenario') for row in completed} == expected
            and all(row.get('state') == 'green' and isinstance(row.get('usage'), dict)
                    and row.get('provider') == 'codex'
                    and row.get('candidate_sha') == prior['candidate_sha']
                    and row.get('caller_harness_sha') == prior['caller_harness_sha'] for row in completed),
            'continuation_prior_paid_evidence_invalid')
    source = supervisor.report['sources']['codex']
    fingerprint = source_fingerprint(source)
    require((history or prior.get('source_snapshots', {})).get(
                'source_identity_sha256' if history else 'codex') == fingerprint,
            'continuation_saved_source_changed')
    validate_current_binding(supervisor, 'codex', source)
    carry = evidence.observation_delta(prior['baseline'], baseline, completed)
    require(carry['state'] == 'green', 'continuation_prior_usage_mismatch')
    return {'prior_run_index': run_index, 'prior_scenario_start': start, 'prior_scenario_end': end,
            'source_identity_sha256': fingerprint, 'prior_caller_harness_sha': prior['caller_harness_sha'],
            'prior_observation': carry, **({'reconstructed_from': history['reconstructed_from']} if history else {})}, completed, prior['baseline']


def reused_native_evidence(supervisor, provider, source, manifest_path=None, private_read=None):
    fingerprint = source_fingerprint(source)
    validate_current_binding(supervisor, provider, source)
    history, original = None, None
    if manifest_path is not None:
        _, original, history = reconstructed_history(
            supervisor, provider, manifest_path, private_read, 'claude_native_reuse')
    for index in range(len(supervisor.report['scenarios']) - 1, -1, -1):
        row = supervisor.report['scenarios'][index]
        client = row.get('client_evidence') or {}
        if (row.get('scenario') == provider + '-real-native-client-dialogue'
                and row.get('provider') == provider and row.get('model') == source['model']
                and row.get('candidate_sha') == supervisor.configuration['candidate_sha']
                and (row.get('source_identity_sha256') == fingerprint or (history is not None
                     and index < len(original['scenarios']) and original['scenarios'][index] == row
                     and row.get('caller_harness_sha') == history['reconstructed_from']['original_caller_harness_sha']))
                and row.get('state') == 'green'
                and row.get('answer_verified') is True and type(row.get('production_upstream_sends')) is int
                and row['production_upstream_sends'] > 0 and client.get('exit_code') == 0):
            return {'scenario': provider + '-native-evidence-reused', 'provider': provider, 'state': 'green',
                    'original_scenario_index': index, 'original_caller_harness_sha': row['caller_harness_sha'],
                    'original_candidate_sha': row['candidate_sha'], 'source_identity_sha256': fingerprint,
                    'new_upstream_inference_requests': 0, 'parsed_usage_counted_again': False,
                    **({'reconstructed_from': history['reconstructed_from']} if history else {})}
    raise RuntimeFailure('matching_native_business_evidence_missing')
