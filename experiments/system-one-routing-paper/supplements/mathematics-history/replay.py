#!/usr/bin/env python3
"""Offline replay of the three mathematics panels (Python standard library only)."""
import argparse
from collections import Counter, defaultdict
from decimal import Decimal
import gzip
import hashlib
import itertools
import json
import math
from pathlib import Path
import random
import statistics
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
PAPER = ROOT.parents[1]
sys.path.insert(0, str(PAPER / 'analysis'))
from fair_methods import parse
from methods import make_state

RELEASE = {
    'confirmation': {'repaired', 'withdrawn', 'independent'},
    'targeted': {'completed_repair', 'released_target'},
    'chronological': {'assistant_repaired', 'requirement_withdrawn', 'independent_target',
                      'external_repair_evidenced', 'partial_repair_independent_field', 'explicit_supersession'},
}


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8'))


def rows(path):
    opener = gzip.open if path.suffix == '.gz' else open
    with opener(path, 'rt', encoding='utf-8') as stream:
        return [json.loads(line) for line in stream if line.strip()]


def unique(items, key):
    result = {}
    for item in items:
        if item[key] in result:
            raise ValueError('duplicate ' + key + ': ' + item[key])
        result[item[key]] = item
    return result


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_manifest():
    manifest = read_json(ROOT / 'MANIFEST.json')
    for entry in manifest['files']:
        path = (ROOT / entry['path']).resolve()
        if not path.is_relative_to(ROOT) or sha256(path) != entry['sha256']:
            raise ValueError('supplement file hash mismatch: ' + entry['path'])
        if path.stat().st_size != entry['bytes']:
            raise ValueError('supplement size mismatch: ' + entry['path'])
    expected = {entry['path'] for entry in manifest['files']}
    actual = {p.relative_to(ROOT).as_posix() for p in ROOT.rglob('*') if p.is_file()
              and not any(part in {'.checks', '__pycache__'} for part in p.relative_to(ROOT).parts)
              and p.name != 'MANIFEST.json'}
    if expected != actual:
        raise ValueError('manifest inventory mismatch: ' + str(sorted(expected ^ actual)))
    for entry in manifest['public_dependencies']:
        path = (PAPER / entry['path']).resolve()
        if not path.is_relative_to(PAPER) or sha256(path) != entry['sha256']:
            raise ValueError('public dependency hash mismatch: ' + entry['path'])
    return len(expected)


def load():
    studies = read_json(ROOT / 'execution/studies.json')
    inputs, labels = {}, {}
    for panel in studies['panels']:
        name = panel['id']
        inputs[name] = unique(rows(ROOT / ('datasets/' + name + '-inputs.jsonl')), 'id')
        labels[name] = unique(rows(ROOT / ('datasets/' + name + '-labels.jsonl')), 'id')
        if set(inputs[name]) != set(labels[name]) or len(labels[name]) != panel['records']:
            raise ValueError('input/label coverage mismatch: ' + name)
        if len({r['family_id'] for r in labels[name].values()}) != panel['families']:
            raise ValueError('family count mismatch: ' + name)
        for cid, label in labels[name].items():
            if any(inputs[name][cid][k] != label[k] for k in ('id', 'family_id', 'state', 'language', 'pair_id')):
                raise ValueError('input/label identity mismatch: ' + cid)
            if label['action_status'] != 'determinate' and label['action'] is not None:
                raise ValueError('non-determinate record has an exact action: ' + cid)
    jobs = rows(ROOT / 'execution/requests.jsonl.gz')
    by_job = unique(jobs, 'call_id')
    responses = unique(rows(ROOT / 'responses/answers.jsonl.gz'), 'call_id')
    if set(responses) - set(by_job):
        raise ValueError('unplanned response IDs')
    expected = set()
    for panel in studies['panels']:
        name = panel['id']
        for cid, backend, definition, form, repeat in itertools.product(
                inputs[name], studies['models'], studies['definitions'], studies['forms'], range(panel['repeats'])):
            expected.add('/'.join((name, cid, backend, definition, form, str(repeat))))
    if set(by_job) != expected:
        raise ValueError('planned factorial is incomplete or duplicated')
    questions = {(definition, form): read_json(ROOT / 'questions' /
                 (definition + ('-direct.json' if form == 'direct' else '-three.json')))
                 for definition in studies['definitions'] for form in studies['forms']}
    for job in jobs:
        label = labels[job['panel']][job['id']]
        if any(job[k] != label[k] for k in ('family_id', 'state', 'language')):
            raise ValueError('request identity mismatch: ' + job['call_id'])
        if job['expected_model'] != studies['models'][job['backend']]:
            raise ValueError('request model mismatch')
        body = job['body']
        if set(body) != {'model', 'state', 'questions'}:
            raise ValueError('unexpected request body fields')
        if body['model'] != job['expected_model'].rsplit('-', 1)[0]:
            raise ValueError('request model alias mismatch')
        state = make_state(inputs[job['panel']][job['id']]['conversation'], 'c_three')
        if state != body['state'] or state['history_partial']:
            raise ValueError('request context mismatch or truncation: ' + job['call_id'])
        if body['questions'] != questions[job['definition'], job['form']]:
            raise ValueError('question definition mismatch')
        expected_id = '/'.join(str(job[k]) for k in ('panel', 'id', 'backend', 'definition', 'form', 'repeat'))
        if job['call_id'] != expected_id:
            raise ValueError('logical call ID mismatch')
    return studies, inputs, labels, jobs, responses


def parse_saved(job, event):
    pred = {k: v for k, v in job.items() if k != 'body'}
    pred.update(valid=False, action=None)
    try:
        if event is None:
            raise ValueError('missing_response')
        if event['status'] != 200 or event['response']['model'] != job['expected_model']:
            raise ValueError('status_or_model_identity')
        pred.update(parse(event['response'], job['backend'],
                          'direct' if job['form'] == 'direct' else 'decomposed'))
    except (AssertionError, KeyError, TypeError, ValueError, IndexError, AttributeError) as exc:
        pred.update(valid=False, action=None, invalid_reason=str(exc) or type(exc).__name__)
    return pred


def correct(pred, label):
    return bool(pred.get('valid') and pred.get('action') == label['action'])


def metrics(predictions, labels):
    eligible = [p for p in predictions if labels[p['id']]['action_status'] == 'determinate']
    family = defaultdict(list)
    for p in eligible:
        family[p['family_id']].append(int(correct(p, labels[p['id']])))
    count = sum(correct(p, labels[p['id']]) for p in eligible)
    return {'correct': count, 'n': len(eligible), 'record_accuracy': count / len(eligible) if eligible else None,
            'family_accuracy': statistics.mean(sum(v) / len(v) for v in family.values()) if family else None,
            'families': len(family), 'non_determinate': len(predictions) - len(eligible),
            'invalid': sum(not p.get('valid', False) for p in predictions)}


def paired(predictions, labels, panel):
    grid = {(p['id'], p['repeat'], p['form']): p for p in predictions}
    differences = defaultdict(list)
    gains = losses = 0
    details = []
    for label in labels.values():
        if label['action_status'] != 'determinate':
            continue
        for repeat in range(panel['repeats']):
            direct = grid[label['id'], repeat, 'direct']
            three = grid[label['id'], repeat, 'three_questions']
            d, t = int(correct(direct, label)), int(correct(three, label))
            differences[label['family_id']].append(t - d)
            gains += t > d
            losses += t < d
            if d != t:
                details.append({'id': label['id'], 'repeat': repeat, 'gold': label['action'],
                                'direct': direct.get('action'), 'three_questions': three.get('action')})
    keys = sorted(differences) if panel['bootstrap_family_order'] == 'sorted' else list(differences)
    values = [sum(differences[k]) / len(differences[k]) for k in keys]
    rng = random.Random(panel['bootstrap_seed'])
    draws = sorted(statistics.mean(rng.choices(values, k=len(values)))
                   for _ in range(panel['bootstrap_draws'])) if values else []
    return {'family_difference': statistics.mean(values) if values else None,
            'ci95': [draws[i] for i in panel['bootstrap_order_statistics']] if draws else None,
            'gains': gains, 'losses': losses, 'families': len(values), 'details': details}


def scalar_sensitivity(predictions):
    detail = []
    for p in predictions:
        if not p.get('valid'):
            continue
        distribution = p['probabilities']['stage_grade']
        expectation = sum(int(k) * v for k, v in distribution.items())
        scalar = p['historical_score']
        alternate = 'primary' if p['p_simple'] < .5 or (expectation < 1 and p['p_applicable'] >= .5) else 'economy'
        detail.append({'id': p['id'], 'repeat': p['repeat'], 'scalar': scalar, 'expectation': expectation,
                       'difference': scalar - expectation, 'crosses_threshold': (scalar < 1) != (expectation < 1),
                       'changes_action': alternate != p['action']})
    return {'n': len(detail), 'different_above_1e_6': sum(abs(d['difference']) > 1e-6 for d in detail),
            'mean_signed_difference': statistics.mean(d['difference'] for d in detail) if detail else None,
            'max_absolute_difference': max((abs(d['difference']) for d in detail), default=None),
            'threshold_crossings': sum(d['crosses_threshold'] for d in detail),
            'action_changes': sum(d['changes_action'] for d in detail),
            'crossing_cases': [d for d in detail if d['crosses_threshold']]}


def resources(jobs, responses, predictions):
    events = [responses[j['call_id']] for j in jobs if j['call_id'] in responses]
    usage = [e['response']['usage'] for e in events if isinstance(e.get('response'), dict) and 'usage' in e['response']]
    return {'planned_calls': len(jobs), 'observed_results': len(events),
            'missing_results': len(jobs) - len(events), 'valid': sum(bool(p.get('valid')) for p in predictions),
            'status_counts': dict(sorted(Counter(str(e['status']) for e in events).items())),
            'retry_count': sum(e['attempt'] for e in events), 'usage_records': len(usage),
            'input_tokens': sum(u['input_tokens'] for u in usage),
            'output_tokens': sum(u['output_tokens'] for u in usage),
            'reported_cost_usd': str(sum((Decimal(str(u['cost'])) for u in usage), Decimal(0)))}


def compute():
    studies, inputs, labels, jobs, responses = load()
    predictions = [parse_saved(j, responses.get(j['call_id'])) for j in jobs]
    lengths = {d: len(json.dumps(read_json(ROOT / 'questions' / (d + '-three.json'))['q2'], ensure_ascii=False, separators=(',', ':'))) for d in studies['definitions']}
    report = {'schema': 'hiroute.math-history-replay/v1', 'models': studies['models'],
              'applicability_definition_characters': lengths,
              'concise_applicability_character_reduction': 1 - lengths['concise'] / lengths['existing'],
              'execution': resources(jobs, responses, predictions), 'panels': {}}
    for panel in studies['panels']:
        name = panel['id']
        gold = labels[name]
        pp = [p for p in predictions if p['panel'] == name]
        pj = [j for j in jobs if j['panel'] == name]
        out = {'records': len(gold), 'coverage': dict(Counter(l['action_status'] for l in gold.values())),
               'determinate_coverage': sum(l['action_status'] == 'determinate' for l in gold.values()) / len(gold),
               'execution': resources(pj, responses, pp), 'models': {}}
        for backend in studies['models']:
            out['models'][backend] = {}
            for definition in studies['definitions']:
                sub = [p for p in pp if (p['backend'], p['definition']) == (backend, definition)]
                cell = {'paired': paired(sub, gold, panel)}
                for form in studies['forms']:
                    pred = [p for p in sub if p['form'] == form]
                    groups = {
                        'protection': [p for p in pred if gold[p['id']]['current_demand'] == 'routine' and gold[p['id']]['action'] == 'primary'],
                        'regular_targets': [p for p in pred if gold[p['id']]['action'] == 'economy'],
                        'new_reasoning': [p for p in pred if gold[p['id']]['current_demand'] == 'reasoning'],
                        'post_problem_release': [p for p in pred if p['state'] in RELEASE[name] and gold[p['id']]['action'] == 'economy'],
                        'healthy_regular': [p for p in pred if p['state'] not in RELEASE[name] and gold[p['id']]['action'] == 'economy'],
                    }
                    result = {'overall': metrics(pred, gold),
                              'states': {s: metrics([p for p in pred if p['state'] == s], gold) for s in sorted({p['state'] for p in pred})},
                              'languages': {lang: metrics([p for p in pred if p['language'] == lang], gold) for lang in ('zh', 'en')},
                              'groups': {g: {k: v for k, v in metrics(ps, gold).items() if k in ('correct', 'n')} for g, ps in groups.items()},
                              'unforced': {status: dict(Counter(p.get('action') if p.get('valid') else 'invalid'
                                           for p in pred if gold[p['id']]['action_status'] == status)) for status in ('conditional', 'insufficient')},
                              'resources': resources([j for j in pj if (j['backend'], j['definition'], j['form']) == (backend, definition, form)], responses, pred)}
                    if form == 'three_questions':
                        result['components'] = {}
                        for key in ('current_demand', 'stage_grade', 'applicability'):
                            eligible = [p for p in pred if gold[p['id']].get(key) is not None]
                            result['components'][key] = {'correct': sum(bool(p.get('metric_valid', {}).get(key)) and p.get(key) == gold[p['id']][key] for p in eligible), 'n': len(eligible)}
                        result['scalar_sensitivity'] = scalar_sensitivity(pred)
                    cell[form] = result
                out['models'][backend][definition] = cell
        report['panels'][name] = out
    return report, predictions


def same(actual, expected):
    if type(expected) in (int, float):
        return type(actual) in (int, float) and math.isclose(actual, expected, rel_tol=0, abs_tol=1e-11)
    if isinstance(expected, dict):
        return isinstance(actual, dict) and set(actual) == set(expected) and all(same(actual[k], v) for k, v in expected.items())
    if isinstance(expected, list):
        return isinstance(actual, list) and len(actual) == len(expected) and all(same(a, e) for a, e in zip(actual, expected))
    return actual == expected


def check_reference(report):
    checks = read_json(ROOT / 'reference-results.json')['checks']
    for check in checks:
        actual = report
        for part in check['path']:
            actual = actual[part]
        if not same(actual, check['value']):
            raise ValueError('reference mismatch: ' + '/'.join(check['path']) + '\n' + repr(actual) + ' != ' + repr(check['value']))
    return len(checks)


def markdown(report):
    lines = ['# Mathematics history: offline replay', '',
             'Accuracy panels remain separate. Differences and intervals use family weighting.', '']
    for name, panel in report['panels'].items():
        lines.extend(['## ' + name, '', '| Version | Definition | Direct | Three questions | Family difference (95% interval), pp | Gains / losses |',
                      '|---|---|---:|---:|---:|---:|'])
        for backend, variants in panel['models'].items():
            for definition, c in variants.items():
                d, t, p = c['direct']['overall'], c['three_questions']['overall'], c['paired']
                lines.append(f"| {report['models'][backend]} | {definition} | {d['correct']}/{d['n']} | {t['correct']}/{t['n']} | {100*p['family_difference']:.2f} [{100*p['ci95'][0]:.2f}, {100*p['ci95'][1]:.2f}] | {p['gains']} / {p['losses']} |")
        lines.extend(['', 'Label coverage: `' + json.dumps(panel['coverage'], sort_keys=True) + '`.', ''])
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('verify', 'report', 'case'))
    parser.add_argument('--check', action='store_true', help='compare every frozen reference assertion')
    parser.add_argument('--output', type=Path, help='new directory for report and parsed answers; never overwrites')
    parser.add_argument('--panel', choices=('confirmation', 'targeted', 'chronological'), help='panel for case inspection')
    parser.add_argument('--id', help='record ID for case inspection')
    args = parser.parse_args()
    files = verify_manifest()
    report, predictions = compute()
    execution = read_json(ROOT / 'execution/studies.json')['execution']
    for key in ('planned_calls', 'observed_results', 'missing_results'):
        if report['execution'][key] != execution[key]:
            raise ValueError('execution accounting mismatch: ' + key)
    if report['execution']['status_counts'] != {'200': execution['http_200']} or report['execution']['retry_count'] != execution['retries']:
        raise ValueError('execution status/retry mismatch')
    if report['execution']['valid'] != execution['planned_calls']:
        raise ValueError('the retained complete study contains an invalid answer')
    if args.command == 'case':
        if not args.panel or not args.id or args.output:
            parser.error('case requires --panel and --id, without --output')
        _, inputs, labels, jobs, responses = load()
        if args.id not in inputs[args.panel]:
            parser.error('record ID is absent from the selected panel')
        detail = {'input': inputs[args.panel][args.id], 'label': labels[args.panel][args.id],
                  'predictions': [p for p in predictions if (p['panel'], p['id']) == (args.panel, args.id)],
                  'responses': [responses[j['call_id']] for j in jobs if (j['panel'], j['id']) == (args.panel, args.id)]}
        print(json.dumps(detail, ensure_ascii=False, indent=2))
        return
    checks = check_reference(report) if args.check or args.command == 'verify' else 0
    if args.output:
        if args.command != 'report':
            parser.error('--output requires report')
        args.output.mkdir(parents=True, exist_ok=False)
        (args.output / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        (args.output / 'results.md').write_text(markdown(report), encoding='utf-8')
        (args.output / 'parsed-predictions.jsonl').write_text(''.join(json.dumps(p, ensure_ascii=False, separators=(',', ':')) + '\n' for p in predictions), encoding='utf-8')
    print(json.dumps({'files_verified': files, 'reference_assertions': checks, **report['execution']}))
    if args.command == 'report' and not args.output:
        print(markdown(report))


if __name__ == '__main__':
    main()
