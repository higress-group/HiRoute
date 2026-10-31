"""Real source admission, Plan save/reopen, Gateway attempts and observation ingestion."""
import http.client
import json
import sqlite3
import sys
import time

from model_connections_product import NativeUpstream, save_native_source
from plan_content_process import read_content
from publication_process import bootstrap, configure_model_settings_v2, plan_change
from publication_product import Product, encoded


def run(repository):
    product = Product(repository)
    upstreams = [NativeUpstream() for _ in range(3)]
    try:
        bootstrap(product)
        previous_id = product.plan_id
        previous = read_content(product)
        bindings = []
        for index, (upstream, levels, default) in enumerate(zip(
                upstreams, [('low', 'xhigh'), ('medium', 'high'), ('high', 'xhigh')],
                ['low', 'medium', 'high'])):
            saved = save_native_source(product, upstream, variant=f'reasoning-{index}',
                native_reasoning={'kind': 'discrete', 'parameter': 'reasoning_effort',
                                  'profiles': list(levels), 'default_profile': default})
            bindings.append({'binding_id': saved['binding_id'],
                             'reasoning': {'kind': 'profile', 'profile': default}})
        product.editor = dict(product.editor, display_name='Request reasoning',
            follow_request_reasoning=True, candidates=bindings,
            limits=dict(product.editor['limits'], maximum_attempts=3))
        change = plan_change(product, 'create', 'follow-reasoning-product')
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                      {'change': change}, 'save-follow-reasoning')
        product.plan_id = preview['plan_head']['reference']['plan_id']
        product.model_alias = preview['plan_head']['model_alias']
        configure_model_settings_v2(product, [product.plan_id], 'follow-reasoning-agent',
                                    agent_id='agent_claude_default')
        frozen = read_content(product)
        assert frozen[1][-1]['configuration']['follow_request_reasoning'] is True
        assert frozen[1][-1]['configuration']['strategy']['candidates'] == bindings
        product.stop()
        product.start()
        assert read_content(product) == frozen
        status = product.cli('routing show ' + product.plan_id)[1]['data']
        assert status['desired']['follow_request_reasoning'] is True
        assert status['desired']['strategy']['candidates'] == bindings

        def request(effort=None):
            body = {'model': product.model_alias, 'input': 'Reply OK', 'max_output_tokens': 128}
            if effort is not None:
                body['reasoning'] = {'effort': effort}
            connection = http.client.HTTPConnection('127.0.0.1', product.port, timeout=15)
            connection.request('POST', '/v1/responses', encoded(body), {
                'Authorization': 'Bearer ' + product.bearer(), 'Content-Type': 'application/json',
                'session-id': 'follow-reasoning-product'})
            response = connection.getresponse()
            payload = response.read()
            connection.close()
            return response.status, payload

        upstreams[0].rejected_status = 503
        upstreams[1].rejected_status = 503
        code, body = request('xhigh')
        assert code == 200, (code, body)
        attempts = [next(r for r in u.requests if r['method'] == 'POST') for u in upstreams]
        assert [r['body']['reasoning']['effort'] for r in attempts] == ['xhigh', 'medium', 'xhigh']
        for value in [None, 'unsupported-future-level']:
            before = len(upstreams[2].requests)
            assert request(value)[0] == 200
            assert len(upstreams[2].requests) == before + 1
            assert upstreams[2].requests[-1]['body']['reasoning']['effort'] == 'high'
        before = [len(u.requests) for u in upstreams]
        assert request(42)[0] == 400
        assert before == [len(u.requests) for u in upstreams]

        # Reopen and deliberately switch off. Existing fallbacks remain exact.
        change = plan_change(product, 'update', follow_request_reasoning=False)
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                      {'change': change}, 'save-fixed-reasoning')
        assert request('xhigh')[0] == 200
        assert upstreams[2].requests[-1]['body']['reasoning']['effort'] == 'high'
        paths = list(product.storage.rglob('activity.db'))
        assert len(paths) == 1, paths
        deadline = time.monotonic() + 15
        while True:
            with sqlite3.connect(paths[0].as_uri() + '?mode=ro', uri=True) as db:
                facts = [json.loads(row[0]) for row in db.execute('SELECT envelope_json FROM execution_fact_events')]
            facts = [f['fact'] for f in facts if f['trust']['served_model_id'] == product.model_alias]
            resolutions = {f.get('reasoning_resolution') for f in facts if f.get('kind') == 'candidate_decision'}
            if {'request', 'default_missing', 'default_unsupported'} <= resolutions:
                break
            assert time.monotonic() < deadline, ('missing actual reasoning receipts', resolutions)
            time.sleep(.05)
        assert any(f.get('requested_reasoning_disposition') == 'follow_request_with_default' for f in facts)
        product.plan_id = previous_id
        assert read_content(product) == previous
        print(json.dumps({'scenario': 'follow-request-reasoning', 'state': 'green',
            'fallback_efforts': ['xhigh', 'medium', 'xhigh'], 'saved_defaults_preserved': True,
            'restart_preserved': True, 'malformed_rejected_without_upstream': True,
            'existing_route_unchanged': True}), flush=True)
    finally:
        product.close()
        for upstream in upstreams:
            upstream.close()


if __name__ == '__main__':
    run(sys.argv[1])
