"""Saved model IDs, empty/excluded options and authenticated egress through real hirouted."""
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

import model_connections_product as native
import publication_product
from publication_product import Product


MODEL = '内网模型'
FEFF_MODEL = '\ufeff模型🧠\ufeff'
LEGACY_OPTION_FIELDS = {
    'claude_capabilities', 'context_window', 'suggested_alias', 'candidates',
    'free_suggestions', 'ratings', 'codex_capabilities', 'revisions',
}


def encoded(value):
    # Protected action digests use the production canonical UTF-8 JSON representation.
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


native.encoded = encoded
publication_product.encoded = encoded
native.MODEL = MODEL


def options(product, include_unavailable=None):
    payload = {'display_name': '日常开发', 'requirements': {}}
    if include_unavailable is not None:
        payload['include_unavailable'] = include_unavailable
    return native.control(product, 'GetPlanEditorOptions', payload)['data']


def assert_legacy_options(value):
    # Exact released response field set, including its explicit null optional fields.
    assert set(value) == LEGACY_OPTION_FIELDS, sorted(value)


def stable_compute_fields(value):
    # ListCompute evaluates presentation on each read; its timestamp is not durable state.
    stable = json.loads(json.dumps(value))
    for source in stable['sources']:
        for model in source['models']:
            del model['presentation']['evaluated_at_ms']
    return stable


def prepare_many(product, upstream, ids, suffix):
    candidate_ref = 'candidate/native/options-' + suffix
    product.secrets.add(native.NATIVE_TOKEN)
    product.register_protected_frame({
        'schema': 'hiroute.protected-input/v1',
        'registration_id': hashlib.sha256(candidate_ref.encode()).hexdigest(),
        'candidate_ref': candidate_ref,
        'candidate_revision': 1,
        'secret': native.NATIVE_TOKEN,
    })
    draft = {
        'inference_model_id': None,
        'candidate_ref': candidate_ref,
        'lineage_ref': 'lineage/native/options-' + suffix,
        'display_name': '不可用候选来源',
        'existing_source_id': None,
        'expected_source_revision': None,
        'edit_revision': 1,
        'check_id': 'check/native/options-' + suffix,
        'base_url': upstream.base_url,
        'base_kind': 'api_root',
        'request_path_override': None,
        'inventory_path_override': None,
        'protocol': 'responses',
        'protocol_profile_id': 'profile/custom/responses',
        'protocol_profile_revision': 1,
        'authentication': {'kind': 'bearer'},
        'additional_endpoints': [],
        'configuration_revision': 1,
        'models': [{
            'upstream_model_id': model_id,
            'display_name': '保留模型 ' + str(i),
            'catalog_configuration_id': None,
            'membership': 'user_declared',
            'capabilities': {
                'tool': native.declared(True), 'vision': native.declared(False),
                'streaming': native.declared(True),
                'context_tokens': native.declared(32768),
                'max_output_tokens': native.declared(4096),
                'native_reasoning': native.declared({'kind': 'fixed', 'profile': 'provider-default'}),
            },
        } for i, model_id in enumerate(ids)],
    }
    request = {'draft': draft, 'input_candidate': {
        'candidate_ref': candidate_ref, 'candidate_revision': 1,
    }}
    revisions = native.control(product, 'GetClientServiceStatus', {})['data']['revisions']
    grant = native.desktop_grant(product, 'CheckNativeModelConnection',
        'sha256:' + hashlib.sha256(encoded(request)).hexdigest(), revisions, suffix)
    return request, grant


def save_many(product, candidate, suffix):
    snapshot = native.control(product, 'ListCompute', {})['data']
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [model['model_ref'] for model in candidate['models']],
        'intent': 'save_ready', 'key_edits': [],
    }
    preview = native.control(product, 'PreviewComputeSave', {'change': change})['data']
    applied = native.control(product, 'ApplyComputeSave', {
        'spec': preview['spec'], 'accept_digest': preview['accept_digest'],
        'expected_revisions': preview['expected_revisions'], 'idempotency_key': suffix,
    })
    assert applied['data']['state'] == 'succeeded', applied
    saved = native.control(product, 'GetComputeSaveResult', {'operation': applied['operation']})['data']
    assert saved['disposition'] == 'saved', saved
    return saved


def disable(product, source_id):
    snapshot = native.control(product, 'ListCompute', {})['data']
    source = next(value for value in snapshot['sources'] if value['source_id'] == source_id)
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'saved_source', 'source_id': source_id},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [model['model_ref'] for model in source['models']],
        'intent': 'save_disabled', 'key_edits': [],
    }
    preview = native.control(product, 'PreviewComputeSave', {'change': change})['data']
    applied = native.control(product, 'ApplyComputeSave', {
        'spec': preview['spec'], 'accept_digest': preview['accept_digest'],
        'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'options-disable',
    })
    assert applied['data']['state'] == 'succeeded', applied


def ascii_case(product, upstream):
    saved = native.save_native_source(product, upstream, native.NATIVE_TOKEN,
        upstream_model_id='internal-model', variant='ascii-control',
        source_display_name='中文接入', model_display_name='内网模型')
    legacy = options(product)
    assert_legacy_options(legacy)
    assert len(legacy['candidates']) == 1, legacy
    assert legacy['candidates'][0]['binding_id'] == saved['binding_id']
    assert legacy['candidates'][0]['display_name'] == '内网模型'
    assert legacy['candidates'][0]['routable']
    return {'state': 'green', 'candidate_count': 1, 'chinese_display_with_ascii_id': True}


def unicode_case(product, upstream):
    saved = []
    for i, (model_id, display) in enumerate([(MODEL, '中文显示名'), ('外网模型', 'External'),
            ('internal-model', '内网模型'), (FEFF_MODEL, '格式字符与 Emoji'), ('\ufeff', '格式字符 ID'), ('模型🧠', 'Emoji ID')]):
        saved.append(native.save_native_source(product, upstream, native.NATIVE_TOKEN,
            upstream_model_id=model_id, variant='opaque-' + str(i),
            source_display_name='测试接入 ' + str(i), model_display_name=display))
        snapshot = native.control(product, 'ListCompute', {})['data']
        persisted = next(source for source in snapshot['sources'] if source['source_id'] == saved[-1]['source_id'])
        assert persisted['models'][0]['upstream_model_id'] == model_id
        if i == 1:
            only_unicode = options(product)
            assert {row['binding_id'] for row in only_unicode['candidates']} == {row['binding_id'] for row in saved}
            assert all(row['routable'] for row in only_unicode['candidates'])
    legacy = options(product)
    assert_legacy_options(legacy)
    by_binding = {row['binding_id']: row for row in legacy['candidates']}
    assert set(by_binding) == {row['binding_id'] for row in saved}, legacy
    assert by_binding[saved[0]['binding_id']]['display_name'] == '中文显示名'
    assert by_binding[saved[2]['binding_id']]['display_name'] == MODEL
    assert all(row['routable'] for row in by_binding.values())
    assert options(product, True)['candidates'] == legacy['candidates']
    before = native.control(product, 'ListCompute', {})['data']
    with upstream.lock:
        network_before = len(upstream.requests)
    invalid_ids = [' bad', 'bad\tmodel', 'bad\u0085model', 'bad\u0085', '界' * 171, '🧠' * 129]
    for i, invalid in enumerate(invalid_ids):
        request, grant = prepare_many(product, upstream, [invalid], 'invalid-' + str(i))
        native.control(product, 'CheckNativeModelConnection', request, grant, expected_error='INVALID_ARGUMENTS')
    after = native.control(product, 'ListCompute', {})['data']
    assert stable_compute_fields(after) == stable_compute_fields(before), 'Rejected checks changed durable compute state'
    with upstream.lock:
        assert len(upstream.requests) == network_before, 'Invalid ID was sent to the provider'
    native.publish_plan_and_agent(product, saved[0]['binding_id'], native_model_mode='hiroute_only')
    assert product.model_alias != MODEL, 'Route alias replaced the actual upstream ID'
    native.gateway_request(product)
    with upstream.lock:
        posts = [row for row in upstream.requests if row['method'] == 'POST']
    assert len(posts) == 1, posts
    assert posts[0]['body']['model'] == MODEL, posts
    assert posts[0]['authorization'] == 'Bearer ' + native.NATIVE_TOKEN
    assert posts[0]['api_key'] is None
    product.stop()
    product.start()
    assert {row['binding_id'] for row in options(product)['candidates']} == set(by_binding)
    return {'state': 'green', 'candidate_count': len(by_binding), 'authenticated_gateway_posts': 1,
            'invalid_ids_rejected_before_network': len(invalid_ids), 'reopened': True}


def feff_egress_case(product, upstream):
    saved = native.save_native_source(product, upstream, native.NATIVE_TOKEN,
        upstream_model_id=FEFF_MODEL, variant='feff-egress', model_display_name='格式字符与 Emoji 模型')
    legacy = options(product)
    assert_legacy_options(legacy)
    assert [row['binding_id'] for row in legacy['candidates']] == [saved['binding_id']]
    native.publish_plan_and_agent(product, saved['binding_id'], native_model_mode='hiroute_only')
    native.gateway_request(product)
    with upstream.lock:
        posts = [row for row in upstream.requests if row['method'] == 'POST']
    assert len(posts) == 1, posts
    assert posts[0]['body']['model'] == FEFF_MODEL
    assert posts[0]['authorization'] == 'Bearer ' + native.NATIVE_TOKEN
    return {'state': 'green', 'authenticated_gateway_posts': 1,
            'exact_feff_and_non_bmp_id_preserved': True}


def empty_case(product, upstream):
    legacy = options(product)
    assert_legacy_options(legacy)
    assert legacy['candidates'] == []
    assert options(product, True)['candidates'] == []
    request, grant = prepare_many(product, upstream, ['disabled-model'], 'empty-disabled')
    candidate = native.control(product, 'CheckNativeModelConnection', request, grant)['data']['candidate']
    saved = save_many(product, candidate, 'empty-disabled-save')
    disable(product, saved['source_id'])
    unavailable = options(product, True)
    assert unavailable['candidates'] == []
    assert [row['reason'] for row in unavailable['unavailable_candidates']] == ['source_not_ready']
    assert_legacy_options(options(product, False))
    try:
        native.publish_plan_and_agent(product, saved['bindings'][0]['binding_id'], native_model_mode='hiroute_only')
    except AssertionError:
        error = json.loads(product.outputs[-1]).get('error')
        assert error and error['code'] not in ('DAEMON_UNAVAILABLE', 'INTERNAL'), error
    else:
        raise AssertionError('Published an unavailable candidate')
    with upstream.lock:
        assert not upstream.requests, 'Disabled publication reached upstream'
    return {'state': 'green', 'zero_sources': True, 'all_unavailable': True, 'publication_rejected': True}


def many_unavailable_case(product, upstream):
    request, grant = prepare_many(product, upstream, ['disabled-' + str(i) for i in range(257)], 'many-disabled')
    candidate = native.control(product, 'CheckNativeModelConnection', request, grant)['data']['candidate']
    saved = save_many(product, candidate, 'many-disabled-save')
    assert len(saved['bindings']) == 257, saved
    disable(product, saved['source_id'])
    valid = native.save_native_source(product, upstream, native.NATIVE_TOKEN,
        upstream_model_id=MODEL, variant='many-live', model_display_name='仍可用模型')
    legacy = options(product)
    assert_legacy_options(legacy)
    assert [row['binding_id'] for row in legacy['candidates']] == [valid['binding_id']]
    explicit_old = options(product, False)
    assert_legacy_options(explicit_old)
    explained = options(product, True)
    assert len(explained['unavailable_candidates']) == 256, explained
    assert explained['unavailable_candidate_count'] == 257, explained
    assert all(row['reason'] == 'source_not_ready' for row in explained['unavailable_candidates'])
    native.publish_plan_and_agent(product, valid['binding_id'], native_model_mode='hiroute_only')
    native.gateway_request(product)
    return {'state': 'green', 'unavailable_total': 257, 'explanations_returned': 256,
            'valid_candidates': 1, 'publication_and_gateway_succeeded': True}


CASES = {'ascii-control': ascii_case, 'unicode': unicode_case, 'feff-egress': feff_egress_case,
         'empty': empty_case, 'many-unavailable': many_unavailable_case}


def run(repository, selected=None):
    repository = Path(repository).resolve()
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()
    cases = CASES if selected is None else {selected: CASES[selected]}
    for name, scenario in cases.items():
        product = Product(repository)
        native.MODEL = FEFF_MODEL if name == 'feff-egress' else MODEL
        upstream = native.NativeUpstream()
        product.enable_debug_diagnostics()
        # Codex's native metadata remains its actual bundled catalog. The new HiRoute route
        # exposes the opaque upstream ID through its alias without inventing native rights.
        product.install_codex_fixture()
        start = time.monotonic()
        try:
            product.start()
            result = scenario(product, upstream)
            product.stop()
            diagnostics = product.diagnostics_snapshot()
            assert diagnostics['state'] == 'complete', diagnostics
            assert diagnostics['level_applied']['level'] == 'debug', diagnostics
            assert diagnostics['level_applied']['source'] == 'smoke_override', diagnostics
            print(json.dumps({'scenario': 'route-options-' + name, 'candidate': sha,
                'seconds': round(time.monotonic() - start, 3), **result,
                'diagnostics': {key: diagnostics[key] for key in (
                    'state', 'current_boot', 'level_applied', 'record_count', 'invalid_record_count')},
                }, ensure_ascii=False), flush=True)
        except BaseException:
            evidence = repository / 'target/product-e2e-evidence' / ('route-options-' + name + '-' + str(int(time.time_ns())))
            print(json.dumps({'scenario': 'route-options-' + name, 'state': 'red', 'candidate': sha,
                'diagnostics': product.preserve_diagnostics(evidence)}, ensure_ascii=False), flush=True)
            raise
        finally:
            upstream.close()
            product.close()
            native.MODEL = MODEL


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
