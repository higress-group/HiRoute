"""Real role=all content persistence failure; only synthetic loopback providers.

The old environment file collector is deliberately disabled. Its fail_once variable
must not affect Product storage, and the feature-only Product hook must actually fail
one validated content delivery. This is not installed-native or live-model acceptance.
"""
import http.client
import json
from pathlib import Path
import sqlite3
import sys
import time

from additional_native_history_product import native_decision_receipt, assert_assessed_execution
from decision_services_product import DecisionEndpoint, native_history_decisions
from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, save_native_source
from publication_process import configure_model_settings_v2
from publication_product import Product, encoded
from usage_query_gateway_product import publish_plan


class FaultDecisions(DecisionEndpoint):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert set(body) == {'route', 'latest_user', 'visible_conversation',
                             'history_partial', 'preceding_stage'}
        self.requests.append((self.headers.get('Authorization'), body))
        latest = json.dumps(body['latest_user'])
        result = {'category': 'fallback' if 'fault-trigger' in latest else 'native', 'group': 'regular'}
        if (isinstance(body['preceding_stage'], dict)
                and body['preceding_stage']['category'] == result['category']):
            result['assessment'] = {'score': 0.75, 'partial': False}
        raw = encoded(result)
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def request(product, messages, label):
    messages.append({'role': 'user', 'content': label})
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
    try:
        client.request('POST', '/v1/responses', encoded({
            'model': product.model_alias, 'input': messages, 'stream': False}), {
                'Content-Type': 'application/json',
                'X-HiRoute-Token': product.bearer(product.agent_connection),
                'session-id': 'product-content-sink-recovery'})
        response = client.getresponse()
        raw = response.read()  # Enforces the complete declared HTTP entity.
        product.outputs.append(raw)
        assert response.status == 200, (response.status, raw)
        answer = json.loads(raw)
        assert answer['status'] == 'completed' and answer['model'] == product.model_alias, answer
        assert answer['output'] == [{
            'id': 'msg_native_product', 'type': 'message', 'role': 'assistant',
            'status': 'completed', 'content': [{'type': 'output_text',
                'text': 'native product answer', 'annotations': []}]}], answer
        messages.append({'role': 'assistant', 'content': 'native product answer'})
    finally:
        client.close()


def diagnostics(product):
    path = product.diagnostics_root / 'daemon/current.jsonl'
    # The diagnostic writer is asynchronous; only its complete JSONL records are
    # readable. A complete corrupt record still fails instead of being skipped.
    return [json.loads(line)['event'] for line in path.read_bytes().split(b'\n')[:-1] if line]


def content_nacks(product):
    return [event['sink_nack'] for event in diagnostics(product)
            if event.get('sink_nack', {}).get('channel') == 'content']


def wait_for(predicate, explanation):
    deadline = time.monotonic() + 30
    while not predicate():
        assert time.monotonic() < deadline, explanation
        time.sleep(.05)


def activity_database(product):
    paths = list(product.storage.rglob('activity.db'))
    assert len(paths) == 1, paths
    return sqlite3.connect(paths[0].as_uri() + '?mode=ro', uri=True)


def content_rows(product, request_id):
    with activity_database(product) as db:
        return db.execute('SELECT sequence, phase FROM conversation_content_events_v2 '
                          'WHERE request_id=? ORDER BY sequence', (request_id,)).fetchall()


def accepted_requests(product):
    with activity_database(product) as db:
        return {row[0] for row in db.execute(
            "SELECT request_id FROM logical_requests WHERE outcome='accepted' "
            "AND finished_at_ms IS NOT NULL")}


def complete_content(product, request_id):
    with activity_database(product) as db:
        rows = db.execute('SELECT direction, state, completeness '
                          'FROM conversation_content_streams_v2 WHERE request_id=?',
                          (request_id,)).fetchall()
    return sorted(rows) == [('request_input', 'finish', 'complete'),
                            ('response_delivered', 'finish', 'complete')]


def durable_gaps(product):
    with activity_database(product) as db:
        return db.execute('SELECT producer_epoch, stream_id, first_sequence, last_sequence, '
                          'known_loss, reason FROM observation_gaps '
                          "WHERE channel='content' ORDER BY gap_id").fetchall()


def run(repository):
    product = Product(repository)
    upstream = NativeUpstream()
    # Retain the formerly ineffective injection as a negative control. Neither the
    # plaintext collector opt-in nor an observation output directory is supplied.
    product.env['HIROUTE_OBSERVATION_CONTENT_SINK'] = 'fail_once'
    product.enable_debug_diagnostics()
    try:
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN)
        _, product.model_alias = publish_plan(product, saved['binding_id'], 'responses')
        configure_model_settings_v2(product, [product.plan_id], 'content-fault-configure')
        with native_history_decisions(product, product.plan_id, product.editor,
                                      endpoint_base=FaultDecisions) as decisions:
            since = time.time_ns()
            request(product, [], 'legacy-collector-does-not-inject')
            legacy = native_decision_receipt(product, since)
            legacy_rows = content_rows(product, legacy['request_id'])
            assert legacy_rows[0] == (1, 'begin'), legacy_rows
            assert not content_nacks(product), 'legacy collector variable affected Product storage'

            # The exact same published configuration now runs with the one-shot
            # persistence hook. No env-enabled plaintext collector is introduced.
            product.stop()
            product.env['HIROUTE_TEST_OBSERVATION_CONTENT_SINK'] = 'fail_once'
            product.start()
            messages = []
            before = len(decisions())
            prior_requests = accepted_requests(product)
            request(product, messages, 'fault-trigger')
            wait_for(lambda: content_nacks(product), 'actual content sink never consumed injection')
            assert content_nacks(product)[0] == {
                'channel': 'content', 'reason': 'unavailable', 'retryable': True}
            wait_for(lambda: any(event.get('observation_gap', {}).get('channel') ==
                                 'content' for event in diagnostics(product)),
                     'failed content delivery did not report its durable gap')
            gaps = [event['observation_gap'] for event in diagnostics(product)
                    if event.get('observation_gap', {}).get('channel') == 'content']
            assert gaps[0]['first_seq'] == gaps[0]['last_seq'] == 1, gaps
            persisted_gaps = durable_gaps(product)
            assert any(row[2:] == (1, 1, 1, 'sink_nack') for row in persisted_gaps), persisted_gaps
            wait_for(lambda: accepted_requests(product) - prior_requests,
                     'failed content persistence lost the accepted request receipt')
            fault_requests = accepted_requests(product) - prior_requests
            assert len(fault_requests) == 1, fault_requests
            fault_request_id = next(iter(fault_requests))
            assert all(sequence != 1 for sequence, _ in content_rows(product, fault_request_id)), \
                'the injected delivery unexpectedly reached persistence'

            since = time.time_ns()
            request(product, messages, 'recovered-A')
            recovered = native_decision_receipt(product, since)
            assert 'CLASSIFIER_ASSESSMENT_INVALID' not in recovered['decision']['reason_codes']
            wait_for(lambda: complete_content(product, recovered['request_id']),
                     'recovery A did not durably capture both complete directions')
            frozen = recovered['completed_stage']
            assert frozen['request_ids'] == [recovered['request_id']], frozen
            since = time.time_ns()
            request(product, messages, 'recovered-B')
            assessed = native_decision_receipt(product, since, frozen)
            assert 'CLASSIFIER_ASSESSMENT_INVALID' not in assessed['decision']['reason_codes']
            wait_for(lambda: complete_content(product, assessed['request_id']),
                     'recovery B did not durably capture both complete directions')
            assert_assessed_execution(decisions()[-1:], 'recovered-A',
                                      'native product answer', 'recovered-B')
            assert len(decisions()) == before + 3, 'classification or assessment was disabled'
            assert not list(product.root.rglob('conversation-content.jsonl')), 'plaintext collector enabled'
            print(json.dumps({'scenario': 'real-product-content-sink-fail-once-recovery',
                'state': 'green', 'legacy_variable_injected': False,
                'observed_content_nacks': content_nacks(product), 'nacks_are_sampled': True,
                'complete_main_outputs': 4, 'recovered_complete_captures': 2,
                'durable_gaps': durable_gaps(product),
                'fault_request_id': fault_request_id, 'first_injected_sequence': 1,
                'recovered_request_id': recovered['request_id'],
                'assessed_request_ids': assessed['assessments'][0]['history_membership']['request_ids'],
                'synthetic_upstreams_only': True}), flush=True)
    finally:
        try:
            product.close()
        finally:
            upstream.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]))
