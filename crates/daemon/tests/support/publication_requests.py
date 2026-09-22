"""Real Gateway attempts with a counted external CPA fixture and an old in-flight request."""
import concurrent.futures
import http.client
import json
import subprocess
import sys
import time
from publication_process import bootstrap, configure_model_settings_v2, plan_change
from publication_product import Product, encoded


def request(product, alias, prompt, token=None):
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=45)
    body = {'model': alias, 'input': prompt, 'stream': False}
    try:
        client.request('POST', '/v1/responses', body=encoded(body), headers={
            # Codex keeps its native Authorization authority independent. The managed provider
            # sends the scoped Agent grant through this dedicated header, exactly as the native
            # config renderer writes model_providers.hiroute.http_headers.
            'X-HiRoute-Token': token or product.bearer(product.agent_connection),
            'Content-Type': 'application/json'})
        response = client.getresponse()
        data = response.read()
        product.outputs.append(data)
        return response.status, data
    finally:
        client.close()


def attempts(product):
    path = product.cpa_fixture / 'attempts.jsonl'
    return len(path.read_text().splitlines()) if path.exists() else 0


def run(repository, expected_sha=None):
    actual_sha = subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()
    if expected_sha is not None:
        assert actual_sha == expected_sha, 'candidate mismatch'
    product = Product(repository)
    try:
        native_model = 'gpt-5.5'
        product.install_codex_fixture()
        product.enable_cpa(native_model)
        product.model_settings_v2 = True
        product.model_settings_agent_id = 'agent_codex_default'
        bootstrap(product)
        # This scenario exercises native-model retention across publication. The default
        # HiRoute-only mode intentionally exposes only plan aliases, so opt in explicitly.
        configure_model_settings_v2(
            product, [product.plan_id], 'retain-native-a', product.plan_id,
            agent_id='agent_codex_default', native_model_mode='preserve_available')
        catalog, _ = product.catalog()
        native_models = {entry['id'] for entry in catalog['data']
                         if entry['id'] == native_model}
        plan_models = {entry['id'] for entry in catalog['data']
                       if entry['id'] != native_model}
        assert native_models == {native_model}, catalog
        assert plan_models == {product.model_alias}, catalog
        alias_a = product.model_alias
        assert attempts(product) == 0, 'discovery and Preview are not model attempts'
        status, body = request(product, alias_a, 'first request')
        assert status == 200, (status, body)
        assert b'fixture answer' in body
        assert attempts(product) == 1
        # An independent Plan B is published, then explicitly added to this Agent's scope.
        create = plan_change(product, 'create', 'plan-b', display_name='Independent Plan B')
        preview = product.preview('routing preview', {'change': create})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': create}, 'plan-b')
        plan_b = preview['plan_head']['reference']['plan_id']
        configure_model_settings_v2(
            product, [product.plan_id, plan_b], 'allow-b', product.plan_id,
            agent_id='agent_codex_default', native_model_mode='preserve_available')
        catalog, before_etag = product.catalog()
        assert {entry['id'] for entry in catalog['data']} == {
            alias_a, preview['plan_head']['model_alias'], native_model}, catalog
        aliases = {entry['id'] for entry in catalog['data']
                   if entry['id'] != native_model}
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            old = executor.submit(request, product, alias_a, 'hold-old-request')
            deadline = time.monotonic() + 15
            while not (product.cpa_fixture / 'held').exists():
                assert not old.done(), old.result()
                assert time.monotonic() < deadline, 'old request never reached upstream'
                time.sleep(.02)
            update = plan_change(product, 'update', display_name='New Plan A')
            preview = product.preview('routing preview', {'change': update})
            product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': update}, 'update-a')
            fresh, after_etag = product.catalog()
            assert {entry['id'] for entry in fresh['data']} == (
                aliases | {native_model}), 'lost Plan, alias, or native model'
            assert before_etag != after_etag
            status, body = request(product, alias_a, 'new publication request')
            assert status == 200, (status, body)
            (product.cpa_fixture / 'release').touch()
            status, body = old.result(timeout=15)
            assert status == 200 and b'fixture answer' in body, (status, body)
        assert attempts(product) == 3
        for alias, token in [('hiroute/ffffffffffffffff', None), (alias_a, 'invalid-bearer')]:
            status, _ = request(product, alias, 'must not reach upstream', token)
            assert status >= 400, status
        assert attempts(product) == 3, 'rejected request reached credential/upstream execution'
        print(json.dumps({'scenario': 'real-requests-across-publication', 'state': 'green',
                          'candidate': actual_sha,
                          'successful_attempts': 3, 'rejected_request_attempts': 0,
                          'retained_plans': 2, 'old_request_completed': True}), flush=True)
    finally:
        # Release an upstream hold even when an assertion failed, so cleanup stays bounded.
        if hasattr(product, 'cpa_fixture'):
            (product.cpa_fixture / 'release').touch()
        product.close()


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
