"""Zero-model pressure through real Gateway capture, production sink and storage.

Three consecutive turns and four concurrent streams exercise asynchronous persistence.
There is no environment file collector, paid endpoint or modified request deadline.
"""
from concurrent.futures import ThreadPoolExecutor
import http.client
import http.server
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading
import time

from model_connections_product import MODEL, NATIVE_TOKEN, save_native_source
from observation_content_fault_product import activity_database, complete_content, durable_gaps
from publication_process import configure_model_settings_v2
from publication_product import Product, encoded
from usage_query_gateway_product import json_event, publish_plan

PARTS = 4096
TURNS = 3
CONCURRENT = 4


class DaemonUsage:
    """Sample this exact daemon through foreground forwarding and capture drain."""
    def __init__(self, pid):
        self.pid, self.samples, self.errors = pid, [], []
        self.stop = threading.Event()
        self.sample()
        self.thread = threading.Thread(target=self.watch, daemon=True)
        self.thread.start()

    def sample(self):
        values = subprocess.check_output(
            ['ps', '-p', str(self.pid), '-o', 'rss=', '-o', 'time='],
            text=True, timeout=2).split()
        assert len(values) == 2, 'exact daemon resource observation unavailable'
        cpu = sum(float(part) * 60 ** index
                  for index, part in enumerate(reversed(values[1].split(':'))))
        self.samples.append((int(values[0]), cpu))

    def watch(self):
        while not self.stop.wait(.5):
            try:
                self.sample()
            except (OSError, subprocess.SubprocessError, ValueError, AssertionError) as error:
                self.errors.append(type(error).__name__)

    def finish(self):
        self.stop.set()
        self.thread.join(timeout=3)
        assert not self.thread.is_alive(), 'resource observer did not stop'
        self.sample()
        assert not self.errors, ('resource observation gaps', self.errors)
        return {'scope': 'exact_daemon_forwarding_through_capture_drain',
                'sample_period_seconds': .5, 'samples': len(self.samples),
                'initial_rss_kib': self.samples[0][0],
                'sampled_peak_rss_kib': max(row[0] for row in self.samples),
                'end_rss_kib': self.samples[-1][0],
                'cpu_seconds': self.samples[-1][1] - self.samples[0][1]}


def fragment(index):
    return f"streamed field {index:04d} 中文; "


def stream_frames():
    yield json_event('response.created', {'type': 'response.created', 'response': {
        'id': 'pressure-response', 'model': MODEL, 'status': 'in_progress'}})
    for index in range(PARTS):
        yield json_event('response.output_text.delta', {
            'type': 'response.output_text.delta', 'item_id': 'pressure-message',
            'output_index': 0, 'content_index': 0, 'delta': fragment(index)})
    yield json_event('response.completed', {'type': 'response.completed', 'response': {
        'id': 'pressure-response', 'model': MODEL, 'status': 'completed',
        'output': [{'type': 'message', 'id': 'pressure-message', 'role': 'assistant',
                    'status': 'completed', 'content': [{'type': 'output_text',
                    'text': ''.join(map(fragment, range(PARTS))), 'annotations': []}]}],
        'usage': {'input_tokens': 10, 'output_tokens': 4096, 'total_tokens': 4106}}})


class PressureUpstream:
    def __init__(self):
        self.requests = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def do_GET(self):
                assert self.path == '/v1/models'
                self.send_response(404)
                self.send_header('Content-Length', '0')
                self.end_headers()

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert self.path == '/v1/responses' and body['model'] == MODEL
                assert body['stream'] is True
                assert self.headers.get('Authorization') == 'Bearer ' + NATIVE_TOKEN
                owner.requests.append(body)
                frames = list(stream_frames())
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Content-Length', str(sum(map(len, frames))))
                self.send_header('Connection', 'close')
                self.end_headers()
                for frame in frames:
                    self.wfile.write(frame)
                    self.wfile.flush()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f'http://127.0.0.1:{self.server.server_port}/v1'

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def durable_content(product, expected=TURNS):
    with activity_database(product) as db:
        requests = [row[0] for row in db.execute(
            "SELECT request_id FROM logical_requests WHERE outcome='accepted' "
            "AND finished_at_ms IS NOT NULL ORDER BY request_id")]
    return requests if len(requests) == expected and all(
        complete_content(product, request) for request in requests) else []


def verify_exact_bytes(product, requests):
    semantic_parts = 0
    activity = product.storage / 'observation/activity.db'
    assert activity.is_file()
    with activity_database(product) as db:
        for request in requests:
            rows = db.execute(
                "SELECT c.part_ordinal,b.object_path,b.byte_count FROM content_instances_v2 c "
                "JOIN content_blobs_v2 b ON b.workspace_id=c.workspace_id "
                "AND b.blob_digest=c.content_blob_digest WHERE c.request_id=? "
                "AND c.direction='response_delivered' AND c.content_kind='text_delta' "
                "AND c.state='complete' AND b.state='complete' ORDER BY c.part_ordinal",
                (request,)).fetchall()
            expected = ''.join(map(fragment, range(PARTS)))
            assert 0 < len(rows) <= (len(expected.encode()) + 65535) // 65536, (len(rows), PARTS)
            semantic_parts += len(rows)
            texts = []
            for _, locator, byte_count in rows:
                relative = Path(locator)
                assert not relative.is_absolute() and '..' not in relative.parts
                assert relative.parts[0] == 'conversation-content'
                raw = (activity.parent / relative).read_bytes()
                assert len(raw) == byte_count
                event = json.loads(raw)['event']
                assert event['kind'] == 'text_delta' and event['index'] == 0
                assert len(event['text'].encode()) <= 65536
                texts.append(event['text'])
            assert ''.join(texts) == expected, 'semantic segmentation changed accepted content'
    return semantic_parts


def run(repository):
    product, upstream = Product(repository), PressureUpstream()
    usage = None
    product.enable_debug_diagnostics()
    # The normal default-budget workload must not require the new encoded
    # spill lane. This test-only guard fails at that exact write path; original
    # raw Replay behavior and all HTTP/persistence assertions remain unchanged.
    product.env['HIROUTE_TEST_OBSERVATION_CONTENT_OVERFLOW'] = 'forbid'
    try:
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN)
        _, product.model_alias = publish_plan(product, saved['binding_id'], 'responses')
        configure_model_settings_v2(product, [product.plan_id], 'content-pressure-configure')
        usage = DaemonUsage(product.process.pid)
        forwarded = []
        messages = []
        for turn in range(TURNS):
            messages.append({'role': 'user', 'content': f'pressure turn {turn}'})
            client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
            started = time.monotonic()
            try:
                client.request('POST', '/v1/responses', encoded({
                    'model': product.model_alias, 'input': messages, 'stream': True}), {
                    'Content-Type': 'application/json',
                    'X-HiRoute-Token': product.bearer(product.agent_connection),
                    'session-id': 'content-pressure-three-turns'})
                response = client.getresponse()
                payload = response.read()
                assert response.status == 200
                events = [json.loads(line[6:]) for line in payload.decode().splitlines()
                          if line.startswith('data: ') and line[6:] != '[DONE]']
                deltas = [event['delta'] for event in events
                          if event['type'] == 'response.output_text.delta']
                assert deltas == list(map(fragment, range(PARTS)))
                assert events[-1]['type'] == 'response.completed'
                forwarded.append(time.monotonic() - started)
                messages.append({'role': 'assistant', 'content': ''.join(deltas)})
            finally:
                client.close()
        assert len(upstream.requests) == TURNS
        deadline = time.monotonic() + 90
        while not (requests := durable_content(product)):
            assert time.monotonic() < deadline, 'accepted capture never became durable and complete'
            assert not durable_gaps(product), 'production content channel lost an admitted record'
            time.sleep(.05)
        verify_exact_bytes(product, requests)
        first_requests = set(requests)
        # Authenticate once outside parallel request execution. Every request
        # uses a distinct native session, so this tests actual overlap rather
        # than the same conversation's deliberate turn serialization.
        bearer = product.bearer(product.agent_connection)

        def concurrent_request(index):
            client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=30)
            started = time.monotonic()
            try:
                client.request('POST', '/v1/responses', encoded({
                    'model': product.model_alias,
                    'input': [{'role': 'user', 'content': f'parallel pressure {index}'}],
                    'stream': True}), {
                    'Content-Type': 'application/json', 'X-HiRoute-Token': bearer,
                    'session-id': f'content-pressure-parallel-{index}'})
                response = client.getresponse()
                payload = response.read()
                assert response.status == 200
                events = [json.loads(line[6:]) for line in payload.decode().splitlines()
                          if line.startswith('data: ') and line[6:] != '[DONE]']
                assert [event['delta'] for event in events
                        if event['type'] == 'response.output_text.delta'] == list(map(fragment, range(PARTS)))
                assert events[-1]['type'] == 'response.completed'
                return time.monotonic() - started
            finally:
                client.close()

        with ThreadPoolExecutor(max_workers=CONCURRENT) as executor:
            parallel_seconds = list(executor.map(concurrent_request, range(CONCURRENT)))
        expected = TURNS + CONCURRENT
        assert len(upstream.requests) == expected
        deadline = time.monotonic() + 90
        while not (requests := durable_content(product, expected)):
            assert time.monotonic() < deadline, 'concurrent accepted capture never became durable and complete'
            assert not durable_gaps(product), 'concurrent content channel lost an admitted record'
            time.sleep(.05)
        assert first_requests.issubset(requests)
        semantic_parts = verify_exact_bytes(product, requests)
        assert not durable_gaps(product)
        resources = usage.finish()
        usage = None
        product.stop()
        product.start()
        assert durable_content(product, expected) == requests
        verify_exact_bytes(product, requests)
        print(json.dumps({'scenario': 'production-content-pressure', 'state': 'green',
                          'turns': TURNS, 'concurrent_requests': CONCURRENT,
                          'accepted_wire_deltas': expected * PARTS,
                          'semantic_text_parts': semantic_parts,
                          'parallel_seconds': parallel_seconds,
                          'forward_seconds': forwarded, 'resources': resources, 'reopen_exact': True}), flush=True)
    finally:
        if usage is not None:
            try:
                resources = usage.finish()
            except (OSError, subprocess.SubprocessError, ValueError, AssertionError) as error:
                resources = {'state': 'unavailable', 'error_type': type(error).__name__}
            print(json.dumps({'scenario': 'production-content-pressure-resources',
                              'capture_drain_complete': False,
                              'resources': resources}), flush=True)
        product.close()

        upstream.close()


if __name__ == '__main__':
    run(sys.argv[1])
