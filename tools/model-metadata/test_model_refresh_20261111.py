"""Product-scoped metadata survives deterministic client/feed projection."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
NEW_MODELS = {
    'qwen-4-max-preview', 'mistral-large-4', 'minimax-m3', 'minimax-m3-1-flash',
    'glm-5-3-flashx-global', 'glm-5-3-flashx-legal-global', 'hunyuan-hy4',
    'deepseek-v4-1-flash-1017', 'gemini-4-argon', 'gemini-4-carbon', 'grok-4-7',
    'muse-wonder-1-0', 'muse-ember-1-0', 'muse-spark-1-3',
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
        self.assertEqual(len(self.data['models']), 30)
        for model in self.data['models']:
            self.assertFalse(any(key in model['model_configuration_id'] for key in NEW_MODELS))

    def test_qwen4_is_a_timed_credits_preview_without_payg_prices(self):
        m = self.models['qwen-4-max-preview']
        self.assertEqual(m['lifecycle'], 'preview')
        self.assertEqual(m['context_tokens'], {'state': 'unknown', 'value': None})
        self.assertEqual(m['max_output_tokens'], {'state': 'known', 'value': 393216})
        self.assertEqual(self.prices('qwen-4-max-preview'), [])
        bindings = self.bindings('qwen-4-max-preview')
        self.assertEqual(len(bindings), 8)
        self.assertEqual({b['product_key'] for b in bindings}, {
            'bailian-token-personal-cn-beijing', 'bailian-token-team-cn-beijing'})
        for b in bindings:
            self.assertEqual(len(b['interface_candidates']), 1)
            self.assertIn('2026-11-25T16:00:00Z', b['availability']['condition'])
            self.assertIn('no PAYG', b['availability']['condition'])
        for kind in ('personal', 'team'):
            for uid in ('qwen4-max-preview', 'qwen4.0-max-preview'):
                r = self.records[f'official/bailian-token-{kind}-cn-beijing/{uid}']
                self.assertEqual(r['context_tokens'], {'state': 'runtime_required', 'value': None})
                self.assertNotIn('request-limit-prefill', r['usable_for'])
                self.assertIn('output-limit-prefill', r['usable_for'])
                fact = self.source_records[r['model_record_key']]['source_assertions'][0]
                self.assertEqual(fact['preview_start'], '2026-11-11T02:00:00Z')
                self.assertEqual(fact['preview_end_exclusive'], '2026-11-25T16:00:00Z')
                self.assertEqual(fact['billing_model_id'], 'qwen4.0-max-preview')

    def test_qwen4_wire_budgets_and_history_are_not_interchanged(self):
        r = self.records['official/bailian-token-personal-cn-beijing/qwen4-max-preview']
        fact = self.source_records[r['model_record_key']]['source_assertions'][0]
        self.assertEqual(fact['max_output_includes'], ['thinking', 'answer'])
        chat = fact['api_contracts']['openai-chat']
        responses = fact['api_contracts']['openai-responses']
        self.assertEqual(chat['effort_parameter'], 'reasoning_effort')
        self.assertEqual(responses['effort_parameter'], 'reasoning.effort')
        self.assertEqual(chat['output_budget_parameters'], ['max_tokens', 'max_completion_tokens'])
        self.assertEqual(responses['output_budget_parameters'], ['max_output_tokens'])
        self.assertTrue(chat['preserve_thinking_default'])
        self.assertTrue(chat['history_must_remain_separate_from_content'])
        self.assertEqual(chat['history_field'], 'reasoning_content')
        for api in (chat, responses):
            self.assertEqual(api['aliases'], {'minimal': 'low', 'high': 'xhigh', 'max': 'xhigh'})
            self.assertEqual(api['canonical_efforts'], ['none', 'low', 'medium', 'xhigh'])
            self.assertEqual(api['default_effort'], 'xhigh')

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
        price, = self.prices('minimax-m3-1-flash')
        self.assertEqual((price['product_key'], price['currency']), ('minimax-payg-global', 'USD'))
        self.assertEqual(price['schedules'][0]['rates'], {'input': '0.15', 'output': '0.60', 'cache-read': '0.015'})
        price, = self.prices('minimax-m3')
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

    def test_grok_and_ember_keep_published_thresholds_without_assumed_integer_units(self):
        for key, low, high in [('grok-4-7', '< 200k', '>= 200k'), ('muse-ember-1-0', '<= 200K', '> 200K')]:
            price, = self.prices(key)
            schedules = {s['schedule_key']: s for s in price['schedules']}
            self.assertEqual(schedules['standard-short-context']['condition']['provider_threshold'], low)
            self.assertEqual(schedules['standard-long-context']['condition']['provider_threshold'], high)
            for schedule in schedules.values():
                self.assertEqual(schedule['condition']['threshold_token_unit'], 'unresolved')
                self.assertEqual(schedule['application'], 'whole-request')
                self.assertNotIn('maximum_input_tokens', schedule['condition'])
                self.assertNotIn('minimum_input_tokens', schedule['condition'])

    def test_muse_contributor_requires_distinct_product_and_effort(self):
        standard = self.records['official/meta-model-api-standard/muse-spark-1.3']
        contributor = self.records['official/meta-model-api-contributor/muse-spark-1.3-contributor']
        self.assertIn('max', standard['reasoning_rendering_hints']['supported_reasoning_efforts'])
        self.assertNotIn('max', contributor['reasoning_rendering_hints']['supported_reasoning_efforts'])
        self.assertFalse(self.source_records[standard['model_record_key']]['source_assertions'][0]['content_used_for_training'])
        self.assertTrue(self.source_records[contributor['model_record_key']]['source_assertions'][0]['content_used_for_training'])
        for key in ('muse-wonder-1-0', 'muse-ember-1-0'):
            self.assertTrue(all(b['product_key'] == 'meta-model-api-standard' for b in self.bindings(key)))
        self.assertFalse(any('wonder-full' in b['upstream_model_id'] for b in self.metadata['endpoint_bindings']))

    def test_unpublished_integer_limits_and_legal_vision_remain_unset(self):
        for key in ('hunyuan-hy4', 'glm-5-3-flashx-global', 'glm-5-3-flashx-legal-global', 'mistral-large-4'):
            self.assertIsNone(self.models[key]['context_tokens']['value'])
        legal = self.models['glm-5-3-flashx-legal-global']
        self.assertNotIn('image_input', legal['modalities'])
        for key in ('glm-5-3-flashx-global', 'glm-5-3-flashx-legal-global'):
            self.assertTrue(all(b['product_key'] == 'zai-general-global' for b in self.bindings(key)))
        self.assertNotEqual(self.models['hunyuan-hy4']['model_key'], 'hunyuan-hy4-preview')

    def test_callable_ids_and_lifecycle_survive_projection(self):
        old = self.models['mistral-large-3']
        self.assertEqual(old['lifecycle'], 'deprecated')
        self.assertEqual(old['upstream_ids'], ['mistral-large-2512'])
        self.assertEqual(self.models['mistral-large-4']['upstream_ids'], ['mistral-large-4'])
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
        for key in ('source-qwen4-preview', 'source-minimax-pricing-current', 'source-mistral4', 'source-gemini4-carbon'):
            self.assertEqual(sources[key]['collected_on'], '2026-11-11')
            self.assertEqual(sources[key]['digest_state'], 'captured')
            self.assertRegex(sources[key]['bytes_digest'], r'^sha256:[0-9a-f]{64}$')


if __name__ == '__main__':
    unittest.main()
