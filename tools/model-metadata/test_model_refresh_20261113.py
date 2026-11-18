"""Native-only model facts and exact Token Plan protocol associations."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class November13MetadataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())
        cls.data = json.loads((ROOT / 'assets/release-facts/current/bundle/official-model-data.json').read_text())['data']
        cls.metadata = cls.data['metadata_catalog']

    def test_hydrogen_shared_budget_does_not_become_a_global_sync_ceiling(self):
        model = next(m for m in self.metadata['canonical_models'] if m['model_key'] == 'gemini-4-hydrogen')
        self.assertEqual(model['context_tokens'], {'state': 'known', 'value': 1048576})
        self.assertEqual(model['max_output_tokens'], {'state': 'shared_total_budget', 'value': None})
        self.assertEqual(model['reasoning']['profiles'], ['low', 'medium', 'high', 'xhigh'])
        self.assertEqual(model['reasoning']['default'], 'high')
        self.assertTrue(all(state == 'supported' for state in model['modalities'].values()))
        self.assertEqual(set(model['modalities']), {'text_input', 'image_input', 'audio_input', 'video_input', 'pdf_input', 'text_output'})
        bindings = [b for b in self.metadata['endpoint_bindings'] if b['model_key'] == model['model_key']]
        self.assertEqual(len(bindings), 2)
        self.assertEqual({b['product_key'] for b in bindings}, {'gemini-api'})
        for binding in bindings:
            interface, = binding['interface_candidates']
            self.assertIn(interface, {'gemini-api/google-generate-content', 'gemini-api/gemini-interactions'})
            self.assertEqual(binding.get('capability_overrides', {}).get('max_output_tokens'),
                             65536 if interface.endswith('/google-generate-content') else None)
            self.assertEqual(binding['availability']['state'], 'conditional')
            self.assertEqual(binding['protocol_qualification'], 'runtime-required')
            self.assertIn('eligibility and quotas', binding['availability']['condition'])

    def test_hydrogen_paid_standard_price_does_not_grant_free_or_explicit_cache(self):
        price, = [p for p in self.data['pricing_facts'] if p['model_key'] == 'gemini-4-hydrogen']
        self.assertEqual((price['product_key'], price['currency'], price['unit']), ('gemini-api', 'USD', 'per-1m-tokens'))
        schedule, = price['schedules']
        self.assertEqual(schedule['condition']['billing_tier'], 'paid')
        self.assertEqual(schedule['condition']['tier'], 'standard')
        self.assertEqual(schedule['condition']['cache_mode'], 'implicit-only')
        self.assertEqual(schedule['rates'], {'input': '1.50', 'output': '7.50', 'cache-read': '0.075'})
        self.assertFalse(any('hydrogen' in m['model_configuration_id'] for m in self.data['models']))

    def test_hydrogen_native_provider_never_claims_installed_openai_transport(self):
        record, = [r for r in self.metadata['model_records'] if r['upstream_model_id'] == 'gemini-4-hydrogen']
        self.assertEqual(record['provider_record_key'], 'official/gemini-api-native')
        self.assertEqual(record['execution_fit']['state'], 'unsupported')
        self.assertEqual(record['field_provenance']['execution_fit'],
                         {'basis': 'inferred', 'rule_key': 'rule.google.hydrogen-native-transport/v1'})
        self.assertNotIn('output-limit-prefill', record['usable_for'])
        provider = next(p for p in self.metadata['provider_records'] if p['provider_record_key'] == record['provider_record_key'])
        self.assertEqual(provider['protocol_candidates'], [])
        self.assertNotIn('custom-api-endpoint-prefill', provider['usable_for'])
        product = next(p for p in self.metadata['access_products'] if p['product_key'] == 'gemini-api')
        self.assertNotIn('gemini-4-hydrogen', product['documented_upstream_model_ids'])

    def test_qwen4_recommendation_has_registered_connection_intents(self):
        featured_path = ROOT / 'assets/model-feed/featured.json'
        featured = json.loads(featured_path.read_text())[0]
        identity = featured['model_configuration_id']
        self.assertEqual(featured['headline']['en'], 'Qwen4-Max')
        model, = [m for m in self.data['models'] if m['model_configuration_id'] == identity]
        self.assertEqual(model['display_name'], 'Qwen4-Max')
        registry = json.loads((ROOT / 'assets/release-facts/current/bundle/connector-registry.json').read_text())
        capabilities = [c for c in self.data['model_endpoint_capabilities'] if c['model_configuration_id'] == identity]
        self.assertEqual(len(capabilities), 3)
        for plan, billing in [('payg', 'paid')]:
            profile = next(p for p in registry['endpoint_profiles']
                           if p['endpoint_profile_id'] == f'endpoint.bailian.{plan}.cn.v1')
            matching = [c for c in capabilities if c['endpoint_profile_id'] == profile['endpoint_profile_id']]
            self.assertEqual({c['protocol_endpoint_id'] for c in matching},
                             {e['protocol_endpoint_id'] for e in profile['protocol_endpoints']})
            for capability in matching:
                endpoint = next(e for e in profile['protocol_endpoints']
                                if e['protocol_endpoint_id'] == capability['protocol_endpoint_id'])
                self.assertEqual(capability['upstream_model_id'], 'qwen4-max')
                self.assertEqual(capability['required_adapter_ref'], endpoint['adapter_ref'])
                self.assertEqual(capability['upstream_protocol'], endpoint['protocol'])
            offer, = [o for o in self.data['offers'] if o['endpoint_profile_id'] == profile['endpoint_profile_id']]
            self.assertIn(identity, offer['model_configuration_ids'])
            self.assertEqual(offer['billing_class'], billing)
            self.assertEqual(offer['entitlement_id'], profile['entitlement_id'])
        payg = next(o for o in self.data['offers'] if o['offer_id'] == 'offer.bailian.payg.cn')
        self.assertIn('model.bailian.qwen3-max-2026-01-23', payg['model_configuration_ids'])
        # A feed can extend installed Offer membership, not install a new entitlement.
        self.assertFalse(any(identity in o['model_configuration_ids'] for o in self.data['offers']
                             if o['endpoint_profile_id'] != 'endpoint.bailian.payg.cn.v1'))
        # Length-dependent CNY rates and Token Credits retain their scoped contracts.
        self.assertFalse(any(r['model_configuration_id'] == identity for r in self.data['price_rates']))

    def test_qwen4_messages_binds_the_production_id_to_both_plans_and_payg(self):
        bindings = [b for b in self.metadata['endpoint_bindings'] if b['model_key'] == 'qwen-4-max'
                    and any(i.endswith('/anthropic-messages') for i in b['interface_candidates'])]
        self.assertEqual(len(bindings), 3)
        self.assertEqual({b['product_key'] for b in bindings},
                         {'bailian-payg-cn', 'bailian-token-personal-cn-beijing', 'bailian-token-team-cn-beijing'})
        for binding in bindings:
            self.assertEqual(binding['upstream_model_id'], 'qwen4-max')
            self.assertEqual(binding['availability']['state'], 'conditional')
            self.assertEqual(binding['protocol_qualification'], 'runtime-required')
            self.assertIn('output_config.effort defaults to high', binding['availability']['condition'])
            self.assertNotIn('preserve_thinking', binding['availability']['condition'])
            self.assertNotIn('reasoning_effort', binding['availability']['condition'])
            product = next(p for p in self.metadata['access_products'] if p['product_key'] == binding['product_key'])
            interface = next(i for i in product['interfaces'] if i['interface_key'] == binding['interface_candidates'][0])
            host = ('dashscope.aliyuncs.com' if product['product_key'] == 'bailian-payg-cn'
                    else 'token-plan.cn-beijing.maas.aliyuncs.com')
            self.assertEqual(interface['base_url'] + interface['request_path'], f'https://{host}/apps/anthropic/v1/messages')


if __name__ == '__main__':
    unittest.main()
