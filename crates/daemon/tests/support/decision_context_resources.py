"""Exact-binary loopback resource comparison for decision context allocation.

Use the same driver against the original baseline (--legacy) and the candidate.
The baseline retains its fixed allocator; labels do not invent baseline windows.
Output contains numeric evidence only, never provider bodies or credentials.
"""
from concurrent.futures import ThreadPoolExecutor
import argparse
import copy
import hashlib
import http.client
import json
import os
from pathlib import Path
import subprocess
import threading
import time
from http.server import ThreadingHTTPServer

from decision_context_product import Endpoint, forwarded_text
from model_connections_product import (MODEL, NATIVE_TOKEN, NativeUpstream,
                                       publish_plan_and_agent, save_native_source)
from observation_content_fault_product import accepted_requests, complete_content, durable_gaps
from publication_process import bootstrap, plan_change
from publication_product import Product, V1, encoded


class Usage:
    def __init__(self, pid):
        self.pid, self.samples = pid, []
        self.error = None
        self.stop = threading.Event()
        self.sample()
        self.thread = threading.Thread(target=self.watch, daemon=True)
        self.thread.start()

    def sample(self):
        path = Path('/proc') / str(self.pid)
        fields = (path / 'stat').read_text().split(') ', 1)[1].split()
        cpu = (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')
        rss = int((path / 'statm').read_text().split()[1]) * os.sysconf('SC_PAGE_SIZE') // 1024
        self.samples.append((rss, cpu))

    def watch(self):
        while not self.stop.wait(.02):
            try:
                self.sample()
            except Exception as error:
                self.error = error
                return

    def finish(self):
        self.stop.set(); self.thread.join(timeout=3)
        assert not self.thread.is_alive()
        assert self.error is None, 'process resource sampler failed'
        self.sample()
        return {'sample_period_ms': 20, 'samples': len(self.samples),
            'rss_initial_kib': self.samples[0][0], 'rss_sampled_peak_kib': max(v[0] for v in self.samples),
            'rss_after_drain_kib': self.samples[-1][0],
            'cpu_through_drain_ms': round((self.samples[-1][1] - self.samples[0][1]) * 1000, 3)}


def run(repository, legacy, output):
    Endpoint.requests.clear()
    product, upstream = Product(repository), NativeUpstream()
    product.collaboration_only = True
    provider_times, model_times = [], []
    class TimedEndpoint(Endpoint):
        def do_POST(self):
            started = time.monotonic()
            try: super().do_POST()
            finally: provider_times.append(time.monotonic() - started)
    base = upstream.server.RequestHandlerClass
    class TimedModel(base):
        def do_POST(self):
            started = time.monotonic()
            try: super().do_POST()
            finally: model_times.append(time.monotonic() - started)
    upstream.server.RequestHandlerClass = TimedModel
    server = ThreadingHTTPServer(('127.0.0.1', 0), TimedEndpoint)
    worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
    rows, sequence = [], 0
    endpoint = f'http://127.0.0.1:{server.server_port}/decision'
    def data(command, payload=None): return product.public_cli(command, payload)[1]['data']

    def save(codec, window):
        id = f'decision/resource-{codec}-{window}'
        source = next(v for v in data('decision sources list')['sources'] if v['source_id'] == product.source_id)
        service = {'schema': 'hiroute.decision-service/v2' if legacy else 'hiroute.decision-service/v3',
            'id': id, 'revision': 1, 'name': 'Context resource fixture',
            'connection': {'kind': 'native', 'provider': 'compatible', 'wire_protocol': codec,
                'model': 'fixture', 'endpoint': endpoint, 'timeout_ms': 3000,
                'authentication': {'kind': 'shared_source', 'source_id': source['source_id'],
                    'source_lineage_digest': source['lineage_digest']}}}
        if not legacy:
            policy = {'mode': 'custom', 'window_tokens': window, 'typesafe_long_context': 'automatic'}
            preview_input = {k: service['connection'][k] for k in ['endpoint', 'model', 'wire_protocol']}
            preview_input['policy'] = policy
            resolved = data('decision context preview', preview_input)
            service['context'] = {'policy': policy, 'resolved': resolved}
        desired = {'change': {'id': id, 'expected_revision': 0, 'service': service,
            'expected_source_revisions': [{'source_id': source['source_id'], 'revision': source['revision']}]}}
        if not legacy: desired['schema'] = 'hiroute.decision-service-operation-input/v3'
        spec = {'schema_version': V1, 'command_id': 'decision.connections.apply', 'resource_id': id, 'desired_state': desired}
        preview = data('decision connections apply', {'schema_version': V1, 'spec': spec})
        result = data('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': id.replace('/', '-')})
        assert result['state'] == 'succeeded'
        return service

    def publish(service):
        editor = copy.deepcopy(product.editor)
        editor['branch_routing']['classifier'] = {'kind': 'decision_service_v2' if legacy else 'decision_service_v3', 'service': service}
        change = plan_change(product, 'update', **editor)
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, service['id'].replace('/', '-') + '-publish')
        product.editor = editor

    escaped = '中文 "quotes" \\ newline\n'
    short = [{'role': 'user', 'content': 'Describe the next bounded step.'}]
    history = [{'role': 'user', 'content': 'Prepare the checklist.'},
        {'role': 'assistant', 'content': escaped * 16000}, {'role': 'user', 'content': 'Proceed.'}]
    current = [{'role': 'user', 'content': escaped * 16000}]
    tool_heavy = [{'role': 'user', 'content': 'Read the local fixtures.'}]
    for index in range(80):
        tool_heavy.extend([{'type': 'function_call', 'id': f'fc-{index}', 'call_id': f'call-{index}',
            'name': 'native_probe', 'arguments': '{}'}, {'type': 'function_call_output',
            'call_id': f'call-{index}', 'output': escaped * 100}])
    tool_heavy.append({'role': 'user', 'content': 'Summarize the fixture results.'})

    def measure(codec, window, workload, messages, phase, count):
        nonlocal sequence
        before = accepted_requests(product)
        bearer = product.bearer(product.agent_connection)
        body = {'model': product.model_alias, 'input': messages}
        if workload == 'tool_heavy':
            body['tools'] = [{'type': 'function', 'name': 'native_probe', 'description': 'Read fixture', 'parameters': {'type': 'object', 'properties': {}}}]
        wire = encoded(body)
        decision_before = len(Endpoint.requests)
        p_before, m_before = len(provider_times), len(model_times)
        usage = Usage(product.process.pid)
        first = sequence; sequence += count
        def request(index):
            connection = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
            started = time.monotonic()
            try:
                connection.request('POST', '/v1/responses', wire, {'Content-Type': 'application/json',
                    'X-HiRoute-Token': bearer, 'session-id': f'resource-{first + index}'})
                response = connection.getresponse(); raw = response.read()
                elapsed = time.monotonic() - started
                assert response.status == 200 and json.loads(raw)['status'] == 'completed'
                return elapsed
            finally: connection.close()
        started = time.monotonic()
        if workload in ('concurrent_history', 'populate_unrelated_sessions'):
            with ThreadPoolExecutor(max_workers=count) as executor: latencies = list(executor.map(request, range(count)))
        else:
            latencies = [request(index) for index in range(count)]
        foreground = time.monotonic() - started
        deadline = time.monotonic() + 30
        while True:
            requests = accepted_requests(product) - before
            if len(requests) == count and all(complete_content(product, r) for r in requests): break
            assert time.monotonic() < deadline, 'accepted capture failed to drain'
            assert not durable_gaps(product), 'accepted capture has a durable gap'
            time.sleep(.02)
        # Include already accepted asynchronous work through stable post-capture samples.
        time.sleep(.1)
        measured = usage.finish()
        measured.update(codec=codec, request_window_label=window, phase=phase, workload=workload,
            requests=count, foreground_each_ms=[round(t * 1000, 3) for t in latencies],
            foreground_batch_ms=round(foreground * 1000, 3),
            capture_drain_after_foreground_ms=round((time.monotonic() - started - foreground) * 1000, 3),
            decision_attempts=len(Endpoint.requests)-decision_before,
            decision_wire_bytes=[v['bytes'] for v in Endpoint.requests[decision_before:]],
            external_decision_handler_ms=[round(t * 1000, 3) for t in provider_times[p_before:]],
            external_model_handler_ms=[round(t * 1000, 3) for t in model_times[m_before:]])
        assert len(model_times) - m_before == count, 'ordinary model request missing'
        if workload in ('short', 'long_current'):
            assert all(forwarded_text(r['body']) == messages[0]['content'] for r in upstream.requests[-count:])
        rows.append(measured)

    try:
        bootstrap(product)
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        saved = save_native_source(product, upstream, NATIVE_TOKEN, cli_input=True)
        for codec in ['system_one', 'openai_decisions']:
            for window in [32000, 65536, 262144, 1050000]:
                service = save(codec, window)
                if not getattr(product, 'editor', None) or product.editor['mode'] != 'custom_branches':
                    publish_plan_and_agent(product, saved['binding_id'], native_model_mode='hiroute_only', branch_service=service)
                else: publish(service)
                for workload, messages in [('short', short), ('long_history', history), ('long_current', current), ('tool_heavy', tool_heavy)]:
                    product.stop(); product.start()
                    measure(codec, window, workload, messages, 'cold', 1)
                    measure(codec, window, workload, messages, 'warm', 3)
                measure(codec, window, 'concurrent_history', history, 'warm', 4)
        measure('openai_decisions', 1050000, 'unrelated_growth_before', short, 'warm', 4)
        source_count = len(data('decision sources list')['sources'])
        for index in range(16): save_native_source(product, upstream, NATIVE_TOKEN, variant=f'unrelated-{index}', cli_input=True)
        assert len(data('decision sources list')['sources']) == source_count + 16
        # Real unrelated native sessions/history are admitted through the same listener.
        measure('openai_decisions', 1050000, 'populate_unrelated_sessions', short, 'warm', 32)
        measure('openai_decisions', 1050000, 'unrelated_growth_after', short, 'warm', 4)
        result = {'scenario': 'decision-context-resource-matrix', 'state': 'green',
            'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip(),
            'driver_mode': 'baseline_fixed_262144_wire_bytes' if legacy else 'frozen_adaptive_windows',
            'binary_sha256': {name: hashlib.sha256((product.bin / name).read_bytes()).hexdigest() for name in ['hiroute', 'hirouted']},
            'limits': 'One cold and three warm samples per cell, one four-request concurrent batch. Loopback providers only. Wire preparation by the caller and setup are outside the timer; foreground includes fallback and upstream wait. Handler times overlap and must not be subtracted from concurrent wall time. RSS is sampled for the daemon, not an allocator or whole-host bound.',
            'unrelated_sources_added': 16, 'unrelated_sessions_added': 32, 'rows': rows}
        assert len(rows) == 75
        output.parent.mkdir(parents=True, exist_ok=True); output.write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'scenario': result['scenario'], 'state': 'green', 'revision': result['revision'], 'cells': len(rows)}), flush=True)
    finally:
        product.close(); upstream.close(); server.shutdown(); server.server_close(); worker.join(timeout=5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository', type=Path)
    parser.add_argument('--legacy', action='store_true')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--baseline-checkout', type=Path,
        default=os.environ.get('HIROUTE_DECISION_CONTEXT_BASELINE_CHECKOUT'))
    args = parser.parse_args()
    input_file = args.repository / 'artifacts/decision-context-resources/input.json'
    if args.baseline_checkout is None and input_file.is_file():
        resource_input = json.loads(input_file.read_text())
        assert set(resource_input) == {'baseline_checkout'}, 'unknown resource input fields'
        args.baseline_checkout = Path(resource_input['baseline_checkout']).resolve(strict=True)
    output = args.output or args.repository / 'artifacts/decision-context-resources/candidate.json'
    if args.baseline_checkout:
        if args.legacy:
            parser.error('--legacy and --baseline-checkout are mutually exclusive')
        # The managed candidate receipt must never select the baseline's binaries.
        candidate_bin = os.environ.pop('HIROUTE_VALIDATION_PRODUCT_BIN_DIR', None)
        try:
            run(args.baseline_checkout, True, output.with_name('baseline.json'))
        finally:
            if candidate_bin is not None:
                os.environ['HIROUTE_VALIDATION_PRODUCT_BIN_DIR'] = candidate_bin
    run(args.repository, args.legacy, output)
