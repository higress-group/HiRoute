"""Frozen context through real CLI, journal, publication, Gateway and restart.

Both wire codecs use counted loopback providers; no paid endpoint is contacted.
"""
import copy
import hashlib
import http.client
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
import threading
import time

from publication_product import Product, SENTINEL, V1, encoded
from publication_process import bootstrap, plan_change
from model_connections_product import (MODEL, NATIVE_TOKEN, NativeUpstream,
                                       publish_plan_and_agent, save_native_source)


class Endpoint(BaseHTTPRequestHandler):
    requests = []

    def do_POST(self):
        raw = self.rfile.read(int(self.headers['Content-Length']))
        body = json.loads(raw)
        self.requests.append({'body': body, 'bytes': len(raw),
            'authorization': self.headers.get('Authorization'),
            'long_context': self.headers.get('TypeSafe-Danger-Zone-Long-Context')})
        if isinstance(body['questions'], list):
            answers = []
            for q in body['questions']:
                if q['type'] == 'choice':
                    answers.append({'name': q['name'], 'type': 'choice', 'choice': q['choices'][0]['value'],
                        'probabilities': [{'value': choice['value'], 'probability': float(i == 0)}
                                          for i, choice in enumerate(q['choices'])]})
                else:
                    answers.append({'name': q['name'], 'type': 'score', 'score': 0.9,
                        'probabilities': [{'value': i, 'label': level['label'], 'probability': float(i == 0)}
                                          for i, level in enumerate(q['levels'])]})
        else:
            answers = {name: {'type': 'choice', 'choice': next(iter(q['criteria']))}
                if q['type'] == 'choice' else {'type': 'score', 'score': 0.9}
                if len(q['criteria']) == 3 else {'type': 'score', 'probabilities': {'0': 1.0, '1': 0.0}}
                for name, q in body['questions'].items()}
        result = encoded({'answers': answers})
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(result)))
        self.end_headers()
        self.wfile.write(result)

    def log_message(self, *_args):
        pass


def state(body):
    return body['state'] if isinstance(body['questions'], dict) else body['input'][0]['content'][0]['data']['state']


def forwarded_text(body):
    content = body['input'][0]['content']
    return content if isinstance(content, str) else ''.join(part['text'] for part in content)


def run(repository):
    product = Product(repository)
    product.collaboration_only = True
    server = ThreadingHTTPServer(('127.0.0.1', 0), Endpoint)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    upstream = NativeUpstream()
    endpoint = f'http://127.0.0.1:{server.server_port}/decision'

    def data(command, payload=None):
        return product.public_cli(command, payload)[1]['data']

    def save_preview(preview, key):
        result = data('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': key})
        assert result['state'] == 'succeeded', result
        return preview['normalized_spec']['desired_state']['change']['service']

    def delete_connection(id, key, referenced=False):
        preview = data(f'decision connections apply --id {id} --delete')
        code, result = product.public_cli('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': key}, success=False)
        if referenced:
            assert code == 3, result
            assert result.get('status') == 'conflict' and result.get('error', {}).get('code') == 'REVISION_CONFLICT', result
            assert 'operation' not in result and 'data' not in result, 'in-use deletion was accepted before refusal'
        else:
            assert code == 0, result
            assert result.get('data', {}).get('state') == 'succeeded', result

    def publish(service, key):
        editor = copy.deepcopy(product.editor)
        editor['branch_routing']['classifier'] = {'kind': 'decision_service_v3', 'service': service}
        change = plan_change(product, 'update', **editor)
        preview = product.preview('routing preview', {'change': change})
        assert service['context']['resolved']['context_digest'] in json.dumps(preview['plan_version']['compiled'])
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, key)
        product.editor = editor

    def request(content, session, tools=False):
        client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
        body = {'model': product.model_alias, 'input': content}
        if tools:
            body['tools'] = [{'type': 'function', 'name': 'native_probe', 'description': 'Read fixture', 'parameters': {'type': 'object', 'properties': {}}}]
        try:
            client.request('POST', '/v1/responses', body=encoded(body), headers={
                'Content-Type': 'application/json', 'X-HiRoute-Token': product.bearer(product.agent_connection),
                'session-id': session})
            response = client.getresponse()
            raw = response.read()
            product.outputs.append(raw)
            assert response.status == 200, (response.status, raw[:200])
            return json.loads(raw)
        finally:
            client.close()

    def assert_retained_budget_diagnostics():
        deadline = time.monotonic() + 30
        while True:
            result = product.control('GetPlanQualitySamples', {'schema': 'hiroute.observation.query/v2',
                'intent': {'view': 'plan_quality', 'query': {'plan_id': product.plan_id,
                    'include_routing_observations': True, 'limit': 100}}}, success=False)
            error = result.get('error')
            if error is not None:
                assert error['code'] == 'CHANGE_PREVIEW_STALE', result
                budgets = []
            else:
                budgets = [sample.get('selection', {}).get('routing_observation', {}).get('context_budget')
                    for sample in result['data']['samples'] if sample.get('selection') and sample['selection'].get('routing_observation')]
                budgets = [budget for budget in budgets if budget]
            if any(b['encoded_bytes'] is None and b['no_fit_reason'] == 'mandatory_core' for b in budgets):
                assert any((b['encoded_bytes'] or 0) > 262144 for b in budgets), 'large wire count missing from retained query'
                assert all(b['estimator'] == 'escaped-utf8-v1' for b in budgets)
                assert all(not any(word in json.dumps(b) for word in ['quoted', 'fixture complete', 'Bearer']) for b in budgets), 'raw content in safe diagnostic'
                return
            assert time.monotonic() < deadline, 'local no-fit was not observable through the real retained query'
            time.sleep(.05)

    try:
        bootstrap(product)
        # Exercise installed defaults, not hand-authored model IDs that can hide
        # a preset/metadata mismatch. These queries never contact the providers.
        for provider, model, window, header in [
                ('typesafe', 'jev-latest', 262144, True),
                ('openrouter', '~typesafe/jev-latest', 262144, False),
                ('openai', 'gpt-6-luna', 1050000, False)]:
            preset = data(f'decision connections options --provider {provider}')['selected_provider']
            assert preset['model'] == model, preset
            preview_request = {name: preset[name] for name in ('endpoint', 'model', 'wire_protocol')}
            preview_request['policy'] = {'mode': 'automatic', 'typesafe_long_context': 'automatic'}
            resolved = data('decision context preview', preview_request)
            assert resolved['request_window_tokens'] == window and resolved['per_question_window_tokens'] == window, resolved
            assert resolved['request_source'] == 'metadata' and resolved['typesafe_long_context_header'] == header, resolved
            if provider == 'typesafe':
                preview_request['policy']['typesafe_long_context'] = 'disabled'
                smaller = data('decision context preview', preview_request)
                assert (smaller['request_window_tokens'], smaller['per_question_window_tokens']) == (64000, 32000), smaller
                assert not smaller['typesafe_long_context_header'], smaller
        assert not Endpoint.requests, 'Preset previews contacted a provider'
        assert not data('decision connections list')['services'], 'Preset previews saved a connection'
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        source = save_native_source(product, upstream, NATIVE_TOKEN, cli_input=True)
        source_id = product.source_id
        for codec in ('system_one', 'openai_decisions'):
            id = 'decision/context-' + codec
            command = f'decision connections apply --id {id} --name Context --provider compatible --model fixture --endpoint {endpoint} --wire-protocol {codec}'
            # System One uses current shared authority, OpenAI uses protected independent input.
            if codec == 'system_one':
                auth = ' --source ' + source_id
            else:
                token = 'Bearer adaptive-independent-fixture'
                product.secrets.add(token)
                product.register_protected_frame({'schema': 'hiroute.protected-input/v1',
                    'registration_id': hashlib.sha256(b'adaptive-independent').hexdigest(),
                    'candidate_ref': 'candidate/adaptive-independent', 'candidate_revision': 1, 'secret': token})
                auth = ' --candidate candidate/adaptive-independent'
            before = len(Endpoint.requests)
            preview = data(command + auth + ' --context-window-tokens 1050000')
            stale_spec = copy.deepcopy(preview['normalized_spec'])
            stale_context = stale_spec['desired_state']['change']['service']['context']['resolved']
            stale_context['catalog_revision'] += 1
            stale_context.pop('context_digest')
            stale_context['context_digest'] = 'sha256:' + hashlib.sha256(encoded(stale_context)).hexdigest()
            code, conflict = product.public_cli('decision connections apply',
                {'schema_version': V1, 'spec': stale_spec}, success=False)
            assert code != 0 and conflict['error']['code'] == 'REVISION_CONFLICT', conflict
            assert all(v['service']['id'] != id for v in data('decision connections list')['services'])
            service = save_preview(preview, codec + '-create')
            assert len(Endpoint.requests) == before, 'save called provider'
            assert service['schema'] == 'hiroute.decision-service/v3'
            assert service['context']['resolved']['request_window_tokens'] == 1050000
            view = next(v for v in data('decision connections list')['services'] if v['service']['id'] == id)
            assert view['service'] == service
            tested = data('decision connections test', {'id': id, 'revision': 1,
                'expected_configuration_digest': view['configuration_digest']})
            assert tested['outcome'] == 'passed', tested
            assert Endpoint.requests[-1]['long_context'] is None, 'provider header leaked to loopback'
            expected_auth = 'Bearer ' + SENTINEL if codec == 'system_one' else token
            assert Endpoint.requests[-1]['authorization'] == expected_auth
            unused_id = id + '-unused'
            unused = data(f'decision connections apply --id {unused_id} --name Unused --provider compatible'
                f' --source {source_id} --model fixture --endpoint {endpoint} --wire-protocol {codec}'
                ' --context-window-tokens 32000')
            assert save_preview(unused, codec + '-unused-create')['schema'] == 'hiroute.decision-service/v3'
            delete_connection(unused_id, codec + '-unused-delete')
            assert all(v['service']['id'] != unused_id for v in data('decision connections list')['services'])
            if codec == 'system_one':
                publish_plan_and_agent(product, source['binding_id'], native_model_mode='hiroute_only', branch_service=service)
            else:
                publish(service, codec + '-publish')
            # The actual publication pins this V3 record and its shared consumers.
            delete_connection(id, codec + '-referenced-delete', referenced=True)
            assert next(v for v in data('decision connections list')['services'] if v['service']['id'] == id)['service'] == service
            before = len(Endpoint.requests)
            product.stop(); product.start()
            assert len(Endpoint.requests) == before, 'restart called provider'
            assert next(v for v in data('decision connections list')['services'] if v['service']['id'] == id)['service'] == service
            long_user = 'Current 中文 task: "quoted" \\ and newline\n' * 12000
            request([{'role': 'user', 'content': long_user}], codec + '-large')
            admitted = Endpoint.requests[-1]
            assert len(Endpoint.requests) == before + 1 and admitted['bytes'] > 262144, admitted['bytes']
            assert state(admitted['body'])['latest_user'][0]['text'] == long_user
            assert forwarded_text(upstream.requests[-1]['body']) == long_user, 'ordinary request was cropped'
            # Omitted edit flags retain intent; a new revision does not change the active route.
            edited = save_preview(data(command + ' --context-window-tokens 32000'), codec + '-edit')
            assert edited['context']['resolved']['request_window_tokens'] == 32000
            before = len(Endpoint.requests)
            request([{'role': 'user', 'content': long_user}], codec + '-still-pinned')
            assert len(Endpoint.requests) == before + 1, 'saving changed the active publication'
            publish(edited, codec + '-small-publish')
            before = len(Endpoint.requests)
            request([{'role': 'user', 'content': long_user}], codec + '-no-fit')
            assert len(Endpoint.requests) == before, 'mandatory no-fit called the provider'
            assert forwarded_text(upstream.requests[-1]['body']) == long_user, 'fallback changed current input'
            # Fresh short input still reaches the provider and a same-turn tool result reuses it.
            upstream.function_tools = True
            messages = [{'role': 'user', 'content': 'Read the local fixture.'}]
            before = len(Endpoint.requests)
            response = request(messages, codec + '-tools', tools=True)
            assert len(Endpoint.requests) == before + 1 and response['output'][0]['type'] == 'function_call'
            continuation = messages + response['output'] + [{'type': 'function_call_output', 'call_id': 'native-proxy-call', 'output': 'fixture complete'}]
            request(continuation, codec + '-tools', tools=True)
            assert len(Endpoint.requests) == before + 1, 'tool continuation reclassified the same user turn'
            upstream.function_tools = False
            assert_retained_budget_diagnostics()
        print(json.dumps({'scenario': 'adaptive-decision-context-real-cli-gateway', 'state': 'green',
            'cases': ['installed-provider-preset-contexts-without-save-or-provider',
                      'both-codecs', 'shared-and-independent', 'save-without-provider', 'exact-readback',
                      'test-saved-digest', 'restart', 'large-window-beyond-256KiB', 'current-input-exact',
                      'immutable-publication', 'mandatory-no-fit-zero-attempts', 'fallback-exact-input',
                      'same-turn-tool-reuse', 'no-header-on-compatible-endpoint',
                      'unreferenced-v3-deletion', 'referenced-v3-deletion-refused-with-credentials-intact',
                      'retained-safe-large-and-no-fit-diagnostics', 'stale-context-conflict-without-write']}), flush=True)
    finally:
        upstream.close()
        server.shutdown(); server.server_close(); worker.join(timeout=5)
        product.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]))
