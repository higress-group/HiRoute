"""Official Claude 5.2 facts stay complete and conditional through publication."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
IDS = ('claude-fable-5-2', 'claude-mythos-5-2')


def read(path):
    return json.loads((ROOT / path).read_text())


class Anthropic52MetadataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = read('assets/model-data/current/inputs/metadata-catalog.json')
        cls.bundle = read('assets/release-facts/current/bundle/official-model-data.json')

    def test_exact_model_identity_limits_and_always_on_thinking(self):
        models = {x['model_key']: x for x in self.source['models']}
        for model_id in IDS:
            with self.subTest(model=model_id):
                model = models[model_id]
                self.assertEqual(model['upstream_ids'], [model_id])
                self.assertEqual(model['context_tokens'], 1_000_000)
                self.assertEqual(model['max_output_tokens'], 128_000)
                self.assertEqual(model['reasoning']['kind'], 'adaptive-fixed-on')
                self.assertEqual(model['reasoning']['default'], 'high')
                self.assertEqual(model['reasoning']['profiles'],
                                 ['low', 'medium', 'high', 'xhigh', 'max'])
                self.assertEqual(model['modalities']['image_input'], 'supported')

    def test_api_bindings_do_not_grant_subscription_or_verified_access(self):
        for model_id in IDS:
            bindings = [x for x in self.source['endpoint_bindings']
                        if x['model_key'] == model_id]
            self.assertEqual(len(bindings), 1)
            binding = bindings[0]
            self.assertEqual(binding['product_key'], 'anthropic-platform')
            self.assertEqual(binding['upstream_identity_role'], 'versioned-id')
            self.assertEqual(binding['availability']['state'], 'conditional')
            self.assertEqual(binding['protocol_qualification'], 'runtime-required')
            self.assertIn('forced tool_choice any/tool is unsupported',
                          binding['availability']['condition'])
            if model_id == 'claude-mythos-5-2':
                self.assertIn('verification program', binding['availability']['condition'])

    def test_standard_global_rates_keep_cache_ttl_separate(self):
        for model_id in IDS:
            price = next(x for x in self.source['pricing_facts']
                         if x['model_key'] == model_id)
            self.assertEqual(price['currency'], 'USD')
            self.assertEqual(price['unit'], 'per-1m-tokens')
            schedules = {x['schedule_key']: x for x in price['schedules']}
            self.assertEqual(schedules['standard-global']['rates'],
                             {'input': '8', 'output': '40', 'cache-read': '0.20'})
            for ttl, rate in [(300, '10'), (3600, '16')]:
                schedule = schedules[f'standard-global-cache-write-{ttl}s']
                self.assertEqual(schedule['condition']['cache_ttl_seconds'], ttl)
                self.assertEqual(schedule['rates'], {'cache-write': rate})
            for schedule in schedules.values():
                self.assertEqual(schedule['condition']['tier'], 'standard')
                self.assertEqual(schedule['condition']['inference_geo'], 'global')

    def test_provider_prefill_has_observed_provenance_without_inferred_defaults(self):
        records = {x['model_record_key']: x for x in self.source['model_metadata_records']}
        for model_id in IDS:
            record = records[f'official/anthropic-api/{model_id}']
            self.assertEqual(record['normalized_model_matches'], [model_id])
            self.assertTrue(all(x['kind'] == 'evidence-source'
                                for x in record['provenance_refs']))
            self.assertNotIn('field_provenance', record)
            self.assertIn('request-limit-prefill', record['usable_for'])
            self.assertIn('custom-api-model-prefill', record['usable_for'])
            self.assertEqual(record['source_assertions'][0]['released'], '2026-11-09')

    def test_complete_shared_data_contains_both_models_and_prices(self):
        # Search the real generated publication input, not a synthetic fixture.
        serialized = json.dumps(self.bundle)
        for model_id in IDS:
            self.assertIn(f'official/anthropic-api/{model_id}', serialized)
            self.assertIn(f'anthropic-platform/{model_id}/{model_id}', serialized)
        prices = self.bundle['data']['pricing_facts'] if 'data' in self.bundle else self.bundle['pricing_facts']
        for model_id in IDS:
            self.assertEqual(len([p for p in prices if p['model_key'] == model_id]), 1)


if __name__ == '__main__':
    unittest.main()
