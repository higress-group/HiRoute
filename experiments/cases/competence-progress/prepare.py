"""Prepare public diagnostic requests offline; never load credentials or call a model."""
from copy import deepcopy
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'experiments/system-one-routing-paper/analysis'))
from methods import make_state


def current_questions():
    questions = json.loads((ROOT / 'crates/domain/src/routing/fixtures/system_one_c_v4.json').read_text())
    guidance = json.loads((ROOT / 'assets/decision-services/default-assessment-guidance.json').read_text())
    questions['q1']['instructions'] += ' ' + guidance['progress'] + ' ' + guidance['feedback']
    questions['q1']['criteria'] = [guidance[key] for key in
                                  ['lowest_criterion', 'middle_criterion', 'highest_criterion']]
    return questions


def requests(model='typesafe/jev-1.14'):
    questions = current_questions()
    seen = set()
    for line in (HERE / 'inputs.jsonl').read_text().splitlines():
        case = json.loads(line)
        if set(case) != {'id', 'conversation'} or case['id'] in seen:
            raise ValueError('invalid or duplicate input')
        seen.add(case['id'])
        yield {'id': case['id'], 'request': {'model': model,
               'state': make_state(case['conversation'], 'c_three'),
               'questions': deepcopy(questions)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='New JSONL file; existing files are preserved')
    args = parser.parse_args()
    rows = list(requests())
    with args.output.open('x') as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False, allow_nan=False) + '\n')
    print(json.dumps({'requests_prepared': len(rows), 'network_calls': 0}))


if __name__ == '__main__':
    main()
