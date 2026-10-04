#!/usr/bin/env python3
"""Recover the registered input bytes; refuse changed source content."""
import argparse
import hashlib
import html.parser
import json
from pathlib import Path
import re
import shutil
import urllib.request

ROOT = Path(__file__).resolve().parent


class MainText(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.parts, self.active, self.ignore = [], False, 0

    def handle_starttag(self, tag, attrs):
        if tag == 'main':
            self.active = True
        if tag in ('script', 'style'):
            self.ignore += 1
        if self.active and tag in ('h1', 'h2', 'h3', 'h4', 'p', 'li', 'pre', 'tr', 'br'):
            self.parts.append('\n')

    def handle_endtag(self, tag):
        if tag == 'main':
            self.active = False
        if tag in ('script', 'style'):
            self.ignore -= 1
        if self.active and tag in ('h1', 'h2', 'h3', 'h4', 'p', 'li', 'pre', 'tr'):
            self.parts.append('\n')

    def handle_data(self, data):
        if self.active and not self.ignore:
            self.parts.append(data)


def normalize(raw, source):
    text = raw.decode('utf-8')
    if not source['url'].endswith('.md'):
        parser = MainText()
        parser.feed(text)
        text = ''.join(parser.parts)
    lines = [re.sub(r'\s+', ' ', s).strip() for s in text.splitlines()]
    lines = [s for s in lines if s]
    if source['id'] == 'kafka':
        start = [i for i, s in enumerate(lines) if s.rstrip('# ') == 'Message Delivery Semantics'][-1]
        stop = next(i for i in range(start + 1, len(lines)) if lines[i].rstrip('# ') == 'The Share Consumer')
        lines = lines[start:stop]
    return ('\n'.join(f'L{i:04d} {s}' for i, s in enumerate(lines, 1)) + '\n').encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def prepare(output, raw_directory=None):
    output.mkdir(parents=True, exist_ok=False)
    spec = json.loads((ROOT / 'inputs.json').read_text())
    recovered = {}
    for case in spec['case']:
        if case['id'] == 'critical':
            continue
        target = output / case['id']
        shutil.copytree(ROOT / 'tasks' / case['id'], target)
        for source in case['source_identity']:
            sid = source['id']
            if sid not in recovered:
                if raw_directory:
                    raw = (raw_directory / (sid + '.raw')).read_bytes()
                else:
                    request = urllib.request.Request(source['resolved_url'], headers={'User-Agent': 'HiRoute-experiment-reproduction/1.0'})
                    with urllib.request.urlopen(request, timeout=45) as response:
                        raw = response.read(3_000_001)
                if len(raw) > 3_000_000:
                    raise ValueError('source exceeds registered bound')
                normalized = normalize(raw, source)
                if sha(normalized) != source['text_sha256']:
                    raise ValueError(f'{sid}: upstream text changed; use the registered raw snapshot with --raw-directory. Do not silently update the experiment.')
                recovered[sid] = normalized
            (target / (sid + '.txt')).write_bytes(recovered[sid])
    target = output / 'critical'
    shutil.copytree(ROOT / 'tasks/critical', target)
    ids = set(json.loads((target / 'excerpt-lines.json').read_text()))
    excerpt = ''.join(s + '\n' for s in recovered['kafka'].decode().splitlines() if int(s.split()[0][1:]) in ids)
    (target / 'kafka.txt').write_text(excerpt)
    for case in spec['case']:
        for name, expected in case['case_input_sha256'].items():
            if sha((output / case['id'] / name).read_bytes()) != expected:
                raise ValueError('input hash mismatch: ' + case['id'] + '/' + name)
    print(json.dumps({'source_inputs': 'match registered bytes', 'cases': 7, 'paid_calls': 0}))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--raw-directory', type=Path)
    a = p.parse_args()
    prepare(a.output, a.raw_directory)
