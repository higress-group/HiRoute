"""Check documented GPT-6.2 reasoning, API scope and pricing facts."""
import json
from decimal import Decimal
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
IDS = ('gpt-6.2-astra', 'gpt-6.2-sol')
REASONING_PROFILES = ['low', 'medium', 'high', 'xhigh', 'max']


class OpenAI62MetadataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())
        cls.data = json.loads((ROOT / 'assets/release-facts/current/bundle/official-model-data.json').read_text())['data']

    def test_independent_limits_and_documented_effort_levels(self):
        sources = {s['source_key']: s for s in self.source['evidence_sources']}
        for model_id in IDS:
            model = next(m for m in self.source['models'] if m['model_key'] == model_id)
            self.assertEqual(model['upstream_ids'], [model_id])
            self.assertEqual((model['context_tokens'], model['max_input_tokens'], model['max_output_tokens']),
                             (1_050_000, 922_000, 128_000))
            self.assertEqual(model['reasoning']['kind'], 'discrete')
            self.assertEqual(model['reasoning']['profiles'], REASONING_PROFILES)
            self.assertNotIn('default', model['reasoning'])
            self.assertTrue(any(sources[ref]['locator'] == 'https://developers.openai.com/api/docs/models'
                                and sources[ref]['digest_state'] == 'captured' for ref in model['evidence_refs']))
            self.assertEqual(model['identity_stability'], 'unknown')
            self.assertEqual(model['modalities']['image_input'], 'supported')
            for modality in ('audio_input', 'audio_output', 'video_input'):
                self.assertEqual(model['modalities'][modality], 'unsupported')
            self.assertEqual(model['capabilities']['structured_output'], 'supported')

    def test_protocol_and_entitlement_boundaries(self):
        for model_id in IDS:
            bindings = [b for b in self.source['endpoint_bindings'] if b['model_key'] == model_id]
            self.assertEqual(len(bindings), 2)
            by_interface = {b['interface_candidates'][0]: b for b in bindings}
            self.assertEqual(set(by_interface), {'openai-platform-global/openai-chat',
                                                'openai-platform-global/openai-responses'})
            self.assertEqual(by_interface['openai-platform-global/openai-chat']['capability_overrides'],
                             {'tool': False})
            for binding in bindings:
                self.assertEqual(binding['product_key'], 'openai-platform-global')
                self.assertEqual(binding['availability']['state'], 'conditional')
                self.assertEqual(binding['protocol_qualification'], 'runtime-required')
            for key in ('models', 'price_rates'):
                self.assertFalse(any(model_id in row['model_configuration_id'] for row in self.data[key]))
            self.assertFalse(any(model_id in json.dumps(row) for row in self.source['model_metadata_records']))

    def test_threshold_tier_and_ttl_prices_are_separate(self):
        # Independent provider table values, including the full-request 272k boundary.
        for model_id, short, long in [
            ('gpt-6.2-astra', ('8', '40', '.4', '10', '20'), ('16', '60', '.8', '20', '40')),
            ('gpt-6.2-sol', ('2', '10', '.1', '2.5', '5'), ('4', '15', '.2', '5', '10')),
        ]:
            price, = [p for p in self.source['pricing_facts'] if p['model_key'] == model_id]
            self.assertEqual((price['currency'], price['unit']), ('USD', 'per-1m-tokens'))
            schedules = {s['schedule_key']: s for s in price['schedules']}
            self.assertEqual(len(schedules), 18)
            for tier, multiplier in [('standard', Decimal(1)), ('batch', Decimal('.5')), ('fast', Decimal(2))]:
                for context, boundary, values in [('short', {'maximum_input_tokens': 272000}, short),
                                                   ('long', {'minimum_input_tokens': 272001}, long)]:
                    name = f'{tier}-{context}-context'
                    base = schedules[name]
                    for component, value in zip(('input', 'output', 'cache-read'), values[:3]):
                        self.assertEqual(Decimal(base['rates'][component]), Decimal(value) * multiplier)
                    for ttl, value in zip((1800, 43200), values[3:]):
                        write = schedules[f'{name}-cache-write-{ttl}s']
                        self.assertEqual(set(write['rates']), {'cache-write'})
                        self.assertEqual(Decimal(write['rates']['cache-write']), Decimal(value) * multiplier)
                        self.assertEqual(write['condition']['cache_ttl_seconds'], ttl)
                        if ttl == 43200:
                            self.assertEqual(write['condition']['protocol'], 'openai-responses')
                    for s in [v for k, v in schedules.items() if k.startswith(name)]:
                        self.assertEqual(s['application'], 'whole-request')
                        self.assertEqual(s['condition']['service_tier'], tier)
                        self.assertEqual(s['condition']['processing'], 'non-regional')
                        for key, value in boundary.items():
                            self.assertEqual(s['condition'][key], value)
            self.assertFalse(any('ultrafast' in k for k in schedules))

    def test_complete_bundle_keeps_source_facts_and_fresh_evidence(self):
        metadata = self.data['metadata_catalog']
        sources = {s['source_key']: s for s in self.source['evidence_sources']}
        for model_id in IDS:
            model = next(m for m in metadata['canonical_models'] if m['model_key'] == model_id)
            self.assertEqual(model['max_input_tokens'], 922000)
            self.assertEqual(model['reasoning']['kind'], 'discrete')
            self.assertEqual(model['reasoning']['profiles'], REASONING_PROFILES)
            self.assertIsNone(model['reasoning']['default'])
            original = next(p for p in self.source['pricing_facts'] if p['model_key'] == model_id)
            published = next(p for p in self.data['pricing_facts'] if p['model_key'] == model_id)
            self.assertEqual(published['schedules'], original['schedules'])
            for ref in original['evidence_refs']:
                self.assertEqual(sources[ref]['collected_on'], '2026-11-18')
                self.assertEqual(sources[ref]['digest_state'], 'captured')


if __name__ == '__main__':
    unittest.main()
