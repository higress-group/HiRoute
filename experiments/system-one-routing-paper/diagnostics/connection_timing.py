#!/usr/bin/env python3
"""Reduce saved connection timings offline; no network or model execution."""

import argparse
import json
import math
import statistics
from collections import Counter, defaultdict
from pathlib import Path


def summarize(data):
    records = data['records']
    expected_cells = {
        (model, method, route)
        for model in data['models']
        for method in ('direct', 'decomposed')
        for route in ('explicit_proxy', 'no_application_proxy')
    }
    cells = defaultdict(list)
    pairs = defaultdict(list)
    warmups = Counter()
    for call_order, row in enumerate(records):
        key = row['backend'], row['method'], row['route']
        if key not in expected_cells or row['call_order'] != call_order:
            raise ValueError('Unexpected cell or call order')
        if row['model'] != data['models'][row['backend']]:
            raise ValueError('Unexpected model identity')
        if row['http_status'] != 200 or row['valid_answer'] is not True:
            raise ValueError('Invalid response must not be silently excluded')
        if not all(math.isfinite(row[k]) and row[k] >= 0
                   for k in ('elapsed_ms', 'connect_ms')):
            raise ValueError('Invalid duration')
        if row['elapsed_ms'] < row['connect_ms']:
            raise ValueError('Connection exceeds elapsed time')
        if row['phase'] == 'warmup':
            warmups[key] += 1
        elif row['phase'] == 'measured':
            cells[key].append(row)
            pairs[row['pair_index']].append(row)
        else:
            raise ValueError('Unknown phase')
    if len(records) != 88 or set(cells) != expected_cells:
        raise ValueError('Incomplete record population')
    if warmups != Counter({key: 1 for key in expected_cells}):
        raise ValueError('Expected one excluded warmup per cell')
    if set(pairs) != set(range(40)):
        raise ValueError('Incomplete paired population')
    first_paths = Counter()
    for pair in pairs.values():
        if len(pair) != 2 or {r['route'] for r in pair} != {
                'explicit_proxy', 'no_application_proxy'}:
            raise ValueError('Incomplete route pair')
        if len({(r['case_id'], r['backend'], r['method'], r['body_sha256'])
                for r in pair}) != 1:
            raise ValueError('Paired request bodies differ')
        first_paths[(pair[0]['backend'], pair[0]['method'], pair[0]['route'])] += 1
    if first_paths != Counter({key: 5 for key in expected_cells}):
        raise ValueError('Unbalanced path order')
    summaries = []
    for (backend, method, route), rows in sorted(cells.items()):
        if len(rows) != 10 or {r['case_id'] for r in rows} != set(data['selected_ids']):
            raise ValueError('Inconsistent context sample')
        post = sorted(r['elapsed_ms'] - r['connect_ms'] for r in rows)
        summaries.append({
            'backend': backend, 'method': method, 'route': route, 'n': len(rows),
            'median_elapsed_ms': statistics.median(r['elapsed_ms'] for r in rows),
            'median_connect_ms': statistics.median(r['connect_ms'] for r in rows),
            # Subtract each call's own connection phase before aggregation.
            'median_post_connection_ms': statistics.median(post),
            'p95_post_connection_ms': post[math.ceil(.95 * len(post)) - 1],
        })
    return summaries


def summarize_qwen(data, selected_ids):
    if data['model'] != 'qwen3.8-flash' or data['selected_ids'] != selected_ids:
        raise ValueError('Different model or context sample')
    records = data['records']
    if len(records) != 22:
        raise ValueError('Incomplete Qwen population')
    cells = defaultdict(list)
    warmups = Counter()
    for call_order, row in enumerate(records):
        if row['call_order'] != call_order or row['model'] != data['model']:
            raise ValueError('Unexpected Qwen order or model')
        if row['backend'] != 'qwen38flash' or row['method'] not in ('direct', 'decomposed'):
            raise ValueError('Unexpected Qwen cell')
        if row['http_status'] != 200 or row['valid_answer'] is not True:
            raise ValueError('Invalid Qwen response must not be silently excluded')
        if not all(math.isfinite(row[k]) and row[k] >= 0
                   for k in ('elapsed_ms', 'connect_ms')):
            raise ValueError('Invalid Qwen duration')
        if row['connect_ms'] > row['elapsed_ms']:
            raise ValueError('Qwen connection exceeds total duration')
        if row['phase'] == 'warmup':
            warmups[row['method']] += 1
        elif row['phase'] == 'measured':
            cells[row['method']].append(row)
        else:
            raise ValueError('Unknown Qwen phase')
    if warmups != Counter({'direct': 1, 'decomposed': 1}):
        raise ValueError('Expected two excluded Qwen warmups')
    if sum(r['usage_total_tokens'] for r in records) != data['observed_total_tokens']:
        raise ValueError('Qwen token accounting differs')
    summaries = []
    for method in ('direct', 'decomposed'):
        rows = cells[method]
        if len(rows) != 10 or {r['case_id'] for r in rows} != set(selected_ids):
            raise ValueError('Inconsistent Qwen context sample')
        post = sorted(r['elapsed_ms'] - r['connect_ms'] for r in rows)
        summaries.append({
            'backend': 'qwen38flash', 'method': method, 'route': 'study_transport',
            'n': len(rows),
            'median_elapsed_ms': statistics.median(r['elapsed_ms'] for r in rows),
            'median_connect_ms': statistics.median(r['connect_ms'] for r in rows),
            'median_post_connection_ms': statistics.median(post),
            'p95_post_connection_ms': post[math.ceil(.95 * len(post)) - 1],
        })
    return summaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', action='store_true', help='Print all recorded timing cells')
    args = parser.parse_args()
    data = json.loads(Path(__file__).with_name('connection-timing.json').read_text())
    rows = summarize(data)
    qwen = json.loads(Path(__file__).with_name('qwen-connection-timing.json').read_text())
    rows += summarize_qwen(qwen, data['selected_ids'])
    if args.json:
        print(json.dumps(rows, indent=2))
        return
    print('| Backend | Format | Median latency | p95 latency |')
    print('|---|---|---:|---:|')
    for backend, name in (('jev113', 'Jev 1.13'), ('jev114', 'Jev 1.14'),
                          ('qwen38flash', 'Qwen 3.8 Flash')):
        for method, label in (('direct', 'Direct'), ('decomposed', 'Decomposed')):
            row = next(r for r in rows if r['backend'] == backend and r['method'] == method
                       and r['route'] in ('explicit_proxy', 'study_transport'))
            median, p95 = row['median_post_connection_ms'], row['p95_post_connection_ms']
            print(f'| {name} | {label} | {median:,.0f} ms | {p95:,.0f} ms |')


if __name__ == '__main__':
    main()
