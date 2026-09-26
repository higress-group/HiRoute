"""Opt-in real Codex subscription -> pinned CPA -> Gateway -> Observation sample.

The supplied Codex auth file is read once into a private temporary CODEX_HOME. The
original file and the user's normal Agent configuration are never modified. Output
contains only numerical usage and safe resource identities, never credentials.
"""

import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import time

from publication_process import (
    apply_control,
    configure_model_settings_v2,
    prepare_subscription_source,
)
from publication_product import Product, encoded


MODEL = 'gpt-5.6-luna'
QUESTIONS = (
    'In one short sentence, what color belongs to entry 13?',
    'In one short sentence, what color belongs to entry 27?',
    'In one short sentence, what color belongs to entry 41?',
    'In one short sentence, what color belongs to entry 55?',
    'In one short sentence, what color belongs to entry 69?',
)


def copy_private_codex_auth(source, destination):
    fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(fd)
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
                or metadata.st_mode & 0o077 or metadata.st_size > 1024 * 1024):
            raise ValueError('Codex auth source is not a bounded private regular file')
        with os.fdopen(fd, 'rb', closefd=False) as handle:
            body = handle.read(1024 * 1024 + 1)
        if len(body) > 1024 * 1024:
            raise ValueError('Codex auth source is too large')
    finally:
        os.close(fd)
    auth = json.loads(body)
    if auth.get('auth_mode') != 'chatgpt' or not isinstance(auth.get('tokens'), dict):
        raise ValueError('Codex auth source is not a ChatGPT subscription')
    out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(out, 'wb', closefd=False) as handle:
            handle.write(body)
            handle.flush()
            os.fsync(out)
    finally:
        os.close(out)
    return [value for value in auth['tokens'].values() if isinstance(value, str) and value]


def configure_isolated_codex(product, auth_source, codex_cli):
    home = product.root / 'home/.codex'
    home.mkdir(mode=0o700)
    product.codex_settings = home / 'config.toml'
    product.codex_settings.write_text(
        'model = "gpt-5.6-luna"\nmodel_reasoning_effort = "low"\n', encoding='utf-8')
    product.codex_settings.chmod(0o600)
    auth = home / 'auth.json'
    product.secrets.update(copy_private_codex_auth(auth_source, auth))
    cache = home / 'models_cache.json'
    shutil.copyfile(
        product.repo / 'crates/integrations/src/agents/codex_bundled_catalog.json', cache)
    cache.chmod(0o600)
    product.env['CODEX_HOME'] = str(home)
    product.env['PATH'] = f'{codex_cli.parent}:{product.root / "bin"}:/usr/bin:/bin'


def save_luna_source(product):
    candidate, validation = prepare_subscription_source(product)
    luna = next((item for item in candidate['models']
                 if item.get('upstream_model_id') == MODEL and item.get('selectable')), None)
    if luna is None:
        raise AssertionError('CPA subscription did not offer selectable Luna')
    snapshot = product.control('ListCompute', {})['data']
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [luna['model_ref']],
        'intent': 'save_ready', 'key_edits': [], 'validation': validation,
    }
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, 'luna-cpa-save')
    saved = product.control('GetComputeSaveResult', {'operation': applied['operation']})['data']
    if saved['disposition'] != 'saved' or saved['management_state'] != 'ready':
        raise AssertionError('Luna CPA source was not saved Ready')
    if len(saved['bindings']) != 1:
        raise AssertionError('Luna CPA source did not produce exactly one binding')
    return saved['bindings'][0]['binding_id']


def publish_luna_plan(product, binding_id):
    editor = {
        'schema': 'hiroute.plan-editor/v2', 'display_name': 'Luna CPA cache sample',
        'purpose': 'Measure real multi-turn cache usage through Gateway',
        'mode': 'fixed_model',
        'candidates': [{'binding_id': binding_id,
                        'reasoning': {'kind': 'profile', 'profile': 'low'}}],
        'smart': {'economy': [], 'primary': [], 'primary_fallback': False,
                  'reselect_on_user_message': False,
                  'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False, 'requirements': {},
        'limits': {'maximum_attempts': 1, 'request_timeout_ms': 90000,
                   'attempt_timeout_ms': 90000},
    }
    change = {
        'schema': 'hiroute.plan-content-change/v2',
        'target': {'intent': 'create', 'creation_key': 'luna-cpa-cache-sample'},
        'editor': editor, 'consumed_draft': None,
    }
    product.editor = editor
    preview = product.preview('routing preview', {'change': change})
    product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                  {'change': change}, 'luna-cpa-plan')
    product.plan_id = preview['plan_head']['reference']['plan_id']
    product.model_alias = preview['plan_head']['model_alias']
    configure_model_settings_v2(product, [product.plan_id], 'luna-cpa-agent',
                                agent_id='agent_codex_default')
    catalog, _ = product.catalog()
    if product.model_alias not in {item['id'] for item in catalog['data']}:
        raise AssertionError('Published Luna alias is absent from Gateway catalog')


def reference_text():
    # A stable, useful prefix over the GPT-5.6 minimum cacheable length. The exact
    # same prefix is retained while new user/assistant turns are appended.
    lines = [
        'This is a fictional reference ledger. Answer only from its entries. '
        'Each entry has an index, a color and a shape; no external facts are needed.'
    ]
    colors = ('blue', 'green', 'amber', 'violet')
    shapes = ('circle', 'square', 'triangle')
    for number in range(1, 91):
        lines.append(
            f'Entry {number:03d}: the recorded color is {colors[number % 4]} '
            f'and the recorded shape is {shapes[number % 3]}. '
            'The ledger entry is independent from neighboring entries; '
            'use its own recorded fields when answering a question.')
    return '\n'.join(lines)


def output_text(response):
    chunks = [part['text'] for item in response.get('output', [])
              for part in item.get('content', [])
              if part.get('type') == 'output_text' and isinstance(part.get('text'), str)]
    answer = '\n'.join(chunks).strip()
    if not answer:
        raise AssertionError('Luna completed without an output_text answer')
    return answer


def gateway_turn(product, history, question, turn):
    input_items = [*history, {'type': 'message', 'role': 'user', 'content': question}]
    body = {
        'model': product.model_alias, 'input': input_items, 'stream': False,
        'reasoning': {'effort': 'low'}, 'max_output_tokens': 256,
    }
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=100)
    try:
        # The Gateway selects `model` within a bounded leading window. Keep it
        # first on the wire instead of using the sorted control-plane encoder.
        wire = json.dumps(body, separators=(',', ':')).encode()
        client.request('POST', '/v1/responses', body=wire, headers={
            'Content-Type': 'application/json',
            'X-HiRoute-Token': product.bearer(product.agent_connection),
            'session-id': 'luna-cpa-cache-multiturn',
        })
        response = client.getresponse()
        payload = response.read()
        product.outputs.append(payload)
    finally:
        client.close()
    if response.status != 200:
        try:
            error_body = json.loads(payload)
            error = error_body.get('error', {}) if isinstance(error_body, dict) else {}
            safe = {}
            for key in ('code', 'type', 'message'):
                value = error.get(key) if isinstance(error, dict) else None
                if value is None and isinstance(error_body, dict):
                    value = error_body.get(key)
                if isinstance(value, str):
                    for secret in product.secrets:
                        value = value.replace(secret, '[redacted]')
                    safe[key] = value[:200]
            if isinstance(error, str):
                safe['error'] = error[:200]
        except (ValueError, AttributeError):
            safe = {'body_shape': 'not_json'}
        raise AssertionError(f'Gateway turn {turn} returned HTTP {response.status}, error={safe}')
    result = json.loads(payload)
    usage = result.get('usage') or {}
    details = usage.get('input_tokens_details') or {}
    input_tokens = usage.get('input_tokens')
    cached_tokens = details.get('cached_tokens')
    output_tokens = usage.get('output_tokens')
    if (not isinstance(input_tokens, int) or not isinstance(cached_tokens, int)
            or not isinstance(output_tokens, int)
            or input_tokens <= 0 or not 0 <= cached_tokens <= input_tokens):
        raise AssertionError('CPA/Gateway response omitted valid native usage')
    answer = output_text(result)
    history.extend(({'type': 'message', 'role': 'user', 'content': question},
                    {'type': 'message', 'role': 'assistant', 'content': answer}))
    return {'turn': turn, 'input_tokens': input_tokens,
            'cache_read_tokens': cached_tokens, 'output_tokens': output_tokens,
            'cache_hit_percent': round(100 * cached_tokens / input_tokens, 2)}


def observation_value(product, turn):
    query = {'schema': 'hiroute.observation.query/v2', 'intent': {
        'view': 'home_value', 'query': {'period': 'seven_days',
                                      'session_id': None, 'currency': None}}}
    revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
    capability = product.grant('GetValueV2', {
        'change_digest': 'sha256:' + hashlib.sha256(encoded(query)).hexdigest(),
        'expected_revisions': revisions},
        f'luna-value-turn-{turn}-{time.monotonic_ns()}')
    return product.cli('value show', query, capability)[1]['data']


def await_observation(product, turns):
    expected_input = sum(item['input_tokens'] for item in turns)
    expected_output = sum(item['output_tokens'] for item in turns)
    expected_read = sum(item['cache_read_tokens'] for item in turns)
    deadline = time.monotonic() + 30
    while True:
        value = observation_value(product, len(turns))
        usage = {item['metric']: item for item in value['usage']}
        hit = value['input_cache_hit']
        if (usage['input']['known_sum'] == expected_input
                and usage['output']['known_sum'] == expected_output
                and usage['cache_read']['known_sum'] == expected_read
                and value['pending_requests'] == 0):
            expected_bp = (expected_read * 10000 + expected_input // 2) // expected_input
            if (hit['state'] != 'available' or hit['ratio_basis_points'] != expected_bp
                    or hit['cache_read_tokens'] != expected_read
                    or hit['total_input_tokens'] != expected_input
                    or hit['eligible_attempt_count'] != len(turns)):
                raise AssertionError('Observation cache hit differs from provider usage')
            return {'input_tokens': expected_input, 'cache_read_tokens': expected_read,
                    'output_tokens': expected_output, 'ratio_basis_points': expected_bp,
                    'eligible_attempt_count': hit['eligible_attempt_count']}
        if time.monotonic() >= deadline:
            raise AssertionError('Observation did not converge to provider usage')
        time.sleep(.1)


def run(arguments):
    product = Product(arguments.repository)
    result = None
    failure = None
    try:
        configure_isolated_codex(product, arguments.auth_source, arguments.codex_cli)
        product.enable_debug_diagnostics()
        product.cpa_args = [
            '--cpa-binary', str(arguments.cpa_binary),
            '--cpa-sha256', arguments.cpa_sha256,
        ]
        product.startup_timeout = 90
        product.start()
        binding_id = save_luna_source(product)
        publish_luna_plan(product, binding_id)
        history = [{'type': 'message', 'role': 'developer',
                    'content': reference_text()}]
        turns = []
        observation = None
        for index, question in enumerate(QUESTIONS, 1):
            turns.append(gateway_turn(product, history, question, index))
            observation = await_observation(product, turns)
        result = {'state': 'green', 'model': MODEL, 'effort': 'low',
                  'path': 'Codex subscription -> managed CPA -> Gateway -> Observation',
                  'turns': turns, 'observation': observation}
    except BaseException as error:
        failure = error
        evidence = (product.repo / 'target/product-e2e-evidence'
                    / f'cpa-luna-gateway-{os.getpid()}')
        snapshot = product.preserve_diagnostics(evidence)
        print(json.dumps({
            'diagnostics': snapshot, 'daemon_exit': (
                product.process.poll() if product.process is not None else None),
        }, sort_keys=True), file=sys.stderr, flush=True)
        raise
    finally:
        try:
            product.close()
        except Exception as close_error:
            if failure is None:
                raise
            print(f'cleanup_error={type(close_error).__name__}', file=sys.stderr)
    print(json.dumps(result, sort_keys=True), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository', type=Path)
    parser.add_argument('--cpa-binary', required=True, type=Path)
    parser.add_argument('--cpa-sha256', required=True)
    parser.add_argument('--codex-cli', required=True, type=Path)
    parser.add_argument('--auth-source', required=True, type=Path)
    run(parser.parse_args())
