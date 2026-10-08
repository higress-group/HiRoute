"""Hand-computed examples and metamorphic checks; no reference implementation."""
import copy
import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest


PROJECT = Path(os.environ.get('AUDIT_PROJECT') or os.environ['LIFECYCLE_PROJECT']).resolve()
STAGE = int(os.environ.get('AUDIT_STAGE') or os.environ['LIFECYCLE_PHASE'])
SPEC = importlib.util.spec_from_file_location('subject_audit', PROJECT / 'usage_audit.py')
SUBJECT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SUBJECT)


def event(request='a', **changes):
    value = dict(request_id=request, attempt=1, revision=1, team='red', model='small',
                 status='succeeded', input_tokens=10, output_tokens=3, cached_input_tokens=4)
    value.update(changes)
    return value


def measure(known, unknown=0):
    return {'known': known, 'unknown_attempts': unknown}


def at_stage(number):
    return unittest.skipUnless(STAGE >= number, 'requirement not released')


class AuditContract(unittest.TestCase):
    def test_empty_and_generator(self):
        for rows in ([], iter([])):
            result = SUBJECT.audit(rows)
            self.assertEqual(result['attempts'], 0)
            for name in ('input_tokens', 'output_tokens', 'cached_input_tokens'):
                self.assertEqual(result['usage'][name], measure(0))
        self.assertEqual(SUBJECT.audit(iter([event()]))['attempts'], 1)

    def test_unknown_is_not_zero(self):
        rows = [event(), event('b', input_tokens=None, output_tokens=0, cached_input_tokens=None),
                event('c', input_tokens=0, output_tokens=None, cached_input_tokens=0)]
        result = SUBJECT.audit(rows)
        self.assertEqual(result['attempts'], 3)
        for name, expected in [('input_tokens', measure(10, 1)),
                               ('output_tokens', measure(3, 1)),
                               ('cached_input_tokens', measure(4, 1))]:
            self.assertEqual(result['usage'][name], expected)

    def test_no_input_mutation_and_large_integers(self):
        rows = [event(input_tokens=10**18, cached_input_tokens=9)]
        before = copy.deepcopy(rows)
        self.assertEqual(SUBJECT.audit(rows)['usage']['input_tokens'], measure(10**18))
        self.assertEqual(rows, before)

    @at_stage(2)
    def test_bad_shape_and_values(self):
        bad = [None, [], 'x', {}, {**event(), 'extra': 0}]
        for field in event():
            row = event()
            del row[field]
            bad.append(row)
        for field in ('request_id', 'team', 'model'):
            bad += [event(**{field: value}) for value in ('', '  ', None, 1, False)]
        for field in ('attempt', 'revision'):
            bad += [event(**{field: value}) for value in (0, -1, None, 1.0, True, '1')]
        for field in ('input_tokens', 'output_tokens', 'cached_input_tokens'):
            bad += [event(**{field: value}) for value in (-1, 1.5, True, '3', [])]
        bad += [event(status=value) for value in ('pending', None, 1, [])]
        bad.append(event(cached_input_tokens=11))
        for row in bad:
            with self.subTest(row=row), self.assertRaises(ValueError):
                SUBJECT.audit([row])

    def run_cli(self, args, text=None):
        return subprocess.run([sys.executable, str(PROJECT / 'usage_audit.py'), *args],
                              input=text, text=True, capture_output=True,
                              cwd=PROJECT, timeout=8)

    @at_stage(2)
    def test_cli_stdin_and_file(self):
        rows = [event('中文'), event('b', output_tokens=None)]
        text = '\n' + '\n\n'.join(map(json.dumps, rows)) + '\n'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'events.jsonl'
            path.write_text(text, encoding='utf-8')
            for args, stdin in [(['-'], text), ([str(path)], None)]:
                result = self.run_cli(args, stdin)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout), SUBJECT.audit(rows))

    @at_stage(2)
    def test_cli_errors_do_not_emit_partial_json(self):
        for value in ('no-json\n', json.dumps(event()) + '\n{}\n', '[]\n'):
            result = self.run_cli(['-'], value)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, '')
            self.assertTrue(result.stderr.strip())
        result = self.run_cli([str(PROJECT / 'does-not-exist.jsonl')])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, '')

    @at_stage(3)
    def test_revisions_and_identical_replay(self):
        old = event(input_tokens=100, cached_input_tokens=70)
        new = event(revision=3, input_tokens=7, output_tokens=1, cached_input_tokens=2,
                    model='large', status='failed')
        rows = [new, old, new, event(revision=2), old]
        result = SUBJECT.audit(rows)
        self.assertEqual(result['attempts'], 1)
        self.assertEqual(result['usage']['input_tokens'], measure(7))
        self.assertEqual(result['usage']['output_tokens'], measure(1))
        self.assertEqual(result['usage']['cached_input_tokens'], measure(2))
        self.assertEqual(result, SUBJECT.audit(list(reversed(rows))))

    @at_stage(3)
    def test_winning_conflict_rejected_obsolete_conflict_ignored(self):
        a, b = event(output_tokens=8), event(output_tokens=9)
        with self.assertRaises(ValueError):
            SUBJECT.audit([a, b])
        newer = event(revision=2, output_tokens=20)
        for rows in itertools.permutations([a, b, newer]):
            self.assertEqual(SUBJECT.audit(rows)['usage']['output_tokens'], measure(20))

    @at_stage(3)
    def test_invalid_obsolete_event_still_fails(self):
        with self.assertRaises(ValueError):
            SUBJECT.audit([event(input_tokens=-1), event(revision=2)])

    @at_stage(4)
    def test_grouping_filter_and_order(self):
        rows = [event('c', team='z', model='max'), event('a', model='tiny'),
                event('b', model='max', input_tokens=None, cached_input_tokens=None)]
        result = SUBJECT.audit(rows)
        self.assertEqual([x['team'] for x in result['by_team']], ['red', 'z'])
        self.assertEqual([x['model'] for x in result['by_model']], ['max', 'tiny'])
        self.assertEqual(result['by_team'][0]['attempts'], 2)
        self.assertEqual(result['by_model'][0]['usage']['input_tokens'], measure(10, 1))
        filtered = SUBJECT.audit(rows, team='red')
        self.assertEqual(filtered['attempts'], 2)
        self.assertEqual(len(filtered['by_team']), 1)
        self.assertEqual(filtered['usage']['input_tokens'], measure(10, 1))
        empty = SUBJECT.audit(rows, team='missing')
        self.assertEqual(empty['attempts'], 0)
        self.assertEqual(empty['by_team'], [])
        self.assertEqual(empty['by_model'], [])
        for invalid in ('', ' ', 1, False, []):
            with self.subTest(team=invalid), self.assertRaises(ValueError):
                SUBJECT.audit(rows, team=invalid)

    @at_stage(4)
    def test_filter_cannot_hide_conflict_or_malformed_row(self):
        with self.assertRaises(ValueError):
            SUBJECT.audit([event(team='other'), event(team='other', output_tokens=99)], team='red')
        with self.assertRaises(ValueError):
            SUBJECT.audit([event(team='other', input_tokens=-1)], team='red')
        rows = [event(team='old'), event(revision=2, team='new')]
        self.assertEqual(SUBJECT.audit(rows, team='old')['attempts'], 0)
        self.assertEqual(SUBJECT.audit(rows, team='new')['attempts'], 1)

    @at_stage(4)
    def test_exact_labels_and_cli_team(self):
        rows = [event(team=' red '), event('b', team='red')]
        self.assertEqual(SUBJECT.audit(rows, team='red')['attempts'], 1)
        result = self.run_cli(['-', '--team', ' red '], '\n'.join(map(json.dumps, rows)))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), SUBJECT.audit(rows, team=' red '))

    @at_stage(5)
    def test_retry_request_outcome_and_failed_usage(self):
        rows = [event(status='failed'), event(attempt=2, model='large'),
                event('b', status='failed', team='blue'),
                event('b', attempt=3, status='failed', team='blue'), event('c')]
        result = SUBJECT.audit(rows)
        self.assertEqual(result['attempts'], 5)
        self.assertEqual(result['requests'], {'total': 3, 'succeeded': 2, 'failed': 1})
        self.assertEqual(result['usage']['input_tokens'], measure(50))
        self.assertEqual(result['by_team'][0]['requests'], {'total': 1, 'succeeded': 0, 'failed': 1})
        self.assertEqual(SUBJECT.audit(rows, 'red')['requests'], {'total': 2, 'succeeded': 2, 'failed': 0})
        for group in result['by_model']:
            self.assertNotIn('requests', group)

    @at_stage(5)
    def test_latest_status_and_request_team(self):
        rows = [event(), event(revision=2, status='failed')]
        self.assertEqual(SUBJECT.audit(rows)['requests'], {'total': 1, 'succeeded': 0, 'failed': 1})
        with self.assertRaises(ValueError):
            SUBJECT.audit([event(), event(attempt=2, team='blue')], team='absent')
        corrected = [event(team='old'), event(revision=2), event(attempt=2)]
        self.assertEqual(SUBJECT.audit(corrected)['requests']['total'], 1)

    @at_stage(5)
    def test_any_success_survives_later_failed_attempt(self):
        rows = [event(), event(attempt=2, status='failed', model='large')]
        for ordered in (rows, list(reversed(rows))):
            self.assertEqual(SUBJECT.audit(ordered)['requests'],
                             {'total': 1, 'succeeded': 1, 'failed': 0})

    @at_stage(5)
    def test_uncached_unknown_combinations(self):
        rows = [event(), event('b', input_tokens=0, cached_input_tokens=0),
                event('c', input_tokens=None, cached_input_tokens=9),
                event('d', input_tokens=20, cached_input_tokens=None),
                event('e', input_tokens=None, cached_input_tokens=None)]
        result = SUBJECT.audit(rows)
        self.assertEqual(result['usage']['uncached_input_tokens'], measure(6, 3))
        self.assertEqual(result['usage']['input_tokens'], measure(30, 2))
        self.assertEqual(result['usage']['cached_input_tokens'], measure(13, 2))
        for key in ('by_team', 'by_model'):
            self.assertEqual(result[key][0]['usage'], result['usage'])

    @at_stage(6)
    def test_ledger_chunking_replay_and_revisions(self):
        rows = [event(str(i), team='red' if i % 2 else 'blue', output_tokens=i) for i in range(17)]
        rows += [event(str(i), revision=2, team='red' if i % 2 else 'blue', output_tokens=i + 1)
                 for i in range(0, 17, 3)]
        expected = SUBJECT.audit(rows)
        for seed in range(4):
            shuffled = copy.deepcopy(rows)
            random.Random(seed).shuffle(shuffled)
            ledger = SUBJECT.Ledger()
            for offset in range(0, len(shuffled), seed + 1):
                ledger.extend(iter(shuffled[offset:offset + seed + 1]))
            self.assertEqual(ledger.snapshot(), expected)
            ledger.extend(shuffled)
            self.assertEqual(ledger.snapshot(), expected)
            self.assertEqual(ledger.snapshot('red'), SUBJECT.audit(rows, 'red'))

    @at_stage(6)
    def test_ledger_transactional_invalid_batch(self):
        ledger = SUBJECT.Ledger()
        ledger.extend([event()])
        before = ledger.snapshot()
        def rows():
            yield event('new')
            yield event('bad', output_tokens=-1)
        with self.assertRaises(ValueError):
            ledger.extend(rows())
        self.assertEqual(ledger.snapshot(), before)
        with self.assertRaises(ValueError):
            ledger.extend([event('new'), event(output_tokens=19)])
        self.assertEqual(ledger.snapshot(), before)
        with self.assertRaises(ValueError):
            ledger.extend([event('new'), event(attempt=2, team='blue')])
        self.assertEqual(ledger.snapshot(), before)

    @at_stage(6)
    def test_ledger_caller_and_snapshot_mutation(self):
        row = event()
        ledger = SUBJECT.Ledger()
        ledger.extend([row])
        row['output_tokens'] = 999
        output = ledger.snapshot()
        self.assertEqual(output['usage']['output_tokens'], measure(3))
        output['usage']['output_tokens']['known'] = -2
        output['by_team'].clear()
        self.assertEqual(ledger.snapshot(), SUBJECT.audit([event()]))
        self.assertEqual(SUBJECT.Ledger().snapshot(), SUBJECT.audit([]))
        with self.assertRaises(ValueError):
            ledger.snapshot('')

    @at_stage(6)
    def test_ledger_obsolete_conflict_and_higher_resolution(self):
        ledger = SUBJECT.Ledger()
        ledger.extend([event(revision=2)])
        ledger.extend([event(), event(output_tokens=99)])
        self.assertEqual(ledger.snapshot(), SUBJECT.audit([event(revision=2)]))
        ledger.extend([event(revision=3, output_tokens=41)])
        self.assertEqual(ledger.snapshot()['usage']['output_tokens'], measure(41))

    @at_stage(6)
    def test_delivery_document_exists(self):
        path = PROJECT / 'README.md'
        self.assertTrue(path.is_file(), 'README.md is required')
        self.assertGreater(len(path.read_text().strip()), 100, 'README must explain the utility')
