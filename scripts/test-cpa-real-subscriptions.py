#!/usr/bin/env python3
"""Opt-in real Claude and Codex subscription acceptance, with no refresh authority.

Native credentials are filtered on their owning host before transfer. Only an
isolated HOME/CODEX_HOME is passed to the real CLI, daemon and native probes.
Reports contain verdicts, digests, usage and safe error codes, never raw output.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time


REMOTE_READ = r'''
import hashlib, json, os, stat, sys
from pathlib import Path
kind, mode = sys.argv[1:]
path = Path.home() / (".codex/auth.json" if kind == "codex" else ".claude/.credentials.json")
fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
try:
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077 or info.st_size > 1024*1024:
        raise RuntimeError("native source is not a bounded owner-only regular file")
    data = os.read(fd, 1024*1024 + 1)
finally:
    os.close(fd)
result = {"sha256": hashlib.sha256(data).hexdigest(), "mode": oct(stat.S_IMODE(info.st_mode))}
if mode == "borrow":
    source = json.loads(data)
    if kind == "codex":
        if source.get("auth_mode") != "chatgpt" or not isinstance(source.get("tokens"), dict):
            raise RuntimeError("native source is not a ChatGPT subscription")
        credential = {"auth_mode": "chatgpt", "tokens": {key: source["tokens"][key] for key in ("access_token", "id_token", "account_id") if isinstance(source["tokens"].get(key), str)}}
        if not credential["tokens"].get("access_token"):
            raise RuntimeError("native source has no access token")
    else:
        oauth = source.get("claudeAiOauth")
        if not isinstance(oauth, dict) or not isinstance(oauth.get("accessToken"), str) or not oauth["accessToken"]:
            raise RuntimeError("native source has no Claude OAuth access token")
        credential = {"claudeAiOauth": {key: oauth[key] for key in ("accessToken", "expiresAt", "scopes") if key in oauth}}
    # This is the first and only credential serialization leaving the native host.
    # No refresh value or unknown source field is serialized.
    result["credential"] = credential
print(json.dumps(result))
'''


class AcceptanceFailure(Exception):
    pass


def require(condition, message):
    if not condition:
        raise AcceptanceFailure(message)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as handle:
        while data := handle.read(1024 * 1024):
            value.update(data)
    return value.hexdigest()


def remote_source(host, kind, mode):
    command = 'python3 -c ' + shlex.quote(REMOTE_READ) + ' ' + shlex.quote(kind) + ' ' + shlex.quote(mode)
    result = subprocess.run(
        ['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', host, command],
        capture_output=True, timeout=30)
    require(result.returncode == 0, f'{kind}:native_source_read_failed:{result.returncode}')
    return json.loads(result.stdout)


def private_json(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as handle:
        handle.write(json.dumps(data, separators=(',', ':')).encode())


def string_values(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from string_values(item)
    elif isinstance(value, list):
        for item in value:
            yield from string_values(item)


def forbidden_refresh(value):
    if isinstance(value, dict):
        return any(key.lower().replace('_', '').startswith('refreshtoken') or forbidden_refresh(item)
                   for key, item in value.items())
    if isinstance(value, list):
        return any(forbidden_refresh(item) for item in value)
    return False


def configure_private_clients(product, arguments, sources):
    home = Path(product.env['HOME'])
    codex_home = home / '.codex'
    codex_home.mkdir(mode=0o700)
    product.codex_settings = codex_home / 'config.toml'
    product.codex_settings.write_text('model = "gpt-5.4"\nmodel_reasoning_effort = "low"\n')
    product.codex_settings.chmod(0o600)
    shutil.copyfile(product.repo / 'crates/integrations/src/agents/codex_bundled_catalog.json', codex_home / 'models_cache.json')
    (codex_home / 'models_cache.json').chmod(0o600)
    product.settings.write_text('{}')
    for kind, destination in (
            ('codex', codex_home / 'auth.json'),
            ('claude', home / '.claude/.credentials.json')):
        credential = sources[kind].pop('credential')
        require(not forbidden_refresh(credential), f'{kind}:refresh_authority_in_borrow')
        if kind == 'codex':
            # The current native-file parser requires this non-secret metadata.
            # Generate it here: the original host still transfers only access,
            # identity and account fields, never its refresh state or authority.
            credential['last_refresh'] = datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z')
            sources[kind]['synthetic_last_refresh_metadata'] = True
        private_json(destination, credential)
        product.secrets.update(value for value in string_values(credential) if len(value) > 8)
    for name, executable in (('codex', arguments.codex_cli), ('claude', arguments.claude_cli)):
        destination = product.root / 'bin' / name
        if destination.exists():
            destination.unlink()
        destination.symlink_to(executable.resolve())
    for directory in product.root.rglob('*'):
        if directory.is_dir() and not directory.is_symlink():
            directory.chmod(0o700)
    product.env['CODEX_HOME'] = str(codex_home)
    product.env.pop('CLAUDE_SECURESTORAGE_CONFIG_DIR', None)
    product.enable_debug_diagnostics()
    product.cpa_args = ['--cpa-binary', str(arguments.cpa_binary), '--cpa-sha256', arguments.cpa_sha256]
    product.startup_timeout = 90


def safe_envelope(envelope):
    error = envelope.get('error') or {}
    return {'status': envelope.get('status'), 'code': error.get('code')}


def checked_subscription(product, kind, prepare_control_apply, key):
    inventory = product.control('ListComputeSubscriptions', {})['data']
    require(inventory['discovery_state'] == 'complete', f'{kind}:subscription_discovery_incomplete')
    pending = next((item for item in inventory['candidates']
                    if item['candidate']['candidate_ref'].startswith('candidate/cpa/' + kind + '/')), None)
    require(pending is not None, f'{kind}:subscription_candidate_absent')
    preview = product.control('PreviewSubscriptionCheck', {'candidate': pending['candidate']})['data']
    payload, grant = prepare_control_apply(product, 'ApplySubscriptionCheck', preview, key)
    applied = product.control('ApplySubscriptionCheck', payload, grant)
    deadline = time.monotonic() + 90
    while True:
        checked = product.control('GetSubscriptionCheckResult', {'operation': applied['operation']})['data']
        if checked['status'] != 'checking':
            break
        require(time.monotonic() < deadline, f'{kind}:subscription_check_timeout')
        time.sleep(.1)
    require(checked['status'] == 'verified', f'{kind}:subscription_not_verified:{checked["status"]}:{checked.get("reason")}')
    return checked


def subscription_save(product, kind, apply_control, prepare_control_apply):
    checked = checked_subscription(product, kind, prepare_control_apply, kind + '-real-check')
    candidate = checked['checked_candidate']
    models = [item for item in candidate['models'] if item['selectable']]
    if kind == 'claude':
        models = [item for item in models if 'haiku' in item['upstream_model_id'].lower()]
        preferred = ('claude-haiku-4-5-20251001', 'claude-haiku-4-5')
    else:
        preferred = ('gpt-5.6-luna', 'gpt-5.4-mini', 'gpt-5.1-codex-mini')
        models = [item for item in models if item['upstream_model_id'] in preferred]
    require(models, f'{kind}:low_cost_selectable_model_absent')
    model = min(models, key=lambda item: (
        preferred.index(item['upstream_model_id']) if item['upstream_model_id'] in preferred else len(preferred),
        item['upstream_model_id']))
    snapshot = product.control('ListCompute', {})['data']
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [model['model_ref']], 'intent': 'save_ready',
        'key_edits': [], 'validation': checked['validation'],
    }
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, kind + '-real-save')
    saved = product.control('GetComputeSaveResult', {'operation': applied['operation']})['data']
    require(saved['disposition'] == 'saved' and saved['management_state'] == 'ready', f'{kind}:source_not_saved_ready')
    require(len(saved['bindings']) == 1, f'{kind}:unexpected_binding_count')
    return {'source_id': saved['source_id'], 'binding_id': saved['bindings'][0]['binding_id'],
            'model_ref': model['model_ref'], 'model': model['upstream_model_id'],
            'validation': checked['validation']}


def publish_plan(product, kind, source, judgment_fixture, configure_model_settings):
    selection = {'binding_id': source['binding_id']}
    if kind == 'codex':
        selection['reasoning'] = {'kind': 'profile', 'profile': 'low'}
    editor = {
        'schema': 'hiroute.plan-editor/v2', 'display_name': kind + ' real subscription acceptance',
        'purpose': 'Bounded real upstream acceptance', 'mode': 'fixed_model', 'candidates': [selection],
        'smart': {'economy': [], 'primary': [], 'judgment': judgment_fixture(),
                  'reselect_on_user_message': False, 'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False, 'requirements': {},
        'limits': {'maximum_attempts': 1, 'request_timeout_ms': 90000, 'attempt_timeout_ms': 90000},
    }
    if kind == 'claude':
        editor['work'] = {'harness': 'claude_code', 'protocol': 'messages'}
    change = {'schema': 'hiroute.plan-content-change/v2',
              'target': {'intent': 'create', 'creation_key': kind + '-real-subscription'},
              'editor': editor, 'consumed_draft': None}
    product.editor = editor
    preview = product.public_cli('routing preview', {'change': change})[1]['data']
    payload = {'change': change, 'accept_digest': preview['change_digest'],
               'expected_revisions': preview['expected_revisions'], 'idempotency_key': kind + '-real-plan'}
    applied = product.public_cli('routing apply', payload)[1]['data']
    require(applied['state'] == 'succeeded', f'{kind}:plan_not_published')
    product.plan_id = source['plan_id'] = preview['plan_head']['reference']['plan_id']
    source['alias'] = preview['plan_head']['model_alias']
    for attribute in ('agent_context_id', 'agent_settings_spec'):
        if hasattr(product, attribute):
            delattr(product, attribute)
    agent_id = 'agent_' + kind + '_default'
    configure_model_settings(product, [source['plan_id']], kind + '-real-agent', agent_id=agent_id)
    source['connection'] = product.agent_connection
    source['context'] = product.agent_context_id
    require(source['alias'] in {item['id'] for item in product.catalog()[0]['data']}, f'{kind}:alias_not_exposed')


def usage_numbers(usage):
    require(isinstance(usage, dict), 'response_usage_absent')
    selected = {}
    for key in ('input_tokens', 'output_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens'):
        value = usage.get(key)
        if isinstance(value, int):
            selected[key] = value
    details = usage.get('input_tokens_details') or {}
    if isinstance(details.get('cached_tokens'), int):
        selected['cache_read_input_tokens'] = details['cached_tokens']
    require(selected.get('input_tokens', 0) > 0 and selected.get('output_tokens', 0) > 0, 'response_usage_invalid')
    return selected


def wire_request_count(product):
    count = 0
    for path in product.diagnostics_root.rglob('*.jsonl'):
        for line in path.read_text().splitlines():
            try:
                event = json.loads(line).get('event', {}).get('upstream_wire', {})
            except json.JSONDecodeError:
                continue
            count += event.get('phase') == 'request'
    return count


def safe_gateway_error(payload, secrets):
    try:
        decoded = json.loads(payload)
        error = decoded.get('error') or {}
        if not isinstance(error, dict):
            error = {}
        safe = {}
        for field in ('code', 'type', 'phase'):
            value = error.get(field) or decoded.get(field)
            if isinstance(value, str) and re.fullmatch(r'[A-Za-z0-9_.:-]{1,96}', value):
                safe[field] = value
        message = error.get('message')
        if (isinstance(message, str) and len(message) <= 512
                and not any(secret in message for secret in secrets)
                and not re.search(r'@|(?:sk-|eyJ)[A-Za-z0-9_-]{12,}', message)):
            safe['message'] = message
        return safe
    except (ValueError, AttributeError):
        return {'type': 'non_json'}


def gateway(product, kind, source, stream, label, allowed=True):
    body = {'model': source['alias'], 'stream': stream}
    tool_round = None
    if kind == 'claude':
        endpoint = '/v1/messages'
        tool = {'name': 'return_ok', 'description': 'Return the supplied OK value.',
                'input_schema': {'type': 'object', 'properties': {'value': {'type': 'string', 'enum': ['OK']}},
                                 'required': ['value'], 'additionalProperties': False}}
        if allowed and label == 'claude-initial-inference':
            tool_round = 'tool_use'
            body.update(messages=[{'role': 'user', 'content': 'Use return_ok with value OK.'}],
                        tools=[tool], tool_choice={'type': 'tool', 'name': 'return_ok'}, max_tokens=128)
        elif allowed and 'tool_history' in source:
            tool_round = 'tool_result'
            body.update(messages=[*source['tool_history'], {'role': 'user', 'content': [
                {'type': 'tool_result', 'tool_use_id': source['tool_use_id'], 'content': 'OK'},
                {'type': 'text', 'text': 'Reply with the tool result only. Do not use another tool.'}]}],
                tools=[tool], max_tokens=16)
        else:
            body.update(messages=[{'role': 'user', 'content': 'Reply with OK only.'}], max_tokens=8)
    else:
        endpoint = '/v1/responses'
        body.update(input='Reply with OK only.', reasoning={'effort': 'low'}, max_output_tokens=64)
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=100)
    started = time.monotonic()
    if not allowed:
        time.sleep(.3)
    wires_before = wire_request_count(product)
    product.gateway_request_count = getattr(product, 'gateway_request_count', 0) + 1
    if allowed:
        product.inference_gateway_attempts = getattr(product, 'inference_gateway_attempts', 0) + 1
    try:
        client.request('POST', endpoint, body=json.dumps(body, separators=(',', ':')).encode(), headers={
            'Content-Type': 'application/json', 'X-HiRoute-Token': product.bearer(source['connection']),
            'anthropic-version': '2023-06-01', 'session-id': 'real-subscription-' + label,
        })
        response = client.getresponse()
        payload = response.read()
        product.outputs.append(payload)
        status = response.status
        content_type = response.getheader('Content-Type', '')
    finally:
        client.close()
    if not allowed:
        require(status >= 400, f'{kind}:{label}:disabled_source_accepted')
        code = safe_gateway_error(payload, product.secrets).get('code')
        time.sleep(.3)
        new_wires = wire_request_count(product) - wires_before
        require(new_wires == 0, f'{kind}:{label}:disabled_request_prepared_upstream')
        return {'scenario': label, 'provider': kind, 'state': 'green', 'http_status': status,
                'error_code': code, 'upstream_expected': False, 'new_upstream_wire_requests': new_wires}
    if status != 200:
        error = safe_gateway_error(payload, product.secrets)
        time.sleep(.3)
        failures = getattr(product, 'gateway_failures', [])
        failures.append({'scenario': label, 'provider': kind, 'http_status': status,
                         'error': error, 'new_upstream_wire_requests': wire_request_count(product) - wires_before})
        product.gateway_failures = failures
        raise AcceptanceFailure(f'{kind}:{label}:gateway_http_{status}:{error.get("code") or error.get("type")}')
    text_parts, usage = [], {}
    if stream:
        require('text/event-stream' in content_type, f'{kind}:{label}:not_sse')
        events = []
        for line in payload.decode().splitlines():
            if line.startswith('data:') and line[5:].strip() != '[DONE]':
                events.append(json.loads(line[5:].strip()))
        if kind == 'claude':
            for event in events:
                if event.get('type') == 'message_start':
                    usage.update(event['message'].get('usage') or {})
                if event.get('type') == 'message_delta':
                    usage.update(event.get('usage') or {})
                if event.get('type') == 'content_block_delta' and event.get('delta', {}).get('type') == 'text_delta':
                    text_parts.append(event['delta']['text'])
            require(any(item.get('type') == 'message_stop' for item in events), f'{kind}:{label}:missing_terminal_event')
        else:
            terminal = next((item for item in reversed(events) if item.get('type') == 'response.completed'), None)
            require(terminal is not None, f'{kind}:{label}:missing_terminal_event')
            usage = terminal['response'].get('usage')
            text_parts = [item.get('delta', '') for item in events if item.get('type') == 'response.output_text.delta']
    else:
        result = json.loads(payload)
        usage = result.get('usage')
        if kind == 'claude':
            text_parts = [part['text'] for part in result.get('content', []) if part.get('type') == 'text']
            if tool_round == 'tool_use':
                calls = [part for part in result.get('content', []) if part.get('type') == 'tool_use']
                require(result.get('stop_reason') == 'tool_use' and len(calls) == 1, 'claude:forced_tool_use_absent')
                require(calls[0]['name'] == 'return_ok' and calls[0]['input'] == {'value': 'OK'}, 'claude:tool_input_invalid')
                # Execute the bounded tool locally; its matching result is sent
                # with the exact assistant history in the later streaming turn.
                require(calls[0]['input']['value'] == 'OK', 'claude:tool_execution_failed')
                source['tool_use_id'] = calls[0]['id']
                source['tool_history'] = [body['messages'][0], {'role': 'assistant', 'content': result['content']}]
        else:
            text_parts = [part['text'] for item in result.get('output', [])
                          for part in item.get('content', []) if part.get('type') == 'output_text']
    if tool_round != 'tool_use':
        require(''.join(text_parts).strip().strip('.').upper() == 'OK', f'{kind}:{label}:unexpected_answer')
    return {'scenario': label, 'provider': kind, 'state': 'green', 'http_status': status,
            'stream': stream, 'usage': usage_numbers(usage), 'answer_verified': True,
            'tool_round': tool_round, 'multi_turn_verified': tool_round == 'tool_result',
            'duration_seconds': round(time.monotonic() - started, 3)}


def save_enabled(product, kind, source, enabled, apply_control, prepare_control_apply, label, scenarios):
    # Subscription saves always consume a fresh approved candidate. The native
    # API source editor's generic saved_source change is not this producer.
    checked = checked_subscription(product, kind, prepare_control_apply, label + '-check')
    candidate = checked['checked_candidate']
    require(candidate['existing_source_id'] == source['source_id'], kind + ':recheck_source_identity_changed')
    snapshot = product.control('ListCompute', {})['data']
    current = next(item for item in snapshot['sources'] if item['source_id'] == source['source_id'])
    if enabled:
        require(current['state'] == 'disabled', kind + ':recheck_enabled_source_before_save')
        scenarios.append(gateway(product, kind, source, False, kind + '-rechecked-disabled-rejection', allowed=False))
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [item['model_ref'] for item in current['models']],
        'intent': 'save_ready' if enabled else 'save_disabled', 'key_edits': [],
        'validation': checked['validation'],
    }
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, label)
    result = product.control('GetComputeSaveResult', {'operation': applied['operation']})['data']
    require(result['disposition'] == 'saved', f'{kind}:{label}:state_change_not_saved')
    require(result['source_id'] == source['source_id'], kind + ':state_change_source_identity_changed')
    require(len(result['bindings']) == 1 and result['bindings'][0]['binding_id'] == source['binding_id'],
            kind + ':state_change_binding_identity_changed')
    require(result['management_state'] == ('ready' if enabled else 'disabled'), kind + ':state_change_state_mismatch')
    source['validation'] = checked['validation']
    scenarios.append({'scenario': label, 'provider': kind, 'state': 'green',
                      'management_state': result['management_state'], 'source_identity_stable': True,
                      'binding_identity_stable': True})


def auth_audit(product, require_both=True):
    result = {}
    for directory in ('cpa', 'cpa-claude'):
        root = product.storage / directory
        require(root.is_dir(), directory + ':runtime_absent')
        configurations = list(root.rglob('config.yaml'))
        require(len(configurations) == 1, directory + ':config_absent')
        auth_line = next((line for line in configurations[0].read_text().splitlines()
                          if line.startswith('auth-dir:')), None)
        require(auth_line is not None, directory + ':auth_directory_absent')
        auth_root = Path(auth_line.split(':', 1)[1].strip().strip('\"\''))
        require(auth_root.is_dir() and stat.S_IMODE(auth_root.stat().st_mode) == 0o700,
                directory + ':auth_directory_permissions')
        files = [path for path in auth_root.glob('hiroute-managed-*.json') if path.is_file()]
        auth = []
        for path in files:
            require(stat.S_IMODE(path.stat().st_mode) == 0o600, directory + ':auth_permissions')
            value = json.loads(path.read_bytes())
            require(not forbidden_refresh(value), directory + ':refresh_authority_in_cpa')
            require(not any('expir' in key.lower() for key in value), directory + ':expiry_in_cpa')
            auth.append({'sha256': digest(path), 'mode': '0o600', 'refresh_authority': False})
        result[directory] = {'auth_file_count': len(auth), 'auth': auth}
    if require_both:
        require(result['cpa']['auth_file_count'] == 1 and result['cpa-claude']['auth_file_count'] == 1,
                'both_providers_must_have_one_access_only_lease')
    return result


def restoration_assertions(product, sources):
    snapshot = product.control('ListCompute', {})['data']
    for kind, identity in sources.items():
        source = next(item for item in snapshot['sources'] if item['source_id'] == identity['source_id'])
        require(source['state'] == ('disabled' if kind == 'claude' else 'ready'), kind + ':restored_state_mismatch')
        require([item['model_ref'] for item in source['models']] == [identity['model_ref']], kind + ':restored_model_mismatch')
        plan = product.public_cli('routing show ' + identity['plan_id'])[1]['data']
        require(plan['head']['model_alias'] == identity['alias'], kind + ':restored_alias_mismatch')
        settings = product.cli('agents connect status ' + identity['context'])[1]['data']
        require(settings.get('current_selection') is not None, kind + ':restored_connection_absent')


def observed_usage(product, results, encoded):
    expected = {key: sum(item.get('usage', {}).get(field, 0) for item in results)
                for key, field in (('input', 'input_tokens'), ('output', 'output_tokens'),
                                   ('cache_read', 'cache_read_input_tokens'))}
    query = {'schema': 'hiroute.observation.query/v2', 'intent': {
        'view': 'home_value', 'query': {'period': 'seven_days', 'session_id': None, 'currency': None}}}
    deadline = time.monotonic() + 30
    while True:
        revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
        grant = product.grant('GetValueV2', {'change_digest': 'sha256:' + hashlib.sha256(encoded(query)).hexdigest(),
                                          'expected_revisions': revisions}, 'real-value-' + str(time.monotonic_ns()))
        value = product.cli('value show', query, grant)[1]['data']
        actual = {item['metric']: item['known_sum'] for item in value['usage']}
        if all(actual.get(key) == total for key, total in expected.items()) and value['pending_requests'] == 0:
            return {'state': 'green', 'totals': expected, 'pending_requests': 0}
        require(time.monotonic() < deadline, 'observation_usage_did_not_converge')
        time.sleep(.1)


def audit_native_sources(report, before, hosts):
    for kind, host in hosts.items():
        if kind in before:
            scenario = {'scenario': kind + '-native-source-integrity', 'provider': kind, 'state': 'green'}
            try:
                after = remote_source(host, kind, 'snapshot')
                report['native_sources'][kind]['after_sha256'] = after['sha256']
                report['native_sources'][kind]['unchanged'] = after['sha256'] == before[kind]['sha256']
                if not report['native_sources'][kind]['unchanged']:
                    scenario.update(state='red', reason='native_source_changed')
            except BaseException:
                report['native_sources'].setdefault(kind, {})['after_snapshot'] = 'unavailable'
                scenario.update(state='red', reason='native_snapshot_unavailable')
            report['scenarios'].append(scenario)
            if scenario['state'] == 'red':
                report['state'] = 'red'


def run(arguments):
    repository = arguments.repository.resolve()
    candidate = subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'], text=True).strip()
    require(candidate == arguments.candidate_sha, 'candidate_checkout_mismatch')
    pin = json.loads((repository / 'vendor/cpa/source.json').read_text())
    provenance = json.loads(arguments.cpa_binary.with_suffix('.provenance.json').read_text())
    require(all(provenance.get(key) == value for key, value in pin.items()), 'cpa_pin_mismatch')
    require(digest(arguments.cpa_binary) == arguments.cpa_sha256 == provenance['sha256'], 'cpa_binary_digest_mismatch')
    sys.path.insert(0, str(repository / 'crates/daemon/tests/support'))
    from publication_product import Product, encoded
    from publication_process import apply_control, configure_model_settings_v2, judgment_fixture, prepare_control_apply

    class RealProduct(Product):
        def control(self, operation, payload, protected_grant=None, success=True):
            result = super().control(operation, payload, protected_grant, success=False)
            self.last_control = {'operation': operation, **safe_envelope(result),
                                 'data_state': (result.get('data') or {}).get('state'),
                                 'data_status': (result.get('data') or {}).get('status')}
            if success:
                require(result.get('error') is None, f'{operation}:{result.get("status")}:{(result.get("error") or {}).get("code")}')
            return result

        def public_cli(self, command, payload=None, secret=None, success=True):
            code, result = super().public_cli(command, payload, secret, success=False)
            if success:
                require(code == 0, f'cli:{command}:{result.get("status")}:{(result.get("error") or {}).get("code")}')
            return code, result

    os.environ['HIROUTE_VALIDATION_PRODUCT_BIN_DIR'] = str(arguments.product_bin.resolve())
    temporary = tempfile.TemporaryDirectory(prefix='hiroute-real-subscriptions-')
    product = RealProduct(repository, root=Path(temporary.name) / 'p')
    report = {'candidate_sha': candidate, 'state': 'red', 'scenarios': [], 'native_sources': {},
              'binaries': {name: digest(arguments.product_bin / name) for name in ('hiroute', 'hirouted')},
              'cpa': {'version': pin['version'], 'commit': pin['commit'], 'patch_sha256': pin['patch_sha256'],
                      'sha256': arguments.cpa_sha256},
              'limitations': ['Source deletion has no current public API and is not executed.',
                              'Mac Keychain interactive acceptance is separate and not executed here.',
                              'No Desktop UI or Windows acceptance in this headless Linux run.']}
    report['started_at_utc'] = datetime.now(timezone.utc).isoformat()
    stage, before = 'credentials', {}
    hosts = {'codex': arguments.codex_host, 'claude': arguments.claude_host}
    try:
        for kind, host in hosts.items():
            before[kind] = remote_source(host, kind, 'borrow')
        configure_private_clients(product, arguments, before)
        report['native_sources'] = before
        report['native_clients'] = {}
        for kind, executable in (('claude', arguments.claude_cli), ('codex', arguments.codex_cli)):
            result = subprocess.run([str(executable), '--version'], env=product.env, cwd=product.project,
                                    capture_output=True, timeout=15)
            match = re.search(rb'\d+\.\d+\.\d+', result.stdout)
            require(result.returncode == 0 and match is not None, kind + ':native_client_version_unavailable')
            report['native_clients'][kind] = {'version': match.group().decode(), 'sha256': digest(executable.resolve())}
        if arguments.prepare_only:
            report['scenarios'].append({'scenario': 'access-only-isolation', 'state': 'green'})
        else:
            def record_inference(kind, stream, label):
                if arguments.control_only or (arguments.claude_roundtrip_only and kind == 'codex'):
                    report['scenarios'].append({'scenario': label, 'provider': kind, 'state': 'not_executed'})
                else:
                    report['scenarios'].append(gateway(product, kind, sources[kind], stream, label))
            stage = 'daemon-start'
            product.start()
            diagnostics = product.diagnostics_snapshot()
            require(diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug', 'daemon_diagnostics_not_debug')
            report['diagnostics_initial'] = diagnostics
            sources = {}
            for kind in ('claude', 'codex'):
                stage = kind + '-check-save-publish-connect'
                sources[kind] = subscription_save(product, kind, apply_control, prepare_control_apply)
                report.setdefault('saved_sources', {})[kind] = dict(sources[kind])
                publish_plan(product, kind, sources[kind], judgment_fixture, configure_model_settings_v2)
                report['scenarios'].append({'scenario': stage, 'provider': kind, 'state': 'green', 'model': sources[kind]['model']})
                print(json.dumps({'progress': stage, 'state': 'green', 'model': sources[kind]['model']}), flush=True)
            stage = 'access-only-cpa-audit'
            report['cpa_auth'] = auth_audit(product)
            for kind, stream in (('claude', False), ('codex', True)):
                stage = kind + '-initial-inference'
                record_inference(kind, stream, stage)
                print(json.dumps(report['scenarios'][-1]), flush=True)
            stage = 'claude-disable'
            save_enabled(product, 'claude', sources['claude'], False, apply_control, prepare_control_apply, stage, report['scenarios'])
            report['scenarios'].append(gateway(product, 'claude', sources['claude'], False, 'claude-disabled-rejection', allowed=False))
            stage = 'restart-restore'
            product.stop()
            product.start()
            restoration_assertions(product, sources)
            report['scenarios'].append({'scenario': stage, 'state': 'green', 'saved_sources': 2, 'saved_plans': 2, 'saved_connections': 2})
            record_inference('codex', False, 'codex-restarted-claude-disabled')
            stage = 'claude-enable-codex-disable'
            save_enabled(product, 'claude', sources['claude'], True, apply_control, prepare_control_apply, 'claude-reenable', report['scenarios'])
            save_enabled(product, 'codex', sources['codex'], False, apply_control, prepare_control_apply, 'codex-disable', report['scenarios'])
            report['scenarios'].append(gateway(product, 'codex', sources['codex'], False, 'codex-disabled-rejection', allowed=False))
            if arguments.isolated_source_failure:
                stage = 'unreadable-isolated-codex-source'
                auth_path = Path(product.env['CODEX_HOME']) / 'auth.json'
                held_path = auth_path.with_name('access-only-source-held.json')
                auth_path.rename(held_path)
                auth_path.symlink_to(held_path)
                try:
                    inventory = product.control('ListComputeSubscriptions', {})['data']
                    candidates = [item['candidate']['candidate_ref'] for item in inventory['candidates']]
                    require(any(item.startswith('candidate/cpa/claude/') for item in candidates), 'claude_discovery_blocked_by_bad_codex_source')
                    require(not any(item.startswith('candidate/cpa/codex/') for item in candidates), 'symlink_codex_source_was_admitted')
                    report['scenarios'].append({'scenario': stage, 'state': 'green',
                                                'other_provider_discoverable': True, 'only_test_home_changed': True})
                    record_inference('claude', True, 'claude-restarted-codex-disabled')
                finally:
                    auth_path.unlink()
                    held_path.rename(auth_path)
            else:
                record_inference('claude', True, 'claude-restarted-codex-disabled')
            stage = 'observation'
            report['observation'] = ({'state': 'not_executed'} if arguments.control_only
                                     else observed_usage(product, report['scenarios'], encoded))
            report['paid_inference_requests'] = sum('usage' in item for item in report['scenarios'])
            report['cpa_auth_final'] = auth_audit(product, require_both=False)
            report['diagnostics_final'] = product.diagnostics_snapshot()
        report['state'] = 'green'
        report['scope'] = ('control_only' if arguments.control_only else
                           'preparation_only' if arguments.prepare_only else
                           'claude_roundtrip_regression' if arguments.claude_roundtrip_only else 'real_inference')
    except BaseException as error:
        report['failure'] = {'stage': stage, 'class': type(error).__name__,
                             'reason': str(error) if isinstance(error, AcceptanceFailure) else 'harness_assertion_or_runtime_failure',
                             'last_control': getattr(product, 'last_control', None)}
        report['scenarios'].append({'scenario': stage, 'state': 'red'})
        report['diagnostics_final'] = product.diagnostics_snapshot()
        evidence = arguments.report.parent / (arguments.report.stem + '-diagnostics')
        diagnostic_files = list(product.diagnostics_root.rglob('*.jsonl'))
        if all(secret.encode() not in path.read_bytes() for secret in product.secrets for path in diagnostic_files):
            report['diagnostics_evidence'] = str(evidence)
            product.preserve_diagnostics(evidence)
    finally:
        # Never let an exception during preparation leave an unconsumed borrowed
        # credential inside the reporting dictionary.
        for item in before.values():
            item.pop('credential', None)
        report['native_sources'] = before
        try:
            process = product.process
            product.stop(diagnostic_failure=True)
            report['daemon_process_exit'] = process.returncode if process is not None else None
            report['upstream_wire_requests'] = wire_request_count(product)
            report['public_output_secret_scan'] = all(secret.encode() not in output for secret in product.secrets for output in product.outputs)
            require(report['public_output_secret_scan'], 'public_output_secret_scan_failed')
            report['daemon_failure_hints'] = [line[:300]
                for output in product.outputs for line in output.decode(errors='replace').splitlines()
                if line.startswith('routing compilation facts are invalid:')]
        except BaseException as error:
            report['state'] = 'red'
            report['cleanup_error'] = type(error).__name__
        audit_native_sources(report, before, hosts)
        product.listener_port_lease.close()
        temporary.cleanup()
        report['temporary_secrets_removed'] = not Path(temporary.name).exists()
        report['gateway_requests'] = getattr(product, 'gateway_request_count', 0)
        report['inference_gateway_attempts'] = getattr(product, 'inference_gateway_attempts', 0)
        report['gateway_failures'] = getattr(product, 'gateway_failures', [])
    arguments.report.parent.mkdir(parents=True, exist_ok=True)
    report['finished_at_utc'] = datetime.now(timezone.utc).isoformat()
    report['paid_inference_requests'] = sum('usage' in item for item in report['scenarios'])
    arguments.report.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps(report, sort_keys=True), flush=True)
    return 0 if report['state'] == 'green' else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True, type=Path)
    parser.add_argument('--candidate-sha', required=True)
    parser.add_argument('--product-bin', required=True, type=Path)
    parser.add_argument('--cpa-binary', required=True, type=Path)
    parser.add_argument('--cpa-sha256', required=True)
    parser.add_argument('--codex-cli', required=True, type=Path)
    parser.add_argument('--claude-cli', required=True, type=Path)
    parser.add_argument('--codex-host', required=True)
    parser.add_argument('--claude-host', required=True)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--isolated-source-failure', action='store_true')
    parser.add_argument('--control-only', action='store_true')
    parser.add_argument('--claude-roundtrip-only', action='store_true')
    try:
        raise SystemExit(run(parser.parse_args()))
    except AcceptanceFailure as error:
        print(json.dumps({'state': 'red', 'reason': str(error)}), flush=True)
        raise SystemExit(1)
