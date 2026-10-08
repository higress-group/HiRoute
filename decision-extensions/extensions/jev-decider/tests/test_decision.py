from copy import deepcopy
from types import SimpleNamespace
import unittest
from jev_decider.decision import decision_response, prepare_decision, reported_usage
from jev_decider.protocol import ProtocolError, strict_json, validate_hiroute_request
from tests.decision_cases import request, answers

class DecisionContractTests(unittest.TestCase):
    def prepare(self, value, limit=256 * 1024):
        return prepare_decision(SimpleNamespace(model='jev-test', max_request_bytes=limit), validate_hiroute_request(value))

    def test_ordinal_distribution_and_no_unsolicited_assessment(self):
        prepared = self.prepare(request())
        self.assertEqual(list(prepared.body['questions']), ['q0'])
        result = decision_response(prepared, answers(score=0))
        self.assertEqual(result, {'decision': {'kind': 'ordinal', 'probabilities': {'simple': 0.8, 'complex': 0.2}}})

    def test_category_independent_refinements_and_raw_competence(self):
        value = request(1)
        prepared = self.prepare(value)
        self.assertEqual([q['type'] for q in prepared.body['questions'].values()], ['choice', 'score', 'score', 'score'])
        self.assertEqual(prepared.body['questions']['q1']['criteria'], [x['criterion'] for x in value['decision']['options'][0]['refinement']['levels']])
        self.assertEqual(prepared.body['questions']['q3']['criteria'], [x['criterion'] for x in value['assessment_target']['criteria']])
        result = decision_response(prepared, answers(True, 1.2))
        self.assertEqual(result['decision']['choice'], 'review')
        self.assertEqual(result['decision']['refinement']['probabilities']['simple'], 0.8)
        self.assertEqual(result['assessment'], {'score': 0.6, 'partial': False})
        self.assertEqual(decision_response(prepared, answers(True, 0))['assessment']['score'], 0)

    def test_single_group_has_no_degree_question(self):
        value = request(1)
        del value['decision']['options'][1]['refinement']
        prepared = self.prepare(value)
        response = decision_response(prepared, answers(True))
        self.assertNotIn('refinement', response['decision'])
        self.assertEqual(len(prepared.body['questions']), 3)

    def test_bad_selected_degree_keeps_category_but_bad_category_fails(self):
        prepared = self.prepare(request(1))
        upstream = answers(True, 3)
        del upstream['answers']['q2']
        result = decision_response(prepared, upstream)
        self.assertEqual(result['decision']['choice'], 'review')
        self.assertEqual(result['decision']['refinement']['probabilities'], {})
        self.assertNotIn('assessment', result)
        upstream['answers']['q0']['choice'] = 'unknown'
        with self.assertRaises(ProtocolError): decision_response(prepared, upstream)

    def test_probability_keys_sum_finiteness_and_tolerance(self):
        prepared = self.prepare(request())
        for distribution in ({'0': 1}, {'0': True, '1': 0}, {'0': float('nan'), '1': 1}, {'0': .8, '1': .1}, {'0': 1, '1': 0, 'x': 0}):
            upstream = answers()
            upstream['answers']['q0']['probabilities'] = distribution
            self.assertEqual(decision_response(prepared, upstream)['decision']['probabilities'], {})
        upstream = answers()
        upstream['answers']['q0']['probabilities']['1'] = .2000001
        self.assertAlmostEqual(sum(decision_response(prepared, upstream)['decision']['probabilities'].values()), 1)

    def test_current_input_never_truncated_and_removed_target_is_partial(self):
        value = request(1)
        giant = deepcopy(value['visible_conversation'][0])
        giant['user'] = [{'kind': 'text', 'text': 'x' * 10000}]
        value['visible_conversation'].insert(0, giant)
        prepared = self.prepare(value, 8000)
        self.assertEqual(len(prepared.body['state']['visible_conversation']), 1)
        self.assertTrue(decision_response(prepared, answers(True, 1))['assessment']['partial'])
        value['latest_user'][0]['text'] = '界' * 10000
        with self.assertRaises(ProtocolError): self.prepare(value, 8000)

    def test_closed_current_contract_rejects_old_fields_recursive_kinds_and_bad_anchors(self):
        for key, mutation in [('branches', {}), ('assessment_from', None)]:
            value = request(); value[key] = mutation
            with self.assertRaises(ProtocolError): validate_hiroute_request(value)
        value = request(1); value['decision']['options'][0]['refinement']['kind'] = 'categorical'
        with self.assertRaises(ProtocolError): validate_hiroute_request(value)
        value = request(1); value['assessment_target']['criteria'][1]['score'] = .6
        with self.assertRaises(ProtocolError): validate_hiroute_request(value)
        for body in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}'):
            with self.assertRaises(ProtocolError): strict_json(body)

    def test_usage_contains_only_finite_reported_billing_facts(self):
        self.assertEqual(reported_usage({'usage': {'input_tokens': 10, 'output_tokens': 0, 'cost': .01, 'secret': 'omit'}}), {'input_tokens': 10, 'output_tokens': 0, 'cost': .01})
        self.assertIsNone(reported_usage({'usage': {'cost': float('inf'), 'input_tokens': True}}))
