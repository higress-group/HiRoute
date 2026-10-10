#!/usr/bin/python3
"""Counted third-party CPA fixture; HiRoute still owns all admission, stores and routing.

Only the external CPA process is simulated. Implement its pinned stock management contract
and deterministic Responses/Messages endpoints, never HiRoute's business APIs or persistence.
"""
import http.server
import json
from pathlib import Path
import re
import sys
import threading
import time
from urllib.parse import parse_qs, urlparse

VERSION = '8.0.4-hiroute.4'
controls = Path(__file__).resolve().parent
model_override = controls / 'model-id'
MODEL = (model_override.read_text().strip()
         if model_override.exists() else 'gpt-5.3-codex-spark')
assert MODEL and re.fullmatch(r'[A-Za-z0-9._-]+', MODEL)
if '--help' in sys.argv:
    print('CLIProxyAPI Version: ' + VERSION)
    sys.exit(0)
config = Path(sys.argv[sys.argv.index('--config') + 1]).read_text()

def scalar(key):
    value = re.search(r'^\s*' + re.escape(key) + r':\s*(.+)$', config, re.M).group(1)
    return value.strip().strip('\"\'')

auth_dir = Path(scalar('auth-dir'))
management = scalar('secret-key')
assert '--local-password-stdin' in sys.argv
assert sys.stdin.read() == management
downstream = re.search(r'api-keys:\s*\n\s*-\s*(\S+)', config).group(1).strip('\"\'')
count_lock = threading.Lock()


def account():
    path = next(p for p in auth_dir.glob('*.json') if json.loads(p.read_text()).get('type') == 'codex')
    return path, json.loads(path.read_text())


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def send(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('x-cpa-version', VERSION)
        self.end_headers()
        self.wfile.write(body)

    def authorized(self, secret):
        if self.headers.get('Authorization') != 'Bearer ' + secret:
            self.send({'error': 'unauthorized'}, 401)
            return False
        return True

    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path == '/healthz':
            return self.send({'status': 'ok'})
        if not self.authorized(management if parsed.path.startswith('/v0/') else downstream):
            return
        path, data = account()
        model = data['prefix'] + '/' + MODEL
        if parsed.path == '/v1/models':
            return self.send({'object': 'list', 'data': [{'id': model, 'object': 'model'}]})
        if parsed.path.endswith('/models'):
            return self.send({'models': [{'id': model}]})
        name = parse_qs(parsed.query).get('name', [''])[0]
        if name == '.__hiroute_ready_probe__':
            return self.send({'files': []})
        assert name == path.name
        self.send({'files': [{'id': path.name, 'name': path.name, 'auth_index': 'fixture-index',
                   'provider': 'codex', 'source': 'file', 'runtime_only': False,
                   'account_type': 'oauth', 'status': 'active', 'request_retry': data['request_retry']}]})

    def do_PATCH(self):
        if not self.authorized(management):
            return
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        path, data = account()
        assert body.pop('name') == path.name
        data.update(body)
        path.write_text(json.dumps(data))
        self.send({})

    def send_messages(self, body, assistant_text, progress_roundtrip):
        historical_tool_uses = sum(
            1
            for message in body.get('messages', [])
            for block in (message.get('content', [])
                          if isinstance(message.get('content', []), list) else [])
            if isinstance(block, dict) and block.get('type') == 'tool_use')
        latest = body.get('messages', [])[-1:]
        latest_content = latest[0].get('content', []) if latest else []
        latest_tool_result = isinstance(latest_content, list) and any(
            isinstance(block, dict) and block.get('type') == 'tool_result'
            for block in latest_content)
        if progress_roundtrip and not latest_tool_result:
            progress_ordinal = historical_tool_uses + 1
            arguments = json.dumps({'file_path': '/etc/hosts'})
            events = [
                ('message_start', {'message': {
                    'id': f'msg_fixture_progress_{progress_ordinal}',
                    'type': 'message', 'role': 'assistant',
                    'model': body['model'], 'content': [], 'stop_reason': None,
                    'stop_sequence': None,
                    'usage': {'input_tokens': 4, 'output_tokens': 0}}}),
                ('content_block_start', {
                    'index': 0, 'content_block': {'type': 'text', 'text': ''}}),
                ('content_block_delta', {
                    'index': 0,
                    'delta': {'type': 'text_delta', 'text': 'fixture progress one'}}),
                ('content_block_stop', {'index': 0}),
                ('content_block_start', {
                    'index': 1, 'content_block': {
                        'type': 'tool_use',
                        'id': f'toolu_fixture_progress_{progress_ordinal}',
                        'name': 'Read', 'input': {}}}),
                ('content_block_delta', {
                    'index': 1,
                    'delta': {'type': 'input_json_delta', 'partial_json': arguments}}),
                ('content_block_stop', {'index': 1}),
                ('message_delta', {
                    'delta': {'stop_reason': 'tool_use', 'stop_sequence': None},
                    'usage': {'input_tokens': 4, 'output_tokens': 2}}),
                ('message_stop', {}),
            ]
            return self.send_messages_stream(events)
        if progress_roundtrip:
            # Keep the native turn open after its completed assistant progress message.
            time.sleep(15)
            assistant_text = 'fixture answer'
        usage = {'input_tokens': 4, 'output_tokens': 2}
        response = {
            'id': 'msg_fixture', 'type': 'message', 'role': 'assistant',
            'model': body['model'],
            'content': [{'type': 'text', 'text': assistant_text}],
            'stop_reason': 'end_turn', 'stop_sequence': None, 'usage': usage,
        }
        if not body.get('stream'):
            return self.send(response)
        events = [
            ('message_start', {'message': dict(response, content=[], stop_reason=None,
                                               usage={'input_tokens': 4, 'output_tokens': 0})}),
            ('content_block_start', {
                'index': 0, 'content_block': {'type': 'text', 'text': ''}}),
            ('content_block_delta', {
                'index': 0, 'delta': {'type': 'text_delta', 'text': assistant_text}}),
            ('content_block_stop', {'index': 0}),
            ('message_delta', {
                'delta': {'stop_reason': 'end_turn', 'stop_sequence': None},
                'usage': usage}),
            ('message_stop', {}),
        ]
        return self.send_messages_stream(events)

    def send_messages_stream(self, events):
        frames = [
            ('event: ' + kind + '\ndata: '
             + json.dumps(dict(value, type=kind)) + '\n\n').encode()
            for kind, value in events
        ]
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(sum(len(frame) for frame in frames)))
        self.end_headers()
        for frame in frames:
            self.wfile.write(frame)
            self.wfile.flush()

    def do_POST(self):
        if not self.authorized(downstream):
            return
        assert self.path in ('/v1/responses', '/v1/messages')
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        tool_roundtrip = (controls / 'tool-roundtrip').exists()
        search_roundtrip = (controls / 'search-roundtrip').exists()
        pricing_roundtrip = (controls / 'pricing-roundtrip').exists()
        progress_roundtrip = (controls / 'progress-roundtrip').exists()
        search_declared = any(tool.get('type') == 'web_search' and tool.get('external_web_access') is True for tool in body.get('tools', []))
        search_item = {'type': 'web_search_call', 'id': 'ws_fixture_search', 'status': 'completed',
                       'action': {'type': 'search', 'query': 'isolated fixture search',
                                  'sources': [{'type': 'url', 'url': 'https://example.com/search'}]}}
        # Installed Codex discards optional action.sources even on a direct loopback
        # endpoint. Check its actual replay contract, not a fictitious SDK passthrough.
        expected_search_replay = dict(search_item, action={'type': 'search', 'query': 'isolated fixture search'})
        input_items = body.get('input', [])
        if isinstance(input_items, str):
            input_items = []  # Native Responses shorthand contains no tool items.
        search_replayed = expected_search_replay in input_items
        tool_outputs = [item for item in input_items if isinstance(item, dict) and item.get('type') == 'function_call_output']
        tool_output_valid = any(item.get('call_id') == 'fixture-plan-call' and 'Plan updated' in str(item.get('output', '')) for item in tool_outputs)
        with count_lock:
            with (controls / 'attempts.jsonl').open('a') as log:
                log.write(json.dumps({'model': body['model'], 'stream': body.get('stream', False),
                                      'delegated_goal': 'B-' in json.dumps(body),
                                      'search_declared': search_declared, 'search_replayed': search_replayed,
                                      'tool_output_valid': tool_output_valid}) + '\n')
        if 'hold-old-request' in json.dumps(body):
            (controls / 'held').touch()
            deadline = time.monotonic() + 30
            while not (controls / 'release').exists():
                assert time.monotonic() < deadline, 'held request deadline'
                time.sleep(.01)
        usage = {'input_tokens': 4, 'output_tokens': 2, 'total_tokens': 6}
        if pricing_roundtrip:
            usage.update(input_tokens_details={'cached_tokens': 0},
                         output_tokens_details={'reasoning_tokens': 0})
        assistant_text = ('fixture progress one fixture answer'
                          if progress_roundtrip else 'fixture answer')
        if self.path == '/v1/messages':
            return self.send_messages(body, assistant_text, progress_roundtrip)
        response = {'id': 'resp_fixture', 'object': 'response', 'status': 'completed',
                   'model': body['model'], 'output': [{'id': 'msg_fixture', 'type': 'message',
                   'role': 'assistant', 'status': 'completed', 'content': [
                       {'type': 'output_text', 'text': assistant_text, 'annotations': []}]}],
                   'usage': usage}
        if not body.get('stream'):
            return self.send(response)
        item = response['output'][0]
        events = [
            ('response.created', {'response': dict(response, status='in_progress', output=[])}),
            ('response.output_item.added', {'output_index': 0, 'item': dict(item, status='in_progress', content=[])}),
            ('response.content_part.added', {'item_id': item['id'], 'output_index': 0, 'content_index': 0,
                'part': {'type': 'output_text', 'text': '', 'annotations': []}}),
            ('response.output_text.delta', {'item_id': item['id'], 'output_index': 0, 'content_index': 0, 'delta': assistant_text}),
            ('response.output_text.done', {'item_id': item['id'], 'output_index': 0, 'content_index': 0, 'text': assistant_text}),
            ('response.output_item.done', {'output_index': 0, 'item': item}),
            ('response.completed', {'response': response}),
        ]
        if progress_roundtrip:
            events[3:4] = [
                ('response.output_text.delta', {
                    'item_id': item['id'], 'output_index': 0, 'content_index': 0,
                    'delta': 'fixture progress one'}),
                ('response.output_text.delta', {
                    'item_id': item['id'], 'output_index': 0, 'content_index': 0,
                    'delta': ' fixture answer'}),
            ]
        if tool_roundtrip and not tool_outputs:
            assert any(tool.get('name') == 'update_plan' for tool in body.get('tools', []))
            arguments = json.dumps({'plan': [{'step': 'B isolated plan update', 'status': 'completed'}]})
            item = {'type': 'function_call', 'id': 'fixture-plan-item', 'call_id': 'fixture-plan-call',
                    'name': 'update_plan', 'arguments': arguments, 'status': 'completed'}
            events = [
                ('response.created', {'response': dict(response, status='in_progress', output=[])}),
                ('response.output_item.added', {'output_index': 0, 'item': dict(item, status='in_progress', arguments='')}),
                ('response.function_call_arguments.delta', {'item_id': item['id'], 'output_index': 0, 'delta': arguments}),
                ('response.function_call_arguments.done', {'item_id': item['id'], 'output_index': 0, 'arguments': arguments}),
                ('response.output_item.done', {'output_index': 0, 'item': item}),
                ('response.completed', {'response': dict(response, output=[item])}),
            ]
        elif tool_roundtrip:
            assert tool_output_valid, 'tool output identity or result mismatch'
        if search_roundtrip:
            assert search_declared, 'real Codex must declare authorized live search'
            if not tool_outputs:
                for _, value in events[1:-1]:
                    if 'output_index' in value:
                        value['output_index'] = 1
                search_events = [
                    ('response.output_item.added', {'output_index': 0, 'item': dict(search_item, status='in_progress', action=None)}),
                    ('response.web_search_call.in_progress', {'output_index': 0, 'item_id': search_item['id']}),
                    ('response.web_search_call.searching', {'output_index': 0, 'item_id': search_item['id']}),
                    ('response.web_search_call.completed', {'output_index': 0, 'item_id': search_item['id']}),
                    ('response.output_item.done', {'output_index': 0, 'item': search_item}),
                ]
                events[1:1] = search_events
                events[-1][1]['response']['output'].insert(0, search_item)
            else:
                assert search_replayed, 'search action and exact native ID must survive actual Codex replay'
                citation = {'type': 'url_citation', 'start_index': 0, 'end_index': 14,
                            'title': 'Fixture search source', 'url': 'https://example.com/search'}
                response['output'][0]['content'][0]['annotations'] = [citation]
                events.insert(4, ('response.output_text.annotation.added', {
                    'output_index': 0, 'item_id': response['output'][0]['id'], 'content_index': 0,
                    'annotation_index': 0, 'annotation': citation}))
        frames = [
            ('event: ' + kind + '\ndata: '
             + json.dumps(dict(value, type=kind, sequence_number=index)) + '\n\n').encode()
            for index, (kind, value) in enumerate(events)
        ]
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(sum(len(frame) for frame in frames)))
        self.end_headers()
        first_progress_delta = next((index for index, (kind, _) in enumerate(events)
                                     if kind == 'response.output_text.delta'), None)
        for index, frame in enumerate(frames):
            self.wfile.write(frame)
            self.wfile.flush()
            if progress_roundtrip and index == first_progress_delta:
                (controls / 'progress-first-delta').touch()
                # The production default flushes a non-empty assistant batch every ten
                # seconds. Keep the upstream response open long enough to prove that a
                # public read can observe that batch before the turn completes.
                time.sleep(15)


http.server.ThreadingHTTPServer(('127.0.0.1', int(scalar('port'))), Handler).serve_forever()
