"""Shipped Claude facts through login/check/save, routing CLI and real Gateway.

The existing external CPA fixture supplies only OAuth, account inventory and
protocol responses. It supplies no model capabilities and no HiRoute business
state. Every capability comes from the daemon's ordinary bundled catalog.
"""
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import sys
import time
import uuid
from urllib.parse import parse_qs, urlparse

from publication_process import (
    configure_model_settings_v2, judgment_fixture,
    prepare_control_apply, save_compute_candidate,
)
from publication_product import Product, encoded


CASES = {
    'haiku': ('claude-haiku-4-5-20251001', {'kind': 'toggle', 'enabled': True}),
    'sonnet': ('claude-sonnet-5', {'kind': 'profile', 'profile': 'high'}),
}
IMAGE = {'type': 'image', 'source': {'type': 'base64', 'media_type': 'image/png',
    'data': 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jC1sAAAAASUVORK5CYII='}}
FIXTURE_EXTENSION = r'''
class CatalogInferenceHandler(Handler):
    def do_POST(self):
        if self.path != '/v1/messages':
            return super().do_POST()
        if not self.authorized(downstream):
            return
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        model = body['model']
        assert model.endswith('/' + (controls / 'catalog-model').read_text().strip())
        has_image = any(isinstance(message.get('content'), list) and any(
            block.get('type') == 'image' for block in message['content'])
            for message in body.get('messages', []))
        thinking = body.get('thinking') or {}
        facts = {'has_image': has_image, 'max_tokens': body.get('max_tokens'),
                 'thinking_type': thinking.get('type', 'absent'),
                 'thinking_budget': thinking.get('budget_tokens'),
                 'effort': (body.get('output_config') or {}).get('effort')}
        fd = os.open(controls / 'inference.jsonl',
                     os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, 'w') as stream:
            stream.write(json.dumps(facts) + '\n')
        self.send({'id': 'catalog-fixture-message', 'type': 'message',
                   'role': 'assistant', 'model': model,
                   'content': [{'type': 'text', 'text': 'CATALOG_OK'}],
                   'stop_reason': 'end_turn', 'stop_sequence': None,
                   'usage': {'input_tokens': 3, 'output_tokens': 2}})
'''


class RegressionFailure(Exception):
    pass


def require(condition, code):
    if not condition:
        raise RegressionFailure(code)


def public(product, command, payload=None, secret=None):
    code, envelope = product.public_cli(command, payload, secret, success=False)
    require(code == 0, 'public_cli_failed')
    return envelope['data']


def private_json(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(encoded(value) + b'\n')


def fixture(product, model):
    controls = product.root / 'external-cpa-fixture'
    controls.mkdir(mode=0o700)
    pin = json.loads((product.repo / 'vendor/cpa/source.json').read_bytes())
    source = (product.repo / 'crates/cpa-bridge/src/managed_sessions/cpa_oauth_fixture.py').read_text()
    source = source.replace('__HIR_CPA_VERSION__', pin['version'])
    marker = "server = http.server.ThreadingHTTPServer(('127.0.0.1', int(scalar('port'))), Handler)"
    require(source.count(marker) == 1, 'external_fixture_entry_changed')
    source = source.replace(marker, FIXTURE_EXTENSION + '\n' + marker.replace('Handler)', 'CatalogInferenceHandler)'))
    binary = controls / 'cpa-fixture'
    binary.write_text(source)
    binary.chmod(0o700)
    (controls / 'catalog-model').write_text(model)
    (controls / 'catalog-model').chmod(0o600)
    product.cpa_args = ['--cpa-binary', str(binary), '--cpa-sha256', hashlib.sha256(source.encode()).hexdigest()]
    product.secrets.update(('fixture-managed-access', 'fixture-managed-refresh'))
    return controls


def authorize_and_save(product, label):
    started = public(product, 'compute connection login', {'action': 'start', 'provider': 'claude'})['sessions'][0]
    state = parse_qs(urlparse(started['authorization_url']).query)['state'][0]
    callback = 'fixture-authorization-code#' + state
    registered = public(product, 'protected-input register --candidate ' +
                        started['callback_input_candidate']['candidate_ref'], secret=callback)
    require(registered['registered'], 'callback_not_registered')
    authorized = public(product, 'compute connection login', {
        'action': 'callback', 'login_ref': started['login_ref'],
        'input_candidate': started['callback_input_candidate']})['sessions'][0]
    require(authorized['status'] == 'authorized', 'fixture_login_not_authorized')
    # The checked/save DTOs and one-shot host grant use the production Local Control
    # boundary; the fixture never writes a candidate, SavedSource or routing row.
    preview = product.control('PreviewSubscriptionCheck', {'candidate': authorized['candidate']})['data']
    payload, grant = prepare_control_apply(product, 'ApplySubscriptionCheck', preview, label + '-check')
    applied = product.control('ApplySubscriptionCheck', payload, grant)
    deadline = time.monotonic() + 30
    while True:
        checked = product.control('GetSubscriptionCheckResult', {'operation': applied['operation']})['data']
        if checked['status'] != 'checking':
            break
        require(time.monotonic() < deadline, 'subscription_check_timeout')
        time.sleep(.05)
    require(checked['status'] == 'verified', 'subscription_check_not_verified')
    candidate = checked['checked_candidate']
    require(len(candidate['models']) == 1 and candidate['models'][0]['selectable'], 'subscription_inventory_not_selectable')
    saved = save_compute_candidate(product, candidate, label + '-save', checked['validation'])
    binding = saved['bindings'][0]['binding_id']
    # Check intentionally exposes only fact_basis. The normalized complete facts
    # become public on the persisted source; consume that ordinary ListCompute view.
    snapshot = product.control('ListCompute', {'source_id': saved['source_id']})['data']
    require(len(snapshot['sources']) == 1, 'saved_source_not_unique')
    models = [model for model in snapshot['sources'][0]['models'] if model['binding_id'] == binding]
    require(len(models) == 1 and models[0]['model_ref'] == candidate['models'][0]['model_ref'],
            'saved_model_does_not_match_checked_binding')
    return candidate['models'][0], models[0], binding


def baseline_reasoning(model):
    actual = model['capabilities']['native_reasoning']['value']
    if actual['kind'] == 'toggle':
        return {'kind': 'toggle', 'enabled': False}
    if actual['kind'] == 'discrete':
        return {'kind': 'profile', 'profile': actual['profiles'][0]}
    return None


def change_for(binding, label, reasoning=None, requirements=None):
    candidate = {'binding_id': binding}
    if reasoning is not None:
        candidate['reasoning'] = reasoning
    return {'schema': 'hiroute.plan-content-change/v2',
        'target': {'intent': 'create', 'creation_key': label},
        'editor': {'schema': 'hiroute.plan-editor/v2', 'display_name': label,
            'purpose': 'Shipped Claude subscription capability regression', 'mode': 'fixed_model',
            'candidates': [candidate],
            'smart': {'economy': [], 'primary': [], 'judgment': judgment_fixture(),
                'reselect_on_user_message': False, 'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
            'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
            'delegation_enabled': False, 'requirements': requirements or {},
            'work': {'harness': 'claude_code', 'protocol': 'messages'},
            'limits': {'maximum_attempts': 1, 'request_timeout_ms': 10000, 'attempt_timeout_ms': 10000}},
        'consumed_draft': None}


def publish(product, change, preview=None):
    preview = preview or public(product, 'routing preview', {'change': change})
    applied = public(product, 'routing apply', {'change': change,
        'accept_digest': preview['change_digest'], 'expected_revisions': preview['expected_revisions'],
        'idempotency_key': change['target']['creation_key']})
    require(applied['state'] == 'succeeded', 'plan_not_published')
    return preview['plan_head']['reference']['plan_id'], preview['plan_head']['model_alias']


def transport_rows(controls):
    path = controls / 'inference.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def gateway(product, controls, alias, image=False, max_tokens=8):
    before = len(transport_rows(controls))
    content = [IMAGE, {'type': 'text', 'text': 'Return CATALOG_OK.'}] if image else 'Return CATALOG_OK.'
    connection = http.client.HTTPConnection('127.0.0.1', product.port, timeout=15)
    try:
        connection.request('POST', '/v1/messages', body=encoded({
            'model': alias, 'messages': [{'role': 'user', 'content': content}],
            'max_tokens': max_tokens, 'stream': False}), headers={
                'Content-Type': 'application/json', 'anthropic-version': '2023-06-01',
                'X-HiRoute-Token': product.bearer(product.agent_connection)})
        response = connection.getresponse()
        payload = response.read()
    finally:
        connection.close()
    rows = transport_rows(controls)
    answer = False
    error_code = None
    if response.status == 200:
        value = json.loads(payload)
        answer = any(block.get('type') == 'text' and block.get('text') == 'CATALOG_OK'
                     for block in value.get('content', []))
    else:
        try:
            value = json.loads(payload)
            error_code = value.get('code')
        except (json.JSONDecodeError, AttributeError):
            error_code = 'other'
        if error_code not in ('CLIENT_PROTOCOL_UNREPRESENTABLE', 'NO_ELIGIBLE_CANDIDATE'):
            error_code = 'other'
    return {'http_status': response.status, 'answer_verified': answer,
            'error_code': error_code,
            'upstream_sends': len(rows) - before,
            'transport': rows[-1] if len(rows) > before else None}


def run(repository, label):
    model_id, thinking = CASES[label]
    product = Product(repository)
    product.root.chmod(0o700)
    # Explicit independent native and XDG directories; no borrowed credentials or
    # inherited alternative configuration is admitted to this regression process.
    product.env = {key: value for key, value in product.env.items() if key in (
        'HOME', 'CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'QODER_CONFIG_DIR', 'PI_CODING_AGENT_DIR',
        'DSH_HOME', 'PATH', 'HIROUTE_RUNTIME_DIR', 'HIROUTE_WORKER_RECEIPT_DIR', 'HIROUTE_REPLAY_ROOT',
        'LANG', 'LC_ALL', 'LC_CTYPE', 'TZ', 'SSL_CERT_FILE', 'SSL_CERT_DIR')}
    home = Path(product.env['HOME'])
    product.env.update(XDG_CONFIG_HOME=str(home / '.config'), XDG_CACHE_HOME=str(home / '.cache'),
                       XDG_DATA_HOME=str(home / '.local/share'), TMPDIR=str(product.root / 'tmp'),
                       CLAUDE_SECURESTORAGE_CONFIG_DIR=product.env['CLAUDE_CONFIG_DIR'])
    (product.root / 'tmp').mkdir(mode=0o700)
    product.settings.write_bytes(encoded({}))
    product.enable_debug_diagnostics()
    controls = fixture(product, model_id)
    report = {'schema': 'hiroute.claude-catalog-product/v1', 'state': 'red', 'case': label,
              'model': model_id, 'real_provider_requests': 0, 'native_credentials_used': False,
              'binary_sha256': {name: hashlib.sha256((product.bin / name).read_bytes()).hexdigest()
                                for name in ('hiroute', 'hirouted')},
              'bundle_sha256': hashlib.sha256((product.repo / 'assets/release-facts/current/bundle/model-data.json').read_bytes()).hexdigest(),
              'assertions': []}

    def record(name, passed, facts=None):
        report['assertions'].append({'name': name, 'state': 'green' if passed else 'red', 'facts': facts or {}})

    try:
        product.start()
        diagnostics = product.diagnostics_snapshot()
        require(diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug', 'diagnostics_not_debug')
        checked_model, model, binding = authorize_and_save(product, label)
        require(model['upstream_model_id'] == model_id, 'inventory_model_changed')
        baseline_selection = baseline_reasoning(model)
        baseline = change_for(binding, label + '-plain', baseline_selection)
        plan, alias = publish(product, baseline)
        product.editor, product.plan_id = baseline['editor'], plan
        configure_model_settings_v2(product, [plan], label + '-connection', agent_id='agent_claude_default')
        image = gateway(product, controls, alias, image=True)
        record('gateway_image_reaches_exact_saved_subscription', image['http_status'] == 200
               and image['answer_verified'] and image['upstream_sends'] == 1
               and image['transport']['has_image'], image)
        sent = image['transport'] or {}
        baseline_rendered = (sent.get('thinking_type') == 'disabled' if label == 'haiku'
                             else sent.get('thinking_type') == 'adaptive'
                             and baseline_selection is not None
                             and sent.get('effort') == baseline_selection['profile'])
        record('gateway_renders_maintained_baseline_reasoning', image['http_status'] == 200
               and image['answer_verified'] and image['upstream_sends'] == 1
               and baseline_rendered, image)
        output = gateway(product, controls, alias, max_tokens=9000)
        record('gateway_output_above_runtime_fallback_limit', output['http_status'] == 200
               and output['answer_verified'] and output['upstream_sends'] == 1
               and output['transport']['max_tokens'] == 9000, output)
        for name, requirements in [('vision', {'vision': True}),
                                   ('context', {'minimum_context_tokens': 150000}),
                                   ('output', {'minimum_output_tokens': 9000})]:
            change = change_for(binding, label + '-' + name, baseline_reasoning(model), requirements)
            code, envelope = product.public_cli('routing preview', {'change': change}, success=False)
            record('public_plan_qualifies_' + name, code == 0 and envelope.get('data') is not None,
                   {'process_exit': code, 'machine_status': envelope.get('status')})
        change = change_for(binding, label + '-thinking', thinking)
        code, envelope = product.public_cli('routing preview', {'change': change}, success=False)
        record('public_plan_accepts_maintained_reasoning', code == 0 and envelope.get('data') is not None,
               {'process_exit': code, 'machine_status': envelope.get('status')})
        if code == 0 and envelope.get('data') is not None:
            thinking_plan, thinking_alias = publish(product, change, envelope['data'])
            product.editor, product.plan_id = change['editor'], thinking_plan
            configure_model_settings_v2(product, [plan, thinking_plan], label + '-thinking-connection',
                                        default_plan_id=thinking_plan, agent_id='agent_claude_default')
            result = gateway(product, controls, thinking_alias, max_tokens=2048)
            sent = result['transport'] or {}
            correct = (sent.get('thinking_type') == 'enabled' and sent.get('thinking_budget') == 1024
                       if label == 'haiku' else sent.get('thinking_type') == 'adaptive' and sent.get('effort') == 'high')
            record('gateway_renders_exact_native_reasoning', result['http_status'] == 200
                   and result['answer_verified'] and result['upstream_sends'] == 1 and correct, result)
            if label == 'haiku':
                blocked = gateway(product, controls, thinking_alias, max_tokens=1024)
                record('gateway_manual_thinking_rejects_equal_output_budget',
                       (blocked['http_status'], blocked['error_code']) in (
                           (400, 'CLIENT_PROTOCOL_UNREPRESENTABLE'),
                           (502, 'NO_ELIGIBLE_CANDIDATE'))
                       and blocked['upstream_sends'] == 0 and blocked['transport'] is None,
                       blocked)
        caps = model['capabilities']
        record('checked_and_saved_facts_use_registered_catalog', checked_model['fact_basis'] == 'registered_catalog'
               and model.get('catalog_configuration_id') is not None
               and all(caps[name]['basis'] == 'registered_catalog' for name in (
                   'vision', 'context_tokens', 'max_output_tokens', 'native_reasoning')),
               dict({name: caps[name] for name in (
                   'vision', 'context_tokens', 'max_output_tokens', 'native_reasoning')},
                    checked_fact_basis=checked_model['fact_basis']))
        record('native_auth_stores_remain_absent', not (home / '.codex/auth.json').exists()
               and not (home / '.claude/.credentials.json').exists())
        report['state'] = 'green' if all(row['state'] == 'green' for row in report['assertions']) else 'red'
    except Exception as error:
        report['failure'] = {'class': type(error).__name__, 'code': str(error)
            if isinstance(error, RegressionFailure) and re.fullmatch(r'[a-z_]+', str(error)) else 'harness_failure'}
    finally:
        report['diagnostics'] = product.diagnostics_snapshot()
        if report['state'] != 'green':
            evidence = product.repo / 'target/product-e2e-evidence' / ('claude-catalog-' + label + '-' + uuid.uuid4().hex)
            evidence.mkdir(mode=0o700, parents=True)
            product.preserve_diagnostics(evidence / 'diagnostics')
            report['evidence_root'] = str(evidence)
            private_json(evidence / 'report.json', report)
        product.close()
    print(json.dumps(report, sort_keys=True), flush=True)
    require(report['state'] == 'green', 'claude_catalog_product_red')


if __name__ == '__main__':
    run(Path(sys.argv[1]), sys.argv[2])
