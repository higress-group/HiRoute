import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import reproduce
sys.path.insert(0,str(ROOT/'cases/research-cost-quality'))
from run import terminal
from prepare_sources import normalize


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.data=reproduce.read(reproduce.RESULTS/'deliveries.json')

    def test_checker_does_not_inherit_host_proxy_or_credentials(self):
        import os
        import tempfile
        from unittest.mock import patch
        spec=importlib.util.spec_from_file_location('engineering_verify',ROOT/'cases/unattended-engineering/verify.py')
        verifier=importlib.util.module_from_spec(spec);spec.loader.exec_module(verifier)
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ,{'HTTPS_PROXY':'http://127.0.0.1:1','OPENAI_API_KEY':'must-not-propagate'}):
            env=verifier.checker_environment(Path(tmp)/'subject',Path('/usr/bin/python3'),tmp)
            self.assertNotIn('HTTPS_PROXY',env)
            self.assertNotIn('OPENAI_API_KEY',env)
            self.assertEqual(env['PYTEST_DISABLE_PLUGIN_AUTOLOAD'],'1')
            self.assertTrue(Path(env['HOME']).is_dir())

    def test_full_evidence_and_all_original_outcomes(self):
        self.assertGreater(reproduce.verify_manifest(),60)
        value=reproduce.assess(self.data)
        self.assertEqual([value['groups'][g]['strict_whole_passes'] for g in ('mixed','strong','cheap')],[2,3,1])
        self.assertEqual([p['conservative_savings_percent'] for p in value['pairs'] if p['accepted']],['92.01','91.39'])

    def test_rejects_missing_delivery_or_relabelled_failure(self):
        for mutate in (lambda d:d['rows'].pop(),lambda d:d['rows'][0].update(whole_pass=True),lambda d:d.update(qwen_failed_request_cost_unknown=False)):
            data=copy.deepcopy(self.data);mutate(data)
            with self.assertRaises(ValueError):reproduce.assess(data)

    def test_rejects_cost_omission(self):
        self.data['rows'][0]['components'][0]['accounting']['jev_reported_usd']='0'
        with self.assertRaisesRegex(ValueError,'cost mismatch'):reproduce.assess(self.data)

    def test_rejects_artifact_escape(self):
        self.data['rows'][0]['components'][0]['artifact']='../../../../README.md'
        with self.assertRaisesRegex(ValueError,'escapes'):reproduce.assess(self.data)

    def test_stream_requires_one_terminal_and_consistent_text(self):
        delta={'type':'response.output_text.delta','delta':'ok'}
        end={'type':'response.completed','response':{'status':'completed','output':[{'type':'message','content':[{'type':'output_text','text':'ok'}]}],'usage':{'input_tokens':2,'output_tokens':1,'input_tokens_details':{'cached_tokens':0}}}}
        wire=lambda events:''.join('data: '+json.dumps(x)+'\n\n' for x in events).encode()
        self.assertEqual(terminal(wire([delta,end]))['output_text'],'ok')
        for events in ([delta],[end,end],[dict(delta,delta='different'),end]):
            with self.assertRaises(ValueError):terminal(wire(events))

    def test_missing_usage_remains_unknown(self):
        wire=b'data: {"type":"response.completed","response":{"status":"completed"}}\n\n'
        self.assertIsNone(terminal(wire)['normalized_usage'])

    def test_source_normalizer_ignores_script_and_preserves_original_line_rules(self):
        raw=b'<main><h1>Title</h1><script>do not execute</script><p> A  fact </p></main>'
        self.assertEqual(normalize(raw,{'id':'fixture','url':'https://example.invalid/'}),b'L0001 Title\nL0002 A fact\n')

    def test_public_links_do_not_depend_on_private_repository_documents(self):
        import re
        repo=ROOT.parent
        for directory in [ROOT,repo/'news']:
            for p in directory.rglob('*.md'):
                for target in re.findall(r'!?\[[^\]]*\]\(([^\s)]+)',p.read_text()):
                    if '://' in target or target.startswith('#'):continue
                    path=(p.parent/target.split('#')[0]).resolve()
                    self.assertTrue(path.is_file(),(str(p),target))
                    self.assertFalse(path.is_relative_to(repo/'docs'),(str(p),target))


if __name__=='__main__':unittest.main()
