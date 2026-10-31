"""Installed public management: real captured content, price edits and opt-in estimate."""
import json
import time
from functools import partial
from pathlib import Path


def query(view, payload):
    # The reader may retry only an explicit typed reader_busy failure. Keep the
    # released error format unchanged for callers that do not select this option.
    return {'schema': 'hiroute.observation.query/v2', 'read_failure_details': True,
            'intent': {'view': view, 'query': payload}}


def run(product, plan):
    read = partial(product.cli, retry_reader_busy=True)
    snapshots = read('sessions', 'list', '--include-unlinked', '--limit', '50')['data']['sessions']
    assert snapshots
    selected = None
    for session in snapshots:
        detail = read('sessions', 'show', session['session_id'], '--content', 'messages-and-tools')['data']
        assert detail['content_access'] != 'not_requested', detail
        for turn in detail['turns']:
            for rid in turn['receipt_ids']:
                receipt = read('sessions', 'receipt', rid)['data']
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
    facts = read('sessions', 'receipt', '--request-stdin', payload=query('facts', {
        'request_id': request, 'limit': 200, 'cursor': None}))['data']
    # V1 receipts own full decision evidence; V2 facts retain their existing safe summary.
    assert any(f['event_kind'] == 'route_decision' for f in facts['facts']), facts
    catalog = read('sessions', 'show', '--request-stdin', payload=query('catalog', {
        'request_id': request, 'limit': 1, 'cursor': None}))['data']
    entries = list(catalog['contents'])
    pages = 1
    while catalog.get('next_cursor'):
        catalog = read('sessions', 'show', '--request-stdin', payload=query('catalog', {
            'request_id': request, 'limit': 1, 'cursor': catalog['next_cursor']}))['data']
        entries.extend(catalog['contents'])
        pages += 1
        assert pages < 100
    assert entries
    text = ''
    for entry in entries:
        content = read('sessions', 'show', '--request-stdin', payload=query('content', {
            'request_id': request, 'content_id': entry['content_id'], 'cursor': None}))['data']
        text += ''.join(c['text'] for c in content['chunks'])
    assert 'controlled' in text or 'native product answer' in text, text
    now = int(time.time() * 1000)
    # Search is explicitly selected and uses the same current visibility as body reads.
    deadline = time.monotonic() + 20
    while True:
        search = read('sessions', 'list', '--request-stdin', payload=query('search', {
            'from_ms': now - 3600000, 'to_ms': now, 'session_id': session['session_id'],
            'keyword': 'controlled', 'limit': 10, 'cursor': None}))['data']
        if search['hits'] or time.monotonic() >= deadline:
            break
        time.sleep(.2)
    assert search['hits'] and 'next_cursor' in search, search
    quality = read('observation', 'plan-quality', 'samples', '--plan-id', plan)['data']
    assert 'samples' in quality
    current = product.cli('routing', 'show', plan)['data']
    binding = current['desired']['strategy']['primary'][0]['binding_id']
    target = {'kind': 'binding', 'binding_id': binding}
    price_request = {'targets': [{'query_id': 'analysis', 'target_locator': target,
                                 'currency': 'CNY', 'valuation_kind': 'api_equivalent'}]}
    prior = product.cli('prices', 'effective', '--request-stdin', payload=price_request)['data']
    edit = prior['items'][0]['edit_context']
    assert edit
    report = {'agent_plan_id': plan, 'from_ms': None, 'to_ms': None, 'period': None,
              'group_by': 'none', 'currency': None, 'session_id': None}
    historical_request = query('value_report', report)
    equivalent_request = query('api_equivalent_value', {
        'report': {**report, 'currency': 'CNY'}, 'reference_binding_id': binding})
    missing = read('value', 'show', '--request-stdin', payload=equivalent_request)['data']
    assert missing['estimated_savings_micros'] is None and missing['actual_cash_micros'] is None
    assert missing['missing_request_count'] > 0
    historical = read('value', 'show', '--request-stdin', payload=historical_request)['data']
    operation_ids = []
    previous_setting = None
    frozen_summary = frozen_scope = None
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
            estimate = read('value', 'show', '--request-stdin', payload=equivalent_request)['data']
            assert estimate['coverage'] == 'complete' and estimate['compared_request_count'] > 0, estimate
            assert estimate['routed_estimate_micros'] > 0 and estimate['estimated_savings_micros'] == 0, estimate
            assert estimate['actual_cash_micros'] is None

            recent = read('value', 'show', '--view', 'summary', '--routing', plan,
                                 '--currency', 'CNY')['data']
            assert recent['time']['clipped_for_retention'] and 'retention_prefix_clipped' in recent['gaps'], recent
            summary = read('value', 'show', '--view', 'summary', '--routing', plan,
                                  '--period', 'today', '--currency', 'CNY')['data']
            assert summary['schema'] == 'hiroute.observation.analysis/v1', summary
            assert summary['status'] in ('ready', 'partial') and summary['savings'], summary
            assert summary['savings']['coverage'] == 'complete', summary
            assert summary['savings']['estimated_savings_micros'] == 0, summary
            frozen_summary, frozen_scope = summary['savings'], summary['analysis_scope']
            ranking = read('value', 'show', '--group-by', 'session', '--order-by', 'cost-desc',
                                  '--analysis-scope', frozen_scope, '--limit', '1')['data']
            ranked = list(ranking['costs'])
            while ranking.get('next_cursor'):
                ranking = read('value', 'show', '--group-by', 'session', '--order-by', 'cost-desc',
                                      '--cursor', ranking['next_cursor'], '--limit', '1')['data']
                ranked.extend(ranking['costs'])
                assert len(ranked) < 100
            assert sum(r['known_sum_micros'] for r in ranked) == frozen_summary['compared_routed_subtotal_micros'], ranked
            assert all(r['coverage'] == 'complete' for r in ranked), ranked
            assert [r['known_sum_micros'] for r in ranked] == sorted([r['known_sum_micros'] for r in ranked], reverse=True)
            conversations = read('sessions', 'list', '--view', 'conversations',
                                        '--analysis-scope', frozen_scope)['data']
            assert sum(c['request_count'] for c in conversations['conversations']) == frozen_summary['request_count'], conversations
            matching = next(c for c in conversations['conversations']
                            if any(m['session_id'] == session['session_id'] for m in c['members']))
            anchor = matching['analysis_session_id']
            routing = read('sessions', 'show', anchor, '--view', 'routing',
                                  '--analysis-scope', frozen_scope)['data']
            assert routing['transitions'], routing
            assert any(t['reason'] and t['reason']['selection'] for t in routing['transitions']), routing
            detail = read('sessions', 'show', anchor, '--view', 'conversation', '--request', request,
                                 '--analysis-scope', frozen_scope)['data']
            assert len(detail['requests']) == 1 and detail['requests'][0]['request']['request_id'] == request, detail
            assert detail['requests'][0]['content'], detail
            assert all(c['kind'] not in ('provider_state', 'reasoning_delta', 'reasoning_finished')
                       for c in detail['requests'][0]['content']), detail
            scoped_request = read('value', 'show', '--group-by', 'request', '--order-by', 'cost-desc',
                                         '--request', request, '--analysis-scope', frozen_scope)['data']
            assert len(scoped_request['costs']) == 1, scoped_request
            assert scoped_request['costs'][0]['request_id'] == request, scoped_request

        replay = product.cli('prices', 'override', 'apply', '--request-stdin', payload=apply)['data']
        assert replay['operation_id'] == applied['operation_id']
        previous_setting = setting
    pinned = read('value', 'show', '--view', 'summary', '--analysis-scope', frozen_scope)['data']
    assert pinned['savings'] == frozen_summary, (pinned, frozen_summary)
    restored = product.cli('prices', 'effective', '--request-stdin', payload=price_request)['data']
    # Follow-catalog removes manual pricing. The missing-price reason can become more precise
    # once a revisioned target exists; it must never retain the former manual rates.
    assert restored['items'][0]['quote']['origin'] == prior['items'][0]['quote']['origin'] == 'unknown'
    assert all(rate['state'] == 'unknown' for rate in restored['items'][0]['quote']['rates'].values())
    # Time bounds change, frozen monetary amounts and usage do not.
    after = read('value', 'show', '--request-stdin', payload=historical_request)['data']
    assert after['summary']['amounts'] == historical['summary']['amounts']
    assert after['summary']['usage'] == historical['summary']['usage']
    print(json.dumps({'scenario': 'installed-management-analysis', 'state': 'green',
        'content_pages': pages, 'price_operations': len(operation_ids),
        'positive_cost_arithmetic': 'real installed CLI plus normalized-usage regressions',
        'historical_receipts': 'preserved'}), flush=True)
