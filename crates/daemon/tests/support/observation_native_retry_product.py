"""Native failed-request retry through the real Gateway, writer and public readers."""
import http.client
import json
from pathlib import Path
import sys

from decision_services_product import native_history_decisions
from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, save_native_source
from observation_content_fault_product import (
    activity_database, complete_content, content_nacks, durable_gaps, wait_for,
)
from publication_process import configure_model_settings_v2
from publication_product import Product, encoded
from usage_query_gateway_product import publish_plan


def rows(product, sql, values=()):
    with activity_database(product) as db:
        return db.execute(sql, values).fetchall()


def send(product, messages, turn, failed=False, session='native-content-retry', captured=True):
    prior = {row[0] for row in rows(product, 'SELECT request_id FROM logical_requests')}
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
    try:
        client.request('POST', '/v1/responses', encoded({
            'model': product.model_alias, 'input': messages, 'stream': False}), {
                'Content-Type': 'application/json',
                'X-HiRoute-Token': product.bearer(product.agent_connection),
                'session-id': session, 'thread-id': session,
                'user-agent': 'codex_exec/0.147.0 (fixture)',
                'x-codex-turn-metadata': encoded({
                    'session_id': session, 'thread_id': session,
                    'turn_id': turn, 'request_kind': 'turn'}).decode(),
            })
        response = client.getresponse()
        raw = response.read()
        product.outputs.append(raw)
        assert (response.status != 200) if failed else (response.status == 200), raw
    finally:
        client.close()
    result = []

    def finished():
        result[:] = [row[0] for row in rows(product,
            'SELECT request_id FROM logical_requests WHERE finished_at_ms IS NOT NULL')
            if row[0] not in prior]
        return len(result) == 1

    wait_for(finished, 'native request receipt did not persist')
    request_id = result[0]
    if captured:
        wait_for(lambda: rows(product,
            "SELECT 1 FROM conversation_content_streams_v2 WHERE request_id=? "
            "AND direction='request_input' AND state='finish' AND completeness='complete'",
            (request_id,)), 'native input capture did not complete')
    return request_id


def query(product, view, value):
    return product.public_cli('sessions show', {
        'schema': 'hiroute.observation.query/v2',
        'intent': {'view': view, 'query': value}})[1]['data']


def catalog(product, request_id):
    return query(product, 'catalog', {'request_id': request_id, 'limit': 200,
        'cursor': None, 'include_input_selection': True, 'include_semantic_metadata': True})


def verify_carried_input_source(product):
    """Archive provenance survives response-association loss, independently of retry."""
    session = 'native-carried-input-source'
    messages = [{'role': 'user', 'content': 'Preserve this original input occurrence.'}]
    first = send(product, messages, 'source-A', session=session)
    original = catalog(product, first)
    source = [c for c in original['contents'] if c['direction'] == 'request_input'
              and c['role'] == 'user']
    assert len(source) == 1 and source[0]['state'] == 'complete', original
    expected = {'source_request_id': first,
                'message_occurrence_id': source[0]['message_occurrence_id']}
    requests = [first]
    last = None
    for step in range(3):
        # This output was never accepted: the original input prefix remains
        # exact, but response continuity must fail and open an independent stage.
        messages.append({'role': 'assistant', 'content': f'Unmatched output {step}'})
        current = send(product, messages, 'source-A', session=session)
        requests.append(current)
        wait_for(lambda: complete_content(product, current), 'carried source capture incomplete')
        last = catalog(product, current)
        assert not last['roots_partial'] and last['input_anchor'] == expected, last
        assert not [c for c in last['contents'] if c['direction'] == 'request_input'], last
    placeholders = ','.join('?' for _ in requests)
    stages = rows(product, 'SELECT COUNT(DISTINCT turn_id) FROM logical_requests '
                  f'WHERE request_id IN ({placeholders})', requests)
    assert stages == [(4,)], 'archive source identity merged unrelated execution stages'
    product.stop()
    product.start()
    assert catalog(product, requests[-1]) == last, 'carried source changed after reopen'
    text = query(product, 'content', {'request_id': first, 'content_id': source[0]['content_id'],
                                    'cursor': None})
    assert text['state'] == 'available', text
    assert ''.join(chunk['text'] for chunk in text['chunks']) == messages[0]['content']
    # Identical text is a distinct occurrence at a genuinely appended input.
    messages.append({'role': 'user', 'content': messages[0]['content']})
    following = send(product, messages, 'source-B', session=session)
    wait_for(lambda: complete_content(product, following), 'new input capture incomplete')
    page = catalog(product, following)
    assert not page['roots_partial'] and page['input_anchor'] != expected, page
    return {'carried_source_requests': len(requests), 'independent_stages': 4,
            'carried_source_reopened': True, 'repeated_input_is_distinct': True}


def run(repository):
    product, upstream = Product(repository), NativeUpstream()
    product.enable_debug_diagnostics()
    try:
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN)
        _, product.model_alias = publish_plan(product, saved['binding_id'], 'responses')
        configure_model_settings_v2(product, [product.plan_id], 'native-content-retry-configure')
        with native_history_decisions(product, product.plan_id, product.editor) as decisions:
            messages = [{'role': 'user', 'content': 'Retain this native input exactly once.'}]
            # A failed response before accepted output proves no provider affinity.
            # Use a non-cooling rejection so this case tests history, not backoff.
            upstream.rejected_status = 400
            first = send(product, messages, 'turn-A', failed=True)
            original = catalog(product, first)
            users = [c for c in original['contents'] if c['role'] == 'user']
            assert len(users) == 1 and users[0]['state'] == 'complete', original
            upstream.rejected_status = None
            retry = send(product, messages, 'turn-A')
            wait_for(lambda: complete_content(product, retry), 'retry capture did not complete')
            assert len(decisions()) == 1, 'same native retry was unnecessarily reclassified'
            retried = catalog(product, retry)
            assert not retried['roots_partial'], retried
            assert retried['input_anchor'] == {
                'source_request_id': first,
                'message_occurrence_id': users[0]['message_occurrence_id']}, retried
            assert not [c for c in retried['contents'] if c['direction'] == 'request_input'], retried
            outputs = [c for c in retried['contents'] if c['direction'] == 'response_delivered']
            assert outputs and all(c['attempt_id'] and c['state'] == 'complete' for c in outputs)
            messages += [{'role': 'assistant', 'content': 'native product answer'},
                         {'role': 'user', 'content': 'Assess the actual preceding execution.'}]
            following = send(product, messages, 'turn-B')
            wait_for(lambda: complete_content(product, following), 'next native capture incomplete')
            assert len(decisions()) == 2, 'the next native activation did not classify'
            assessed = json.dumps(decisions()[-1])
            assert 'Retain this native input exactly once.' in assessed
            assert 'native product answer' in assessed
            assert not content_nacks(product), content_nacks(product)
            assert not durable_gaps(product), durable_gaps(product)
            product.stop()
            product.start()
            reopened = catalog(product, retry)
            assert reopened == retried, 'native retry history changed after reopen'
            body = query(product, 'content', {'request_id': first,
                'content_id': users[0]['content_id'], 'cursor': None})
            assert body['state'] == 'available' and body['next_cursor'] is None, body
            assert ''.join(c['text'] for c in body['chunks']) == messages[0]['content'], body

            carried_source = verify_carried_input_source(product)

            # Runtime history cannot prove optional body persistence. A failed
            # first capture must remain an explicit gap on a healthy retry.
            product.stop()
            product.env['HIROUTE_TEST_OBSERVATION_CONTENT_SINK'] = 'fail_once'
            product.start()
            upstream.rejected_status = 400
            lost_input = [{'role': 'user', 'content': 'Optional capture was unavailable.'}]
            send(product, lost_input, 'gap-A', failed=True,
                 session='native-content-retry-gap', captured=False)
            wait_for(lambda: content_nacks(product), 'first capture fault was not exercised')
            assert content_nacks(product) == [
                {'channel': 'content', 'reason': 'unavailable', 'retryable': True}]
            upstream.rejected_status = None
            recovered = send(product, lost_input, 'gap-A', session='native-content-retry-gap')
            wait_for(lambda: complete_content(product, recovered), 'healthy retry did not finish')
            for metadata in (False, True):
                page = query(product, 'catalog', {'request_id': recovered, 'limit': 200,
                    'cursor': None, 'include_semantic_metadata': metadata})
                assert page['roots_partial'] and not page.get('input_anchor'), page
                assert [c for c in page['contents'] if c['direction'] == 'response_delivered']
            assert content_nacks(product) == [
                {'channel': 'content', 'reason': 'unavailable', 'retryable': True}]
            print(json.dumps({'scenario': 'native-failed-request-retry-semantic-history',
                'state': 'green', 'real_requests': 5, 'input_occurrences': 1,
                'ordinary_retry_content_nacks': 0, 'ordinary_retry_content_gaps': 0,
                'injected_capture_failures': 1, 'missing_retry_input_explicitly_partial': True,
                'actual_attempts_linked': True, 'history_reopened': True, **carried_source}), flush=True)
    finally:
        try:
            product.close()
        finally:
            upstream.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]))
