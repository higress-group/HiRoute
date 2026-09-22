"""Real Gateway protocol usage -> Observation home_value, with optional Pilot data seeding.

Only synthetic Agent files and a loopback upstream are used. The daemon, Gateway,
Observation writer, query, CLI and protected grants are production entry paths.
"""
import hashlib
import http.client
import http.server
import json
from pathlib import Path
import subprocess
import sys
import threading
import time

from model_connections_product import MODEL, NATIVE_TOKEN, save_native_source
from publication_process import configure_model_settings_v2
from publication_product import Product, encoded


PROTOCOLS = ('responses', 'messages')
PATHS = {
    'responses': '/v1/responses',
    'messages': '/v1/messages',
}
USAGE = {
    ('responses', False): (11, 7, 3, 0, 2),
    ('responses', True): (13, 5, 4, 0, 3),
    ('messages', False): (14, 9, 5, 2, None),
    ('messages', True): (15, 5, 3, 4, None),
}


def provider_usage(protocol, streaming):
    total_input, output, read, write, reasoning = USAGE[(protocol, streaming)]
    if protocol == 'responses':
        return {'input_tokens': total_input, 'output_tokens': output,
                'total_tokens': total_input + output,
                'input_tokens_details': {'cached_tokens': read},
                'output_tokens_details': {'reasoning_tokens': reasoning}}
    return {'input_tokens': total_input - read - write,
            'cache_read_input_tokens': read,
            'cache_creation_input_tokens': write, 'output_tokens': output}


def json_event(event, value):
    return ('event: ' + event + '\ndata: ' + encoded(value).decode() + '\n\n').encode()


def stream_body(protocol):
    usage = provider_usage(protocol, True)
    if protocol == 'responses':
        complete = response_body(protocol, True)
        # CPA repeats request fields in response.completed. Keep this event
        # above the accepted response plan's 64 KiB transport frame limit.
        complete['instructions'] = 'i' * 100_000
        return (json_event('response.created', {
            'type': 'response.created', 'response': {
                'id': 'usage-stream', 'model': MODEL, 'status': 'in_progress'}})
            + json_event('response.output_text.delta', {
                'type': 'response.output_text.delta', 'item_id': 'message',
                'output_index': 0, 'content_index': 0, 'delta': 'usage-ok'})
            + json_event('response.completed', {
                'type': 'response.completed', 'response': complete}))
    start_usage = {key: value for key, value in usage.items()
                   if key != 'output_tokens'}
    return (json_event('message_start', {'type': 'message_start', 'message': {
        'id': 'usage-stream', 'type': 'message', 'role': 'assistant',
        'model': MODEL, 'content': [], 'stop_reason': None,
        'usage': start_usage}})
        + json_event('content_block_start', {'type': 'content_block_start',
            'index': 0, 'content_block': {'type': 'text', 'text': ''}})
        + json_event('content_block_delta', {'type': 'content_block_delta',
            'index': 0, 'delta': {'type': 'text_delta', 'text': 'usage-ok'}})
        + json_event('content_block_stop', {'type': 'content_block_stop', 'index': 0})
        + json_event('message_delta', {'type': 'message_delta',
            'delta': {'stop_reason': 'end_turn', 'stop_sequence': None},
            'usage': {'output_tokens': usage['output_tokens']}})
        + json_event('message_stop', {'type': 'message_stop'}))


def response_body(protocol, streaming):
    common = {'id': 'usage-stream' if streaming else 'usage-complete',
              'model': MODEL, 'usage': provider_usage(protocol, streaming)}
    if protocol == 'responses':
        return dict(common, status='completed', output=[{
            'type': 'message', 'role': 'assistant', 'content': [{
                'type': 'output_text', 'text': 'usage-ok'}]}])
    return dict(common, type='message', role='assistant', content=[{
        'type': 'text', 'text': 'usage-ok'}], stop_reason='end_turn',
        stop_sequence=None)


class UsageUpstream:
    def __init__(self):
        self.requests = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def do_GET(self):
                assert self.path == '/v1/models', self.path
                self.send_response(404)
                self.send_header('Content-Length', '0')
                self.send_header('Connection', 'close')
                self.end_headers()

            def do_POST(self):
                protocol = next((name for name, path in PATHS.items()
                                 if path == self.path), None)
                assert protocol is not None, self.path
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert body['model'] == MODEL, body
                assert self.headers.get('Authorization') == 'Bearer ' + NATIVE_TOKEN
                streaming = body.get('stream') is True
                owner.requests.append((protocol, streaming))
                payload = (stream_body(protocol) if streaming else
                           encoded(response_body(protocol, False)))
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream' if streaming
                                 else 'application/json')
                self.send_header('Content-Length', str(len(payload)))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(payload)

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = 'http://%s:%d/v1' % self.server.server_address

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def publish_plan(product, binding_id, protocol):
    product.editor = {
        'schema': 'hiroute.plan-editor/v2',
        'display_name': 'Usage ' + protocol,
        'purpose': 'Exercise native usage through the product Gateway',
        'mode': 'fixed_model', 'candidates': [{'binding_id': binding_id}],
        'smart': {'economy': [], 'primary': [], 'primary_fallback': False,
                  'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False, 'requirements': {},
        'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000,
                   'attempt_timeout_ms': 30000},
    }
    change = {'schema': 'hiroute.plan-content-change/v2',
              'target': {'intent': 'create', 'creation_key': 'usage-' + protocol},
              'editor': product.editor, 'consumed_draft': None}
    preview = product.preview('routing preview', {'change': change})
    product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                  {'change': change}, 'usage-plan-' + protocol)
    product.plan_id = preview['plan_head']['reference']['plan_id']
    return product.plan_id, preview['plan_head']['model_alias']


def gateway_request(product, protocol, alias, streaming, token):
    if protocol == 'responses':
        body = {'model': alias, 'input': 'usage fixture', 'stream': streaming}
    else:
        body = {'model': alias, 'messages': [{
            'role': 'user', 'content': 'usage fixture'}], 'stream': streaming}
        if protocol == 'messages':
            body['max_tokens'] = 32
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
    try:
        headers = {'Content-Type': 'application/json',
                   'session-id': 'usage-' + protocol
                   + ('-stream' if streaming else '-full')}
        if protocol == 'messages':
            headers['Authorization'] = 'Bearer ' + token
        else:
            headers['X-HiRoute-Token'] = token
        client.request('POST', PATHS[protocol], body=encoded(body), headers=headers)
        response = client.getresponse()
        payload = response.read()
        product.outputs.append(payload)
        assert response.status == 200 and b'usage-ok' in payload, (
            protocol, streaming, response.status, payload)
        if protocol == 'responses' and streaming:
            assert payload.count(b'event: response.completed\n') == 1, (
                'large native terminal was truncated', len(payload))
            assert payload.endswith(b'\n\n'), 'large native terminal lacks SSE boundary'
    finally:
        client.close()


def assert_home_value(product):
    query = {'schema': 'hiroute.observation.query/v2',
             'intent': {'view': 'home_value', 'query': {
                 'period': 'seven_days', 'session_id': None, 'currency': None}}}
    revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
    capability = product.grant('GetValueV2', {
        'change_digest': 'sha256:' + hashlib.sha256(encoded(query)).hexdigest(),
        'expected_revisions': revisions}, 'usage-home-value')
    deadline = time.monotonic() + 30
    while True:
        values = product.cli('value show', query, capability)[1]['data']
        usage = {item['metric']: item for item in values['usage']}
        hit = values['input_cache_hit']
        if (values['pending_requests'] == 0
                and values['provisional_requests'] == 0
                and usage['input']['known_sum'] == 53
                and usage['output']['known_sum'] == 26
                and usage['cache_read']['known_sum'] == 15
                and usage['cache_write']['known_sum'] == 6
                and usage['reasoning']['known_sum'] == 5
                and hit['state'] == 'available'
                and hit['ratio_basis_points'] == 2830
                and hit['eligible_attempt_count'] == 4
                and hit['cache_read_tokens'] == 15
                and hit['total_input_tokens'] == 53):
            assert values['excluded_requests'] == 0, values
            assert not values['retention_boundary_partial'], values
            assert usage['cache_write']['coverage'] == 'partial', values
            assert usage['cache_write']['missing_attempt_count'] == 2, values
            assert usage['reasoning']['coverage'] == 'partial', values
            assert usage['reasoning']['missing_attempt_count'] == 2, values
            return values
        assert time.monotonic() < deadline, values
        time.sleep(.05)


def assert_plan_usage(product, plans, started_ms):
    revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
    for protocol, plan_id in plans.items():
        cases = [facts for (name, _), facts in USAGE.items() if name == protocol]
        input_tokens = sum(facts[0] for facts in cases)
        output_tokens = sum(facts[1] for facts in cases)
        read = sum(facts[2] for facts in cases)
        write = sum(facts[3] for facts in cases)
        reasoning = sum(facts[4] or 0 for facts in cases)
        query = {'schema': 'hiroute.observation.query/v2',
                 'intent': {'view': 'value', 'query': {
                     'from_ms': started_ms - 1000,
                     'to_ms': int(time.time() * 1000) + 60000,
                     'session_id': None, 'plan_id': plan_id,
                     'currency': None}}}
        capability = product.grant('GetValueV2', {
            'change_digest': 'sha256:' + hashlib.sha256(encoded(query)).hexdigest(),
            'expected_revisions': revisions}, 'usage-value-plan-' + protocol)
        deadline = time.monotonic() + 30
        while True:
            values = product.cli('value show', query, capability)[1]['data']
            usage = {item['metric']: item for item in values['usage']}
            hit = values['input_cache_hit']
            if (usage['input']['known_sum'] == input_tokens
                    and usage['output']['known_sum'] == output_tokens
                    and usage['cache_read']['known_sum'] == read
                    and usage['cache_write']['known_sum'] == (write if write else None)
                    and usage['reasoning']['known_sum'] ==
                    (reasoning if reasoning else None)
                    and hit['ratio_basis_points'] ==
                    (read * 10000 + input_tokens // 2) // input_tokens
                    and hit['eligible_attempt_count'] == 2):
                assert not values['retention_boundary_partial'], values
                break
            assert time.monotonic() < deadline, (protocol, values)
            time.sleep(.05)


def run(repository, expected_sha=None, desktop_data_root=None):
    repository = Path(repository).resolve()
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository,
                                  text=True).strip()
    if expected_sha is not None:
        assert sha == expected_sha, (sha, expected_sha)
    data_root = None
    if desktop_data_root is not None:
        # The caller supplies a newly created, offline Pilot test root. Never
        # point this fixture at a user's normal HiRoute data directory.
        data_root = Path(desktop_data_root).resolve(strict=True)
        assert data_root.name == 'data' and (data_root.parent / 'session.json').is_file()
        # Keep this synthetic Agent root at its exact path after daemon exit:
        # managed-artifact evidence in the same storage is path-bound.
        product = Product(repository, root=data_root / 'seed-agent')
        product.storage = data_root / 'storage'
        product.storage.mkdir(mode=0o700, exist_ok=True)
    else:
        product = Product(repository)
    upstream = UsageUpstream()
    product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
    started_ms = int(time.time() * 1000)
    try:
        product.start()
        plans = {}
        for protocol in PROTOCOLS:
            saved = save_native_source(product, upstream, NATIVE_TOKEN,
                                       protocol=protocol, variant=protocol,
                                       # Claude requires at least 100K after output reservation.
                                       context_tokens=131072 if protocol == 'messages' else 32768)
            plan_id, alias = publish_plan(product, saved['binding_id'], protocol)
            plans[protocol] = plan_id
            agent = ('agent_codex_default' if protocol == 'responses'
                     else 'agent_claude_default')
            if protocol == 'messages':
                product.agent_context_id = None
            configure_model_settings_v2(product, [plan_id], 'usage-agent-' + protocol,
                                        agent_id=agent)
            token = product.bearer(product.agent_connection)
            for streaming in (False, True):
                gateway_request(product, protocol, alias, streaming, token)
        values = assert_home_value(product)
        assert_plan_usage(product, plans, started_ms)
        assert sorted(upstream.requests) == sorted(USAGE), upstream.requests
        print(json.dumps({'scenario': 'gateway-native-usage-to-home-value',
                          'state': 'green', 'candidate': sha, 'requests': 4,
                          'input_tokens': 53, 'output_tokens': 26,
                          'cache_read_tokens': 15, 'cache_write_tokens': 6,
                          'cache_hit_basis_points': 2830,
                          'retention_boundary_partial': values['retention_boundary_partial'],
                          'desktop_data_root': str(data_root) if data_root else None,
                          'pilot_process_home': str(product.root / 'home') if data_root else None
                          }), flush=True)
    finally:
        upstream.close()
        product.close()


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None,
        sys.argv[3] if len(sys.argv) > 3 else None)
