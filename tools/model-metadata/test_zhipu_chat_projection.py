"""Current Coding Plan Chat qualification cannot promote another model or protocol."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('model_generator', ROOT / 'assets/model-data/current/generate.py')
GENERATOR = importlib.util.module_from_spec(spec)
spec.loader.exec_module(GENERATOR)


class ZhipuChatProjectionTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads((ROOT / 'assets/model-data/current/inputs/metadata-catalog.json').read_text())
        self.data = json.loads((ROOT / 'assets/model-data/current/model-data.json').read_text())['data']

    def project(self):
        projection = {'models': [], 'model_endpoint_capabilities': [], 'offers': [], 'registrations': [], 'attempts': []}
        GENERATOR.add_qualified_zhipu_chat_capabilities(projection, self.catalog, self.data)
        return projection

    def test_both_models_reuse_the_connector_exact_chat_address_and_subscription(self):
        projection = self.project()
        self.assertEqual({c['upstream_model_id'] for c in projection['model_endpoint_capabilities']}, {'glm-5.3', 'glm-5.3-flash'})
        for cap in projection['model_endpoint_capabilities']:
            self.assertEqual(cap['endpoint_profile_id'], 'endpoint.zhipu.coding-chat.cn.v1')
            self.assertEqual(cap['connector_id'], 'connector.zhipu.p0')
            self.assertEqual(cap['upstream_protocol'], 'chat_completions')
            self.assertEqual(cap['required_adapter_ref'], 'adapter.openai-chat.v1')
        for model in projection['models']:
            self.assertEqual(model['native_reasoning']['native_render_convention'], 'zhipu_thinking_chat')
            self.assertEqual(model['native_reasoning']['capability']['profiles'], ['low', 'high', 'max'])
        self.assertEqual(projection['offers'][0]['billing_class'], 'subscription')

    def test_conditional_binding_is_not_executable(self):
        for binding in self.catalog['endpoint_bindings']:
            if binding['product_key'] == 'zhipu-coding-cn' and binding['interface_candidates'] == ['zhipu-coding-cn/openai-chat']:
                binding['protocol_qualification'] = 'runtime-required'
        self.assertEqual(self.project()['models'], [])

    def test_documentation_or_another_endpoint_cannot_replace_authenticated_chat_evidence(self):
        for source in self.catalog['evidence_sources']:
            if source['source_key'] == 'source-193':
                source['locator'] = 'https://open.bigmodel.cn/api/paas/v4/chat/completions'
        with self.assertRaises(ValueError):
            self.project()

    def test_duplicate_model_binding_is_ambiguous(self):
        binding = next(b for b in self.catalog['endpoint_bindings']
            if b['product_key'] == 'zhipu-coding-cn' and b['interface_candidates'] == ['zhipu-coding-cn/openai-chat'])
        duplicate = copy.deepcopy(binding)
        duplicate['binding_key'] += '/duplicate'
        self.catalog['endpoint_bindings'].append(duplicate)
        with self.assertRaises(ValueError):
            self.project()


if __name__ == '__main__':
    unittest.main()
