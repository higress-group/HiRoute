"""Scoped product facts and lifecycle warnings reach the client projection."""
import json
from pathlib import Path
import hashlib
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = 'assets/model-data/current/inputs/metadata-catalog.json'


class ProductCompletionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads((ROOT / SOURCE).read_text())
        cls.data = json.loads((ROOT / 'assets/release-facts/current/bundle/official-model-data.json').read_text())['data']
        cls.metadata = cls.data['metadata_catalog']
        cls.records = {r['model_record_key']: r for r in cls.metadata['model_records']}
        cls.models = {m['model_key']: m for m in cls.metadata['canonical_models']}
        cls.products = {p['product_key']: p for p in cls.metadata['access_products']}

    def test_minimax_china_has_independent_keys_prices_and_protocols(self):
        product = self.products['minimax-payg-cn']
        self.assertIn('separate billing', product['restriction'])
        faces = {f['interface_key']: f for f in product['interfaces']}
        for model in ('minimax-m3', 'minimax-m3-1-flash'):
            binds = [b for b in self.metadata['endpoint_bindings']
                     if b['model_key'] == model and b['product_key'] == 'minimax-payg-cn']
            self.assertEqual(len(binds), 3)
            for binding in binds:
                self.assertEqual(binding['availability']['state'], 'conditional')
                self.assertEqual(binding['protocol_qualification'], 'runtime-required')
                face = faces[binding['interface_candidates'][0]]
                self.assertIn('api.minimax.cn', face['base_url'])
                cap = binding.get('capability_overrides', {}).get('max_output_tokens')
                self.assertEqual(cap, 524288 if face['protocol'] == 'openai-chat' else None)
        prices = {p['model_key']: p for p in self.data['pricing_facts'] if p['product_key'] == 'minimax-payg-cn'}
        flash = prices['minimax-m3-1-flash']
        self.assertEqual(flash['currency'], 'CNY')
        self.assertEqual(flash['schedules'][0]['rates'], {'input': '1.00', 'output': '4.00', 'cache-read': '0.10'})
        self.assertEqual(len(flash['schedules']), 1)
        for schedule in prices['minimax-m3']['schedules']:
            self.assertEqual(schedule['condition']['threshold_token_unit'], 'unresolved')
            self.assertNotIn('cache-write', schedule['rates'])
        self.assertEqual(len(self.data['models']), 30)

    def test_flashx_china_is_not_a_coding_plan_grant(self):
        binds = [b for b in self.metadata['endpoint_bindings'] if b['model_key'] == 'glm-5-3-flashx-global']
        self.assertEqual({b['product_key'] for b in binds}, {'zai-general-global', 'zhipu-general-cn'})
        price, = [p for p in self.data['pricing_facts'] if p['product_key'] == 'zhipu-general-cn' and p['model_key'] == 'glm-5-3-flashx-global']
        self.assertEqual(price['schedules'][0]['rates'], {'input': '2', 'output': '7', 'cache-read': '0.57'})
        self.assertEqual(price['currency'], 'CNY')
        self.assertEqual(self.models['glm-5-3-flash-global']['modalities']['video_input'], 'supported')
        for b in self.metadata['endpoint_bindings']:
            if b['product_key'] == 'zai-devpack-global' and b['model_key'] in {'glm-5-3-global', 'glm-5-3-flash-global'}:
                self.assertEqual(len(b['interface_candidates']), 3)

    def test_kimi_limits_and_access_stay_scoped(self):
        self.assertEqual(self.models['kimi-k2-6']['context_tokens']['value'], 262144)
        m = next(m for m in self.source['models'] if m['model_key'] == 'kimi-k3')
        variants = {v['upstream_id']: v['tokens'] for v in m['context_variants']}
        self.assertEqual((variants['k3'], variants['k3-256k']), (1048576, 262144))
        self.assertNotIn('default', m['reasoning'])
        old = [b for b in self.metadata['endpoint_bindings'] if b['product_key'] in {'kimi-platform-cn', 'kimi-platform-global'} and b['model_key'] in {'kimi-k2-6', 'kimi-k2-7-code'}]
        self.assertEqual(len(old), 6)
        self.assertTrue(all('retirement planned for the end of December 2026' in b['availability']['condition'] for b in old))
        for b in self.metadata['endpoint_bindings']:
            if b['product_key'] == 'kimi-code-subscription':
                self.assertNotIn('newly registered users', b['availability']['condition'])

    def test_exact_direct_lifecycle_records_preserve_client_assertions(self):
        expected = {
            'openclaw/mistral/mistral-medium-3-5': ('deprecated', 'mistral-large-4'),
            'hermes-agent/anthropic/claude-sonnet-4-5-20250929': ('deprecated', 'claude-sonnet-5-5'),
            'hermes-agent/anthropic/claude-opus-4-20250514': ('retired', 'claude-opus-4-8'),
            'hermes-agent/anthropic/claude-sonnet-4-20250514': ('retired', 'claude-sonnet-5-5'),
            'openclaw/google/gemini-2.5-pro': ('deprecated', 'gemini-3.8-flash'),
            'openclaw/google/gemini-2.5-flash': ('deprecated', 'gemini-3.5-flash-lite'),
        }
        # Pinned client assertions remain byte-equivalent, independently of the
        # newer official lifecycle facts. No Git history is needed by this test.
        assertion_digests = {
            'openclaw/mistral/mistral-medium-3-5': '0d46063d22fe9c8f95267a4c46c740b4f5cc4ada31a9e90b06fdbafc059db666',
            'openclaw/google/gemini-2.5-pro': 'a038719892303e29510a54e3992c469708a034d2d2730f869f1cdf101704df89',
            'openclaw/google/gemini-2.5-flash': '5639d5bdf9c80ec9e7dbd11dc194d2fde41d380960d9c28c70829b2522aadbde',
        }
        current = {r['model_record_key']: r for r in self.source['model_metadata_records']}
        for key, (state, replacement) in expected.items():
            self.assertEqual(self.records[key]['lifecycle'], state)
            self.assertEqual(self.records[key]['replacement_upstream_ids'], [replacement])
            raw = json.dumps(current[key]['source_assertions'], sort_keys=True, separators=(',', ':')).encode()
            self.assertEqual(hashlib.sha256(raw).hexdigest(), assertion_digests.get(key,
                             '4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945'))
            self.assertNotIn('lifecycle', current[key].get('field_provenance', {}))
        self.assertEqual(self.records['hermes-agent/copilot/gemini-2.5-pro']['lifecycle'], 'active')
        self.assertEqual(self.records['openclaw/github-copilot/gemini-2.5-pro']['lifecycle'], 'deprecated')
        for key, record in current.items():
            if key not in expected:
                self.assertFalse(any(ref.get('source_key') in {
                    'source-anthropic-lifecycle-20261112',
                    'source-mistral-medium-35-lifecycle-20261112',
                    'source-google-deprecations-20261112'} for ref in record['provenance_refs']))
        self.assertNotIn('mistral-large-4', self.models)


if __name__ == '__main__':
    unittest.main()
