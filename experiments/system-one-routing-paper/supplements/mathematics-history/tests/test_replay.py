"""Contract checks for saved-response replay; no network or fixture-only claims."""
from copy import deepcopy
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import replay


class ReplayTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.studies, cls.inputs, cls.labels, cls.jobs, cls.answers = replay.load()

    def test_entire_saved_study_matches_reference(self):
        report, predictions = replay.compute()
        self.assertEqual(replay.check_reference(report), 1024)
        self.assertEqual(len(predictions), 2912)
        self.assertEqual(report['execution']['valid'], 2912)
        self.assertEqual(report['execution']['reported_cost_usd'], '0.287666736')
        self.assertEqual(report['panels']['chronological']['coverage'],
                         {'determinate': 32, 'conditional': 8, 'insufficient': 4})

    def test_missing_or_invalid_response_remains_in_determinate_denominator(self):
        job = next(j for j in self.jobs if j['panel'] == 'targeted')
        label = self.labels[job['panel']][job['id']]
        missing = replay.parse_saved(job, None)
        invalid_event = deepcopy(self.answers[job['call_id']])
        invalid_event['response']['model'] = 'wrong-version'
        invalid = replay.parse_saved(job, invalid_event)
        for pred in (missing, invalid):
            self.assertFalse(pred['valid'])
            result = replay.metrics([pred], {job['id']: label})
            self.assertEqual((result['correct'], result['n'], result['invalid']), (0, 1, 1))

    def test_unforced_labels_never_receive_action_accuracy_credit(self):
        for status in ('conditional', 'insufficient'):
            label = next(l for l in self.labels['chronological'].values() if l['action_status'] == status)
            job = next(j for j in self.jobs if j['panel'] == 'chronological' and j['id'] == label['id'])
            pred = replay.parse_saved(job, self.answers[job['call_id']])
            self.assertTrue(pred['valid'])
            result = replay.metrics([pred], {label['id']: label})
            self.assertEqual((result['correct'], result['n'], result['non_determinate']), (0, 0, 1))
            self.assertIsNone(result['record_accuracy'])

    def test_scalar_threshold_is_strict_and_does_not_use_modal_grade(self):
        job = next(j for j in self.jobs if j['form'] == 'three_questions')
        event = deepcopy(self.answers[job['call_id']])
        answers = event['response']['answers']
        answers['q0'].update(probabilities={'0': .9, '1': .1}, score=.1)
        answers['q1'].update(probabilities={'0': .2, '1': .6, '2': .2})
        answers['q2'].update(probabilities={'not_established': .1, 'applicable': .9}, choice='applicable')
        for scalar, expected in ((.99, 'primary'), (1, 'economy'), (1.01, 'economy')):
            answers['q1']['score'] = scalar
            p = replay.parse_saved(job, event)
            self.assertTrue(p['valid'])
            self.assertEqual(p['stage_grade'], 1)
            self.assertEqual(p['action'], expected)
        answers['q1']['score'] = .99
        sensitivity = replay.scalar_sensitivity([replay.parse_saved(job, event)])
        self.assertEqual((sensitivity['threshold_crossings'], sensitivity['action_changes']), (1, 1))

    def test_duplicate_and_unplanned_id_contracts(self):
        item = {'call_id': 'duplicate'}
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            replay.unique([item, item], 'call_id')
        self.assertEqual(set(self.answers), {j['call_id'] for j in self.jobs})

    def test_direct_uses_native_choice_when_probabilities_tie(self):
        job = next(j for j in self.jobs if j['form'] == 'direct')
        event = deepcopy(self.answers[job['call_id']])
        event['response']['answers']['route'].update(
            probabilities={'economy': .5, 'primary': .5}, choice='primary')
        self.assertEqual(replay.parse_saved(job, event)['action'], 'primary')

    def test_exact_three_definitions_are_embedded_in_each_direct_question(self):
        marker = 'Exact assessment definitions (shared with the typed reference, provided as guidance rather than a required output format):\n'
        for definition in self.studies['definitions']:
            direct = replay.read_json(replay.ROOT / 'questions' / (definition + '-direct.json'))
            embedded = json.loads(direct['route']['instructions'].split(marker, 1)[1])
            self.assertEqual(embedded, replay.read_json(replay.ROOT / 'questions' / (definition + '-three.json')))

    def test_answer_semantics_and_usage_are_retained(self):
        for event in self.answers.values():
            self.assertEqual(set(event), {'call_id', 'status', 'attempt', 'response'})
            response = event['response']
            self.assertEqual(set(response), {'model', 'answers', 'usage'})
            self.assertEqual(set(response['usage']), {'input_tokens', 'output_tokens', 'cost'})
            for answer in response['answers'].values():
                self.assertTrue({'type', 'probabilities', 'confidence'} <= set(answer))
                self.assertIn('score' if answer['type'] == 'score' else 'choice', answer)
                if answer['type'] == 'score':
                    self.assertIn('legend', answer)


if __name__ == '__main__':
    unittest.main()
