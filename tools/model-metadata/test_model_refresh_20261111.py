"""Product-scoped metadata survives deterministic client/feed projection."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
NEW_MODELS = {
    'qwen-4-max', 'minimax-m3', 'minimax-m3-1-flash',
    'glm-5-3-flashx-global',
    'deepseek-v4-1-flash-1017', 'gemini-4-argon', 'gemini-4-carbon', 'grok-4-7',
}


class November11ModelProjectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())
        cls.snapshot = json.loads((ROOT / 'assets/release-facts/current/bundle/official-model-data.json').read_text())
        cls.data = cls.snapshot['data']
        cls.metadata = cls.data['metadata_catalog']
        cls.models = {m['model_key']: m for m in cls.metadata['canonical_models']}
        cls.records = {r['model_record_key']: r for r in cls.metadata['model_records']}
        cls.products = {p['product_key']: p for p in cls.metadata['access_products']}
        cls.source_records = {r['model_record_key']: r for r in cls.source['model_metadata_records']}

    def prices(self, model, product=None):
        return [p for p in self.data['pricing_facts'] if p['model_key'] == model
                and (product is None or p['product_key'] == product)]

    def bindings(self, model):
        return [b for b in self.metadata['endpoint_bindings'] if b['model_key'] == model]

    def test_new_directory_facts_are_not_executable_model_promotions(self):
        self.assertTrue(NEW_MODELS.issubset(self.models))
        for binding in self.metadata['endpoint_bindings']:
            if binding['model_key'] in NEW_MODELS:
                self.assertEqual(binding['availability']['state'], 'conditional')
                self.assertEqual(binding['protocol_qualification'], 'runtime-required')
        for model in self.data['models']:
            if model['model_configuration_id'] == 'model.catalog.qwen-4-max':
                continue
            self.assertFalse(any(key in model['model_configuration_id'] for key in NEW_MODELS))

    def test_qwen4_production_identity_replaces_preview_across_the_snapshot(self):
        m = self.models['qwen-4-max']
        self.assertEqual(m['lifecycle'], 'active')
        self.assertEqual(m['upstream_ids'], ['qwen4-max'])
        self.assertEqual(m['context_tokens'], {'state': 'known', 'value': 1000000})
        self.assertEqual(m['max_output_tokens'], {'state': 'known', 'value': 262144})
        self.assertIn('thinking and final answer combined', m['max_output_scope'])
        bindings = self.bindings('qwen-4-max')
        products = {'bailian-payg-cn', 'bailian-token-personal-cn-beijing', 'bailian-token-team-cn-beijing'}
        self.assertEqual(len(bindings), 9)
        self.assertEqual({b['product_key'] for b in bindings}, products)
        for b in bindings:
            self.assertEqual(b['upstream_model_id'], 'qwen4-max')
        for product in products:
            self.assertIn('qwen4-max', self.products[product]['documented_upstream_model_ids'])
            r = self.records[f'official/{product}/qwen4-max']
            for field in ('context_tokens', 'max_output_tokens'):
                self.assertEqual(r[field], {'state': 'runtime_required', 'value': None})
            self.assertNotIn('request-limit-prefill', r['usable_for'])
            self.assertNotIn('output-limit-prefill', r['usable_for'])
        for artifact in (self.source, self.data):
            encoded = json.dumps(artifact)
            for obsolete in ('qwen-4-max-preview', 'qwen4-max-preview', 'qwen4.0-max-preview'):
                self.assertNotIn(obsolete, encoded)

    def test_qwen4_wire_budgets_and_history_are_not_interchanged(self):
        r = self.records['official/bailian-token-personal-cn-beijing/qwen4-max']
        fact = self.source_records[r['model_record_key']]['source_assertions'][0]
        self.assertEqual(fact['token_limits_by_thinking_mode'], {
            'thinking': {'max_input_tokens': 983616, 'max_output_tokens': 262144,
                         'output_includes': ['thinking', 'answer']},
            'non-thinking': {'max_input_tokens': 991808, 'max_output_tokens': 131072,
                             'output_includes': ['answer']},
        })
        protocols = {
            'openai-chat': ('reasoning_effort', ['max_tokens', 'max_completion_tokens']),
            'openai-responses': ('reasoning.effort', ['max_output_tokens']),
            'anthropic-messages': ('output_config.effort', ['max_tokens']),
        }
        for protocol, (parameter, budget_fields) in protocols.items():
            api = fact['api_contracts'][protocol]
            self.assertEqual(api['effort_parameter'], parameter)
            self.assertEqual(api['output_budget_parameters'], budget_fields)
            self.assertEqual(api['canonical_efforts'], ['low', 'high', 'xhigh'])
            self.assertEqual(api['default_effort'], 'high')
            aliases = {'medium': 'high', 'max': 'xhigh'}
            if protocol != 'anthropic-messages':
                aliases['minimal'] = 'low'
            self.assertEqual(api['aliases'], aliases)
        chat = fact['api_contracts']['openai-chat']
        self.assertEqual(chat['default_thinking_budget'], 131072)
        self.assertTrue(chat['preserve_thinking_default'])
        self.assertTrue(chat['history_must_remain_separate_from_content'])
        self.assertEqual(chat['history_field'], 'reasoning_content')

    def test_minimax_chat_ceiling_does_not_leak_to_other_protocols(self):
        for key in ('minimax-m3', 'minimax-m3-1-flash'):
            self.assertEqual(self.models[key]['context_tokens']['value'], 1000000)
            self.assertIsNone(self.models[key]['max_output_tokens']['value'])
            for binding in self.bindings(key):
                protocol = binding['interface_candidates'][0].split('/')[-1]
                override = binding.get('capability_overrides', {}).get('max_output_tokens')
                self.assertEqual(override, 524288 if protocol == 'openai-chat' else None)
        flash = self.models['minimax-m3-1-flash']['reasoning']
        self.assertEqual(flash['default'], 'max')
        self.assertNotIn('none', flash['profiles'])
        self.assertIn('Messages and Responses default off', self.models['minimax-m3']['reasoning']['note'])

    def test_minimax_prices_stay_global_and_tier_specific(self):
        price, = self.prices('minimax-m3-1-flash', 'minimax-payg-global')
        self.assertEqual((price['product_key'], price['currency']), ('minimax-payg-global', 'USD'))
        self.assertEqual(price['schedules'][0]['rates'], {'input': '0.15', 'output': '0.60', 'cache-read': '0.015'})
        price, = self.prices('minimax-m3', 'minimax-payg-global')
        rates = {s['schedule_key']: s for s in price['schedules']}
        self.assertEqual(rates['priority-long']['rates']['input'], '0.90')
        self.assertEqual(rates['standard-long']['rates']['cache-read'], '0.12')
        self.assertTrue(all(s['condition']['threshold_token_unit'] == 'unresolved' for s in rates.values()))

    def test_gemini_default_is_not_a_hard_output_cap_or_free_cache(self):
        for variant in ('argon', 'carbon'):
            key = 'gemini-4-' + variant
            self.assertEqual(self.models[key]['context_tokens']['value'], 1048576)
            self.assertEqual(self.models[key]['max_output_tokens'], {'state': 'shared_total_budget', 'value': None})
            record = self.records[f'official/gemini-api/{key}']
            self.assertNotIn('output-limit-prefill', record['usable_for'])
            self.assertEqual(self.source_records[record['model_record_key']]['source_assertions'][0]['default_output_tokens'], 65536)
            self.assertEqual(self.source_records[record['model_record_key']]['source_assertions'][0]['long_decode_interface'], 'gemini-interactions')
            price, = self.prices(key)
            schedules = {s['schedule_key']: s for s in price['schedules']}
            self.assertEqual(schedules['standard-through-2026-11-30']['rates']['cache-read'], '0.10')
            self.assertEqual(schedules['standard-from-2026-12-01']['rates']['input'], '4')
            for s in schedules.values():
                self.assertEqual(s['condition']['timezone_state'], 'not-published')
                self.assertEqual(s['condition']['cache_mode'], 'implicit-only')
                self.assertNotIn('cache-write', s['rates'])

    def test_grok_keeps_published_thresholds_without_assumed_integer_units(self):
        for key, low, high in [('grok-4-7', '< 200k', '>= 200k')]:
            price, = self.prices(key)
            schedules = {s['schedule_key']: s for s in price['schedules']}
            self.assertEqual(schedules['standard-short-context']['condition']['provider_threshold'], low)
            self.assertEqual(schedules['standard-long-context']['condition']['provider_threshold'], high)
            for schedule in schedules.values():
                self.assertEqual(schedule['condition']['threshold_token_unit'], 'unresolved')
                self.assertEqual(schedule['application'], 'whole-request')
                self.assertNotIn('maximum_input_tokens', schedule['condition'])
                self.assertNotIn('minimum_input_tokens', schedule['condition'])

    def test_scope_deferral_does_not_add_specialist_or_extra_vendor_bindings(self):
        deferred = {'muse-wonder-1-0', 'muse-ember-1-0', 'muse-spark-1-3',
                    'mistral-large-4', 'hunyuan-hy4', 'glm-5-3-flashx-legal-global'}
        self.assertTrue(deferred.isdisjoint(self.models))
        self.assertFalse(any(b['model_key'] in deferred for b in self.metadata['endpoint_bindings']))
        self.assertFalse(any(p['model_key'] in deferred for p in self.data['pricing_facts']))
        self.assertIn('hunyuan-hy4-preview', self.models)
        # Previously published client-scoped Muse records are preserved.
        self.assertIn('openclaw/meta/muse-spark-1.3', self.records)

    def test_flashx_stays_general_api_without_assumed_integer_limits(self):
        model = self.models['glm-5-3-flashx-global']
        self.assertIsNone(model['context_tokens']['value'])
        self.assertIsNone(model['max_output_tokens']['value'])
        self.assertTrue(all(b['product_key'] in {'zai-general-global', 'zhipu-general-cn'}
                            for b in self.bindings('glm-5-3-flashx-global')))

    def test_deepseek_dated_revision_has_product_and_protocol_scoped_bindings(self):
        model = self.models['deepseek-v4-1-flash-1017']
        self.assertEqual(model['context_tokens']['state'], 'unknown')
        self.assertEqual(model['max_output_tokens']['state'], 'unknown')
        self.assertIsNone(model['reasoning']['default'])
        self.assertEqual(model['reasoning']['profiles'], [])
        bindings = self.bindings('deepseek-v4-1-flash-1017')
        self.assertEqual({b['product_key'] for b in bindings}, {
            'bailian-payg-cn', 'bailian-token-personal-cn-beijing', 'bailian-token-team-cn-beijing'})
        for binding in bindings:
            product = self.products[binding['product_key']]
            self.assertIn('deepseek-v4.1-flash-1017', product['documented_upstream_model_ids'])
            interfaces = {i['interface_key']: i for i in product['interfaces']}
            self.assertEqual({interfaces[i]['protocol'] for i in binding['interface_candidates']},
                             {'openai-chat', 'openai-responses', 'anthropic-messages'})
            if binding['product_key'] == 'bailian-payg-cn':
                face = interfaces['bailian-payg-cn/anthropic-messages-workspace-beijing']
                self.assertEqual(face['base_url_template'],
                                 'https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/apps/anthropic')
                self.assertEqual(face['request_path'], '/v1/messages')

    def test_qwen4_beijing_payg_rates_and_token_plan_credits_remain_product_scoped(self):
        price, = self.prices('qwen-4-max', 'bailian-payg-cn')
        self.assertEqual((price['product_key'], price['currency'], price['unit']),
                         ('bailian-payg-cn', 'CNY', 'per-1m-tokens'))
        standard, cache_5m, cache_1h = price['schedules']
        self.assertEqual(standard['rates'], {'input': '10', 'output': '30', 'cache-read': '0.5'})
        for schedule, ttl, rate in ((cache_5m, 300, '12.5'), (cache_1h, 3600, '20')):
            self.assertEqual(schedule['condition']['cache_ttl_seconds'], ttl)
            self.assertEqual(schedule['rates'], {'cache-write': rate})
        self.assertTrue(all(s['condition']['region'] == 'cn-beijing' for s in price['schedules']))
        for tier in ('personal', 'team'):
            product = self.products[f'bailian-token-{tier}-cn-beijing']
            self.assertFalse(self.prices('qwen-4-max', product['product_key']))
            self.assertTrue(any(fact.startswith('qwen4-max uses ') for fact in product['quota_facts']))
            record = self.source_records[f"official/{product['product_key']}/qwen4-max"]
            billing = record['source_assertions'][0]['billing']
            self.assertEqual(billing, {'unit': 'credits', 'components': ['input', 'cache', 'output'],
                                       'model_conversion': 'runtime-required'})

    def test_callable_ids_and_lifecycle_survive_projection(self):
        old = self.models['mistral-large-3']
        self.assertEqual(old['lifecycle'], 'deprecated')
        self.assertEqual(old['upstream_ids'], ['mistral-large-2512'])
        self.assertFalse(any(b['upstream_model_id'] == 'mistral-large-3-25-12' for b in self.metadata['endpoint_bindings']))
        self.assertIn('qwen3.8-max-0902', self.models['qwen-3-8-max']['upstream_ids'])
        self.assertIn('deepseek-v4.1-flash-1017', self.models['deepseek-v4-1-flash-1017']['upstream_ids'])

    def test_projection_retains_prices_and_existing_manual_capable_models(self):
        self.assertEqual(self.data['pricing_facts'], self.source['pricing_facts'])
        self.assertEqual(self.models['gpt-6.1-terra']['max_output_tokens']['value'], 128000)
        self.assertEqual(self.models['kimi-k3-1-pro']['model_key'], 'kimi-k3-1-pro')
        self.assertLessEqual((ROOT / 'assets/model-data/current/model-data.json').stat().st_size, 8 * 1024 * 1024)

    def test_reviewed_evidence_does_not_freshen_old_sources(self):
        sources = {s['source_key']: s for s in self.source['evidence_sources']}
        self.assertEqual(sources['source-001']['collected_on'], '2026-09-13')
        self.assertEqual(sources['source-214']['collected_on'], '2026-11-10')
        for key in ('source-minimax-pricing-current', 'source-mistral3-lifecycle', 'source-gemini4-carbon'):
            self.assertEqual(sources[key]['collected_on'], '2026-11-11')
            self.assertEqual(sources[key]['digest_state'], 'captured')
            self.assertRegex(sources[key]['bytes_digest'], r'^sha256:[0-9a-f]{64}$')
        self.assertEqual(sources['source-qwen4-max-model']['collected_on'], '2026-11-16')
        self.assertEqual(sources['source-qwen4-max-model']['digest_state'], 'captured')


if __name__ == '__main__':
    unittest.main()
