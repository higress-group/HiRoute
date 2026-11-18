"""Real form prefill -> protected check/save -> published Gateway input/output budgets."""
import json
import os
from pathlib import Path
import subprocess
import sys

from model_feed_product import restart
from publication_process import judgment_fixture
from recommended_model_product import Provider, TrialProduct
from standalone_headless_product import apply_from_preview, apply_plan, plan_change, wire


def frontend(repository, value):
    bridge = repository / 'apps/desktop/tests/registered-input-limits-fixture.mjs'
    result = subprocess.run(['node', '--experimental-strip-types', str(bridge)],
                            input=wire(value), capture_output=True, check=True, timeout=30)
    return json.loads(result.stdout)


def capability(plan, binding, protocol):
    compiled = plan['compiled']
    candidate, = [c for group in compiled['groups'] for c in group['candidates'] if c['binding_id'] == binding]
    profile, = [p for p in candidate['protocols'] if p['ingress_protocol'] == protocol]
    return profile['capability']


def assert_limits(value, model, expected_input, expected_output):
    assert value['native_model'] == model, value
    assert value['max_input_tokens'] == {'state': 'exact', 'value': expected_input}, value
    assert value['max_output_tokens'] == {'state': 'exact', 'value': expected_output}, value
    assert value['max_total_tokens'] == {'state': 'exact', 'value': expected_input + expected_output}, value


def run(repository):
    product = TrialProduct(repository)
    provider = Provider(product.root, hosts=('api.openai.com',), models=('gpt-6.2-astra', 'gpt-6.2-sol'))
    stage = 'install'
    observations = []
    try:
        for name in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
                     'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy'):
            product.env.pop(name, None)
        product.env.update(LD_PRELOAD=str(provider.transport), SSL_CERT_FILE=str(provider.ca),
                           HIROUTE_RECOMMENDATION_TLS_PORT=str(provider.direct.server_port),
                           HIROUTE_E2E_DIAL_CONFIG=str(provider.dial))
        product.install()
        product.start()
        options = product.cli('compute', 'connection', 'options')['data']
        cases = frontend(repository, {'stage': 'prefill', 'options': options})
        assert len(cases) == 8, cases
        for case in cases:
            stage = case['key']
            draft = case['draft']
            protected = {'candidate_ref': draft['candidate_ref'], 'candidate_revision': 1}
            product.register_secret(draft['candidate_ref'], 'isolated-input-limits-key')
            product.secrets.add('isolated-input-limits-key')
            network = {'expected_network_revision': 0, 'policy': {'mode': 'direct'}}
            checked = product.cli('compute', 'connection', 'test-network', '--request-stdin', payload={
                'kind': 'native', 'request': {'request': {'draft': draft, 'input_candidate': protected},
                                             'network_edit': network}})['data']['check']
            assert checked['inference'] == 'verified', checked
            member, = [m for m in checked['candidate']['models'] if m['upstream_model_id'] == case['modelId']]
            assert member['selectable'], member
            change = frontend(repository, {'stage': 'save', 'save': {
                'result': checked, 'expectedRevisions': product.cli('compute', 'list')['data']['revisions'],
                'selectedModelRefs': [member['model_ref']], 'enable': True, 'protectedInput': protected}})
            preview = product.cli('compute', 'connection', 'preview-network', '--request-stdin', payload={
                'schema': 'hiroute.compute-management-change/v4', 'change': change, 'network_edit': network})['data']
            applied = apply_from_preview(product, ('compute', 'connection', 'apply'), preview, stage)
            assert applied['operation']['state'] == 'succeeded', applied
            source, = [s for s in product.cli('compute', 'list')['data']['sources'] if s['display_name'] == stage]
            saved, = source['models']
            expected_input = 800_000 if case['manual'] else 922_000
            expected_output = 64_000 if case['manual'] else 128_000
            assert saved['capabilities']['context_tokens'] == {'value': expected_input, 'basis': 'user_declared'}, saved
            assert saved['capabilities']['max_output_tokens'] == {'value': expected_output, 'basis': 'user_declared'}, saved
            editor = {
                'schema': 'hiroute.plan-editor/v2', 'display_name': stage, 'purpose': 'Verify exact input and output budgets',
                'mode': 'fixed_model', 'candidates': [{'binding_id': saved['binding_id']}],
                'smart': {'economy': [], 'primary': [], 'judgment': judgment_fixture(), 'reselect_on_user_message': False,
                          'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
                'free': {'candidates': [], 'primary': [], 'primary_fallback': False}, 'requirements': {},
                'delegation_enabled': False,
                'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000, 'attempt_timeout_ms': 30000,
                           'context_window_tokens': 750_000 if case['manual'] else 900_000},
            }
            created, _, _ = apply_plan(product, plan_change('create', editor, creation_key=stage), stage + '-publish')
            plan_id = created['plan_head']['reference']['plan_id']
            frozen = product.cli('routing', 'show', plan_id)['data']
            assert frozen['head']['reference'] == created['plan_version']['reference'], frozen
            limits = capability(created['plan_version'], saved['binding_id'], case['protocol'])
            assert_limits(limits, case['modelId'], expected_input, expected_output)
            observations.append({'case': stage, 'model': case['modelId'], 'protocol': case['protocol'],
                                 'input': expected_input, 'output': expected_output, 'capability': limits,
                                 'plan': plan_id, 'reference': frozen['head']['reference']})
        restart(product)
        for row in observations:
            frozen = product.cli('routing', 'show', row['plan'])['data']
            assert frozen['head']['reference'] == row['reference'], row
        assert {r['model'] for r in provider.requests if r['method'] == 'POST'} == {'gpt-6.2-astra', 'gpt-6.2-sol'}
        print(json.dumps({'scenario': 'registered-input-limits', 'state': 'green', 'revision': product.sha,
                          'cases': [{k: v for k, v in row.items() if k not in ('capability', 'plan', 'reference')} for row in observations]}), flush=True)
    except Exception:
        print(json.dumps({'scenario': 'registered-input-limits', 'state': 'red', 'revision': product.sha, 'stage': stage}), flush=True)
        raise
    finally:
        try:
            product.close()
        finally:
            provider.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]).resolve())
