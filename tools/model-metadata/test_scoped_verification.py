"""A scoped metadata refresh must not claim new evidence for other products."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('model_generator', ROOT / 'assets/model-data/current/generate.py')
GENERATOR = importlib.util.module_from_spec(spec)
spec.loader.exec_module(GENERATOR)


def profile_dates(catalog):
    return {row['endpoint_profile']['endpoint_profile_id']: row['endpoint_profile']['last_verified_at']
            for row in GENERATOR.build_runtime_projection(catalog)['registrations']}


class ScopedVerificationTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())

    def test_global_snapshot_date_does_not_refresh_provider_evidence(self):
        before = profile_dates(self.catalog)
        self.catalog['as_of'] = '2027-01-01'
        self.assertEqual(profile_dates(self.catalog), before)

    def test_only_product_with_reviewed_evidence_advances(self):
        before = profile_dates(self.catalog)
        product = next(p for p in self.catalog['access_products'] if p['product_key'] == 'openai-platform-global')
        source = copy.deepcopy(next(s for s in self.catalog['evidence_sources'] if s['source_key'] == product['evidence_refs'][0]))
        source.update(source_key='test-reviewed-openai', collected_on='2027-01-01')
        self.catalog['evidence_sources'].append(source)
        product['evidence_refs'] = [source['source_key']]
        after = profile_dates(self.catalog)
        changed = [key for key in before if before[key] != after[key]]
        self.assertEqual(changed, ['endpoint.openai.platform.global.v1'])
        self.assertGreater(after[changed[0]], before[changed[0]])

    def test_missing_product_evidence_fails_closed(self):
        product = next(p for p in self.catalog['access_products'] if p['product_key'] == 'openai-platform-global')
        for refs in ([], ['absent-source']):
            with self.subTest(refs=refs):
                product['evidence_refs'] = refs
                with self.assertRaises((ValueError, KeyError)):
                    profile_dates(self.catalog)


if __name__ == '__main__':
    unittest.main()
