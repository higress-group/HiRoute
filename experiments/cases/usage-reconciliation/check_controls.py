"""Offline positive/negative controls for the frozen audit verifier."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys


CASE = Path(__file__).parent.resolve()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True,
                        help='fresh task-owned evidence directory')
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    reference = (CASE / 'grading/reference.py').read_text()
    mutants = {
        'unknown-as-zero': ('sum(v is None for v in values)', '0'),
        'oldest-revision': ('latest = max(', 'latest = min('),
        'last-status-wins': ("outcomes[row['request_id']] |=", "outcomes[row['request_id']] ="),
        'nontransactional-ledger': (
            'accepted = canonical([*self._rows, *pending])',
            'self._rows.extend(pending)\n        accepted = canonical(self._rows)'),
        'missing-cache-is-zero': (
            "known = [row['input_tokens'] - row['cached_input_tokens'] for row in rows\n"
            "             if row['input_tokens'] is not None and row['cached_input_tokens'] is not None]",
            "known = [row['input_tokens'] - (row['cached_input_tokens'] or 0) for row in rows\n"
            "             if row['input_tokens'] is not None]"),
    }
    cases = [('stub', (CASE / 'subject/usage_audit.py').read_text(), (1, 6), False),
             ('reference', reference, range(1, 7), True)]
    for name, (old, new) in mutants.items():
        if reference.count(old) != 1:
            raise RuntimeError(f'mutation no longer applies exactly once: {name}')
        cases.append((name, reference.replace(old, new), (6,), False))
    results = []
    for name, implementation, stages, expected_green in cases:
        project = root / name
        shutil.copytree(CASE / 'subject', project)
        (project / 'usage_audit.py').write_text(implementation)
        (project / 'README.md').write_text(
            'Positive-control documentation only. Revisions replace earlier usage; '
            'retries count as separate attempts; unknown usage is not zero. Ledger '
            'updates are transactional and cached input is a subset of input tokens.\n')
        for stage in stages:
            completed = subprocess.run([sys.executable, str(CASE / 'verify.py'),
                                        '--project', str(project), '--stage', str(stage)],
                                       text=True, capture_output=True, timeout=30)
            (root / f'{name}-s{stage}.log').write_text(completed.stdout + completed.stderr)
            try:
                grade = json.loads(completed.stdout.strip().splitlines()[-1])
            except (ValueError, IndexError):
                raise RuntimeError(f'no grade returned for {name}, stage {stage}')
            matched = (completed.returncode == 0) == expected_green and grade['selected'] > 0
            results.append(dict(control=name, stage=stage, expected_green=expected_green,
                                matched=matched, exit_code=completed.returncode, grade=grade))
    summary = {'green': all(r['matched'] for r in results), 'runs': results}
    (root / 'controls.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))
    return 0 if summary['green'] else 1


if __name__ == '__main__':
    sys.exit(main())
