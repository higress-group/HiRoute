"""Actual source policy CLI save/check -> Gateway, live SSE snapshot and restart.

All listeners belong to this fixture. The origin uses this host's non-loopback
address so the product's mandatory loopback bypass remains intact.
"""
import base64
import concurrent.futures
import hashlib
import http.client
import http.server
import ipaddress
import json
import socket
import sys
import threading
from urllib.parse import urlsplit

from model_connections_product import (
    MODEL, NATIVE_TOKEN, NativeUpstream, control, desktop_grant,
    gateway_request, publish_plan_and_agent, save_native_source,
)
from publication_product import Product, encoded


def own_address():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        # UDP connect selects the local interface without sending a packet.
        probe.connect(('192.0.2.1', 9))
        address = probe.getsockname()[0]
    assert not ipaddress.ip_address(address).is_loopback, 'a local non-loopback interface is required'
    return address


def event(kind, **value):
    return b'event: ' + kind.encode() + b'\ndata: ' + encoded(dict(type=kind, **value)) + b'\n\n'


class Proxy:
    def __init__(self, upstream, name):
        self.requests = []
        self.reject = False
        self.held = threading.Event()
        self.release = threading.Event()
        self.auth = None
        owner = self
        origin = urlsplit(upstream.base_url)
        self.destinations = {(origin.hostname, origin.port)}

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def do_GET(self):
                self.forward()

            def do_POST(self):
                self.forward()

            def forward(self):
                uri = urlsplit(self.path)
                assert uri.scheme == 'http' and (uri.hostname, uri.port) in owner.destinations, self.path
                assert self.headers.get('Authorization') == 'Bearer ' + NATIVE_TOKEN
                assert self.headers.get('Proxy-Authorization') == owner.auth
                body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                owner.requests.append((self.command, uri.path))
                if owner.reject:
                    self.send_response(407)
                    self.send_header('Content-Length', '0')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    return
                request = json.loads(body) if body else {}
                if request.get('stream'):
                    response = {'id': 'native-proxy-stream', 'object': 'response', 'status': 'completed',
                                'model': MODEL, 'output': [{'id': 'message', 'type': 'message',
                                'role': 'assistant', 'status': 'completed', 'content': [
                                    {'type': 'output_text', 'text': name + ' completed', 'annotations': []}]}],
                                'usage': {'input_tokens': 4, 'output_tokens': 3, 'total_tokens': 7}}
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.wfile.write(event('response.created', response=dict(response, status='in_progress', output=[])))
                    self.wfile.write(event('response.output_text.delta', item_id='message', output_index=0,
                                           content_index=0, delta=name + ' started'))
                    self.wfile.flush()
                    owner.held.set()
                    assert owner.release.wait(15), 'held SSE was not released'
                    self.wfile.write(event('response.completed', response=response))
                    self.wfile.flush()
                    self.close_connection = True
                    return
                headers = {key: value for key, value in self.headers.items()
                           if key.lower() not in ('proxy-authorization', 'connection', 'proxy-connection')}
                connection = http.client.HTTPConnection(uri.hostname, uri.port, timeout=10)
                try:
                    connection.request(self.command, uri.path, body=body, headers=headers)
                    result = connection.getresponse()
                    payload = result.read()
                    self.send_response(result.status)
                    self.send_header('Content-Type', result.getheader('Content-Type', 'application/json'))
                    self.send_header('Content-Length', str(len(payload)))
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.wfile.write(payload)
                finally:
                    connection.close()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = 'http://127.0.0.1:' + str(self.server.server_port)

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def policy_view(product, source_id):
    return product.public_cli('compute network show', {'source_id': source_id})[1]['data']


def set_policy(product, source_id, policy, key):
    before = policy_view(product, source_id)
    request = {'source_id': source_id, 'expected_source_revision': before['source_revision'],
               'network_edit': {'expected_network_revision': before['network_revision'], 'policy': policy}}
    preview = product.public_cli('compute network preview', request)[1]['data']
    assert preview['before'] == before, preview
    saved = product.public_cli('compute network apply', {
        'spec': preview['preview']['spec'], 'accept_digest': preview['preview']['accept_digest'],
        'expected_revisions': preview['preview']['expected_revisions'], 'idempotency_key': key})[1]
    assert saved['data']['state'] == 'succeeded', saved
    after = policy_view(product, source_id)
    assert after['policy']['mode'] == policy['mode'], after
    return after


def check_saved(product, view, key):
    request = {'request': {'source_id': view['source_id'], 'expected_source_revision': view['source_revision'],
                          'edit_revision': 1, 'check_id': 'network-check-' + key},
               'network_edit': None}
    revisions = control(product, 'GetClientServiceStatus', {})['data']['revisions']
    grant = desktop_grant(product, 'CheckSavedModelConnectionV2',
                          'sha256:' + hashlib.sha256(encoded(request)).hexdigest(), revisions, key)
    return control(product, 'CheckSavedModelConnectionV2', request, grant)['data']


def raw_gateway(product, stream=False, input_value='proxy snapshot test', tools=None):
    connection = http.client.HTTPConnection('127.0.0.1', product.port, timeout=20)
    try:
        request = {'model': product.model_alias, 'input': input_value, 'stream': stream}
        if tools is not None:
            request['tools'] = tools
        connection.request('POST', '/v1/responses', body=encoded(request), headers={
            'X-HiRoute-Token': product.bearer(product.agent_connection), 'Content-Type': 'application/json'})
        response = connection.getresponse()
        body = response.read()
        product.outputs.append(body)
        return response.status, body
    finally:
        connection.close()


def run(repository):
    product = Product(repository)
    upstream = NativeUpstream(base_host=own_address(), bind_host='0.0.0.0')
    a, b = Proxy(upstream, 'proxy-a'), Proxy(upstream, 'proxy-b')
    try:
        # A malformed inherited proxy cannot affect the default Direct source.
        for name in ('HTTP_PROXY', 'http_proxy', 'HTTPS_PROXY', 'https_proxy', 'NO_PROXY', 'no_proxy'):
            product.env.pop(name, None)
        product.env['HTTP_PROXY'] = 'socks5://invalid-proxy.invalid'
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN, inventory_path=None)
        publish_plan_and_agent(product, saved['binding_id'])
        source_id = saved['source_id']
        initial = policy_view(product, source_id)
        assert initial['policy'] == {'mode': 'direct'} and initial['network_revision'] == 1, initial
        gateway_request(product)
        assert not a.requests and not b.requests
        head = product.cli('routing show ' + product.plan_id)[1]['data']['head']
        prior_requests = len(upstream.requests)
        manual_a = {'mode': 'manual', 'proxy_url': a.url, 'no_proxy': ''}
        view = set_policy(product, source_id, manual_a, 'proxy-a')
        assert len(upstream.requests) == prior_requests and not a.requests, 'configuration save performed provider I/O'
        assert view['application_state'] == 'applied' and view['applied_revision'] == 2, view
        gateway_request(product)
        assert len(a.requests) == 1 and not b.requests
        # An admitted SSE retains A while subsequent requests use B.
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            old = executor.submit(raw_gateway, product, True)
            assert a.held.wait(10), 'SSE did not reach the selected proxy'
            view = set_policy(product, source_id, {'mode': 'manual', 'proxy_url': b.url, 'no_proxy': ''}, 'proxy-b')
            gateway_request(product)
            assert len(b.requests) == 1
            a.release.set()
            status, body = old.result(timeout=15)
            assert status == 200 and b'proxy-a completed' in body and b'proxy-b' not in body, (status, body)
        assert product.cli('routing show ' + product.plan_id)[1]['data']['head'] == head, 'network edit republished the Plan'
        # Both HTTP requests of a real tool round trip retain provider authority
        # and the selected proxy; tool output reaches the original origin intact.
        upstream.function_tools = True
        tools = [{'type': 'function', 'name': 'native_probe', 'description': 'Return local fixture data',
                  'parameters': {'type': 'object', 'properties': {}, 'additionalProperties': False},
                  'strict': True}]
        inputs = [{'role': 'user', 'content': 'Run the native fixture probe'}]
        before_origin, before_b = len(upstream.requests), len(b.requests)
        status, body = raw_gateway(product, input_value=inputs, tools=tools)
        assert status == 200, (status, body)
        output = json.loads(body)['output']
        call, = [item for item in output if item['type'] == 'function_call']
        assert call['name'] == 'native_probe' and json.loads(call['arguments']) == {}, call
        tool_result = {'type': 'function_call_output', 'call_id': call['call_id'], 'output': 'local fixture result'}
        status, body = raw_gateway(product, input_value=inputs + output + [tool_result], tools=tools)
        assert status == 200 and b'native product answer' in body, (status, body)
        assert len(b.requests) == before_b + 2 and len(upstream.requests) == before_origin + 2
        assert tool_result in upstream.requests[-1]['body']['input'], upstream.requests[-1]['body']
        upstream.function_tools = False
        # A 407 cannot become Key rejection, disable the saved key or retry directly.
        b.reject = True
        before_origin, before_b = len(upstream.requests), len(b.requests)
        status, _ = raw_gateway(product)
        assert status >= 500 and len(upstream.requests) == before_origin and len(b.requests) > before_b
        keys = next(source for source in control(product, 'ListCompute', {})['data']['sources'] if source['source_id'] == source_id)['keys']
        assert keys and all(key['enabled'] for key in keys), keys
        # Restore B; bypass rules apply to the actual destination and avoid B.
        b.reject = False
        view = set_policy(product, source_id, {'mode': 'manual', 'proxy_url': b.url, 'no_proxy': urlsplit(upstream.base_url).hostname}, 'proxy-b-bypass')
        count = len(b.requests)
        gateway_request(product)
        assert len(b.requests) == count
        view = set_policy(product, source_id, {'mode': 'inherit'}, 'inherit-invalid')
        assert view['application_state'] == 'unavailable' and view['applied_revision'] is None, view
        count = len(upstream.requests)
        assert raw_gateway(product)[0] >= 500
        assert len(upstream.requests) == count, 'invalid inherit fell back to direct'
        view = set_policy(product, source_id, manual_a, 'restart-manual')
        revision = view['network_revision']
        product.stop(); product.start()
        view = policy_view(product, source_id)
        assert view['network_revision'] == revision and view['policy']['mode'] == 'manual', view
        gateway_request(product)
        # Inherit captures the service environment, not changes in a later CLI shell.
        product.stop()
        product.env['HTTP_PROXY'] = b.url.replace('http://', 'http://proxy:secret@')
        product.secrets.add('proxy:secret')
        b.auth = 'Basic ' + base64.b64encode(b'proxy:secret').decode()
        product.start()
        view = set_policy(product, source_id, {'mode': 'inherit'}, 'inherit-valid')
        product.env['HTTP_PROXY'] = 'socks5://client-shell.invalid'
        gateway_request(product)
        assert b.requests
        # No inventory is configured: this check is route evidence, not a provider claim.
        checked = check_saved(product, view, 'saved-route')
        assert checked['network']['route']['route'] == 'proxy', checked
        assert checked['check']['reachability'] == 'not_run' and checked['check']['authentication'] != 'verified', checked
        assert 'proxy:secret' not in json.dumps(checked)
        # A new connection can check through its chosen proxy before its first save,
        # despite an invalid client environment. Equal targets/Keys remain separate sources.
        before_a, before_b = len(a.requests), len(b.requests)
        manual_draft = dict(manual_a, proxy_url=a.url.rstrip('/') + '/')
        second = save_native_source(product, upstream, NATIVE_TOKEN, variant='proxy-draft',
            cli_input=True, source_display_name='Proxy draft API', network_policy=manual_draft)
        assert second['source_id'] != source_id and len(a.requests) == before_a + 1
        assert policy_view(product, second['source_id'])['policy'] == manual_draft
        assert policy_view(product, source_id)['policy'] == {'mode': 'inherit'}
        gateway_request(product)
        assert len(a.requests) == before_a + 1 and len(b.requests) == before_b + 1
        # The released probe remains strict; the public V2 probe reports its admitted route.
        second_view = policy_view(product, second['source_id'])
        public = product.public_cli('compute connection test-network', {'kind': 'saved', 'request': {
            'request': {'source_id': second['source_id'], 'expected_source_revision': second_view['source_revision'],
                        'candidate_ref': None, 'edit_revision': 2, 'check_id': 'network-public-saved'},
            'network_edit': None}})[1]['data']
        assert public['network']['route']['route'] == 'proxy' and public['failure']['phase'] == 'directory', public
        # The published fallback order keeps each source's own policy. An
        # unavailable first source must not block a separate admitted source.
        product.editor['candidates'].append({'binding_id': second['binding_id']})
        change = {'schema': 'hiroute.plan-content-change/v2',
                  'target': {'intent': 'update', 'plan_id': product.plan_id,
                             'expected_head_revision': head['head_revision']},
                  'editor': product.editor, 'consumed_draft': None}
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                      {'change': change}, 'native-network-fallback-plan')
        product.stop(); product.start()
        assert policy_view(product, source_id)['application_state'] == 'unavailable'
        before_origin, before_a, before_b = len(upstream.requests), len(a.requests), len(b.requests)
        status, body = raw_gateway(product)
        assert status == 200, ('unavailable source prevented admitted fallback', status, body)
        assert len(a.requests) == before_a + 1 and len(b.requests) == before_b
        assert len(upstream.requests) == before_origin + 1, 'fallback bypassed the second source proxy'
        # A rejected proxy also advances to the next source, never directly to
        # the first source's provider or to another provider Key.
        product.stop()
        product.env['HTTP_PROXY'] = b.url.replace('http://', 'http://proxy:secret@')
        b.reject = True
        product.start()
        before_origin, before_a, before_b = len(upstream.requests), len(a.requests), len(b.requests)
        status, body = raw_gateway(product)
        assert status == 200, ('proxy rejection prevented admitted fallback', status, body)
        assert len(b.requests) == before_b + 1 and len(a.requests) == before_a + 1
        assert len(upstream.requests) == before_origin + 1, 'proxy rejection retried the provider directly'
        keys = next(source for source in control(product, 'ListCompute', {})['data']['sources']
                    if source['source_id'] == source_id)['keys']
        assert keys and all(key['enabled'] for key in keys), keys
        print(json.dumps({'state': 'green', 'scenario': 'native-network-policy',
                          'cli_save_and_get': True, 'draft_check_before_first_save': True, 'two_source_scope': True, 'gateway_proxy_and_bypass': True,
                          'hot_apply_and_sse_snapshot': True, 'no_direct_fallback_or_key_disable': True,
                          'tool_call_and_continuation_through_proxy': True,
                          'unavailable_and_proxy_auth_fallback_use_next_source_policy': True,
                          'restart_and_service_environment_capture': True}))
    finally:
        a.release.set(); b.release.set()
        product.close()
        a.close(); b.close(); upstream.close()


if __name__ == '__main__':
    run(sys.argv[1])
