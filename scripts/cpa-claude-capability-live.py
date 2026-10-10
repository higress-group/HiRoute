#!/usr/bin/env python3
"""One isolated real Haiku image/thinking probe using a managed access lease.

The source managed runtime remains read-only. This is neither a native login nor
an independent managed OAuth acceptance. Its filtered lease has no refresh token.
Prepare/self-test never read credentials or start a provider process.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import http.client
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zlib


def load_live():
    spec = importlib.util.spec_from_file_location(
        'managed_capability_support', Path(__file__).with_name('cpa-managed-login-live.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


live = load_live()
MODEL = 'claude-haiku-4-5-20251001'
MAX_TOKENS = 2048
ORIGIN = 'managed_access_lease'


def require(condition, code):
    live.require(condition, code)


def caller_identity():
    repository = Path(__file__).resolve().parents[1]
    sha = live.runtime_support.caller_harness_sha(repository)
    relative = 'scripts/' + Path(__file__).name
    tracked = subprocess.check_output(['git', '-C', str(repository), 'ls-files', '--', relative], text=True)
    dirty = subprocess.check_output(['git', '-C', str(repository), 'status', '--porcelain',
                                    '--untracked-files=normal', '--', relative])
    require(tracked.strip() == relative and not dirty and not Path(__file__).is_symlink(),
            'capability_caller_uncommitted')
    return sha


def disjoint_roots(run_dir, source_dir):
    run_dir, source_dir = Path(run_dir).absolute(), Path(source_dir).absolute()
    require(not run_dir.is_symlink() and not source_dir.is_symlink(), 'runtime_symlink_denied')
    run, source = run_dir.resolve(), source_dir.resolve()
    require(run != source and run not in source.parents and source not in run.parents,
            'runtime_source_overlap')
    return run, source


def image_fixture():
    """Small RGB PNG with two unambiguous colors; no external imaging process."""
    palette = [('red', (255, 0, 0)), ('blue', (0, 0, 255)), ('green', (0, 255, 0)),
               ('yellow', (255, 255, 0))]
    left = int.from_bytes(os.urandom(1), 'big') % len(palette)
    right = (left + 1 + int.from_bytes(os.urandom(1), 'big') % (len(palette)-1)) % len(palette)
    colors = [palette[left], palette[right]]
    row = b'\0' + bytes(colors[0][1])*32 + bytes(colors[1][1])*32

    def chunk(kind, value):
        return struct.pack('!I', len(value)) + kind + value + struct.pack('!I', zlib.crc32(kind+value))

    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', 64, 32, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(row*32)) + chunk(b'IEND', b'')
    return png, {'width': 64, 'height': 32, 'left': colors[0][0], 'right': colors[1][0]}


def prepare(run_dir, source_dir):
    caller = caller_identity()
    run_dir, source_dir = disjoint_roots(run_dir, source_dir)
    require(len(str(run_dir/'p/runtime/hiroute/control.sock').encode()) < 100,
            'runtime_path_too_long')
    live.private_directory(run_dir, create=True)
    png, facts = image_fixture()
    path = run_dir/'image.png'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(png)
    manifest = {'schema': 'hiroute.claude-capability-preparation/v1', 'caller_harness_sha': caller,
                'origin': ORIGIN, 'image_sha256': live.sha256(path), 'image': facts,
                'source_run_dir': str(source_dir),
                'model': MODEL, 'max_tokens': MAX_TOKENS, 'maximum_model_calls': 1,
                'provider_calls': 0, 'oauth_calls': 0, 'credential_reads': 0}
    live.private_write(run_dir/'preparation.json', manifest)
    return {'state': 'green', 'scope': 'preparation_only', 'run_dir': str(run_dir),
            'provider_calls': 0, 'oauth_calls': 0, 'credential_reads': 0}


def prepared_image(run_dir, manifest, caller, source_dir):
    require(manifest.get('caller_harness_sha') == caller and manifest.get('origin') == ORIGIN
            and manifest.get('source_run_dir') == str(source_dir)
            and manifest.get('maximum_model_calls') == 1 and manifest.get('model') == MODEL
            and manifest.get('max_tokens') == MAX_TOKENS
            and (manifest['image']['width'], manifest['image']['height']) == (64, 32),
            'capability_preparation_changed')
    fd = os.open(run_dir/'image.png', os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
                and stat.S_IMODE(info.st_mode) == 0o600 and 0 < info.st_size < 1024,
                'private_small_image_required')
        png = os.read(fd, 1024)
    finally:
        os.close(fd)
    require(hashlib.sha256(png).hexdigest() == manifest.get('image_sha256')
            and png[:8] == b'\x89PNG\r\n\x1a\n' and len(png) >= 24
            and struct.unpack('!II', png[16:24]) == (64, 32), 'prepared_image_changed')
    return png


def managed_account_ref(document):
    account = document.get('account_uuid')
    require(document.get('type') == 'claude' and document.get('disabled') is not True
            and isinstance(account, str) and 0 < len(account) <= 4096
            and all(ord(char) >= 32 for char in account), 'managed_account_invalid')
    # Same domain and subject algorithm as CpaManagedEvidence::account_ref.
    digest = hashlib.sha256(('hiroute.cpa-account/v1\0claude\0'+account+'\0').encode()).hexdigest()
    return 'account/cpa/'+digest


def require_lineage(session, prior_session, source, prior_source, account_ref):
    require(session.get('provider') == prior_session.get('provider') == 'claude'
            and session.get('status') == prior_session.get('status') == 'authorized'
            and re.fullmatch(r'login-[A-Za-z0-9_-]+', session.get('login_ref', '')) is not None
            and session['login_ref'] == prior_session.get('login_ref')
            and session.get('account_ref') == prior_session.get('account_ref') == account_ref,
            'managed_authorization_lineage_mismatch')
    require(live.runtime_support.source_fingerprint(source)
            == live.runtime_support.source_fingerprint(prior_source), 'managed_saved_binding_lineage_mismatch')


def filtered_lease(document, expected_account_ref, now):
    """Only these three adapter fields can leave the managed source reader."""
    token, expiry = document.get('access_token'), document.get('expired')
    require(isinstance(token, str) and 0 < len(token) <= 65536 and not any(
        value in token for value in ('\n', '\r', '\0')), 'managed_access_invalid')
    require(isinstance(expiry, str), 'managed_expiry_missing')
    try:
        parsed = datetime.fromisoformat(expiry.replace('Z', '+00:00'))
        require(parsed.tzinfo is not None, 'managed_expiry_timezone_missing')
        expires = int(parsed.timestamp())
    except (ValueError, OverflowError):
        raise live.LiveFailure('managed_expiry_invalid') from None
    require(expires > now + 150, 'managed_lease_too_close_to_expiry')
    require(managed_account_ref(document) == expected_account_ref, 'managed_account_lineage_mismatch')
    # CPA does not persist granted scopes. This is an adapter-format declaration
    # derived from pinned requested user:inference + the immutable managed login
    # and account lineage, checked publicly with its original binding and writer.
    # CPA may normally rotate access. Neither its new token's usability nor a
    # provider-returned granted-scope list is established before this real probe.
    return {'claudeAiOauth': {'accessToken': token, 'expiresAt': expires*1000,
                             'scopes': ['user:inference']}}


def read_source(source_dir):
    configuration = live.private_read(source_dir/'configuration.json')
    report = live.private_read(source_dir/'report.json')
    session = report['sessions'].get('claude') or {}
    require(session.get('status') == 'authorized', 'managed_source_not_authorized')
    source = report['sources'].get('claude') or {}
    require(source.get('model') == MODEL, 'managed_source_model_changed')
    fingerprint = live.runtime_support.source_fingerprint(source)
    # The immutable completed lifecycle snapshot binds the prior model success
    # to its provider, candidate, login, account and original saved source.
    prior_path = source_dir/'claude-managed-lifecycle-d44-7bdeb782.json'
    prior = live.private_read(prior_path)
    run = prior.get('smoke_runs', [])[-1]
    require(run.get('state') == 'green' and run.get('selected_providers') == ['claude']
            and run.get('candidate_sha') == configuration['candidate_sha']
            and run.get('source_snapshots', {}).get('claude') == fingerprint
            and live.runtime_support.source_fingerprint(prior['sources']['claude']) == fingerprint,
            'managed_prior_inference_identity_mismatch')
    rows = prior['scenarios'][run['scenario_start']:run['scenario_end']]
    require(sum(isinstance(row.get('usage'), dict) and row.get('state') == 'green'
                and row.get('provider') == 'claude' for row in rows) == 3,
            'managed_prior_success_missing')
    prior_session = prior['sessions']['claude']
    directory = live.managed_auth_directory(Path(configuration['product_root'])/'storage',
                                            session['login_ref'])
    live.private_directory(directory)
    files = list(directory.glob('*.json'))
    require(len(files) == 1, 'managed_source_credential_count')
    path = files[0]
    current = live.credential_projection(path, 'claude')
    document = live.private_read(path)
    account = managed_account_ref(document)
    require_lineage(session, prior_session, source, prior['sources']['claude'], account)
    lease = filtered_lease(document, account, int(time.time()))
    require(current['access_sha256'] == hashlib.sha256(lease['claudeAiOauth']['accessToken'].encode()).hexdigest()
            and current['refresh_sha256'] == hashlib.sha256(document['refresh_token'].encode()).hexdigest()
            and current['access_expires_at_unix']
            == lease['claudeAiOauth']['expiresAt']//1000, 'managed_access_read_race')
    original = session['original_credential']
    require(not any(key.lower().replace('_', '').startswith('refreshtoken')
                    for key in lease['claudeAiOauth']), 'lease_refresh_authority')
    facts = {'origin': ORIGIN, 'source_candidate_sha': configuration['candidate_sha'],
             'source_identity_sha256': fingerprint, 'prior_success_sha256': live.sha256(prior_path),
             'source_projection': current, 'source_configuration_sha256':
                 live.sha256(source_dir/'configuration.json'), 'original_expiry':
                 original['access_expires_at_unix'], 'current_lease_expiry': current['access_expires_at_unix'],
             'prior_same_access_digest': current['access_sha256']
                 == prior_session['current_credential']['access_sha256'],
             'immutable_managed_authorization_lineage_verified': True,
             'scopes_are_derived_adapter_metadata': True,
             'scope_basis': 'pinned_requested_user_inference_and_immutable_managed_authorization_lineage',
             'provider_granted_scope_list_available': False, 'refresh_authority_transferred': False}
    return configuration, report, lease, facts


def source_reader(configuration, Product):
    class ReadOnlyProduct(Product):
        def __init__(self):
            self.repo, self.bin = Path(configuration['repository']), Path(configuration['product_bin'])
            self.root = Path(configuration['product_root'])
            self.env, self.project = live.isolated_environment(self.root), self.root/'project'
            self.outputs, self.secrets, self.process = [], set(), None
            self.release_version = json.loads((self.repo/'contracts/cli/local-control-hello.v2.schema.json'
                ).read_text())['properties']['client_version']['const']
    return ReadOnlyProduct()


def verify_source(configuration, report, Product):
    reader = source_reader(configuration, Product)
    source = report['sources']['claude']
    live.runtime_support.validate_current_binding(SimpleNamespace(product=reader), 'claude', source)
    session = report['sessions']['claude']
    status = reader.public_cli('compute connection login',
        {'action': 'status', 'login_ref': session['login_ref']})[1]['data']['sessions'][0]
    require(status.get('status') == 'authorized' and status.get('provider') == 'claude'
            and status.get('login_ref') == session['login_ref']
            and status.get('account_ref') == session['account_ref'], 'managed_source_public_status_changed')


def source_writer(configuration, session, proc_root=Path('/proc')):
    """Inspect process/config identities; never start or signal the source writer."""
    storage = Path(configuration['product_root'])/'storage'
    expected = live.private_directory(live.managed_auth_directory(storage, session['login_ref'])).resolve()
    # fork_managed_oauth derives state_root from this exact auth directory's
    # parent. The provider's borrowed-access runtime is a different owner/root.
    runtime = live.private_directory(expected.parent/'runtime')
    configs = []
    for path in runtime.rglob('config.yaml'):
        info = path.lstat()
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
                and not info.st_mode & 0o077, 'source_cpa_config_not_private')
        line = next((line for line in path.read_text().splitlines()
                     if line.startswith('auth-dir:')), '')
        auth = Path(line.split(':', 1)[1].strip().strip('\"\'')) if line else None
        if auth is not None and auth.resolve() == expected:
            configs.append(path.resolve())
    require(len(configs) == 1, 'source_cpa_writer_config_count')
    owners = []
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.geteuid():
                continue
            argv = (entry/'cmdline').read_bytes().split(b'\0')
            paths = [Path(os.fsdecode(argv[index+1])).resolve()
                     for index, arg in enumerate(argv[:-1]) if arg == b'--config']
            if configs[0] in paths:
                require((entry/'exe').resolve() == Path(configuration['cpa_binary']).resolve(),
                        'source_cpa_writer_binary_changed')
                owners.append(int(entry.name))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    require(len(owners) == 1, 'source_cpa_single_writer_required')
    return {'pid': owners[0], 'config_sha256': live.sha256(configs[0]), 'writer_count': 1}


def check_capabilities(model):
    caps = model.get('capabilities') or {}
    expected = {'vision': True, 'context_tokens': 200000, 'max_output_tokens': 64000,
                'native_reasoning': {'kind': 'toggle', 'parameter': 'enable_thinking'}}
    for name, value in expected.items():
        actual = caps.get(name) or {}
        require(actual.get('basis') == 'registered_catalog' and actual.get('value') == value,
                'catalog_' + name + '_mismatch')
    return {name: {'value': caps[name]['value'], 'basis': 'registered_catalog'} for name in expected}


def save_selected(product, native, apply_control, prepare_control_apply):
    checked = native.checked_subscription(product, 'claude', prepare_control_apply, 'capability-check')
    candidate = checked['checked_candidate']
    models = [model for model in candidate['models'] if model.get('upstream_model_id') == MODEL
              and model.get('selectable') is True]
    require(len(models) == 1, 'exact_haiku_not_selectable')
    model = models[0]
    require(model.get('fact_basis') == 'registered_catalog', 'checked_catalog_basis_mismatch')
    revisions = product.control('ListCompute', {})['data']['revisions']
    change = {'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': revisions, 'selected_model_refs': [model['model_ref']],
        'intent': 'save_ready', 'key_edits': [], 'validation': checked['validation']}
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, 'capability-save')
    result = product.control('GetComputeSaveResult', {'operation': applied['operation']})['data']
    require(result.get('disposition') == 'saved' and result.get('management_state') == 'ready'
            and len(result.get('bindings', [])) == 1, 'capability_save_not_ready')
    inventory = product.control('ListCompute', {})['data']
    saved = next((item for item in inventory['sources'] if item['source_id'] == result['source_id']), None)
    require(saved is not None and saved.get('state') == 'ready' and len(saved['models']) == 1,
            'capability_saved_inventory_mismatch')
    saved_model = saved['models'][0]
    require(saved_model.get('model_ref') == model['model_ref']
            and saved_model.get('binding_id') == result['bindings'][0]['binding_id']
            and saved_model.get('upstream_model_id') == MODEL
            and saved_model.get('catalog_configuration_id') is not None,
            'capability_saved_facts_changed')
    facts = check_capabilities(saved_model)
    facts['checked_fact_basis'] = model['fact_basis']
    return {'model': MODEL, 'binding_id': result['bindings'][0]['binding_id'],
            'model_ref': model['model_ref'], 'source_id': result['source_id']}, facts


def publish(product, source, judgment, configure):
    editor = {'schema': 'hiroute.plan-editor/v2', 'display_name': 'Isolated Haiku capability probe',
        'purpose': 'One bounded image and thinking request', 'mode': 'fixed_model',
        'candidates': [{'binding_id': source['binding_id'], 'reasoning': {'kind': 'toggle', 'enabled': True}}],
        'smart': {'economy': [], 'primary': [], 'judgment': judgment(),
            'reselect_on_user_message': False, 'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False, 'requirements': {'vision': True},
        'work': {'harness': 'claude_code', 'protocol': 'messages'},
        'limits': {'maximum_attempts': 1, 'request_timeout_ms': 90000, 'attempt_timeout_ms': 90000}}
    change = {'schema': 'hiroute.plan-content-change/v2',
        'target': {'intent': 'create', 'creation_key': 'haiku-capability'},
        'editor': editor, 'consumed_draft': None}
    preview = product.public_cli('routing preview', {'change': change})[1]['data']
    body = {'change': change, 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'haiku-capability-plan'}
    applied = product.public_cli('routing apply', body)[1]['data']
    require(applied.get('state') == 'succeeded', 'capability_plan_not_published')
    product.editor, product.plan_id = editor, preview['plan_head']['reference']['plan_id']
    source.update(plan_id=product.plan_id, alias=preview['plan_head']['model_alias'])
    configure(product, [product.plan_id], 'haiku-capability-connection', agent_id='agent_claude_default')
    source.update(connection=product.agent_connection, context=product.agent_context_id)


def project_reply(document, image):
    blocks = document.get('content')
    require(isinstance(blocks, list) and all(isinstance(part, dict) for part in blocks),
            'provider_content_missing')
    thinking = [part for part in blocks if part.get('type') in ('thinking', 'redacted_thinking')]
    require(bool(thinking), 'provider_thinking_block_missing')
    answer = ''.join(part.get('text', '') for part in blocks if part.get('type') == 'text')
    normalized = re.sub(r'\s+', '', answer).upper().strip('.;')
    expected = 'LEFT=' + image['left'].upper() + ';RIGHT=' + image['right'].upper()
    require(normalized == expected, 'image_answer_not_verified')
    usage = document.get('usage') or {}
    fields = ('input_tokens', 'output_tokens', 'cache_read_input_tokens', 'cache_creation_input_tokens')
    require(all(type(usage.get(key, 0)) is int and usage.get(key, 0) >= 0 for key in fields)
            and usage.get('input_tokens', 0) > 0 and usage.get('output_tokens', 0) > 0,
            'provider_usage_invalid')
    return {'answer_verified': True, 'thinking_block_count': len(thinking),
            'thinking_block_types': sorted({part['type'] for part in thinking}),
            'usage': {key: usage.get(key, 0) for key in fields},
            'thinking_text_retained': False, 'answer_text_retained': False}


def gateway(product, source, png, image):
    import base64
    body = {'model': source['alias'], 'stream': False, 'max_tokens': MAX_TOKENS,
        'messages': [{'role': 'user', 'content': [
            {'type': 'image', 'source': {'type': 'base64', 'media_type': 'image/png',
                                       'data': base64.b64encode(png).decode()}},
            {'type': 'text', 'text': 'State the left and right panel colors as LEFT=<color>; RIGHT=<color>, with no other text.'}]}]}
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=100)
    try:
        client.request('POST', '/v1/messages', body=live.encoded(body), headers={
            'Content-Type': 'application/json', 'anthropic-version': '2023-06-01',
            'X-HiRoute-Token': product.bearer(source['connection']),
            'session-id': 'isolated-haiku-capability'})
        response = client.getresponse()
        payload = response.read()
        product.outputs.append(payload)
        if response.status != 200:
            failure = {'http_status': response.status, 'error': json.loads(payload).get('error')}
            product.capability_gateway_failure = live.evidence.gateway_projection([
                dict(failure, scenario='request', provider='claude')])
        require(response.status == 200, 'capability_provider_http_' + str(response.status))
        return dict(project_reply(json.loads(payload), image), http_status=response.status)
    finally:
        client.close()


def project_wire(requests, responses):
    require(len(requests) == len(responses) == 1, 'one_upstream_request_required')
    reason = requests[0].get('request_reasoning') or {}
    require(reason.get('messages_thinking') == 'enabled'
            and type(reason.get('messages_budget_tokens')) is int
            and reason['messages_budget_tokens'] == 1024,
            'plan_thinking_render_mismatch')
    model = requests[0].get('native_model') or ''
    require(model == MODEL or model.endswith('/'+MODEL), 'wire_native_model_changed')
    require(responses[0].get('http_status') == 200, 'wire_provider_response_not_success')
    counters = responses[0].get('cpa_execution') or {}
    require(counters.get('status') == 'known' and counters.get('inference_attempts') == 1
            and all(type(counters.get(key)) is int and counters[key] == 0 for key in (
                'auth_recovery_attempts', 'auth_recovery_successes', 'unauthorized_responses')),
            'one_access_only_provider_attempt_required')
    return {'upstream_wire_requests': 1, 'upstream_wire_responses': 1, 'native_model': MODEL,
            'plan_thinking_type': 'enabled', 'plan_thinking_budget': 1024,
            'cpa_execution': {key: counters[key] for key in (
                'inference_attempts', 'auth_recovery_attempts', 'auth_recovery_successes', 'unauthorized_responses')}}


def wire_facts(product):
    requests, responses = [], []
    for path in product.diagnostics_root.rglob('*.jsonl'):
        for line in path.read_text().splitlines():
            try:
                record = json.loads(line)
            except ValueError:
                raise live.LiveFailure('diagnostic_record_invalid') from None
            value = (record.get('event') or {}).get('upstream_wire')
            if isinstance(value, dict):
                require(value.get('phase') in ('request', 'response'), 'wire_phase_unknown')
                (requests if value['phase'] == 'request' else responses).append(value)
    return project_wire(requests, responses)


def owned_processes(root, proc_root=Path('/proc')):
    owned = []
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.geteuid():
                continue
            argv = (entry/'cmdline').read_bytes().split(b'\0')
            for index, arg in enumerate(argv[:-1]):
                if arg in (b'--storage-root', b'--config'):
                    path = Path(os.fsdecode(argv[index+1])).resolve()
                    if path == root or root in path.parents:
                        owned.append(int(entry.name))
                        break
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return owned


def finish_owned(product, run_dir, report):
    process = product.process
    try:
        product.stop(diagnostic_failure=True)
        report['daemon_exit'] = None if process is None else process.poll()
        require(process is None or report['daemon_exit'] == 0, 'owned_daemon_exit_not_zero')
        require(not owned_processes(product.root), 'owned_process_still_active')
    except BaseException as error:
        report['state'] = 'red'
        report.setdefault('cleanup_failure', live.safe_code(error))
    try:
        for path in product.diagnostics_root.rglob('*.jsonl'):
            require(not path.is_symlink() and all(secret.encode() not in path.read_bytes()
                    for secret in product.secrets), 'diagnostics_secret_leak')
        report['diagnostics'] = product.preserve_diagnostics(run_dir/'diagnostics')
        for path in (run_dir/'diagnostics').rglob('*'):
            require(not path.is_symlink(), 'preserved_evidence_symlink')
            path.chmod(0o700 if path.is_dir() else 0o600)
        (run_dir/'diagnostics').chmod(0o700)
        report['public_secret_scan_passed'] = all(secret.encode() not in data
            for secret in product.secrets for data in product.outputs)
        require(report['public_secret_scan_passed'], 'public_secret_leak')
    except BaseException as error:
        report['state'] = 'red'
        report.setdefault('cleanup_failure', live.safe_code(error))
    finally:
        report['owned_runtime_removed'] = False
        try:
            remaining = owned_processes(product.root)
            report['owned_processes_remaining'] = len(remaining)
            if not remaining:
                product.listener_port_lease.close()
                require(product.root == run_dir/'p' and not product.root.is_symlink(),
                        'owned_cleanup_root_mismatch')
                shutil.rmtree(product.root)
                report['owned_runtime_removed'] = True
        except BaseException as error:
            report['state'] = 'red'
            report.setdefault('cleanup_failure', live.safe_code(error))


def execute(arguments):
    caller = caller_identity()
    run_dir, source_dir = disjoint_roots(arguments.run_dir, arguments.source_run_dir)
    live.private_directory(run_dir)
    preparation = live.private_read(run_dir/'preparation.json')
    png = prepared_image(run_dir, preparation, caller, source_dir)
    require(not (run_dir/'report.json').exists() and not (run_dir/'p').exists(),
            'capability_run_already_used')
    configuration = live.validate_candidate(arguments.repository, arguments.candidate_sha,
        arguments.product_bin, arguments.cpa_binary)
    Product, _, native, apply_control, configure, judgment, control_prepare = live.modules(configuration['repository'])
    require(Path(sys.modules[Product.__module__].__file__).resolve()
            == Path(configuration['repository'])/'crates/daemon/tests/support/publication_product.py',
            'product_support_revision_mismatch')
    report = {'schema': 'hiroute.claude-capability-live/v1', 'state': 'red', 'origin': ORIGIN,
        'candidate_sha': configuration['candidate_sha'], 'caller_harness_sha': caller,
        'cpa': configuration['cpa'], 'binaries': configuration['binaries'], 'model': MODEL,
        'bundle_sha256': live.sha256(Path(configuration['repository'])/
                                    'assets/release-facts/current/bundle/model-data.json'),
        'maximum_model_calls': 1, 'model_calls_dispatched': 0, 'oauth_calls': 0,
        'new_native_login': False, 'independent_managed_oauth_acceptance': False,
        'refresh_authority_transferred': False, 'source_runtime_is_read_only': True,
        'image': {key: preparation['image'][key] for key in ('width', 'height')},
        'image_sha256': preparation['image_sha256'], 'max_tokens': MAX_TOKENS,
        'automatic_original_expiry': 'user_deferred_not_executed'}
    product, source_config, source_report, facts, stage = None, None, None, None, 'source-read'
    try:
        source_config, source_report, lease, facts = read_source(source_dir)
        verify_source(source_config, source_report, Product)
        report['source_writer_before'] = source_writer(source_config, source_report['sessions']['claude'])
        report['source'] = {key: value for key, value in facts.items() if key != 'source_projection'}
        configuration.update(product_root=str(run_dir/'p'), codex_cli=source_config['codex_cli'],
                             claude_cli=source_config['claude_cli'])
        product = live.make_product(configuration, fresh=True)
        live.private_write(product.root/'home/.claude/.credentials.json', lease)
        product.secrets.add(lease['claudeAiOauth']['accessToken'])
        del lease
        stage = 'start'
        product.start()
        debug = product.diagnostics_snapshot()
        require(debug['state'] == 'complete' and debug['level_applied']['level'] == 'debug',
                'actual_debug_required')
        require(json.loads(product.outputs[0]).get('role') == 'all', 'both_roles_required')
        debug['process_role'] = 'all'
        report['debug'] = debug
        stage = 'public-check-save'
        source, report['catalog_capabilities'] = save_selected(product, native, apply_control, control_prepare)
        stage = 'public-plan'
        publish(product, source, judgment, configure)
        require(native.wire_request_count(product) == 0, 'preflight_prepared_upstream_request')
        report['access_only_cpa'] = live.access_only_cpa_audit(product, 'claude')
        require(facts['source_projection']['access_expires_at_unix'] > int(time.time()) + 100,
                'lease_expired_before_dispatch')
        stage = 'request'
        report['model_calls_dispatched'] = 1  # Seal before dispatch; no retry path exists.
        live.private_write(run_dir/'report.json', report)
        scenario = gateway(product, source, png, preparation['image'])
        report['scenario'] = dict(scenario, state='green', provider='claude', scenario='image-and-plan-thinking')
        stage = 'observation'
        report['observation'] = native.observed_usage(product, [scenario], live.encoded)
        report['observation']['scope'] = 'fresh_runtime_single_request_absolute_totals'
        report['wire'] = wire_facts(product)
        report['access_only_cpa_after'] = live.access_only_cpa_audit(product, 'claude')
        report['state'] = 'green'
    except BaseException as error:
        report['failure'] = {'stage': stage, 'code': live.safe_code(error)}
        if product is not None:
            report['failure_projection'] = live.evidence.failure_projection(product, stage)
    finally:
        if product is not None:
            finish_owned(product, run_dir, report)
        if source_config is not None and facts is not None:
            try:
                verify_source(source_config, source_report, Product)
                _, _, _, after = read_source(source_dir)
                report['source_writer_after'] = source_writer(source_config, source_report['sessions']['claude'])
                require(report['source_writer_after'] == report['source_writer_before'],
                        'source_single_writer_identity_changed')
                fields = ('access_sha256', 'refresh_sha256', 'access_expires_at_unix')
                report['source_credential_field_comparison'] = list(fields)
                report['source_whole_credential_file_stability_asserted'] = False
                report['source_credentials_preserved'] = all(
                    facts['source_projection'][key] == after['source_projection'][key] for key in fields)
                report['source_configuration_preserved'] = (
                    facts['source_configuration_sha256'] == after['source_configuration_sha256'])
                require(report['source_credentials_preserved'] and report['source_configuration_preserved'],
                        'managed_source_changed_during_probe')
            except BaseException as error:
                report['state'] = 'red'
                report['source_after_failure'] = live.safe_code(error)
        live.private_write(run_dir/'report.json', report)
    return {'state': report['state'], 'origin': ORIGIN, 'model_calls_dispatched':
            report['model_calls_dispatched'], 'report': str(run_dir/'report.json')}


def self_test():
    class Safety(unittest.TestCase):
        def credential(self, token='fixture', expiry='2026-10-11T00:00:00Z'):
            return {'type': 'claude', 'account_uuid': 'fixture-account-subject',
                    'access_token': token, 'expired': expiry}

        def lineage(self):
            document = self.credential()
            session = {'provider': 'claude', 'status': 'authorized', 'login_ref': 'login-fixture',
                       'account_ref': managed_account_ref(document)}
            source = {key: 'fixture-'+key for key in live.runtime_support.IDENTITY_FIELDS}
            source['model'] = MODEL
            return document, session, source

        def test_filter_has_no_refresh_or_unknown_fields(self):
            token = 'fixture-access'
            doc = dict(self.credential(token), **{
                   'refresh_token': 'fixture-refresh-sentinel', 'email': 'fixture-account',
                   'unknown': {'refreshToken': 'fixture-refresh-sentinel'}})
            value = filtered_lease(doc, managed_account_ref(doc), 0)
            self.assertEqual(set(value), {'claudeAiOauth'})
            self.assertEqual(set(value['claudeAiOauth']), {'accessToken', 'expiresAt', 'scopes'})
            self.assertNotIn('fixture-refresh-sentinel', json.dumps(value))
            self.assertEqual(value['claudeAiOauth']['expiresAt'], 1791676800000)

        def test_foreign_account_evidence_fails_closed(self):
            with self.assertRaises(live.LiveFailure):
                filtered_lease(self.credential(), 'wrong-account', 0)

        def test_no_expiry_extension(self):
            document = self.credential(expiry='1970-01-01T00:02:00Z')
            with self.assertRaises(live.LiveFailure):
                filtered_lease(document, managed_account_ref(document), 0)

        def test_naive_expiry_is_not_reinterpreted_in_local_timezone(self):
            document = self.credential(expiry='2026-10-11T00:00:00')
            with self.assertRaises(live.LiveFailure):
                filtered_lease(document, managed_account_ref(document), 0)

        def test_rotated_access_keeps_immutable_managed_lineage(self):
            original, session, source = self.lineage()
            rotated = dict(original, access_token='fixture-rotated-access', expired='2026-10-11T04:00:00Z',
                           refresh_token='fixture-refresh-must-not-leave-source')
            require_lineage(session, dict(session), source, dict(source), managed_account_ref(rotated))
            lease = filtered_lease(rotated, session['account_ref'], 0)
            self.assertEqual(lease['claudeAiOauth']['accessToken'], 'fixture-rotated-access')
            self.assertEqual(lease['claudeAiOauth']['expiresAt'], 1791691200000)
            self.assertEqual(original['expired'], '2026-10-11T00:00:00Z')
            self.assertNotIn('fixture-refresh-must-not-leave-source', json.dumps(lease))

        def test_account_login_or_saved_binding_drift_rejects(self):
            document, session, source = self.lineage()
            changes = [(dict(session, login_ref='login-changed'), source, managed_account_ref(document)),
                       (session, dict(source, binding_id='changed-binding'), managed_account_ref(document)),
                       (session, source, managed_account_ref(dict(document, account_uuid='changed-account')))]
            for current_session, current_source, account in changes:
                with self.assertRaises(live.LiveFailure):
                    require_lineage(current_session, session, current_source, source, account)

        def test_public_authorized_session_account_is_verified_again(self):
            _, session, source = self.lineage()
            report = {'sessions': {'claude': session}, 'sources': {'claude': source}}
            for public_status in (dict(session, account_ref='changed-account'),
                                  dict(session, status='failed'), dict(session, login_ref='login-changed')):
                reader = SimpleNamespace(public_cli=lambda *_args, **_kwargs: (
                    0, {'data': {'sessions': [public_status]}}))
                with patch.dict(verify_source.__globals__, {'source_reader': lambda *_args: reader}), \
                        patch.object(live.runtime_support, 'validate_current_binding', return_value=None), \
                        self.assertRaises(live.LiveFailure):
                    verify_source({}, report, object)

        def test_source_overlap_fails(self):
            for run, source in (('/tmp/same', '/tmp/same'), ('/tmp/a', '/tmp/a/b'), ('/tmp/a/b', '/tmp/a')):
                with self.assertRaises(live.LiveFailure):
                    disjoint_roots(run, source)

        def test_prepare_rejects_overlap_before_any_write(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                source = Path(tmp)/'source'
                source.mkdir(mode=0o700)
                with patch.object(live.runtime_support, 'caller_harness_sha', return_value='a'*40), \
                        patch.object(subprocess, 'check_output', side_effect=[
                            'scripts/cpa-claude-capability-live.py\n', b'']):
                    with self.assertRaises(live.LiveFailure):
                        prepare(source/'probe', source)
                self.assertEqual(list(source.iterdir()), [])

        def test_runtime_symlink_is_rejected(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                source = Path(tmp)/'source'
                source.mkdir()
                alias = Path(tmp)/'alias'
                alias.symlink_to(source)
                with self.assertRaises(live.LiveFailure):
                    disjoint_roots(alias, source)

        def test_png_is_small_valid_two_panel_rgb(self):
            png, facts = image_fixture()
            self.assertLess(len(png), 1024)
            self.assertEqual(png[:8], b'\x89PNG\r\n\x1a\n')
            self.assertEqual(struct.unpack('!II', png[16:24]), (64, 32))
            self.assertNotEqual(facts['left'], facts['right'])
            offset = 8
            while offset < len(png):
                size = struct.unpack('!I', png[offset:offset+4])[0]
                block = png[offset+4:offset+8+size]
                self.assertEqual(zlib.crc32(block), struct.unpack('!I', png[offset+8+size:offset+12+size])[0])
                offset += size+12
            self.assertEqual(offset, len(png))

        def test_reply_exposes_no_answer_or_thinking(self):
            response = {'content': [{'type': 'thinking', 'thinking': 'fixture-hidden-thought'},
                {'type': 'text', 'text': 'LEFT=red; RIGHT=blue'}],
                'usage': {'input_tokens': 3, 'output_tokens': 4, 'unknown': 'fixture-hidden-thought'}}
            result = project_reply(response, {'left': 'red', 'right': 'blue'})
            self.assertTrue(result['answer_verified'])
            self.assertNotIn('fixture-hidden-thought', json.dumps(result))
            self.assertNotIn('LEFT=', json.dumps(result))

        def test_reply_rejects_boolean_usage(self):
            with self.assertRaises(live.LiveFailure):
                project_reply({'content': [{'type': 'thinking'},
                    {'type': 'text', 'text': 'LEFT=red;RIGHT=blue'}],
                    'usage': {'input_tokens': True, 'output_tokens': 1}}, {'left': 'red', 'right': 'blue'})

        def test_missing_thinking_or_wrong_image_answer_fails(self):
            for content in ([{'type': 'text', 'text': 'LEFT=red;RIGHT=blue'}],
                            [{'type': 'thinking'}, {'type': 'text', 'text': 'wrong'}]):
                with self.assertRaises(live.LiveFailure):
                    project_reply({'content': content, 'usage': {'input_tokens': 1, 'output_tokens': 1}},
                                  {'left': 'red', 'right': 'blue'})

        def test_old_fallback_caps_are_rejected(self):
            with self.assertRaises(live.LiveFailure):
                check_capabilities({'capabilities': {'vision': {'value': False,
                    'basis': 'runtime_fallback'}}})

        def test_registered_caps_use_public_saved_basis(self):
            values = {'vision': True, 'context_tokens': 200000, 'max_output_tokens': 64000,
                      'native_reasoning': {'kind': 'toggle', 'parameter': 'enable_thinking'}}
            model = {'capabilities': {key: {'value': value, 'basis': 'registered_catalog'}
                                     for key, value in values.items()}}
            self.assertEqual(set(check_capabilities(model)), set(values))
            model['capabilities']['native_reasoning']['basis'] = 'runtime_fallback'
            with self.assertRaises(live.LiveFailure):
                check_capabilities(model)

        def wire(self):
            return [{'native_model': 'fixture-prefix/'+MODEL,
                     'request_reasoning': {'messages_thinking': 'enabled', 'messages_budget_tokens': 1024}}], [
                {'http_status': 200, 'cpa_execution': {'status': 'known', 'inference_attempts': 1,
                    'auth_recovery_attempts': 0, 'auth_recovery_successes': 0, 'unauthorized_responses': 0}}]

        def test_wire_uses_actual_producer_field_names(self):
            self.assertEqual(project_wire(*self.wire())['plan_thinking_budget'], 1024)

        def test_guessed_wire_field_names_fail_closed(self):
            requests, responses = self.wire()
            requests[0]['request_reasoning'] = {'messages_thinking_type': 'enabled',
                                               'messages_thinking_budget_tokens': 1024}
            with self.assertRaises(live.LiveFailure):
                project_wire(requests, responses)

        def test_wire_rejects_extra_attempt_or_refresh(self):
            requests, responses = self.wire()
            with self.assertRaises(live.LiveFailure):
                project_wire(requests*2, responses*2)
            for name in ('inference_attempts', 'auth_recovery_attempts', 'unauthorized_responses'):
                requests, responses = self.wire()
                responses[0]['cpa_execution'][name] += 1
                with self.assertRaises(live.LiveFailure):
                    project_wire(requests, responses)

        def test_wire_requires_exact_native_model_and_known_counters(self):
            requests, responses = self.wire()
            requests[0]['native_model'] = 'another-model'
            with self.assertRaises(live.LiveFailure):
                project_wire(requests, responses)
            requests, responses = self.wire()
            responses[0]['cpa_execution'] = {'status': 'unknown'}
            with self.assertRaises(live.LiveFailure):
                project_wire(requests, responses)

        def test_request_is_one_image_without_client_thinking_or_forced_tool(self):
            calls = []
            class Connection:
                def __init__(self, *_args, **_kwargs):
                    pass
                def request(self, *args, **kwargs):
                    calls.append((args, kwargs))
                def getresponse(self):
                    return SimpleNamespace(status=200, read=lambda: live.encoded({
                        'content': [{'type': 'thinking', 'thinking': 'fixture-private'},
                            {'type': 'text', 'text': 'LEFT=red;RIGHT=blue'}],
                        'usage': {'input_tokens': 1, 'output_tokens': 2}}))
                def close(self):
                    pass
            product = SimpleNamespace(port=1, outputs=[], bearer=lambda _key: 'fixture-bearer')
            png, _ = image_fixture()
            with patch.object(http.client, 'HTTPConnection', Connection):
                gateway(product, {'alias': 'fixture-alias', 'connection': 'fixture-connection'},
                        png, {'left': 'red', 'right': 'blue'})
            self.assertEqual(len(calls), 1)
            body = json.loads(calls[0][1]['body'])
            self.assertEqual((body['model'], body['max_tokens']), ('fixture-alias', 2048))
            self.assertNotIn('thinking', body)
            self.assertNotIn('tool_choice', body)
            self.assertEqual(body['messages'][0]['content'][0]['type'], 'image')

        def test_failed_request_has_no_retry(self):
            calls = []
            class Connection:
                def __init__(self, *_args, **_kwargs):
                    pass
                def request(self, *args, **kwargs):
                    calls.append(1)
                def getresponse(self):
                    return SimpleNamespace(status=400, read=lambda: b'{"error":{"type":"invalid_request_error"}}')
                def close(self):
                    pass
            product = SimpleNamespace(port=1, outputs=[], bearer=lambda _key: 'fixture-bearer')
            with patch.object(http.client, 'HTTPConnection', Connection), self.assertRaises(live.LiveFailure):
                gateway(product, {'alias': 'fixture-alias', 'connection': 'fixture-connection'},
                        b'fixture', {'left': 'red', 'right': 'blue'})
            self.assertEqual(len(calls), 1)

        def test_dirty_entry_and_support_are_rejected(self):
            with patch.object(live.runtime_support, 'caller_harness_sha', return_value='a'*40), \
                    patch.object(subprocess, 'check_output', side_effect=[
                        'scripts/cpa-claude-capability-live.py\n', b' M scripts/cpa-claude-capability-live.py']):
                with self.assertRaises(live.LiveFailure):
                    caller_identity()
            with patch.object(live.runtime_support, 'caller_harness_sha',
                              side_effect=live.runtime_support.RuntimeFailure('caller_harness_uncommitted')):
                with self.assertRaises(live.runtime_support.RuntimeFailure):
                    caller_identity()

        def test_image_bound_to_preparation_and_small_budget(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                root, source = Path(tmp)/'run', Path(tmp)/'source'
                with patch.object(live.runtime_support, 'caller_harness_sha', return_value='a'*40), \
                        patch.object(subprocess, 'check_output', side_effect=[
                            'scripts/cpa-claude-capability-live.py\n', b'']):
                    prepare(root, source)
                manifest = live.private_read(root/'preparation.json')
                self.assertLess(len(prepared_image(root, manifest, 'a'*40, source)), 1024)
                manifest['max_tokens'] = 9000
                with self.assertRaises(live.LiveFailure):
                    prepared_image(root, manifest, 'a'*40, source)

        def test_prepare_command_routes_both_isolated_paths(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                root, source = Path(tmp)/'run', Path(tmp)/'source'
                output = io.StringIO()
                with patch.object(sys, 'argv', ['fixture', 'prepare', '--run-dir', str(root),
                                              '--source-run-dir', str(source)]), \
                        patch.dict(prepare.__globals__, {'caller_identity': lambda: 'a'*40}), \
                        patch.object(sys, 'stdout', output):
                    self.assertEqual(main(), 0)
                self.assertEqual(json.loads(output.getvalue())['credential_reads'], 0)
                self.assertEqual(live.private_read(root/'preparation.json')['source_run_dir'], str(source))

        def test_source_requires_exactly_one_existing_writer(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                root, proc = Path(tmp)/'p', Path(tmp)/'proc'
                auth = live.managed_auth_directory(root/'storage', 'login-fixture')
                auth.mkdir(mode=0o700, parents=True)
                config = auth.parent/'runtime/fixture-instance/config.yaml'
                config.parent.mkdir(mode=0o700, parents=True)
                config.parent.parent.chmod(0o700)
                config.write_text('auth-dir: '+str(auth)+'\n')
                config.chmod(0o600)
                binary = Path(tmp)/'cpa'
                binary.write_text('fixture executable')
                proc.mkdir()
                settings = {'product_root': str(root), 'cpa_binary': str(binary)}
                for number in (1, 2):
                    entry = proc/str(number)
                    entry.mkdir()
                    (entry/'cmdline').write_bytes(os.fsencode(binary)+b'\0--config\0'+os.fsencode(config)+b'\0')
                    (entry/'exe').symlink_to(binary)
                    if number == 1:
                        self.assertEqual(source_writer(settings, {'login_ref': 'login-fixture'}, proc)['writer_count'], 1)
                    else:
                        with self.assertRaises(live.LiveFailure):
                            source_writer(settings, {'login_ref': 'login-fixture'}, proc)
                config.write_text('auth-dir: '+str(auth.parent/'foreign-auth')+'\n')
                with self.assertRaises(live.LiveFailure):
                    source_writer(settings, {'login_ref': 'login-fixture'}, proc)

        def test_cleanup_never_deletes_active_owned_runtime(self):
            with tempfile.TemporaryDirectory(prefix='haiku-capability-fixture-') as tmp:
                root = Path(tmp)
                owned = root/'p'
                owned.mkdir()
                product = SimpleNamespace(root=owned, process=None, stop=lambda **_kwargs: None)
                report = {'state': 'green'}
                with patch.dict(finish_owned.__globals__, {'owned_processes': lambda _root: [1]}):
                    finish_owned(product, root, report)
                self.assertEqual(report['state'], 'red')
                self.assertFalse(report['owned_runtime_removed'])
                self.assertTrue(owned.is_dir())

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Safety)
    result = unittest.TextTestRunner(stream=io.StringIO(), verbosity=2).run(suite)
    try:
        caller, kind = caller_identity(), 'exact_clean_committed'
    except (live.LiveFailure, live.runtime_support.RuntimeFailure):
        caller, kind = None, 'iterative_uncommitted_diagnostic'
    return {'state': 'green' if result.wasSuccessful() and result.testsRun == 27 else 'red',
            'scope': 'safety_fixtures_only', 'tests': result.testsRun, 'caller_harness_sha': caller,
            'failed_tests': [case._testMethodName for case, _ in result.failures+result.errors],
            'candidate_kind': kind, 'provider_calls': 0, 'oauth_calls': 0, 'credential_reads': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('self-test')
    prepared = commands.add_parser('prepare')
    prepared.add_argument('--run-dir', required=True, type=Path)
    prepared.add_argument('--source-run-dir', required=True, type=Path)
    run = commands.add_parser('run')
    for name in ('run-dir', 'source-run-dir', 'repository', 'product-bin', 'cpa-binary'):
        run.add_argument('--'+name, required=True, type=Path)
    run.add_argument('--candidate-sha', required=True)
    arguments = parser.parse_args()
    try:
        result = self_test() if arguments.command == 'self-test' else (
            prepare(arguments.run_dir, arguments.source_run_dir) if arguments.command == 'prepare' else execute(arguments))
    except BaseException as error:
        result = {'state': 'red', 'code': live.safe_code(error)}
    print(json.dumps(result, sort_keys=True), flush=True)
    return 1 if result['state'] == 'red' else 0


if __name__ == '__main__':
    raise SystemExit(main())
