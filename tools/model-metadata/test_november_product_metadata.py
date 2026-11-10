"""Keep November provider facts scoped to their product, protocol and entitlement."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
TERRA = 'gpt-6.1-terra'
KIMI = 'kimi-k3-1-pro'
DOUBAO = ('doubao-seed-2-0-lite-260428', 'doubao-seed-2-1-lite-260915')


class NovemberProductMetadataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())
        cls.data = json.loads((ROOT / 'assets/release-facts/current/bundle/official-model-data.json').read_text())['data']
        cls.models = {m['model_key']: m for m in cls.source['models']}
        cls.products = {p['product_key']: p for p in cls.source['access_products']}
        cls.records = {r['model_record_key']: r for r in cls.source['model_metadata_records']}

    def bindings(self, model):
        return [b for b in self.source['endpoint_bindings'] if b['model_key'] == model]

    def test_personal_credit_increase_does_not_change_team_quota(self):
        personal = self.products['bailian-token-personal-cn-beijing']
        self.assertEqual(personal['quota_facts'][:4], [
            'Lite: 20000 Credits/month', 'Essential: 45000 Credits/month',
            'Standard: 80000 Credits/month', 'Pro: 200000 Credits/month'])
        self.assertIn('2026-11-09 20:00 Asia/Shanghai', personal['quota_facts'][4])
        self.assertIn('without resetting used Credits', personal['quota_facts'][4])
        self.assertIn('do not roll over', personal['quota_facts'][5])
        team = self.products['bailian-token-team-cn-beijing']
        self.assertEqual(team['quota_facts'], [
            '25000, 100000, or 250000 Credits per seat/month depending on tier'])
        self.assertNotIn('source-211', team['evidence_refs'])

    def test_doubao_eos_is_not_an_alias_of_either_migration_destination(self):
        historical = self.models['doubao-seed-2-0-lite']
        self.assertEqual(historical['lifecycle'], 'retired')
        self.assertEqual(historical['upstream_ids'], ['doubao-seed-2-0-lite-260215'])
        self.assertIn('2026-10-22', historical['data_note'])
        for binding in self.bindings('doubao-seed-2-0-lite'):
            self.assertEqual(binding['availability']['state'], 'unavailable')
            self.assertEqual(binding['lifecycle'], 'retired')
            self.assertNotIn('replaced_by_upstream_id', binding)
        for model_id in DOUBAO:
            model = self.models[model_id]
            self.assertEqual(model['upstream_ids'], [model_id])
            self.assertEqual(model['lifecycle'], 'active')
            for modality in ('text_input', 'text_output', 'image_input', 'video_input', 'audio_input'):
                self.assertEqual(model['modalities'][modality], 'supported')
            self.assertIsNone(model['context_tokens'])  # Do not guess the table's k unit.
            self.assertIsNone(model['max_output_tokens'])
        self.assertEqual(self.products['volcengine-ark-payg-cn']['documented_upstream_model_ids'], list(DOUBAO))

    def test_terra_direct_limits_efforts_and_protocol_boundary(self):
        model = self.models[TERRA]
        self.assertEqual((model['context_tokens'], model['max_input_tokens'], model['max_output_tokens']),
                         (1_050_000, 922_000, 128_000))
        self.assertEqual(model['reasoning']['profiles'], ['none', 'low', 'medium', 'high', 'xhigh', 'max'])
        self.assertEqual(model['reasoning']['default'], 'medium')
        self.assertEqual(model['modalities']['image_input'], 'supported')
        self.assertEqual(model['modalities']['audio_input'], 'unsupported')
        self.assertEqual(model['modalities']['video_input'], 'unsupported')
        self.assertEqual(model['evidence_refs'], ['source-214'])
        binding, = self.bindings(TERRA)
        self.assertEqual(binding['product_key'], 'openai-platform-global')
        self.assertIn('requires Responses', binding['availability']['condition'])
        self.assertIn('unsupported on Chat Completions', binding['availability']['condition'])

    def test_terra_whole_request_threshold_and_cache_retention_prices(self):
        price, = [p for p in self.source['pricing_facts'] if p['model_key'] == TERRA]
        self.assertEqual((price['currency'], price['unit']), ('USD', 'per-1m-tokens'))
        schedules = {s['schedule_key']: s for s in price['schedules']}
        for label, threshold, rates, writes in [
            ('short', {'maximum_input_tokens': 272000}, {'input': '1', 'output': '5', 'cache-read': '0.05'}, ('1.25', '2.50')),
            ('long', {'minimum_input_tokens': 272001}, {'input': '2', 'output': '7.5', 'cache-read': '0.10'}, ('2.50', '5')),
        ]:
            base = schedules[f'standard-{label}-context']
            self.assertEqual(base['rates'], rates)
            for ttl, rate in zip((1800, 43200), writes):
                schedule = schedules[f'standard-{label}-context-cache-write-{ttl}s']
                self.assertEqual(schedule['rates'], {'cache-write': rate})
                self.assertEqual(schedule['condition']['cache_ttl_seconds'], ttl)
            for schedule in [s for k, s in schedules.items() if k.startswith(f'standard-{label}-context')]:
                self.assertEqual(schedule['application'], 'whole-request')
                self.assertEqual(schedule['condition']['service_tier'], 'standard')
                self.assertEqual(schedule['condition']['processing'], 'non-regional')
                for key, value in threshold.items():
                    self.assertEqual(schedule['condition'][key], value)

    def test_kimi_platform_and_code_ids_defaults_limits_stay_distinct(self):
        self.assertNotIn('default', self.models[KIMI]['reasoning'])
        bindings = self.bindings(KIMI)
        self.assertEqual(len(bindings), 6)
        for binding in bindings:
            code = binding['product_key'] == 'kimi-code-subscription'
            model_id = binding['upstream_model_id']
            self.assertEqual(model_id.startswith('kimi-'), not code)
            provider = 'kimi-code' if code else binding['product_key']
            record = self.records[f'official/{provider}/{model_id}']
            self.assertEqual(record['input_modalities'], ['image', 'text', 'video'])
            reasoning = record['source_assertions'][0]['reasoning_effort']
            self.assertEqual(reasoning['supported'], ['low', 'high', 'max'])
            self.assertTrue(reasoning['always_on'])
            self.assertEqual(reasoning['default'], 'high' if code else 'max')
            if code:
                self.assertEqual(record['context_tokens'], {'state': 'entitlement-dependent', 'value': None})
                self.assertEqual(record['max_output_tokens'], {'state': 'runtime-required', 'value': None})
                self.assertNotIn('max_output_tokens', binding.get('capability_overrides', {}))
                self.assertEqual(record['cost_hints'], [])
            else:
                self.assertEqual(record['context_tokens'], {'state': 'known', 'value': 1048576})
                self.assertEqual(record['max_output_tokens'], {'state': 'known', 'value': 1048576})
                self.assertEqual(record['source_assertions'][0]['max_completion_tokens'],
                                 {'default': 131072, 'maximum': 1048576})

    def test_kimi_currency_highspeed_and_cache_write_rates(self):
        prices = [p for p in self.source['pricing_facts'] if p['model_key'] == KIMI]
        self.assertEqual(len(prices), 4)
        for price in prices:
            cn = price['product_key'] == 'kimi-platform-cn'
            self.assertIn(price['product_key'], ('kimi-platform-cn', 'kimi-platform-global'))
            self.assertEqual(price['currency'], 'CNY' if cn else 'USD')
            self.assertEqual(price['unit'], 'per-1m-tokens')
            multiplier = 2 if price['upstream_model_id'].endswith('highspeed') else 1
            input_rate, output_rate = ((13, 68) if cn else (2, 10))
            schedules = {s['schedule_key']: s for s in price['schedules']}
            self.assertEqual(schedules['standard']['rates'], {
                'input': str(input_rate * multiplier), 'output': str(output_rate * multiplier),
                'cache-read': '1.3' if cn else '0.2'})
            for ttl, scale in ((300, 1), (3600, 2)):
                schedule = schedules[f'standard-cache-write-{ttl}s']
                self.assertEqual(schedule['condition']['cache_ttl_seconds'], ttl)
                self.assertEqual(schedule['rates'], {'cache-write': str(input_rate * multiplier * scale)})

    def test_published_metadata_does_not_promote_documentation_into_entitlement(self):
        metadata = self.data['metadata_catalog']
        selected = {TERRA, KIMI, *DOUBAO}
        self.assertTrue(selected.issubset({m['model_key'] for m in metadata['canonical_models']}))
        for binding in metadata['endpoint_bindings']:
            if binding['model_key'] in selected:
                self.assertEqual(binding['availability']['state'], 'conditional')
                self.assertEqual(binding['protocol_qualification'], 'runtime-required')
        for field in ('models', 'price_rates'):
            for entry in self.data[field]:
                self.assertFalse(any(model in entry['model_configuration_id'] for model in selected))
        code_records = [r for r in metadata['model_records']
                        if r['model_record_key'].startswith('official/kimi-code/k3.1-')]
        self.assertEqual(len(code_records), 2)
        for record in code_records:
            self.assertIsNone(record['max_output_tokens']['value'])
            self.assertIsNone(record['context_tokens']['value'])


if __name__ == '__main__':
    unittest.main()
