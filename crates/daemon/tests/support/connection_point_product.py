"""Released CLI -> real daemon -> exact multi-model save, using passive native fixtures."""
import json
from pathlib import Path
import sys
from publication_product import Product, encoded


def write(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(value if isinstance(value, bytes) else encoded(value))
    path.chmod(0o600)


def data(product, command, payload=None):
    return product.public_cli(command, payload)[1]['data']


def prepare(product, point, name):
    return data(product, 'compute connection prepare', {
        'discovery': point['discovery'], 'prepare_id': name,
    })


def preview(product, candidate, selected):
    snapshot = data(product, 'compute list')
    return data(product, 'compute connection preview', {'change': {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'], 'selected_model_refs': selected,
        'intent': 'save_ready', 'key_edits': [],
    }})


def apply(product, draft, key, success=True):
    return product.public_cli('compute connection apply', {
        'spec': draft['spec'], 'accept_digest': draft['accept_digest'],
        'expected_revisions': draft['expected_revisions'], 'idempotency_key': key,
    }, success=success)


def run(repository):
    product = Product(repository)
    try:
        hermes = product.root / 'selected-hermes-profile'
        hermes.mkdir(mode=0o700)
        hermes_config = {'model': {'provider': 'team-native', 'default': 'first'}, 'providers': {
            'team-native': {'api': 'https://point.example.invalid/v1', 'transport': 'openai_chat',
                            'key_env': 'POINT_PROFILE_KEY', 'models': {'first': {}, 'second': {}}},
            'china-coding': {'api': 'https://open.bigmodel.cn/api/coding/paas/v4', 'transport': 'openai_chat',
                             'key_env': 'POINT_PROFILE_KEY', 'default_model': 'glm-5.3'}}}
        write(hermes / 'config.yaml', hermes_config)
        write(hermes / '.env', b'POINT_PROFILE_KEY=point-profile-fixture-secret\nALTERNATIVE_KEY=point-profile-fixture-secret\n')
        product.secrets.add('point-profile-fixture-secret')
        pi = product.root / 'home/.pi/agent/models.json'
        write(pi, {'providers': {'team-native': {
            'baseUrl': 'https://point.example.invalid/v1', 'api': 'openai-responses', 'apiKey': 'pi-point-fixture-secret',
            'models': [{'id': 'first'}, {'id': 'second'}, {'id': 'unavailable', 'contextWindow': 0}],
        }}})
        product.secrets.add('pi-point-fixture-secret')
        before = {path: path.read_bytes() for path in [hermes / 'config.yaml', hermes / '.env', pi]}
        product.start()
        request = {'schema': 'hiroute.compute-scan-request/v2',
                   'native_target': {'kind': 'hermes', 'config_root': str(hermes)}}
        scan = data(product, 'compute scan', request)
        assert scan['schema'] == 'hiroute.compute-scan-result/v2'
        hermes_point = next(p for p in scan['connections'] if p['agent_id'] == 'agent_hermes_default' and p['native_provider_id'] == 'team-native')
        pi_point = next(p for p in scan['connections'] if p['agent_id'] == 'agent_pi_default')
        assert hermes_point['default_model_id'] == 'first'
        assert hermes_point['declared_model_ids'] == ['first', 'second']
        assert hermes_point['protocol'] == 'chat_completions'
        assert hermes_point['connection_ref'] != pi_point['connection_ref']
        other = product.root / 'second-hermes-profile'
        other.mkdir(mode=0o700)
        write(other / 'config.yaml', {'model': {'provider': 'team-native', 'default': 'other-profile-model'}, 'providers': {
            'team-native': {'api': 'https://point.example.invalid/v1', 'transport': 'openai_chat',
                            'key_env': 'POINT_PROFILE_KEY', 'default_model': 'other-profile-model'}}})
        write(other / '.env', b'POINT_PROFILE_KEY=point-profile-fixture-secret\n')
        other_request = {'schema': request['schema'], 'native_target': {'kind': 'hermes', 'config_root': str(other)}}
        other_scan = data(product, 'compute scan', other_request)
        other_points = [p for p in other_scan['connections'] if p['agent_id'] == 'agent_hermes_default']
        assert len(other_points) == 1 and other_points[0]['declared_model_ids'] == ['other-profile-model']
        assert other_points[0]['connection_ref'] != hermes_point['connection_ref']
        invalid_scan = data(product, 'compute scan', {'schema': request['schema'],
            'native_target': {'kind': 'hermes', 'config_root': str(product.root / 'missing-profile')}})
        assert not any(p['agent_id'] == 'agent_hermes_default' for p in invalid_scan['connections'])
        assert any(d['reason'] == 'selected_profile_unavailable' for d in invalid_scan['diagnostics'])
        assert len(pi_point['declared_model_ids']) == 3
        for point in [hermes_point, pi_point]:
            assert point['discovery'] and 'credential_binding' not in point and 'field_selector' not in point
        for version in ['hiroute.compute-scan-request/v3', 'unrecognized']:
            status, _ = product.public_cli('compute scan', {'schema': version}, success=False)
            assert status != 0
        legacy = data(product, 'compute scan')
        assert legacy['schema'] == 'hiroute.compute-scan-result/v1' and 'connections' not in legacy
        legacy_pi = next(p for p in legacy['items'] if p['agent_id'] == 'agent_pi_default' and p.get('observed_model_id') == 'first')
        single = prepare(product, legacy_pi, 'prepare/point-product/legacy')
        assert [m['upstream_model_id'] for m in single['models']] == ['first']
        multiple = prepare(product, pi_point, 'prepare/point-product/multiple')
        assert {m['upstream_model_id'] for m in multiple['models']} == {'first', 'second', 'unavailable'}
        assert sum(m['selectable'] for m in multiple['models']) == 2
        candidate = prepare(product, hermes_point, 'prepare/point-product/hermes')
        assert {m['upstream_model_id'] for m in candidate['models']} == {'first', 'second'}
        assert all(m['membership'] == 'observed' for m in candidate['models'])
        assert all(path.read_bytes() == contents for path, contents in before.items()), 'Scan/prepare mutated a native source'
        assert not data(product, 'compute list')['sources'], 'Scan/prepare saved a source'
        coding_point = next(p for p in scan['connections'] if p.get('native_provider_id') == 'china-coding')
        assert coding_point['connection_option_id'] == 'zhipu.coding-chat.cn.v1'
        assert coding_point['declared_model_ids'] == ['glm-5.3']
        coding = prepare(product, coding_point, 'prepare/point-product/coding')
        assert {m['upstream_model_id'] for m in coding['models']} == {'glm-5.3', 'glm-5.3-flash'}
        assert all(m['selectable'] for m in coding['models'])
        assert next(m for m in coding['models'] if m['upstream_model_id'] == 'glm-5.3')['membership'] == 'observed'
        assert next(m for m in coding['models'] if m['upstream_model_id'] == 'glm-5.3-flash')['membership'] == 'catalog'
        selection = [m['model_ref'] for m in candidate['models'] if m['selectable']]
        draft = preview(product, candidate, selection[:1])
        # Changing only credential selection provenance still invalidates the snapshot.
        hermes_config['providers']['team-native']['key_env'] = 'ALTERNATIVE_KEY'
        write(hermes / 'config.yaml', hermes_config)
        status, _ = apply(product, draft, 'point-source-stale', success=False)
        assert status != 0 and not data(product, 'compute list')['sources']
        fresh = data(product, 'compute scan', request)
        point = next(p for p in fresh['connections'] if p['agent_id'] == 'agent_hermes_default' and p['native_provider_id'] == 'team-native')
        candidate = prepare(product, point, 'prepare/point-product/fresh')
        selection = [m['model_ref'] for m in candidate['models'] if m['selectable']]
        draft = preview(product, candidate, selection)
        admitted = apply(product, draft, 'point-exact-save')[1]['data']
        assert admitted['state'] == 'succeeded'
        snapshot = data(product, 'compute list')
        assert len(snapshot['sources']) == 1
        source = snapshot['sources'][0]
        assert {m['upstream_model_id'] for m in source['models']} == {'first', 'second'}
        assert source['target']['upstream_protocol'] == 'chat_completions'
        product.stop()
        product.start()
        snapshot = data(product, 'compute list')
        assert len(snapshot['sources']) == 1 and snapshot['sources'][0]['source_id'] == source['source_id']
        assert {m['upstream_model_id'] for m in snapshot['sources'][0]['models']} == {'first', 'second'}
        write(hermes / 'config.yaml', {**hermes_config, 'providers': {'team-native': {
            **hermes_config['providers']['team-native'], 'models': {'first': {}, 'second': {}, 'newly-added': {}}}}})
        data(product, 'compute scan', request)
        assert {m['upstream_model_id'] for m in data(product, 'compute list')['sources'][0]['models']} == {'first', 'second'}
        write(hermes / 'config.yaml', hermes_config)
        # Hermes' ordinary custom-provider setup keeps the selected connection in
        # model itself, with no providers map. Exercise that actual native shape
        # through scan, prepare, exact save and routing eligibility.
        bare = product.root / 'bare-custom-hermes-profile'
        write(bare / 'config.yaml', {'model': {
            'provider': 'custom', 'default': 'glm-5.3',
            'base_url': 'https://open.bigmodel.cn/api/coding/paas/v4',
            'api_mode': 'chat_completions', 'api_key': 'bare-custom-fixture-secret'}})
        product.secrets.add('bare-custom-fixture-secret')
        bare_before = (bare / 'config.yaml').read_bytes()
        refreshed = data(product, 'compute scan', {'schema': request['schema'],
            'native_target': {'kind': 'hermes', 'config_root': str(bare)}})
        coding_points = [p for p in refreshed['connections'] if p['agent_id'] == 'agent_hermes_default']
        assert len(coding_points) == 1
        coding_point = coding_points[0]
        assert coding_point['native_provider_id'] == 'custom'
        assert coding_point['default_model_id'] == 'glm-5.3'
        assert coding_point['connection_option_id'] == 'zhipu.coding-chat.cn.v1'
        coding = prepare(product, coding_point, 'prepare/point-product/routable-coding')
        chosen = [m['model_ref'] for m in coding['models'] if m['selectable']]
        assert apply(product, preview(product, coding, chosen), 'point-coding-exact-save')[1]['data']['state'] == 'succeeded'
        saved = next(s for s in data(product, 'compute list')['sources'] if {m['upstream_model_id'] for m in s['models']} == {'glm-5.3', 'glm-5.3-flash'})
        assert (bare / 'config.yaml').read_bytes() == bare_before
        options = data(product, 'routing options', {})['candidates']
        for model in saved['models']:
            option = next(o for o in options if o['binding_id'] == model['binding_id'])
            assert option['routable'] and {'responses', 'chat_completions', 'messages'} <= set(option['ingress_protocols'])
            assert option['reasoning']['profiles'] == ['low', 'high', 'max']
        print(json.dumps({'scenario': 'released-cli-connection-points', 'state': 'green',
                          'cases': ['legacy-one-model', 'private-multi-model-prepare', 'hermes-default-chat',
                                    'unselectable-member', 'equal-secret-selector-drift', 'exact-save-restart', 'no-refresh-expansion',
                                    'bare-custom-native-and-catalog-models-are-routable']}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]))
