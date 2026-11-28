"""Replay the November Bailian calls alongside the frozen October transfer study."""
import gzip
import hashlib
import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
TRANSFER = HERE.parents[1] / 'system-one-routing-paper/supplements/decision-interface-transfer'
sys.path.insert(0, str(TRANSFER))
import replay

STABLE = 'decision-model'
PREVIEW = 'decision-model-preview'
MODELS = (STABLE, PREVIEW, *replay.MODELS)
METHODS = ('direct', 'decomposed')


def records(file, digest):
    raw = gzip.decompress((HERE / file).read_bytes())
    assert hashlib.sha256(raw).hexdigest() == digest, file
    return [json.loads(row) for row in raw.splitlines()]


def compare():
    replay.verify_files()
    labels, predictions, retained = replay.reconstruct()
    templates = {(r['id'], r['method']): r['body'] for r in replay.read('requests.jsonl.gz')
                 if r['backend'] == 'jev114'}
    calls = defaultdict(list)
    measured = records('bailian-responses.jsonl.gz', 'cb01cfd3566a56ff8f7e95887afb9c8a8baa23cd79886bd8cacf99ee5b8c7473')
    expected = {(model, rid, method) for model in (STABLE, PREVIEW) for rid, method in templates}
    assert len(measured) == len(expected) == 608
    for row in measured:
        rid, model, method = row['id'], row['model'], row['method']
        expected.remove((model, rid, method))
        body = dict(templates[rid, method], model=model)
        wire = json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()
        assert hashlib.sha256(wire).hexdigest() == row['request_sha256']
        assert row['status'] == 200
        # Both providers use the keyed System One wire format parsed by the frozen native evaluator.
        parsed = replay.parse(row, 'jev114', method)
        assert parsed['valid']
        predictions.append(dict(parsed, id=rid, backend=model, method=method))
        calls[model, method].append((row['input_tokens'], None, row['elapsed_ms'], None))

    timing = records('historical-timing.jsonl.gz', '23101308ee8d9558f40cc2fc84b0ba4e3ffdca31e61c6f40d4d25c93b5d1f14f')
    times = {(r['backend'], r['id'], r['method']): r['elapsed_ms'] for r in timing}
    assert len(times) == len(timing) == 912
    for row in replay.read('responses.jsonl.gz'):
        model, rid, method = row['backend'], row['id'], row['method']
        usage = row['response']['usage']
        calls[model, method].append((usage['input_tokens'], usage['output_tokens'],
                                     times.pop((model, rid, method)), usage.get('cost')))
    assert not times

    for population in ('full', 'reviewed'):
        print('\n', population, '— family-macro final-action accuracy and correct / eligible responses')
        for panel in ('engineering', 'mathematics'):
            gold = [r for r in labels if r['study_panel'] == panel and (population == 'full' or r['id'] in retained)]
            ids = {r['id'] for r in gold}
            for model in MODELS:
                cells = []
                for method in METHODS:
                    selected = [r for r in predictions if r['id'] in ids and r['backend'] == model and r['method'] == method]
                    summary = replay.summarize(gold, selected, metrics=('action',))
                    assert summary['valid_responses'] == len(gold)
                    metric = summary['metrics']['action']
                    cells.append(f"{100 * metric['family_accuracy']:.2f}% ({metric['correct_calls']}/{metric['eligible_calls']})")
                print(panel, model, *cells, sep=' | ')

    print('\nAll formal requests — model | form | valid | input | output | p50/p95 ms | cost basis')
    for model in MODELS:
        for method in METHODS:
            rows = calls[model, method]
            inputs = sum(r[0] for r in rows)
            outputs = 'unreported' if model in (STABLE, PREVIEW) else sum(r[1] for r in rows)
            durations = sorted(r[2] for r in rows)
            p95 = durations[math.ceil(0.95 * len(durations)) - 1]
            if model == STABLE:
                cost = f'CNY {inputs * 0.30 / 1e6:.6f} at Beijing list price'
            elif model == PREVIEW:
                cost = 'Beijing limited-time free offer'
            elif model == 'openai_decisions':
                cost = f'USD {inputs * 0.10 / 1e6:.7f} at historical uncached rate'
            else:
                cost = f'USD {sum(r[3] for r in rows):.9f} API-reported'
            print(model, method, len(rows), inputs, outputs, f'{statistics.median(durations):.0f}/{p95:.0f}', cost, sep=' | ')


if __name__ == '__main__':
    compare()
