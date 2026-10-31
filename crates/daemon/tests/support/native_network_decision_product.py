"""The real shared decision and generation calls retain one admitted network snapshot."""
import concurrent.futures
import json
import sys
import threading
from http.server import ThreadingHTTPServer
from urllib.parse import urlsplit

from decision_connections_v2_product import Endpoint
from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, control, gateway_request, publish_plan_and_agent, save_native_source
from native_network_product import Proxy, own_address, policy_view, raw_gateway, set_policy
from publication_product import Product, V1


class DecisionEndpoint(Endpoint):
    requests = []
    hold_next = False
    held = threading.Event()
    release = threading.Event()

    def do_POST(self):
        if type(self).hold_next:
            type(self).hold_next = False
            self.held.set()
            assert self.release.wait(12), 'held classifier was not released'
        super().do_POST()


def run(repository):
    product = Product(repository)
    address = own_address()
    upstream = NativeUpstream(base_host=address, bind_host='0.0.0.0')
    endpoint = ThreadingHTTPServer(('0.0.0.0', 0), DecisionEndpoint)
    endpoint_thread = threading.Thread(target=endpoint.serve_forever, daemon=True)
    endpoint_thread.start()
    decision_url = f'http://{address}:{endpoint.server_port}/v1/decisions'
    a, b = Proxy(upstream, 'decision-a'), Proxy(upstream, 'decision-b')
    for proxy in (a, b):
        proxy.destinations.add((address, endpoint.server_port))

    def data(command, payload=None):
        return product.public_cli(command, payload)[1]['data']

    def test(view, legacy=False):
        service = view['service']
        return data('decision connections test' if legacy else 'decision network test', {
            'id': service['id'], 'revision': service['revision'],
            'expected_configuration_digest': view['configuration_digest']})

    try:
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN, inventory_path=None)
        source_id = saved['source_id']
        source = next(row for row in data('decision sources list')['sources'] if row['source_id'] == source_id)
        service = {'schema': 'hiroute.decision-service/v2', 'id': 'decision/network', 'revision': 1,
            'name': 'Shared decision network', 'connection': {'kind': 'native', 'provider': 'openai',
                'wire_protocol': 'openai_decisions', 'model': 'decision-fixture-model',
                'endpoint': decision_url, 'timeout_ms': 15000,
                'authentication': {'kind': 'shared_source', 'source_id': source_id,
                                   'source_lineage_digest': source['lineage_digest']}}}
        change = {'id': service['id'], 'expected_revision': 0, 'service': service,
                  'expected_source_revisions': [{'source_id': source_id, 'revision': source['revision']}]}
        spec = {'schema_version': V1, 'command_id': 'decision.connections.apply',
                'resource_id': service['id'], 'desired_state': {'change': change}}
        preview = data('decision connections apply', {'schema_version': V1, 'spec': spec})
        applied = data('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'network-decision'})
        assert applied['state'] == 'succeeded' and not DecisionEndpoint.requests, applied
        view = next(row for row in data('decision connections list')['services'] if row['service']['id'] == service['id'])
        initial_credentials = view['credential_state_revision']
        network = set_policy(product, source_id, {'mode': 'manual', 'proxy_url': a.url, 'no_proxy': ''}, 'decision-proxy-a')
        assert network['decisions'] == [{'service_id': service['id'], 'revision': 1,
            'display_name': service['name'], 'model': service['connection']['model']}], network
        resolved = data('compute network resolve', {'target': {'kind': 'decision', 'id': service['id'], 'revision': 1}, 'policy': None})
        assert resolved['route'] == 'proxy' and resolved['destination'].endswith(':' + str(endpoint.server_port)), resolved
        assert not DecisionEndpoint.requests and not a.requests, 'configuration reads contacted a provider'
        checked = test(view)
        assert checked['check']['outcome'] == 'passed' and checked['failure'] is None, checked
        assert checked['network']['expected_network_revision'] == network['network_revision'], checked
        assert checked['network']['route']['state_revision'] == resolved['state_revision'], checked
        assert checked['check']['tested_credential_state_revision'] == initial_credentials, checked
        assert a.requests[-1] == ('POST', '/v1/decisions') and len(DecisionEndpoint.requests) == 1
        legacy = test(view, legacy=True)
        assert legacy['outcome'] == 'passed' and 'network' not in legacy and 'check' not in legacy, legacy
        publish_plan_and_agent(product, saved['binding_id'], native_model_mode='hiroute_only', branch_service=service)
        head = product.cli('routing show ' + product.plan_id)[1]['data']['head']
        # Hold the decision leg of an admitted HTTP request. New heads use B,
        # while both legs of the old request keep A, even after B opens its pool.
        DecisionEndpoint.hold_next = True
        before_a, before_b = len(a.requests), len(b.requests)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            old = executor.submit(raw_gateway, product)
            assert DecisionEndpoint.held.wait(8), 'classification did not use the actual decision destination'
            set_policy(product, source_id, {'mode': 'manual', 'proxy_url': b.url, 'no_proxy': ''}, 'decision-proxy-b')
            gateway_request(product)
            assert b.requests[before_b:] == [('POST', '/v1/decisions'), ('POST', '/v1/responses')], b.requests
            DecisionEndpoint.release.set()
            status, body = old.result(timeout=15)
            assert status == 200 and b'native product answer' in body, (status, body)
        assert a.requests[before_a:] == [('POST', '/v1/decisions'), ('POST', '/v1/responses')], a.requests
        assert product.cli('routing show ' + product.plan_id)[1]['data']['head'] == head
        # A port-specific bypass distinguishes the decision URL from ordinary generation.
        set_policy(product, source_id, {'mode': 'manual', 'proxy_url': b.url,
                   'no_proxy': f'{address}:{endpoint.server_port}'}, 'decision-only-bypass')
        after = data('compute network resolve', {'target': {'kind': 'decision', 'id': service['id'], 'revision': 1}, 'policy': None})
        assert after['route'] == 'direct' and after['reason'] == 'bypass_matched', after
        assert after['state_revision'] != checked['network']['route']['state_revision']
        before = len(b.requests)
        gateway_request(product)
        assert b.requests[before:] == [('POST', '/v1/responses')], b.requests
        bypassed = test(view)
        assert bypassed['check']['outcome'] == 'passed' and bypassed['network']['route']['route'] == 'direct', bypassed
        assert bypassed['check']['tested_credential_state_revision'] == initial_credentials
        # Explicit and production calls never interpret proxy 407 as provider Key rejection.
        set_policy(product, source_id, {'mode': 'manual', 'proxy_url': b.url, 'no_proxy': ''}, 'decision-auth-rejection')
        b.reject = True
        count = len(DecisionEndpoint.requests)
        failed = test(view)
        assert failed['check']['failure_code'] == 'DECISION_TRANSPORT_UNAVAILABLE', failed
        assert failed['failure']['phase'] == 'proxy_auth' and len(DecisionEndpoint.requests) == count, failed
        assert raw_gateway(product)[0] >= 500 and len(DecisionEndpoint.requests) == count
        keys = next(row for row in control(product, 'ListCompute', {})['data']['sources'] if row['source_id'] == source_id)['keys']
        assert keys and all(key['enabled'] for key in keys), keys
        print(json.dumps({'state': 'green', 'scenario': 'native-shared-decision-network',
            'actual_decision_destination': True, 'bounded_preview_without_provider_io': True,
            'whole_http_head_retains_network_snapshot': True, 'decision_only_bypass': True,
            'versioned_admitted_test_evidence': True, 'proxy_auth_preserves_provider_key': True}))
    finally:
        DecisionEndpoint.release.set()
        product.close()
        a.close(); b.close(); upstream.close()
        endpoint.shutdown(); endpoint.server_close(); endpoint_thread.join(timeout=5)


if __name__ == '__main__':
    run(sys.argv[1])
