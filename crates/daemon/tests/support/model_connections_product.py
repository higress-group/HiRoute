"""Native API save, publication, Gateway request, and session record through real hirouted."""
import hashlib
import http.client
import http.server
import json
import os
import socket
import subprocess
import sys
import threading
import time
from copy import deepcopy
from pathlib import Path

from publication_process import configure_model_settings_v2
from publication_product import Product, encoded


MODEL = 'gpt-5.4'
NATIVE_TOKEN = 'manual-native-product-token'
V2 = {'major': 2, 'minor': 0}


class NativeUpstream:
    def __init__(self):
        self.requests = []
        self.lock = threading.Lock()
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def send_json(self, status, value):
                body = encoded(value)
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(body)

            def record(self, body=None):
                with owner.lock:
                    owner.requests.append({
                        'method': self.command,
                        'path': self.path,
                        'authorization': self.headers.get('Authorization'),
                        'api_key': self.headers.get('x-api-key'),
                        'body': body,
                    })

            def do_GET(self):
                self.record()
                assert self.path == '/v1/models', self.path
                self.send_json(404, {'error': 'directory not implemented'})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                self.record(body)
                assert self.path == '/v1/responses', self.path
                assert body['model'] == MODEL, body
                self.send_json(200, {
                    'id': 'resp_native_product',
                    'object': 'response',
                    'status': 'completed',
                    'model': MODEL,
                    'output': [{
                        'id': 'msg_native_product',
                        'type': 'message',
                        'role': 'assistant',
                        'status': 'completed',
                        'content': [{
                            'type': 'output_text',
                            'text': 'native product answer',
                            'annotations': [],
                        }],
                    }],
                    'usage': {'input_tokens': 4, 'output_tokens': 3, 'total_tokens': 7},
                })

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = 'http://%s:%d/v1' % self.server.server_address

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def control(product, operation, payload, protected_grant=None, expected_error=None):
    request_id = 'native-product-' + hashlib.sha256(
        encoded((operation, time.monotonic_ns()))).hexdigest()[:24]
    request = {
        'schema_version': V2,
        'request_id': request_id,
        'operation_id': operation,
        'payload': payload,
    }
    if protected_grant is not None:
        request['protected_grant'] = protected_grant
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(15)
        sock.connect(str(product.root / 'runtime/hiroute/control.sock'))
        stream = sock.makefile('rwb')
        stream.write(encoded({
            'api_version': V2,
            'machine_schema_version': V2,
            'client_name': 'native-product-test',
            'client_version': '0.1.0',
        }) + b'\n')
        stream.flush()
        hello = json.loads(stream.readline())
        assert 'local-control-v2' in hello['capabilities'], hello
        stream.write(encoded(request) + b'\n')
        stream.flush()
        envelope_bytes = stream.readline()
    product.outputs.append(envelope_bytes)
    envelope = json.loads(envelope_bytes)
    assert envelope.get('request_id') == request_id, envelope
    if expected_error is None:
        assert envelope.get('error') is None, envelope
    else:
        assert envelope.get('error', {}).get('code') == expected_error, envelope
    return envelope


def desktop_grant(product, operation, digest, revisions, key):
    capability = 'native-product-capability-' + hashlib.sha256(key.encode()).hexdigest()
    product.secrets.add(capability)
    frame = {
        'schema': 'hiroute.protected-apply-grant/v2',
        'registration_id': hashlib.sha256(
            encoded((operation, key, time.monotonic_ns()))).hexdigest(),
        'capability': capability,
        'principal_kind': 'desktop',
        'workspace_id': 'personal/default',
        'operation_kind': operation,
        'accepted_digest': digest,
        'expected_revisions': revisions,
        'expires_at_unix': int(time.time()) + 120,
    }
    product.register_protected_frame(frame)
    return {'principal_kind': 'desktop', 'capability': capability}


def declared(value):
    return {'value': value, 'basis': 'user_declared'}


def save_native_source(product, upstream, token=None, unknown=False,
                       protocol='responses', upstream_model_id=MODEL, variant='',
                       context_tokens=32768):
    status = control(product, 'GetClientServiceStatus', {})['data']
    suffix = ('-unknown' if unknown else '') + ('-' + variant if variant else '')
    candidate_ref = 'candidate/native/product' + suffix
    if token is not None:
        product.secrets.add(token)
        product.register_protected_frame({
            'schema': 'hiroute.protected-input/v1',
            'registration_id': hashlib.sha256(
                encoded(('native-input', candidate_ref, time.monotonic_ns()))).hexdigest(),
            'candidate_ref': candidate_ref,
            'candidate_revision': 1,
            'secret': token,
        })
    draft = {
        'inference_model_id': None,
        'candidate_ref': candidate_ref if token is not None else None,
        'lineage_ref': 'lineage/native/product' + suffix,
        'display_name': 'Product Native API',
        'existing_source_id': None,
        'edit_revision': 1,
        'check_id': 'check/native/product-1',
        'base_url': upstream.base_url,
        'base_kind': 'api_root',
        'request_path_override': None,
        'inventory_path_override': '/v1/models',
        'protocol': protocol,
        'protocol_profile_id': 'profile/custom/' + protocol,
        'protocol_profile_revision': 1,
        'authentication': {'kind': 'bearer'} if token is not None else {'kind': 'none'},
        'configuration_revision': 1,
        'models': [{
            'upstream_model_id': upstream_model_id,
            'display_name': 'Manual Native Model',
            'catalog_configuration_id': None,
            'membership': 'user_declared',
            'capabilities': {
                # A Codex-visible Plan must preserve the client's function-tool contract. This
                # fixture declares and exercises a Responses-capable source rather than asking
                # the catalog adapter to publish a knowingly incompatible route.
                'tool': declared(True),
                'vision': declared(False),
                'streaming': declared(True),
                'context_tokens': declared(context_tokens),
                'max_output_tokens': declared(4096),
                'native_reasoning': declared({
                    'kind': 'fixed', 'profile': 'provider-default'}),
            },
        }],
    }
    if unknown:
        draft['display_name'] = 'Unknown text API'
        draft['check_id'] += suffix
        draft['inventory_path_override'] = None
        draft['display_template_id'] = 'openai.platform.global.v1'
        draft['models'][0]['capabilities'] = {
            key: {'value': None, 'basis': 'unknown'}
            for key in draft['models'][0]['capabilities']}
    check_request = {'draft': draft}
    if token is not None:
        check_request['input_candidate'] = {
            'candidate_ref': candidate_ref,
            'candidate_revision': 1,
        }
    check_digest = 'sha256:' + hashlib.sha256(encoded(check_request)).hexdigest()
    grant = desktop_grant(
        product, 'CheckNativeModelConnection', check_digest,
        status['revisions'], 'native-check' + suffix)
    checked = control(
        product, 'CheckNativeModelConnection', check_request, grant)['data']
    assert checked['directory'] == ('not_run' if unknown else 'unavailable'), checked
    assert checked['inference'] == 'not_run', checked
    manual = next(model for model in checked['candidate']['models']
                  if model['upstream_model_id'] == upstream_model_id)
    assert manual['selectable'] and manual['membership'] == 'user_declared', manual

    snapshot = control(product, 'ListCompute', {})['data']
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': checked['candidate']['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [manual['model_ref']],
        'intent': 'save_ready',
        'key_edits': [],
    }
    preview = control(product, 'PreviewComputeSave', {'change': change})['data']
    applied = control(product, 'ApplyComputeSave', {
        'spec': preview['spec'],
        'accept_digest': preview['accept_digest'],
        'expected_revisions': preview['expected_revisions'],
        'idempotency_key': 'native-product-save' + suffix,
    })
    assert applied['data']['state'] == 'succeeded', applied
    saved = control(product, 'GetComputeSaveResult', {
        'operation': applied['operation']})['data']
    assert saved['disposition'] == 'saved', saved
    assert saved['management_state'] == 'ready', saved
    assert len(saved['bindings']) == 1, saved
    return {
        'binding_id': saved['bindings'][0]['binding_id'],
        'source_id': saved['source_id'],
        'source_revision': saved['saved_revision'],
        'candidate_ref': saved['candidate']['candidate_ref'],
        'candidate_revision': saved['candidate']['candidate_revision'],
    }


def recheck_saved_native_source(product, saved, edit_revision, suffix, unknown=False):
    request = {
        'source_id': saved['source_id'],
        'expected_source_revision': saved['source_revision'],
        'candidate_ref': saved['candidate_ref'],
        'edit_revision': edit_revision,
        'check_id': 'check/native/saved-' + suffix,
    }
    revisions = control(product, 'GetClientServiceStatus', {})['data']['revisions']
    digest = 'sha256:' + hashlib.sha256(encoded(request)).hexdigest()
    grant = desktop_grant(
        product, 'CheckSavedModelConnection', digest, revisions,
        'saved-native-' + suffix)
    checked = control(product, 'CheckSavedModelConnection', request, grant)['data']
    assert checked['directory'] == ('not_run' if unknown else 'unavailable'), checked
    if unknown:
        assert checked['candidate']['models'][0]['fact_basis'] == 'unknown', checked
    assert checked['candidate']['candidate']['candidate_ref'] == saved['candidate_ref'], checked
    assert checked['candidate']['candidate']['candidate_revision'] == (
        saved['candidate_revision'] + 1), checked
    assert checked['candidate']['existing_source_id'] == saved['source_id'], checked
    assert checked['candidate']['provenance'] == 'user_configured', checked
    manual = next(model for model in checked['candidate']['models']
                  if model['upstream_model_id'] == MODEL)
    assert manual['selectable'] and manual['membership'] == 'user_declared', manual
    public = encoded(checked)
    for forbidden in (b'native_recheck', b'credential_id', b'lineage_ref',
                      b'trusted_lineage_digest', b'input_slot'):
        assert forbidden not in public, (forbidden, checked)
    return checked


def edit_saved_credentials(product, upstream, saved):
    """The Desktop path registers input and saves directly, without a model check."""
    def snapshot_source():
        snapshot = control(product, 'ListCompute', {})['data']
        return snapshot, next(source for source in snapshot['sources']
                              if source['source_id'] == saved['source_id'])

    def register(label, token):
        candidate = {'candidate_ref': 'candidate/native/key-' + label,
                     'candidate_revision': 1}
        product.secrets.add(token)
        product.register_protected_frame({
            'schema': 'hiroute.protected-input/v1',
            'registration_id': hashlib.sha256(('key-' + label).encode()).hexdigest(),
            **candidate, 'secret': token,
        })
        return candidate

    def save(edits, label):
        snapshot, source = snapshot_source()
        change = {
            'schema': 'hiroute.compute-management-change/v2',
            'subject': {'kind': 'saved_source', 'source_id': saved['source_id']},
            'expected_revisions': snapshot['revisions'],
            'selected_model_refs': [model['model_ref'] for model in source['models']],
            'intent': 'save_ready', 'key_edits': edits,
        }
        # A rejected preview must allow another attempt using the same live input.
        stale = dict(change, expected_revisions={**snapshot['revisions'],
                                               'target': snapshot['revisions']['target'] + 1})
        control(product, 'PreviewComputeSave', {'change': stale},
                expected_error='REVISION_CONFLICT')
        preview = control(product, 'PreviewComputeSave', {'change': change})['data']
        assert control(product, 'PreviewComputeSave', {'change': change})['data'] == preview
        applied = control(product, 'ApplyComputeSave', {
            'spec': preview['spec'], 'accept_digest': preview['accept_digest'],
            'expected_revisions': preview['expected_revisions'],
            'idempotency_key': 'key-save-' + label,
        })
        result = control(product, 'GetComputeSaveResult',
                         {'operation': applied['operation']})['data']
        assert result['disposition'] == 'saved', result
        saved['source_revision'] = result['saved_revision']
        _, after = snapshot_source()
        before_models = deepcopy(source['models'])
        after_models = deepcopy(after['models'])
        assert len(before_models) == len(after_models), (source, after)
        for before_model, after_model in zip(before_models, after_models):
            before_time = before_model['presentation'].pop('evaluated_at_ms')
            after_time = after_model['presentation'].pop('evaluated_at_ms')
            assert after_time >= before_time, (before_time, after_time)
        assert after_models == before_models, (source, after)
        return after

    with upstream.lock:
        request_count = len(upstream.requests)
    added = register('add', 'native-add-test-token')
    source = save([{'action': 'add', 'input_candidate': added}], 'add')
    assert len(source['keys']) == 2, source
    first, second = source['keys']
    replacement_token = 'native-replacement-test-token'
    replacement = register('replace', replacement_token)
    source = save([
        {'action': 'replace', 'key_id': second['key_id'],
         'expected_generation': second['generation'], 'input_candidate': replacement},
        {'action': 'set_enabled', 'key_id': first['key_id'],
         'expected_generation': first['generation'], 'enabled': False},
    ], 'replace')
    assert source['keys'][1]['generation'] == second['generation'] + 1, source
    assert not source['keys'][0]['enabled'], source
    with upstream.lock:
        assert len(upstream.requests) == request_count, 'credential save performed network I/O'
    return replacement_token


def assert_saved_recheck_fences_before_network(product, upstream, saved):
    with upstream.lock:
        before = len(upstream.requests)
    stale = {
        'source_id': saved['source_id'],
        'expected_source_revision': saved['source_revision'] + 1,
        'candidate_ref': saved['candidate_ref'],
        'edit_revision': 20,
        'check_id': 'check/native/saved-stale',
    }
    revisions = control(product, 'GetClientServiceStatus', {})['data']['revisions']
    stale_grant = desktop_grant(
        product, 'CheckSavedModelConnection',
        'sha256:' + hashlib.sha256(encoded(stale)).hexdigest(), revisions,
        'saved-native-stale')
    control(product, 'CheckSavedModelConnection', stale, stale_grant,
            expected_error='REVISION_CONFLICT')

    bound = dict(stale, expected_source_revision=saved['source_revision'],
                 edit_revision=21, check_id='check/native/saved-bound')
    bound_grant = desktop_grant(
        product, 'CheckSavedModelConnection',
        'sha256:' + hashlib.sha256(encoded(bound)).hexdigest(), revisions,
        'saved-native-bound')
    substituted = dict(bound, source_id='source/substituted')
    control(product, 'CheckSavedModelConnection', substituted, bound_grant,
            expected_error='CAPABILITY_DENIED')
    with upstream.lock:
        assert len(upstream.requests) == before, upstream.requests


def publish_plan_and_agent(product, binding_id):
    product.editor = {
        'schema': 'hiroute.plan-editor/v2',
        'display_name': 'Native API Product Plan',
        'purpose': 'Exercise a saved user configured Native API',
        'mode': 'fixed_model',
        'candidates': [{'binding_id': binding_id}],
        'smart': {'economy': [], 'primary': [], 'primary_fallback': False,
                  'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False,
        'requirements': {},
        'limits': {'maximum_attempts': 2, 'request_timeout_ms': 30000,
                   'attempt_timeout_ms': 30000},
    }
    change = {
        'schema': 'hiroute.plan-content-change/v2',
        'target': {'intent': 'create', 'creation_key': 'native-product-plan'},
        'editor': product.editor,
        'consumed_draft': None,
    }
    preview = product.preview('routing preview', {'change': change})
    # The second source has the same upstream ID but unknown capabilities. Its facts must
    # not replace those of the explicitly selected binding.
    candidate = preview['plan_version']['compiled']['body']['materialized']['attempt_owned']['groups'][0]['candidates'][0]
    profile = next(profile for profile in candidate['protocol_profiles']
                   if profile['ingress_protocol'] == 'responses')
    assert profile['capability']['native_model'] == MODEL
    assert profile['capability']['request']['function_tools'] == 'exact'
    assert profile['capability']['native_streaming'] == {'state': 'exact', 'value': True}
    product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                  {'change': change}, 'native-product-plan')
    product.plan_id = preview['plan_head']['reference']['plan_id']
    product.model_alias = preview['plan_head']['model_alias']
    configure_model_settings_v2(
        product, [product.plan_id], 'native-product-agent',
        agent_id='agent_codex_default', native_model_mode='preserve_available')
    catalog, _ = product.catalog()
    expected_models = [MODEL, product.model_alias]
    assert [model['id'] for model in catalog['data']] == expected_models, catalog


def gateway_request(product):
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
    try:
        client.request('POST', '/v1/responses', body=encoded({
            'model': product.model_alias,
            'input': 'native product request',
            'stream': False,
        }), headers={
            'X-HiRoute-Token': product.bearer(product.agent_connection),
            'Content-Type': 'application/json',
        })
        response = client.getresponse()
        body = response.read()
        product.outputs.append(body)
        assert response.status == 200 and b'native product answer' in body, (response.status, body)
    finally:
        client.close()


def assert_session_record(product, started_ms):
    query = {
        'schema': 'hiroute.observation.query/v2',
        'intent': {'view': 'sessions', 'query': {
            'from_ms': started_ms - 1000,
            'to_ms': int(time.time() * 1000) + 60000,
            'session_id': None,
            'request_id': None,
            'limit': 50,
            'cursor': None,
            'agent_id': None,
            'plan_id': None,
            'native_model': None,
            'outcome': None,
            'only_model_switch': False,
        }},
    }
    revisions = control(product, 'GetClientServiceStatus', {})['data']['revisions']
    capability = product.grant('ListSessionsV2', {
        'change_digest': 'sha256:' + hashlib.sha256(encoded(query)).hexdigest(),
        'expected_revisions': revisions,
    }, 'native-product-sessions')
    deadline = time.monotonic() + 30
    while True:
        sessions = product.cli('sessions list', query, capability)[1]['data']['sessions']
        if sessions:
            session_id = sessions[0]['session_id']
            break
        assert time.monotonic() < deadline, 'native Gateway request did not reach session records'
        time.sleep(.05)
    timeline = {
        'schema': 'hiroute.observation.query/v2',
        'intent': {'view': 'timeline', 'query': {
            **query['intent']['query'],
            'session_id': session_id,
        }},
    }
    timeline_capability = product.grant('GetSessionTimelineV2', {
        'change_digest': 'sha256:' + hashlib.sha256(encoded(timeline)).hexdigest(),
        'expected_revisions': revisions,
    }, 'native-product-timeline')
    while True:
        requests = product.cli(
            'sessions show', timeline, timeline_capability)[1]['data']['requests']
        accepted = [request for request in requests
                    if request['attempted_model_count'] == 1
                    and request['final_native_model'] == MODEL
                    and request['outcome'] == 'accepted']
        if accepted:
            return session_id, accepted[0]['final_native_model']
        assert time.monotonic() < deadline, requests
        time.sleep(.05)


def run(repository, expected_sha=None):
    repository = Path(repository).resolve()
    actual_sha = subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()
    if expected_sha is not None:
        assert actual_sha == expected_sha, (actual_sha, expected_sha)
    product = Product(repository)
    upstream = NativeUpstream()
    product.install_codex_fixture(
        MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
    started_ms = int(time.time() * 1000)
    try:
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN)
        unknown = save_native_source(product, upstream, unknown=True)
        assert unknown['source_id'] != saved['source_id']
        assert unknown['binding_id'] != saved['binding_id']
        snapshot = control(product, 'ListCompute', {})['data']
        unknown_source = next(source for source in snapshot['sources'] if source['source_id'] == unknown['source_id'])
        assert unknown_source['display_template_id'] == 'openai.platform.global.v1'
        assert unknown_source['provenance'] == 'user_configured'
        recheck_saved_native_source(product, unknown, 2, 'unknown-before', unknown=True)
        assert_saved_recheck_fences_before_network(product, upstream, saved)
        recheck_saved_native_source(product, saved, 2, 'before-restart')
        replacement_token = edit_saved_credentials(product, upstream, saved)
        # Rotation invalidates the original Codex credential. The user updates that native
        # connection before enabling routing, so its catalog model retains an exact saved
        # source/account route instead of weakening the provider-switch coverage check.
        product.install_codex_fixture(
            MODEL, 'medium', upstream.base_url, replacement_token)
        publish_plan_and_agent(product, saved['binding_id'])
        gateway_request(product)
        session_id, observed_model = assert_session_record(product, started_ms)
        product.stop()
        product.start()
        recheck_saved_native_source(product, saved, 3, 'after-restart')
        recheck_saved_native_source(product, unknown, 3, 'unknown-after', unknown=True)
        with upstream.lock:
            requests = list(upstream.requests)
        assert [request['path'] for request in requests] == [
            '/v1/models', '/v1/models', '/v1/responses', '/v1/models'], requests
        assert [request['authorization'] for request in requests] == [
            'Bearer ' + token for token in
            [NATIVE_TOKEN, NATIVE_TOKEN, replacement_token, replacement_token]], requests
        assert all(request['api_key'] is None for request in requests), requests
        print(json.dumps({
            'scenario': 'native-model-desktop-contract-to-gateway',
            'state': 'green',
            'candidate': actual_sha,
            'directory_status': 404,
            'saved_binding': saved['binding_id'],
            'saved_source_rechecks': 2,
            'gateway_attempts': 1,
            'credential_resolver_mode': 'saved_bearer',
            'session_record': session_id,
            'session_model': observed_model,
        }), flush=True)
    finally:
        upstream.close()
        product.close()


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
