"""Real Control/Secret/CLI/Gateway allowance admission; only provider responses are synthetic."""
import hashlib
import http.client
import json
import sqlite3
import sys
import threading
import time
from credential_pool_product import save_source
from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, control, declared, publish_plan_and_agent, save_native_source
from publication_product import Product, V1, encoded
from plan_content_process import read_content


def query(product, source):
    return control(product, 'GetComputeKeyBudgets', {'source_id': source, 'cursor': None})['data']


def wait_for_settled_balance(product, source_id, key_id):
    # Response EOS reaches the client before the attempt finalizes its charge.
    deadline = time.monotonic() + 5
    while True:
        current = query(product, source_id)
        budget_id = current['bindings'][key_id]
        balance = next(item for item in current['budgets'] if item['definition']['budget_id'] == budget_id)
        if balance['reserved_micros'] == 0:
            return balance
        assert time.monotonic() < deadline, balance
        time.sleep(.02)


def budget_preview_body(product, source_id, key, amount, definition=None, currency='USD'):
    snapshot = control(product, 'ListCompute', {})['data']
    source = next(row for row in snapshot['sources'] if row['source_id'] == source_id)
    change = {'schema': 'hiroute.compute-management-change/v3', 'api_link': None, 'default_preferences': {}, 'change': {
        'schema': 'hiroute.compute-management-change/v2', 'subject': {'kind': 'saved_source', 'source_id': source_id},
        'expected_revisions': snapshot['revisions'], 'selected_model_refs': [model['model_ref'] for model in source['models']],
        'intent': 'save_ready', 'key_edits': []}}
    return {'change': change, 'network_edit': None, 'key_budget_edits': [{
        'key': {'kind': 'saved', 'key_id': key}, 'assignment': {
            'intent': 'configure', 'budget_id': definition['budget_id'] if definition else None,
            'expected_revision': definition['revision'] if definition else 0, 'display_name': 'API credits',
            'policy': {'currency': currency, 'amount_micros': amount, 'renewal': None}, 'initial_remaining_micros': None}}]}


def cli_leaf_regression(product, source_id, key):
    """Prove both released allowance leaves reach Local Control through the real binary."""
    # A well-formed read must succeed instead of failing with UNKNOWN_COMMAND, and
    # it must return the same view the internal Local Control read produces.
    queried = product.public_cli('compute budgets get', {'source_id': source_id, 'cursor': None})[1]['data']
    assert queried == query(product, source_id), (queried, query(product, source_id))
    assert queried['source_id'] == source_id, queried
    # A well-formed allowance preview must return the same spec and digest the
    # internal Preview computes without mutating durable allowance state.
    body = budget_preview_body(product, source_id, key, 1_000_000)
    preview = product.public_cli('compute budgets preview', body)[1]['data']
    internal = control(product, 'PreviewComputeKeyBudgets', budget_preview_body(product, source_id, key, 1_000_000))['data']
    assert preview['spec'] == internal['spec'], (preview['spec'], internal['spec'])
    assert preview['accept_digest'] == internal['accept_digest'], preview
    assert preview['accept_digest'].startswith('sha256:'), preview
    # A malformed allowance preview is rejected at the CLI codec boundary (exit 2)
    # before any Local Control mutation.
    rejected = budget_preview_body(product, source_id, key, 1_000_000)
    rejected['key_budget_edits'][0]['assignment']['policy']['amount_micros'] = -1
    exit_code, envelope = product.public_cli('compute budgets preview', rejected, success=False)
    assert exit_code == 2 and envelope['error']['code'] == 'INVALID_ARGUMENTS', envelope
    return queried


def allowance(product, source_id, key, amount, label, definition=None, currency='USD'):
    preview = control(product, 'PreviewComputeKeyBudgets',
                      budget_preview_body(product, source_id, key, amount, definition, currency))['data']
    apply = {'spec': preview['spec'], 'accept_digest': preview['accept_digest'], 'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'budget-' + label}
    applied = product.public_cli('compute connection apply', apply)[1]
    replay = product.public_cli('compute connection apply', apply)[1]
    assert replay.get('operation') == applied.get('operation'), (applied, replay)
    result = control(product, 'GetComputeSaveResult', {'operation': applied['operation']})['data']
    assert result['disposition'] == 'saved', result
    result = query(product, source_id)
    return next(item for item in result['budgets'] if item['definition']['budget_id'] == result['bindings'][key])


def price(product, binding, currency='USD'):
    sources = control(product, 'ListCompute', {})['data']['sources']
    model = next(model for source in sources for model in source['models'] if model['binding_id'] == binding)
    assert {'currency': currency, 'valuation_kind': 'usage_estimate'} in model['presentation']['price_contexts'], model['presentation']
    target = {'query_id': 'key-budget', 'target_locator': {'kind': 'binding', 'binding_id': binding}, 'currency': currency, 'valuation_kind': 'usage_estimate'}
    before = product.preview('prices effective', {'targets': [target]})
    rates = {field: {'state': 'known', 'micros_per_million_tokens': 1_000_000} for field in ['input_uncached', 'output', 'cache_read', 'cache_write']}
    preview = product.preview('prices override preview', {**before['items'][0]['edit_context'], 'currency': currency, 'valuation_kind': 'usage_estimate', 'action': {'kind': 'set', 'rates': rates}})
    product.apply('prices override apply', 'ApplyPriceOverrideChange', preview, {'spec': preview['spec']}, 'key-budget-price-' + currency)


def saved_probe(product, upstream, saved, inference_model=None):
    source = next(row for row in control(product, 'ListCompute', {})['data']['sources']
                  if row['source_id'] == saved['source_id'])
    draft = {'inference_model_id': inference_model, 'candidate_ref': None,
        'lineage_ref': 'lineage/native/product', 'display_name': source['display_name'],
        'existing_source_id': saved['source_id'], 'expected_source_revision': source['revision'],
        'edit_revision': 1, 'check_id': 'budget-probe-' + str(time.monotonic_ns()),
        'base_url': upstream.base_url, 'base_kind': 'api_root', 'request_path_override': None,
        'inventory_path_override': '/v1/models', 'protocol': 'responses',
        'protocol_profile_id': 'profile/custom/responses', 'protocol_profile_revision': 1,
        'authentication': {'kind': 'bearer'}, 'additional_endpoints': [], 'configuration_revision': 1,
        'models': [{'upstream_model_id': MODEL, 'display_name': MODEL,
            'catalog_configuration_id': None, 'membership': 'user_declared', 'capabilities': {
                'tool': declared(True), 'vision': declared(False), 'streaming': declared(True),
                'context_tokens': declared(32768), 'max_output_tokens': declared(4096),
                'native_reasoning': declared({'kind': 'fixed', 'profile': 'provider-default'})}}]}
    return product.public_cli('compute connection test', {'kind': 'native', 'request': {'draft': draft}}, success=False)


def decision_budget(product, source_id, first, second):
    from http.server import ThreadingHTTPServer
    from decision_connections_v2_product import Endpoint
    Endpoint.requests = []
    Endpoint.failure = None
    Endpoint.reject_input = False
    endpoint = ThreadingHTTPServer(('127.0.0.1', 0), Endpoint)
    thread = threading.Thread(target=endpoint.serve_forever, daemon=True)
    thread.start()
    try:
        command = ('decision connections apply --id decision/local-budget --name Budgeted '
            '--provider openai --source ' + source_id + ' --endpoint '
            f'http://127.0.0.1:{endpoint.server_port}/v1/decisions')
        preview = product.public_cli(command)[1]['data']
        applied = product.public_cli('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'budget-decision-save'})[1]
        assert applied['data']['state'] == 'succeeded', applied

        def test(detail=True):
            view = next(row for row in control(product, 'ListDecisionServicesV3', {})['data']['services']
                        if row['service']['id'] == 'decision/local-budget')
            request = {'id': view['service']['id'], 'revision': view['service']['revision'],
                'expected_configuration_digest': view['configuration_digest']}
            if detail:
                request['include_decision_failure'] = True
            return product.public_cli('decision network test', request)[1]['data']

        before = query(product, source_id)
        accepted = test()
        assert accepted['check']['outcome'] == 'passed', accepted
        assert Endpoint.requests[-1][0] == 'Bearer ' + NATIVE_TOKEN, Endpoint.requests
        after = query(product, source_id)
        assert [(b['lifetime_spent_micros'], b['reserved_micros']) for b in after['budgets']] == [
            (b['lifetime_spent_micros'], b['reserved_micros']) for b in before['budgets']]
        with sqlite3.connect(product.storage / 'live/control.db') as db:
            pending = db.execute("SELECT COUNT(*) FROM compute_key_spend_reservations WHERE state='pending' AND amount_micros=0 AND key_id=?", (first,)).fetchone()[0]
            assert pending > 0, 'unpriced decision did not retain pending evidence'
        # Both local budgets are known exhausted. Do not make another provider
        # call, and expose the actual local cause only to opted-in callers.
        for key in (first, second):
            current = query(product, source_id)
            budget = next(b for b in current['budgets'] if b['definition']['budget_id'] == current['bindings'][key])
            assert budget['lifetime_spent_micros'] > 0, budget
            allowance(product, source_id, key, budget['lifetime_spent_micros'], 'exhaust-' + key.rsplit('-', 1)[-1], budget['definition'], currency=budget['definition']['policy']['currency'])
        count = len(Endpoint.requests)
        refused = test()
        assert refused['check']['outcome'] == 'failed', refused
        assert refused['decision_failure'] == {'kind': 'budget_exhausted', 'http_status': None}, refused
        assert len(Endpoint.requests) == count, 'exhausted decision called the provider'
        legacy = test(False)
        assert 'decision_failure' not in legacy, legacy
        current = query(product, source_id)
        first_budget = next(b for b in current['budgets'] if b['definition']['budget_id'] == current['bindings'][first])
        allowance(product, source_id, first, 1_000_000, 'decision-restore', first_budget['definition'])
        assert test()['check']['outcome'] == 'passed'
        product.stop(); product.start()
        assert test()['check']['outcome'] == 'passed'
        print(json.dumps({'scenario': 'decision-key-budget-real-cli', 'state': 'green',
            'same_key_and_budget': True, 'unknown_price_allowed': True, 'unpriced_pending': True,
            'budget_failure_detail': True, 'legacy_result_unchanged': True, 'restart_preserved': True}), flush=True)
    finally:
        endpoint.shutdown(); endpoint.server_close(); thread.join(timeout=5)


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
        # Both released allowance leaves must reach Local Control through the real
        # binary and agree with the internal reads without mutating allowance state.
        before_cli = query(product, source_id)
        cli_leaf_regression(product, source_id, first)
        assert query(product, source_id) == before_cli, 'compute budgets leaves mutated allowance state'
        # The same public saved-Key probe is reachable before a limit is set.
        assert saved_probe(product, upstream, saved, MODEL)[0] == 0
        assert any(row['method'] == 'POST' for row in upstream.requests), upstream.requests
        configured = allowance(product, source_id, first, 1_000_000, 'initial')
        with upstream.lock:
            before_probe = len(upstream.requests)
        exit_code, accepted = saved_probe(product, upstream, saved, MODEL)
        assert exit_code == 0, accepted
        assert any(row['method'] == 'POST' for row in upstream.requests[before_probe:]), 'unpriced saved-Key probe did not reach upstream'
        before_rejected = len(upstream.requests)
        exit_code, rejected = saved_probe(product, upstream, saved, 'model-not-in-draft')
        assert exit_code != 0, rejected
        assert len(upstream.requests) == before_rejected, 'rejected probe reached upstream'
        with sqlite3.connect(product.storage / 'live/control.db') as db:
            records = db.execute(
                'SELECT state,amount_micros FROM compute_key_spend_reservations WHERE key_id=? ORDER BY rowid',
                (first,)).fetchall()
            assert records == [('pending', 0), ('released', 0)], records
        before_directory = len(upstream.requests)
        assert saved_probe(product, upstream, saved)[0] == 0
        assert all(row['method'] == 'GET' for row in upstream.requests[before_directory:])
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

        # Unknown prices allow the first admitted Key, with unpriced pending
        # evidence instead of a fabricated zero-cost charge.
        request(NATIVE_TOKEN)
        assert query(product, source_id)['budgets'][0]['lifetime_spent_micros'] == 0
        price(product, saved['binding_id'])
        request(NATIVE_TOKEN)
        balance = wait_for_settled_balance(product, source_id, first)
        assert balance['lifetime_spent_micros'] == 7 and balance['available_micros'] == 999_993, balance
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
        restored = wait_for_settled_balance(product, source_id, first)
        assert restored['lifetime_spent_micros'] == 14, restored
        request(NATIVE_TOKEN, hosted=True)
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
        cny = wait_for_settled_balance(product, source_id, second)
        assert cny['lifetime_spent_micros'] == 7 and cny['available_micros'] == 999_993, cny

        decision_budget(product, source_id, first, second)

        print(json.dumps({'scenario': 'key-budget-real-gateway', 'state': 'green', 'unknown_price_allowed': True, 'saved_probe_allowed': True, 'key_fallback': True, 'usage_settled': True, 'restart_preserved': True, 'cap_edits_retain_spending': True, 'frozen_plan_unchanged': True}), flush=True)
    finally:
        product.close(); upstream.close()


def run_streams(repository):
    from publication_process import configure_model_settings_v2
    from usage_query_gateway_product import UsageUpstream, gateway_request, publish_plan, USAGE
    for protocol in ['responses', 'messages']:
        product, upstream = Product(repository), UsageUpstream()
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        try:
            product.start()
            saved = save_native_source(product, upstream, NATIVE_TOKEN, protocol=protocol,
                                       context_tokens=131072 if protocol == 'messages' else 32768)
            source = next(row for row in control(product, 'ListCompute', {})['data']['sources']
                          if row['source_id'] == saved['source_id'])
            key_id = source['keys'][0]['key_id']
            allowance(product, saved['source_id'], key_id, 1_000_000, 'stream-' + protocol)
            price(product, saved['binding_id'])
            plan_id, alias = publish_plan(product, saved['binding_id'], protocol)
            agent = 'agent_codex_default' if protocol == 'responses' else 'agent_claude_default'
            configure_model_settings_v2(product, [plan_id], 'budget-stream-' + protocol, agent_id=agent)
            gateway_request(product, protocol, alias, True, product.bearer(product.agent_connection))
            expected = sum(USAGE[(protocol, True)][:2])
            balance = wait_for_settled_balance(product, saved['source_id'], key_id)
            assert balance['lifetime_spent_micros'] == expected and balance['available_micros'] == 1_000_000 - expected, balance
            assert upstream.requests == [(protocol, True)], upstream.requests
            print(json.dumps({'scenario': 'key-budget-stream-' + protocol, 'state': 'green', 'charged_micros': expected, 'pending_micros': 0}), flush=True)
        finally:
            product.close(); upstream.close()


if __name__ == '__main__':
    run(sys.argv[1])
    run_streams(sys.argv[1])
