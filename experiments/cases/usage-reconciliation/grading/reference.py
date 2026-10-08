"""Positive control only. Never copy into an experimental subject."""
import argparse
from collections import defaultdict
import json
import sys


FIELDS = {'request_id', 'attempt', 'revision', 'team', 'model', 'status',
          'input_tokens', 'output_tokens', 'cached_input_tokens'}
TOKENS = ('input_tokens', 'output_tokens', 'cached_input_tokens')


def nonempty(value):
    return isinstance(value, str) and bool(value.strip())


def validate(row):
    if not isinstance(row, dict) or set(row) != FIELDS:
        raise ValueError('event fields do not match the contract')
    if any(not nonempty(row[key]) for key in ('request_id', 'team', 'model')):
        raise ValueError('request_id, team and model must be nonempty strings')
    if any(type(row[key]) is not int or row[key] < 1 for key in ('attempt', 'revision')):
        raise ValueError('attempt and revision must be positive integers')
    if row['status'] not in ('succeeded', 'failed'):
        raise ValueError('invalid status')
    for key in TOKENS:
        value = row[key]
        if value is not None and (type(value) is not int or value < 0):
            raise ValueError('invalid token count')
    if (row['input_tokens'] is not None and row['cached_input_tokens'] is not None
            and row['cached_input_tokens'] > row['input_tokens']):
        raise ValueError('cached input exceeds input')
    return dict(row)


def canonical(rows):
    groups = defaultdict(list)
    for value in rows:
        row = validate(value)
        groups[(row['request_id'], row['attempt'])].append(row)
    selected = []
    for candidates in groups.values():
        latest = max(row['revision'] for row in candidates)
        winners = [row for row in candidates if row['revision'] == latest]
        if any(row != winners[0] for row in winners[1:]):
            raise ValueError('conflicting latest revision')
        selected.append(winners[0])
    teams = {}
    for row in selected:
        previous = teams.setdefault(row['request_id'], row['team'])
        if previous != row['team']:
            raise ValueError('one request has different teams')
    return selected


def usage(rows):
    result = {}
    for key in TOKENS:
        values = [row[key] for row in rows]
        result[key] = {'known': sum(v for v in values if v is not None),
                       'unknown_attempts': sum(v is None for v in values)}
    known = [row['input_tokens'] - row['cached_input_tokens'] for row in rows
             if row['input_tokens'] is not None and row['cached_input_tokens'] is not None]
    result['uncached_input_tokens'] = {'known': sum(known),
                                       'unknown_attempts': len(rows) - len(known)}
    return result


def requests(rows):
    outcomes = defaultdict(bool)
    for row in rows:
        outcomes[row['request_id']] |= row['status'] == 'succeeded'
    succeeded = sum(outcomes.values())
    return {'total': len(outcomes), 'succeeded': succeeded,
            'failed': len(outcomes) - succeeded}


def audit(rows, team=None):
    if team is not None and not nonempty(team):
        raise ValueError('team must be a nonempty string')
    selected = canonical(rows)
    if team is not None:
        selected = [row for row in selected if row['team'] == team]
    result = {'attempts': len(selected), 'usage': usage(selected),
              'requests': requests(selected)}
    for key in ('team', 'model'):
        groups = defaultdict(list)
        for row in selected:
            groups[row[key]].append(row)
        result['by_' + key] = [
            {key: name, 'attempts': len(group), 'usage': usage(group),
             **({'requests': requests(group)} if key == 'team' else {})}
            for name, group in sorted(groups.items())]
    return result


class Ledger:
    def __init__(self):
        self._rows = []

    def extend(self, rows):
        pending = [validate(row) for row in rows]
        accepted = canonical([*self._rows, *pending])
        self._rows = accepted

    def snapshot(self, team=None):
        return audit(self._rows, team)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('path')
    parser.add_argument('--team')
    args = parser.parse_args()
    try:
        if args.path == '-':
            text = sys.stdin.read()
        else:
            with open(args.path, encoding='utf-8') as stream:
                text = stream.read()
        rows = [json.loads(line) for line in text.splitlines() if line.strip()]
        result = audit(rows, args.team)
    except (ValueError, OSError, TypeError) as exc:
        print(str(exc), file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
