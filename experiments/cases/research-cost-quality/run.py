#!/usr/bin/env python3
"""Execute a fresh arm via an explicitly supplied HiRoute profile; paid when invoked."""
import argparse
import hashlib
import http.client
import json
from pathlib import Path
import time
import tomllib
from urllib.parse import urlsplit
import uuid
from response_task import source_prompt, build_body
from response_base import extract_response, structural_error

ROOT = Path(__file__).resolve().parent
PARTS = ('rabbitmq', 'kafka', 'nats', 'pulsar', 'redis', 'rocketmq', 'critical')


def terminal(payload):
    events = []
    for block in payload.decode().replace('\r\n', '\n').split('\n\n'):
        data = '\n'.join(s[5:].lstrip(' ') for s in block.splitlines() if s.startswith('data:'))
        if data and data != '[DONE]':
            events.append(json.loads(data))
    ends = [e for e in events if e.get('type') in ('response.completed', 'response.failed', 'response.incomplete')]
    if len(ends) != 1:
        raise ValueError('missing or duplicate terminal event; cost unknown')
    result = extract_response(json.dumps(ends[0]['response']).encode())
    parts = {}
    for e in events:
        if e.get('type') == 'response.output_text.delta':
            key = (e.get('output_index', 0), e.get('content_index', 0))
            parts[key] = parts.get(key, '') + e['delta']
    streamed = '\n'.join(parts[k] for k in sorted(parts)).strip()
    if result['output_text'] and streamed and result['output_text'] != streamed:
        raise ValueError('terminal and delta text conflict')
    result['output_text'] = result['output_text'] or streamed
    return result


def write(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')


def run(args):
    spec = json.loads((ROOT / 'inputs.json').read_text())
    for case in spec['case']:
        for name, sha in case['case_input_sha256'].items():
            if hashlib.sha256((args.sources / case['id'] / name).read_bytes()).hexdigest() != sha:
                raise ValueError('source mismatch before any model call')
    # The supplied profile is the one HiRoute exported for the chosen fixed/smart plan.
    config = tomllib.loads(args.profile.read_text())
    provider = config['model_providers'][config['model_provider']]
    url = urlsplit(provider['base_url'])
    if url.scheme != 'http' or url.hostname != '127.0.0.1' or not url.port:
        raise ValueError('use an exported loopback HiRoute profile')
    args.output.mkdir(parents=True, exist_ok=False)
    args.output.chmod(0o700)
    run_id = 'reproduction-' + uuid.uuid4().hex
    write(args.output / 'run.json', dict(stage='fresh_reproduction_not_original_evidence', declared_group=args.group,
          run_id=run_id, parts=PARTS, semantic_review='pending', cost_accounting='pending immutable receipts and Jev usage',
          required_model_profiles=spec['model_profiles'], transport_retries=0, maximum_structural_attempts=2))
    for part in PARTS:
        out = args.output / part
        out.mkdir()
        prompt, hashes = source_prompt(args.sources / part)
        write(out / 'inputs.json', hashes)
        for attempt in range(2):
            body = build_body(config['model'], prompt, attempt > 0)
            body['stream'] = True
            write(out / f'{attempt}.request.json', body)
            wire = json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()
            headers = dict(provider.get('http_headers', {}))
            headers.update({'Content-Type': 'application/json', 'session-id': run_id + '-' + part + '-' + str(attempt)})
            connection = http.client.HTTPConnection(url.hostname, url.port, timeout=1900)
            start = time.monotonic()
            try:
                connection.request('POST', url.path.rstrip('/') + '/responses', wire, headers)
                response = connection.getresponse()
                payload = response.read()
                if response.status != 200:
                    raise ValueError('HTTP status ' + str(response.status))
                result = terminal(payload)
                output = result.pop('output_text')
                (out / f'{attempt}.output.txt').write_text(output)
                result.update(seconds=time.monotonic()-start, request_sha256=hashlib.sha256(wire).hexdigest())
                write(out / f'{attempt}.result.json', result)
                if result['status'] != 'completed' or result['normalized_usage'] is None:
                    raise ValueError('delivery or usage incomplete')
                problem = structural_error(output, args.sources / part, 'critical_m2' if part == 'critical' else 'bulk')
                if problem is None:
                    (out / 'delivery.json').write_text(output + '\n')
                    break
                if attempt == 1:
                    raise ValueError('structural coverage still incomplete')
            except Exception as error:
                write(out / f'{attempt}.failure.json', dict(error_class=type(error).__name__, cost='unknown until receipts reconciled', retry=False))
                raise
            finally:
                connection.close()
    print('Delivered all seven parts. Semantic review and complete receipt/Jev accounting remain required.')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--sources', type=Path, required=True)
    p.add_argument('--profile', type=Path, required=True)
    p.add_argument('--group', choices=('mixed', 'strong', 'cheap'), required=True)
    p.add_argument('--output', type=Path, required=True)
    run(p.parse_args())
