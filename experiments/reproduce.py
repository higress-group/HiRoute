#!/usr/bin/env python3
"""Offline evidence replay. Does not call a model or re-grade semantic correctness."""
import argparse
from decimal import Decimal, ROUND_FLOOR
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RESULTS = ROOT / 'cases/research-cost-quality/results/2026-10-04'


def read(path):
    return json.loads(path.read_text())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def assess(data, root=RESULTS):
    rows = data['rows']
    require(len(rows) == 9 and len({r['id'] for r in rows}) == 9, 'nine unique deliveries required')
    summary = {}
    for group in ('mixed', 'strong', 'cheap'):
        selected = [r for r in rows if r['group'] == group]
        require(len(selected) == 3, 'three repeats per group required')
        for row in selected:
            require(len(row['components']) == 7, 'seven components required')
            cards, errors, memos = [], set(), 0
            amounts = {k: Decimal(0) for k in ('lower', 'upper')}
            for c in row['components']:
                p = (root / c['artifact']).resolve()
                require(p.is_relative_to(root.resolve()), 'artifact escapes results directory')
                require(digest(p) == c['sha256'], 'artifact hash mismatch: ' + c['id'])
                v = read(p)
                if c['kind'] == 'bulk':
                    require(len(v['cards']) == 60, '60 cards per bulk component required')
                    cards.extend(x['id'] for x in v['cards'])
                    errors.update(c['error_ids'])
                    require(set(c['error_reasons']) == set(c['error_ids']), 'missing error explanations')
                else:
                    require(c['kind'] == 'critical' and len(v['memos']) == 1, 'one critical memo required')
                    memos += 1
                    critical = not any(x.get('severity') == 'material' for x in c['findings'])
                    require(critical == row['critical_pass'], 'critical grade disagrees with recorded review')
                a = c['accounting']
                require(a.get('unknown_attempts') == 0 and a.get('jev_unknown') == 0, 'successful component cost incomplete')
                require(a.get('client_usage_reconciled') is True, 'usage not reconciled')
                # Recompute component bounds from all recorded attempts plus reported routing cost.
                for bound in amounts:
                    amount = sum((Decimal(x['cost']['usd_' + bound]) for x in a['attempts']), Decimal(0)) + Decimal(a['jev_reported_usd'])
                    require(amount == Decimal(a['delivery_usd_' + bound]), 'component cost mismatch')
                    amounts[bound] += amount
            require(len(cards) == len(set(cards)) == 360 and memos == 1, 'incomplete delivery')
            require(errors.issubset(set(cards)), 'error references an undelivered card')
            correct = 360 - len(errors)
            require(correct == row['correct_cards'], 'card score mismatch')
            require((correct >= 353 and row['critical_pass']) == row['whole_pass'], 'strict acceptance mismatch')
            require(all(amounts[k] == Decimal(row['usd'][k]) for k in amounts), 'arm cost mismatch')
        summary[group] = dict(correct_cards=sum(r['correct_cards'] for r in selected), total_cards=1080,
                              critical_passes=sum(r['critical_pass'] for r in selected),
                              strict_whole_passes=sum(r['whole_pass'] for r in selected))
    pairs = []
    for i in range(1, 4):
        key = f'formal-draft98-v1-p{i:02d}'
        m = next(r for r in rows if r['id'] == key + '-mixed')
        s = next(r for r in rows if r['id'] == key + '-strong')
        savings = (1 - Decimal(m['usd']['upper']) / Decimal(s['usd']['lower'])) * 100
        pairs.append(dict(pair=i, accepted=m['whole_pass'] and s['whole_pass'],
                          conservative_savings_percent=str(savings.quantize(Decimal('.01'), rounding=ROUND_FLOOR))))
    require(data['original_three_pair_gate_passed'] is False and not all(p['accepted'] for p in pairs), 'original gate must remain failed')
    require(data['qwen_original_execution_complete'] is False and data['qwen_failed_request_cost_unknown'] is True,
            'original transport failure and unknown cost must remain explicit')
    return dict(groups=summary, pairs=pairs, semantic_review='Recorded unblinded judgments replayed; not a fresh evaluation.',
                original_three_pair_gate_passed=False, qwen_failed_request_cost_unknown=True)


def report():
    result = assess(read(RESULTS / 'deliveries.json'))
    u = read(ROOT / 'cases/unattended-engineering/results/2026-10-04/assessment.json')
    require(u['independent_assertions']['total_passed'] == 343, 'unattended acceptance mismatch')
    require(u['intermediate_operator_prompts'] == u['operator_product_patches'] == 0, 'operator intervention changed')
    result['unattended'] = {k: u[k] for k in ('native_seconds', 'independent_assertions', 'upgrade', 'savings_claim')}
    return result


def verify_manifest():
    manifest = read(ROOT / 'evidence-manifest.json')
    for name, sha in manifest['files'].items():
        p = (ROOT / name).resolve()
        require(p.is_relative_to(ROOT) and digest(p) == sha, 'frozen evidence mismatch: ' + name)
    return len(manifest['files'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('verify', 'report'))
    args = parser.parse_args()
    result = report()
    if args.command == 'verify':
        result = dict(status='pass', frozen_files=verify_manifest(), deliveries=9, cards=3240, memos=9,
                      original_three_pair_gate_passed=False, paid_model_calls=0)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
