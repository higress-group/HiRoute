#!/usr/bin/env python3
"""Recompute full and post-hoc reviewed comparisons; standard library, no network."""
import argparse
import csv
import gzip
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PAPER = ROOT.parent.parent
sys.path.insert(0, str(PAPER / 'analysis'))
from context_eval import summarize, paired_comparison, gold_for
from fair_methods import parse
from methods import make_state
from adapter import normalize

MODELS = ('jev113', 'jev114', 'openai_decisions')
NAMES = {'jev113': 'Jev 1.13', 'jev114': 'Jev 1.14', 'openai_decisions': 'OpenAI Decisions / gpt-6-luna'}


def read(name):
    p = ROOT / name
    text = gzip.decompress(p.read_bytes()).decode() if p.suffix == '.gz' else p.read_text()
    return [json.loads(s) for s in text.splitlines() if s.strip()]


def load(name):
    return json.loads((ROOT / name).read_text())


def reconstruct():
    labels = read('labels.jsonl')
    inputs = {x['id']: x['conversation'] for x in read('inputs.jsonl')}
    requests = read('requests.jsonl.gz')
    responses = read('responses.jsonl.gz')
    by_job = {x['job_id']: x for x in responses}
    assert len(inputs) == len(labels) == 152
    assert len(by_job) == len(responses) == len(requests) == 912
    selection = load('selection.json')
    excluded = {x['id'] for x in selection['excluded']}
    retained = {x['id'] for x in labels} - excluded
    assert len(excluded) == 14 and len(retained) == 138
    assert retained == set(selection['retained_ids'])
    for pair in {x['pair_id'] for x in selection['excluded']}:
        assert {x['language'] for x in selection['excluded'] if x['pair_id'] == pair} == {'zh', 'en'}
    expected = {(k, b, m) for k in inputs for b in MODELS for m in ('direct', 'decomposed')}
    assert {(j['id'], j['backend'], j['method']) for j in requests} == expected
    questions = load('questions.json')
    predictions = []
    for job in requests:
        event = by_job[job['job_id']]
        assert event['status'] == 200
        for key in ('id', 'backend', 'method', 'study_panel', 'repeat', 'family_id', 'language'):
            assert event[key] == job[key]
        body = job['body']
        defs = questions[job['study_panel']]['three_questions' if job['method'] == 'decomposed' else 'direct']
        if job['backend'] == 'openai_decisions':
            state = body['input'][0]['content'][0]['data']['state']
            converted = {}
            for q in body['questions']:
                converted[q['name']] = {
                    'type': q['type'], 'instructions': q['instructions'],
                    'criteria': ([x['description'] for x in q['levels']] if q['type'] == 'score'
                                 else {x['value']: x['description'] for x in q['choices']})}
            assert converted == defs
            native = normalize(event['response'], body)
        else:
            state = body['state']
            assert body['questions'] == defs
            native = event['response']
        assert state == make_state(inputs[job['id']], 'c_three')
        pred = {k: v for k, v in job.items() if k != 'body'}
        pred.update(parse(native, 'jev114', job['method']))
        assert pred['valid']
        predictions.append(pred)
    return labels, predictions, retained


def compute():
    labels, predictions, retained = reconstruct()
    results = {'review_timing': 'post-hoc; not a fresh blind test', 'populations': {}}
    for population in ('full', 'reviewed'):
        panels = {}
        for domain in ('engineering', 'mathematics'):
            gold = [x for x in labels if x['study_panel'] == domain and (population == 'full' or x['id'] in retained)]
            ids = {x['id'] for x in gold}
            panel = {'records': len(gold), 'families': len({x['family_id'] for x in gold}),
                     'action_status_counts': dict(Counter(x['action_status'] for x in gold)),
                     'action_gold_counts': dict(Counter(gold_for(x, 'action') for x in gold if gold_for(x, 'action') is not None)),
                     'models': {}}
            for backend in MODELS:
                chosen = [x for x in predictions if x['id'] in ids and x['backend'] == backend]
                block = {'formats': {}}
                for method in ('direct', 'decomposed'):
                    pred = [x for x in chosen if x['method'] == method]
                    metrics = ('action',) if method == 'direct' else ('current_demand', 'stage_grade', 'applicability', 'action')
                    cell = {'overall': summarize(gold, pred, repeats=1, metrics=metrics)}
                    cell['languages'] = {lang: summarize([x for x in gold if x['language'] == lang],
                        [x for x in pred if x['language'] == lang], repeats=1, metrics=metrics) for lang in ('en', 'zh')}
                    cell['action_by_gold'] = {}
                    for target in ('primary', 'economy'):
                        subset = [x for x in gold if gold_for(x, 'action') == target]
                        subset_ids = {x['id'] for x in subset}
                        cell['action_by_gold'][target] = summarize(subset, [x for x in pred if x['id'] in subset_ids],
                                                                   repeats=1, metrics=('action',))
                    block['formats'][method] = cell
                block['paired'] = paired_comparison(gold, [x for x in chosen if x['method'] == 'direct'],
                                                    [x for x in chosen if x['method'] == 'decomposed'], repeats=1)
                panel['models'][backend] = block
            panels[domain] = panel
        results['populations'][population] = panels
    return results


def verify_files():
    manifest = load('MANIFEST.json')
    for name, digest in manifest['files'].items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == digest, name
    for name, digest in manifest['paper_dependencies'].items():
        assert hashlib.sha256((PAPER / name).read_bytes()).hexdigest() == digest, name


def report_text(result):
    out = ['# Decision-interface transfer: reviewed subset', '',
           'Post-hoc exclusions, unchanged original labels and thresholds; no new API calls.',
           'All headline percentages below are workflow-family macro action accuracy.', '']
    for population, panels in result['populations'].items():
        out += ['## ' + population, '', '| Panel | Backend | Direct | Three questions | Difference, pp | Correct direct → three / eligible |',
                '|---|---|---:|---:|---:|---:|']
        for domain, panel in panels.items():
            for backend, block in panel['models'].items():
                d, t = [block['formats'][m]['overall']['metrics']['action'] for m in ('direct', 'decomposed')]
                out.append(f"| {domain} | {NAMES[backend]} | {100*d['family_accuracy']:.2f}% | {100*t['family_accuracy']:.2f}% | {100*block['paired']['difference']:+.2f} | {d['correct_calls']} → {t['correct_calls']} / {t['eligible_calls']} |")
        out.append('')
    out += ['## Reviewed components', '', '| Panel | Backend | Current demand | Historical grade | Applicability | Final action |',
            '|---|---|---:|---:|---:|---:|']
    for domain, panel in result['populations']['reviewed'].items():
        for backend, block in panel['models'].items():
            mm = block['formats']['decomposed']['overall']['metrics']
            out.append('| ' + ' | '.join([domain, NAMES[backend]] + [f"{100*mm[k]['family_accuracy']:.2f}%" for k in ('current_demand', 'stage_grade', 'applicability', 'action')]) + ' |')
    return '\n'.join(out) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('verify', 'report'))
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    verify_files()
    result = compute()
    assert result == load('reference-results.json'), 'Recomputed results differ from saved reference'
    if args.command == 'verify':
        print('Verified: 912 native responses; unchanged labels; 14 paired exclusions; full and reviewed metrics.')
        return
    assert args.output is not None, '--output is required'
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    (args.output / 'results.md').write_text(report_text(result))
    print(args.output)


if __name__ == '__main__':
    main()
