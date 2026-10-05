#!/usr/bin/env python3
"""Prepare a fresh pinned HTTPX task. Network is used only to clone the baseline."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
ROOT = Path(__file__).resolve().parent


def prepare(output, baseline=None):
    spec = json.loads((ROOT / 'case.json').read_text())
    if output.exists():
        raise ValueError('output must not exist')
    output.mkdir(parents=True)
    subprocess.run(['git', 'init', '--quiet', str(output)], check=True)
    subprocess.run(['git', 'fetch', '--quiet', '--depth=1', str(baseline or spec['repository']), spec['baseline']], cwd=output, check=True)
    subprocess.run(['git', 'checkout', '--detach', spec['baseline']], cwd=output, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(['git', 'apply', str(ROOT / 'fixtures/test.patch')], cwd=output, check=True)
    shutil.copyfile(ROOT / 'TASK.md', output / 'TASK.md')
    protected = [p for p in output.rglob('*') if p.is_file() and (p.is_relative_to(output/'tests') or p.name in ('pyproject.toml','requirements.txt','test.sh'))]
    manifest = {str(p.relative_to(output)): hashlib.sha256(p.read_bytes()).hexdigest() for p in protected}
    (output / '.experiment-protected.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print('Prepared task. Install the pinned task environment before agent execution; keep this manifest and tests unchanged.')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--baseline', type=Path, help='Optional local clone, still checked out at the pinned revision')
    a = p.parse_args()
    prepare(a.output.resolve(), a.baseline)
