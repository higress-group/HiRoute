"""Source-scoped reference prices through the installed CLI and real Gateway valuation."""
import time


def configure_reference_price(product, binding_id, rate):
    target = {'query_id': 'source-price', 'target_locator': {
        'kind': 'binding', 'binding_id': binding_id},
        'currency': 'USD', 'valuation_kind': 'api_equivalent'}
    before = product.preview('prices effective', {'targets': [target]})
    edit = before['items'][0]['edit_context']
    rates = {field: {'state': 'known', 'micros_per_million_tokens': rate}
             for field in ['input_uncached', 'output', 'cache_read', 'cache_write']}
    preview = product.preview('prices override preview', {
        **edit, 'currency': 'USD', 'valuation_kind': 'api_equivalent',
        'action': {'kind': 'set', 'rates': rates}})
    product.apply('prices override apply', 'ApplyPriceOverrideChange', preview,
                  {'spec': preview['spec']}, 'source-price-' + str(rate))
    after = product.preview('prices effective', {'targets': [target]})
    assert not after['pending_activation'], after
    assert after['items'][0]['quote']['rates'] == rates, after


def timeline_amounts(product, session_id, expected_requests):
    deadline = time.monotonic() + 20
    while True:
        code, reply = product.public_cli('sessions timeline ' + session_id, success=False)
        if code:
            assert reply.get('error', {}).get('retryable'), reply
            amounts = {}
        else:
            amounts = {
                request['request_id']: [cost for cost in request['costs']
                    if cost['valuation_kind'] == 'api_equivalent'
                    and cost['currency'] == 'USD' and cost['known_micros'] is not None]
                for turn in reply['data']['turns'] for request in turn['requests']}
            amounts = {key: costs for key, costs in amounts.items() if costs}
        if len(amounts) == expected_requests:
            assert all(sum(cost['known_micros'] for cost in costs) > 0
                       for costs in amounts.values()), amounts
            return amounts
        assert time.monotonic() < deadline, reply
        time.sleep(.05)
