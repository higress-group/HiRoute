"""Real Control/Secret/CLI/Gateway allowance admission; only provider responses are synthetic."""
import hashlib
import http.client
import json
import sys
import time
from credential_pool_product import save_source
from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, control, publish_plan_and_agent, save_native_source
from publication_product import Product, encoded
from plan_content_process import read_content


def query(product, source):
    return control(product, 'GetComputeKeyBudgets', {'source_id': source, 'cursor': None})['data']


def allowance(product, source_id, key, amount, label, definition=None, currency='USD'):
    snapshot = control(product, 'ListCompute', {})['data']
    source = next(row for row in snapshot['sources'] if row['source_id'] == source_id)
    change = {'schema': 'hiroute.compute-management-change/v3', 'api_link': None, 'default_preferences': {}, 'change': {
        'schema': 'hiroute.compute-management-change/v2', 'subject': {'kind': 'saved_source', 'source_id': source_id},
        'expected_revisions': snapshot['revisions'], 'selected_model_refs': [model['model_ref'] for model in source['models']],
        'intent': 'save_ready', 'key_edits': []}}
    preview = control(product, 'PreviewComputeKeyBudgets', {'change': change, 'network_edit': None, 'key_budget_edits': [{
        'key': {'kind': 'saved', 'key_id': key}, 'assignment': {
            'intent': 'configure', 'budget_id': definition['budget_id'] if definition else None,
            'expected_revision': definition['revision'] if definition else 0, 'display_name': 'API credits',
            'policy': {'currency': currency, 'amount_micros': amount, 'renewal': None}, 'initial_remaining_micros': None}}]})['data']
    apply = {'spec': preview['spec'], 'accept_digest': preview['accept_digest'], 'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'budget-' + label}
    applied = product.public_cli('compute connection apply', apply)[1]
    replay = product.public_cli('compute connection apply', apply)[1]
    assert replay.get('operation') == applied.get('operation'), (applied, replay)
    result = control(product, 'GetComputeSaveResult', {'operation': applied['operation']})['data']
    assert result['disposition'] == 'saved', result
    result = query(product, source_id)
    return next(item for item in result['budgets'] if item['definition']['budget_id'] == result['bindings'][key])


def price(product, binding, currency='USD'):
    target = {'query_id': 'key-budget', 'target_locator': {'kind': 'binding', 'binding_id': binding}, 'currency': currency, 'valuation_kind': 'usage_estimate'}
    before = product.preview('prices effective', {'targets': [target]})
    rates = {field: {'state': 'known', 'micros_per_million_tokens': 1_000_000} for field in ['input_uncached', 'output', 'cache_read', 'cache_write']}
    preview = product.preview('prices override preview', {**before['items'][0]['edit_context'], 'currency': currency, 'valuation_kind': 'usage_estimate', 'action': {'kind': 'set', 'rates': rates}})
    product.apply('prices override apply', 'ApplyPriceOverrideChange', preview, {'spec': preview['spec']}, 'key-budget-price-' + currency)


def run(repository):
    product, upstream = Product(repository), NativeUpstream()
    product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
    try:
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN)
        source_id = saved['source_id']
        source = next(row for row in control(product, 'ListCompute', {})['data']['sources'] if row['source_id'] == source_id)
        first = source['keys'][0]['key_id']
        fallback = 'fixture-budget-fallback-key'
        candidate = {'candidate_ref': 'candidate/budget/fallback', 'candidate_revision': 1}
        product.secrets.add(fallback)
        product.register_protected_frame({'schema': 'hiroute.protected-input/v1', 'registration_id': hashlib.sha256(b'budget-fallback').hexdigest(), **candidate, 'secret': fallback})
        save_source(product, saved, [{'action': 'add', 'input_candidate': candidate}], 'budget-add-fallback')
        configured = allowance(product, source_id, first, 1_000_000, 'initial')
        publish_plan_and_agent(product, saved['binding_id'])
        frozen = read_content(product)

        def request(expected_key, stream=False, hosted=False):
            with upstream.lock:
                before = len(upstream.requests)
            client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
            try:
                body = {'model': product.model_alias, 'input': 'allowance request', 'stream': stream}
                if hosted: body['tools'] = [{'type': 'web_search'}]
                client.request('POST', '/v1/responses', body=encoded(body), headers={'X-HiRoute-Token': product.bearer(product.agent_connection), 'Content-Type': 'application/json', 'session-id': 'key-budget-product'})
                response = client.getresponse(); body = response.read(); product.outputs.append(body)
                assert response.status == 200 and b'native product answer' in body, (response.status, body)
            finally:
                client.close()
            with upstream.lock:
                attempts = upstream.requests[before:]
            assert [row['authorization'] for row in attempts] == ['Bearer ' + expected_key], attempts
            assert read_content(product) == frozen, 'allowance change republished the frozen plan'

        # A native manual source without a source price cannot use its limited
        # Key as free traffic. The admitted next Key is still available.
        request(fallback)
        assert query(product, source_id)['budgets'][0]['lifetime_spent_micros'] == 0
        price(product, saved['binding_id'])
        request(NATIVE_TOKEN)
        deadline = time.monotonic() + 5
        while True:
            balance = query(product, source_id)['budgets'][0]
            if balance['lifetime_spent_micros'] == 7:
                break
            assert time.monotonic() < deadline, balance
            time.sleep(.02)
        assert balance['reserved_micros'] == 0 and balance['available_micros'] == 999_993, balance
        # Editing the cap retains the seven micros already spent. One remaining
        # micro cannot cover the bounded reservation, so the next Key is used.
        lowered = allowance(product, source_id, first, 8, 'lower', balance['definition'])
        assert lowered['available_micros'] == 1 and lowered['lifetime_spent_micros'] == 7, lowered
        request(fallback)
        product.stop(); product.start()
        reopened = query(product, source_id)['budgets'][0]
        assert reopened['available_micros'] == 1 and reopened['lifetime_spent_micros'] == 7, reopened
        request(fallback)
        allowance(product, source_id, first, 1_000_000, 'restore', reopened['definition'])
        request(NATIVE_TOKEN)
        assert query(product, source_id)['budgets'][0]['lifetime_spent_micros'] == 14
        request(fallback, hosted=True)
        stable = query(product, source_id)['budgets'][0]
        assert stable['lifetime_spent_micros'] == 14 and stable['reserved_micros'] == 0, stable
        # Explicit CNY prices use CNY allowance; the receipt's USD observation
        # remains independent and no currency conversion is performed.
        allowance(product, source_id, first, 15, 'lower-for-cny', stable['definition'])
        source = next(row for row in control(product, 'ListCompute', {})['data']['sources'] if row['source_id'] == source_id)
        second = source['keys'][1]['key_id']
        allowance(product, source_id, second, 1_000_000, 'cny', currency='CNY')
        price(product, saved['binding_id'], currency='CNY')
        request(fallback)
        cny = query(product, source_id)
        cny = next(item for item in cny['budgets'] if item['definition']['budget_id'] == cny['bindings'][second])
        assert cny['lifetime_spent_micros'] == 7 and cny['available_micros'] == 999_993, cny

        print(json.dumps({'scenario': 'key-budget-real-gateway', 'state': 'green', 'unknown_price_skipped': True, 'key_fallback': True, 'usage_settled': True, 'restart_preserved': True, 'cap_edits_retain_spending': True, 'frozen_plan_unchanged': True}), flush=True)
    finally:
        product.close(); upstream.close()


if __name__ == '__main__':
    run(sys.argv[1])
