"""Installed public management: real captured content, price edits and opt-in estimate."""
import json
import time
from pathlib import Path


def query(view, payload):
    return {'schema': 'hiroute.observation.query/v2', 'intent': {'view': view, 'query': payload}}


def run(product, plan):
    snapshots = product.cli('sessions', 'list', '--include-unlinked', '--limit', '50')['data']['sessions']
    assert snapshots
    selected = None
    for session in snapshots:
        detail = product.cli('sessions', 'show', session['session_id'], '--content', 'messages-and-tools')['data']
        assert detail['content_access'] != 'not_requested', detail
        for turn in detail['turns']:
            for rid in turn['receipt_ids']:
                receipt = product.cli('sessions', 'receipt', rid)['data']
                route_decisions = [e['fact'] for e in receipt['ordered_facts']
                    if e['fact']['kind'] == 'route_decision'
                    and e['fact'].get('complexity', {}).get('decision_source') == 'external_classifier']
                accepted = any(e['fact']['kind'] == 'request_finished'
                    and e['fact']['outcome'] == 'accepted' for e in receipt['ordered_facts'])
                if receipt['trust'].get('agent_plan_id') == plan and route_decisions and accepted:
                    selected = (session, receipt)
                    break
            if selected:
                break
        if selected:
            break
    assert selected
    session, receipt = selected
    request = receipt['request_id']
    assert any(e['fact']['kind'] == 'usage_and_cache' and e['fact']['cost_class'] != 'free'
               for e in receipt['ordered_facts']), 'Custom/paid/unknown access must not be called free'
    facts = product.cli('sessions', 'receipt', '--request-stdin', payload=query('facts', {
        'request_id': request, 'limit': 200, 'cursor': None}))['data']
    # V1 receipts own full decision evidence; V2 facts retain their existing safe summary.
    assert any(f['event_kind'] == 'route_decision' for f in facts['facts']), facts
    catalog = product.cli('sessions', 'show', '--request-stdin', payload=query('catalog', {
        'request_id': request, 'limit': 1, 'cursor': None}))['data']
    entries = list(catalog['contents'])
    pages = 1
    while catalog.get('next_cursor'):
        catalog = product.cli('sessions', 'show', '--request-stdin', payload=query('catalog', {
            'request_id': request, 'limit': 1, 'cursor': catalog['next_cursor']}))['data']
        entries.extend(catalog['contents'])
        pages += 1
        assert pages < 100
    assert entries
    text = ''
    for entry in entries:
        content = product.cli('sessions', 'show', '--request-stdin', payload=query('content', {
            'request_id': request, 'content_id': entry['content_id'], 'cursor': None}))['data']
        text += ''.join(c['text'] for c in content['chunks'])
    assert 'controlled' in text or 'native product answer' in text, text
    now = int(time.time() * 1000)
    # Search is explicitly selected and uses the same current visibility as body reads.
    deadline = time.monotonic() + 20
    while True:
        search = product.cli('sessions', 'list', '--request-stdin', payload=query('search', {
            'from_ms': now - 3600000, 'to_ms': now, 'session_id': session['session_id'],
            'keyword': 'controlled', 'limit': 10, 'cursor': None}))['data']
        if search['hits'] or time.monotonic() >= deadline:
            break
        time.sleep(.2)
    assert search['hits'] and 'next_cursor' in search, search
    quality = product.cli('observation', 'plan-quality', 'samples', '--plan-id', plan)['data']
    assert 'samples' in quality
    current = product.cli('routing', 'show', plan)['data']
    binding = current['desired']['strategy']['primary'][0]['binding_id']
    target = {'kind': 'binding', 'binding_id': binding}
    price_request = {'targets': [{'query_id': 'analysis', 'target_locator': target,
                                 'currency': 'CNY', 'valuation_kind': 'api_equivalent'}]}
    prior = product.cli('prices', 'effective', '--request-stdin', payload=price_request)['data']
    edit = prior['items'][0]['edit_context']
    assert edit
    missing = product.cli('value', 'show', '--routing', plan, '--basis', 'api-equivalent',
                          '--reference-binding', binding, '--currency', 'CNY')['data']
    assert missing['estimated_savings_micros'] is None and missing['actual_cash_micros'] is None
    assert missing['missing_request_count'] > 0
    historical = product.cli('value', 'show', '--routing', plan)['data']
    operation_ids = []
    previous_setting = None
    for setting in [
        {'kind': 'set', 'rates': {k: {'state': 'known', 'micros_per_million_tokens': 1000000}
          for k in ['input_uncached', 'output', 'cache_read', 'cache_write']}},
        {'kind': 'follow_catalog'},
    ]:
        effective = product.cli('prices', 'effective', '--request-stdin', payload=price_request)['data']
        edit = effective['items'][0]['edit_context']
        preview = product.cli('prices', 'override', 'preview', '--request-stdin', payload={
            **edit, 'currency': 'CNY', 'valuation_kind': 'api_equivalent', 'action': setting})['data']
        before = preview['spec']['desired_state']['before']
        assert (before['setting'] if before else None) == previous_setting
        apply = {'spec': preview['spec'], 'accept_digest': preview['change_digest'],
                 'expected_revisions': preview['expected_revisions'],
                 'idempotency_key': f'analysis-price-{len(operation_ids)}'}
        applied = product.cli('prices', 'override', 'apply', '--request-stdin', payload=apply)['data']
        assert applied['state'] == 'succeeded', applied
        operation_ids.append(applied['operation_id'])
        if setting['kind'] == 'set':
            estimate = product.cli('value', 'show', '--routing', plan, '--basis', 'api-equivalent',
                '--reference-binding', binding, '--currency', 'CNY')['data']
            assert estimate['coverage'] == 'complete' and estimate['compared_request_count'] > 0, estimate
            assert estimate['routed_estimate_micros'] > 0 and estimate['estimated_savings_micros'] == 0, estimate
            assert estimate['actual_cash_micros'] is None

        replay = product.cli('prices', 'override', 'apply', '--request-stdin', payload=apply)['data']
        assert replay['operation_id'] == applied['operation_id']
        previous_setting = setting
    restored = product.cli('prices', 'effective', '--request-stdin', payload=price_request)['data']
    # Follow-catalog removes manual pricing. The missing-price reason can become more precise
    # once a revisioned target exists; it must never retain the former manual rates.
    assert restored['items'][0]['quote']['origin'] == prior['items'][0]['quote']['origin'] == 'unknown'
    assert all(rate['state'] == 'unknown' for rate in restored['items'][0]['quote']['rates'].values())
    # Time bounds change, frozen monetary amounts and usage do not.
    after = product.cli('value', 'show', '--routing', plan)['data']
    assert after['summary']['amounts'] == historical['summary']['amounts']
    assert after['summary']['usage'] == historical['summary']['usage']
    print(json.dumps({'scenario': 'installed-management-analysis', 'state': 'green',
        'content_pages': pages, 'price_operations': len(operation_ids),
        'positive_cost_arithmetic': 'real installed CLI plus normalized-usage regressions',
        'historical_receipts': 'preserved'}), flush=True)
