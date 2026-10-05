"""Owned receipt skills and protocol oracle for an explicitly selected native context.

The setup CLI creates a NEW private directory for a real Desktop/Pilot journey.
It does not launch agents, install settings through IPC, or claim a product verdict.
"""
import argparse
import hashlib
import http.server
import json
from pathlib import Path
import secrets
import shlex
import threading


BUSINESS_CASES = tuple('worker.context.' + name for name in (
    'native-skills', 'concurrent-routing', 'run-authority', 'exact-continue',
    'cancel-owned-work', 'retention-ownership'))
CORE_CASES = (BUSINESS_CASES[0], BUSINESS_CASES[3])
CONTINUE_PROMPT = 'NATIVE-CONTEXT-CONTINUE: continue the same task using its previous tool history.'


def write_new(path, text, executable=False):
    missing = []
    parent = path.parent
    while not parent.exists():
        missing.append(parent)
        parent = parent.parent
    # mkdir(parents=True) ignores mode for intermediate ancestors. Create only
    # our absent directories privately; never change a borrowed parent's mode.
    for parent in reversed(missing):
        parent.mkdir(mode=0o700)
    with path.open('x', encoding='utf-8') as stream:
        stream.write(text)
    path.chmod(0o700 if executable else 0o600)


def prepare_skill(parent, name, scope, writer=write_new):
    nonce = secrets.token_hex(12)
    directory = parent / name
    discovery, contents, executed = ('discovery-' + nonce, 'contents-' + nonce, 'executed-' + nonce)
    script = directory / 'receipt.sh'
    writer(script, '#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(executed) + '\n', True)
    skill = directory / 'SKILL.md'
    writer(skill, '\n'.join((
        '---', 'name: ' + name,
        'description: Native context read-only receipt ' + discovery, '---', '',
        'Read this skill and run its receipt script using the native shell tool.',
        'Content receipt: ' + contents, 'Script: ' + str(script), '',
    )))
    return dict(name=name, scope=scope, discovery=discovery, contents=contents,
                       executed=executed, skill=str(skill), script=str(script))


def prepare(home, config, project, harness, suffix='', owned_files=None):
    """Add only new fixture-owned skills and neighbors; never replace existing files."""
    assert harness in ('codex', 'claude', 'qoder', 'pi')
    home, config, project = map(lambda path: Path(path).resolve(), (home, config, project))
    def owned_write(path, text, executable=False):
        write_new(path, text, executable)
        if owned_files is not None:
            owned_files[str(path)] = digest(path)

    skills = []
    for scope, parent in (
        ('user', home / '.agents/skills' if harness == 'codex' else config / 'skills'),
        ('project', project / {'codex': '.agents/skills', 'claude': '.claude/skills', 'qoder': '.qoder/skills', 'pi': '.pi/skills'}[harness]),
    ):
        skills.append(prepare_skill(parent, 'native-context-' + scope + suffix, scope, owned_write))
    neighbor = config / ('native-context-neighbor' + suffix + '.txt')
    owned_write(neighbor, 'A neighboring user file must survive Worker cleanup.\n')
    protected = [neighbor, *(Path(item[key]) for item in skills for key in ('skill', 'script'))]
    return dict(harness=harness, home=str(home), config=str(config), project=str(project),
                skills=skills, receipt='completed-' + secrets.token_hex(12),
                protected={str(path): digest(path) for path in protected})


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def protect_configuration(fixture, paths):
    fixture['protected'].update({str(path): digest(path) for path in paths})


def assert_preserved(fixture):
    if fixture.get('foreign_route_events'):
        assert not Path(fixture['foreign_route_events']).read_text().strip(), \
            'native Worker used the foreign project model route'
    for path, expected in fixture['protected'].items():
        assert digest(path) == expected, 'changed native user material: ' + path
    if fixture.get('proxy_trap_events'):
        assert not Path(fixture['proxy_trap_events']).read_text().strip(), \
            'native Worker used the user-configured proxy'


def tool_results(body, harness):
    """Only correlated tool-result records count, never arbitrary prompt substrings."""
    if harness in ('codex', 'qoder', 'pi'):
        source = body.get('input', [])
        return {item.get('call_id'): item.get('output', '') for item in source
                if isinstance(item, dict) and item.get('type') == 'function_call_output'} \
            if isinstance(source, list) else {}
    return {block.get('tool_use_id'): block.get('content', '')
            for message in body.get('messages', [])
            for block in (message.get('content', []) if isinstance(message.get('content'), list) else [])
            if isinstance(block, dict) and block.get('type') == 'tool_result'
            and block.get('is_error') is not True}


def qoder_skill_expansions(body, directory):
    """Read exact native Skill user-message expansions, never a path substring."""
    expected_header = 'Base directory for this skill: ' + str(directory)
    for item in body.get('input', []):
        if not isinstance(item, dict) or item.get('role') != 'user':
            continue
        content = item.get('content', [])
        texts = ([content] if isinstance(content, str) else
                 [part.get('text', '') for part in content if isinstance(part, dict)])
        for text in texts:
            header, separator, contents = text.partition('\n\n')
            if separator and header == expected_header:
                yield contents


def native_text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return '\n'.join(native_text(item) for item in value)
    if isinstance(value, dict):
        return '\n'.join(native_text(item) for key, item in value.items()
                         if key in ('text', 'content', 'input', 'instructions', 'output'))
    return ''


def is_native_compaction(body):
    """Pinned Qoder 1.1.65 request signature, independent of request order."""
    items = body.get('input', [])
    last = native_text(items[-1]) if isinstance(items, list) and items else native_text(items)
    return 'detailed summary' in last.lower() and 'summarization' in last.lower()


def decision(fixture, body):
    """Small deterministic model: require native discovery, then real tools, then history."""
    harness = fixture['harness']
    if harness == 'qoder' and 'expected_max_output_tokens' in fixture:
        assert body.get('max_output_tokens') == fixture['expected_max_output_tokens'], \
            'Qoder request did not carry the frozen source output budget'
    assert harness != 'qoder' or not is_native_compaction(body), \
        'unexpected Qoder compaction in the ordinary Skill journey; check the native context budget'
    rendered = json.dumps(body)
    results = tool_results(body, harness)
    result_text = json.dumps(results.get('native_context_execute', ''))
    if CONTINUE_PROMPT in rendered:
        # This receipt was only emitted by the prior native shell tool, not by the prompt.
        assert all(skill['executed'] in result_text for skill in fixture['skills']), \
            'Continue did not retain the exact prior tool result'
        return dict(kind='text', text='continued-' + fixture['receipt'], continued=True)
    if 'native_context_execute' in results:
        assert all(skill['contents'] in result_text and skill['executed'] in result_text
                   for skill in fixture['skills']), 'native skill execution receipt mismatch'
        return dict(kind='text', text=fixture['receipt'], continued=False)
    for skill in fixture['skills']:
        assert skill['discovery'] in rendered or skill['contents'] in rendered, \
            'native skill discovery missing: ' + skill['scope']
    names = {tool.get('name') for tool in body.get('tools', [])}
    if harness in ('claude', 'qoder'):
        assert 'Skill' in names, 'native Skill tool unavailable'
        for skill in fixture['skills']:
            call_id = 'native_context_skill_' + skill['scope']
            if call_id not in results:
                return dict(kind='tool', id=call_id, name='Skill',
                            arguments={'skill': skill['name']})
            # Some native releases expand the body in a following user message rather
            # than in the Skill tool_result itself. The matching non-error call is still required.
            assert skill['contents'] in rendered, 'Skill did not load its body'
            if harness == 'qoder':
                contents = qoder_skill_expansions(body, Path(skill['skill']).parent)
                assert any(skill['contents'] in text for text in contents), \
                    'expected native Skill directory was not loaded'
        assert 'Bash' in names, 'native Bash tool unavailable'
        name = 'Bash'
    elif harness == 'pi':
        assert 'bash' in names and 'read' in names, 'native Pi tools unavailable'
        name = 'bash'
    else:
        name = next((name for name in ('exec_command', 'shell_command') if name in names), None)
        assert name, 'native shell tool unavailable'
    command = ' && '.join('/bin/cat ' + shlex.quote(skill['skill']) + ' && /bin/sh '
                          + shlex.quote(skill['script']) for skill in fixture['skills'])
    arguments = ({'cmd': command, 'max_output_tokens': 2000, 'yield_time_ms': 1000}
                 if name == 'exec_command' else {'command': command, 'timeout_ms': 10000}
                 if name == 'shell_command' else {'command': command, 'timeout': 10 if harness == 'pi' else 10000})
    return dict(kind='tool', id='native_context_execute', name=name, arguments=arguments)


def events(body, action, protocol):
    """Use the real native client's declared protocol; no fake ACP adapter."""
    if protocol == 'messages':
        tool = action['kind'] == 'tool'
        block = ({'type': 'tool_use', 'id': action['id'], 'name': action['name'],
                  'input': action['arguments']} if tool else {'type': 'text', 'text': action['text']})
        initial = dict(block, input={}) if tool else dict(block, text='')
        delta = ({'type': 'input_json_delta', 'partial_json': json.dumps(action['arguments'])}
                 if tool else {'type': 'text_delta', 'text': action['text']})
        return [
            ('message_start', {'message': {'id': 'msg_native_context', 'type': 'message',
              'role': 'assistant', 'model': body['model'], 'content': [], 'stop_reason': None,
              'stop_sequence': None, 'usage': {'input_tokens': 1, 'output_tokens': 0}}}),
            ('content_block_start', {'index': 0, 'content_block': initial}),
            ('content_block_delta', {'index': 0, 'delta': delta}),
            ('content_block_stop', {'index': 0}),
            ('message_delta', {'delta': {'stop_reason': 'tool_use' if tool else 'end_turn',
                                       'stop_sequence': None}, 'usage': {'output_tokens': 1}}),
            ('message_stop', {}),
        ]
    item = ({'type': 'function_call', 'id': 'fc_' + action['id'], 'call_id': action['id'],
             'name': action['name'], 'arguments': json.dumps(action['arguments']), 'status': 'completed'}
            if action['kind'] == 'tool' else
            {'type': 'message', 'id': 'msg_native_context', 'role': 'assistant', 'status': 'completed',
             'content': [{'type': 'output_text', 'text': action['text'], 'annotations': []}]})
    response = {'id': 'resp_native_context', 'object': 'response', 'model': body['model'],
                'status': 'completed', 'output': [item],
                'usage': {'input_tokens': action.get('input_tokens', 1), 'output_tokens': 1,
                          'total_tokens': action.get('input_tokens', 1) + 1}}
    added = dict(item, status='in_progress')
    added['arguments' if action['kind'] == 'tool' else 'content'] = '' if action['kind'] == 'tool' else []
    middle = ([('response.function_call_arguments.delta', {'item_id': item['id'],
                'output_index': 0, 'delta': item['arguments']}),
               ('response.function_call_arguments.done', {'item_id': item['id'],
                'output_index': 0, 'arguments': item['arguments']})]
              if action['kind'] == 'tool' else [
                  ('response.content_part.added', {'item_id': item['id'], 'output_index': 0,
                   'content_index': 0, 'part': {'type': 'output_text', 'text': '', 'annotations': []}}),
                  ('response.output_text.delta', {'item_id': item['id'], 'output_index': 0,
                   'content_index': 0, 'delta': action['text']}),
                  ('response.output_text.done', {'item_id': item['id'], 'output_index': 0,
                   'content_index': 0, 'text': action['text']})])
    return [('response.created', {'response': dict(response, status='in_progress', output=[])}),
            ('response.output_item.added', {'output_index': 0, 'item': added}), *middle,
            ('response.output_item.done', {'output_index': 0, 'item': item}),
            ('response.completed', {'response': response})]


def handle(handler, body, controls, lock, reply=decision):
    path = controls / 'native-context.json'
    if not path.exists():
        return False
    fixture = json.loads(path.read_text())
    protocol = 'messages' if handler.path == '/v1/messages' else 'responses'
    try:
        action = reply(fixture, body)
    except AssertionError as error:
        with lock, (controls / 'native-context-events.jsonl').open('a') as stream:
            stream.write(json.dumps({'state': 'red', 'reason': str(error)}) + '\n')
        handler.send({'error': {'message': str(error), 'type': 'invalid_request_error'}}, 400)
        return True
    with lock, (controls / 'native-context-events.jsonl').open('a') as stream:
        stream.write(json.dumps({'state': 'green', 'action': action['kind'],
                                'call_id': action.get('id'), 'continued': action.get('continued'),
                                'model': body['model'], 'protocol': protocol,
                                **({'request_kind': action['request_kind']} if 'request_kind' in action else {})}) + '\n')
    handler.send_messages_stream(events(body, action, protocol))
    return True


class NativeProxyTrap:
    """Count every proxy method without recording URLs, headers, bodies or credentials."""

    def __init__(self, path):
        self.path = Path(path)
        write_new(self.path, '')
        lock = threading.Lock()
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reject_proxy(self):
                with lock, owner.path.open('a') as stream:
                    stream.write('proxy-request\n')
                self.send_response(502)
                self.send_header('Content-Length', '0')
                self.send_header('Connection', 'close')
                self.end_headers()
                self.close_connection = True

            def __getattr__(self, name):
                # BaseHTTPRequestHandler dispatches even custom methods through
                # do_<method>; count those too, without retaining request content.
                if name.startswith('do_'):
                    return self.reject_proxy
                raise AttributeError(name)

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = 'http://%s:%d' % self.server.server_address

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def install_proxy_conflict(fixture, upstream):
    """Keep the proxy alive with the model fixture, including real Pilot takeover."""
    assert fixture['harness'] == 'claude'
    assert upstream.proxy_trap is None
    trap = NativeProxyTrap(upstream.controls / 'native-context-proxy-attempts.log')
    upstream.proxy_trap = trap
    fixture['proxy_trap_events'] = str(trap.path)
    return {**{name: trap.url for name in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY',
                                          'http_proxy', 'https_proxy', 'all_proxy')},
            'NO_PROXY': 'synthetic-no-bypass.invalid', 'no_proxy': 'synthetic-no-bypass.invalid'}


class NativeContextUpstream:
    """Loopback model server shared by headless and real Desktop journeys."""
    token = 'synthetic-native-context-source-token'
    model = 'gpt-5.4'

    def __init__(self, controls):
        owner = self
        self.controls = Path(controls)
        self.lock = threading.Lock()
        self.reply = decision
        self.requests = 0
        self.proxy_trap = None

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def send(self, value, status=200):
                body = json.dumps(value).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.send_header('Connection', 'close')
                self.close_connection = True
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                # A manually declared model remains usable without directory discovery.
                self.send({'error': 'directory not implemented'}, 404)

            def do_POST(self):
                with owner.lock:
                    owner.requests += 1
                if self.headers.get('Authorization') != 'Bearer ' + owner.token:
                    return self.send({'error': 'incorrect synthetic source credential'}, 401)
                if self.path not in ('/v1/responses', '/v1/messages'):
                    return self.send({'error': 'incorrect source endpoint'}, 404)
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if body.get('model') != owner.model:
                    return self.send({'error': 'incorrect source model'}, 400)
                if not handle(self, body, owner.controls, owner.lock, owner.reply):
                    self.send({'error': 'native context fixture is not prepared'}, 503)

            def send_messages_stream(self, frames):
                encoded = []
                for index, (kind, value) in enumerate(frames):
                    payload = dict(value, type=kind)
                    if self.path == '/v1/responses':
                        payload['sequence_number'] = index
                    encoded.append(f'event: {kind}\ndata: {json.dumps(payload)}\n\n'.encode())
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Content-Length', str(sum(map(len, encoded))))
                self.end_headers()
                for frame in encoded:
                    self.wfile.write(frame)
                    self.wfile.flush()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = 'http://%s:%d/v1' % self.server.server_address

    def request_count(self):
        """All source attempts, including rejected credentials; never record a secret."""
        with self.lock:
            return self.requests

    def close(self):
        try:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=5)
        finally:
            if self.proxy_trap:
                self.proxy_trap.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True, help='new private synthetic context root')
    parser.add_argument('--harness', choices=('codex', 'claude'), required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    root.mkdir(mode=0o700)  # Refuse any existing directory: never adopt daily user material.
    home, project = root / 'home', root / 'project'
    home.mkdir(mode=0o700)
    project.mkdir(mode=0o700)
    config = home / ('.codex' if args.harness == 'codex' else '.claude')
    fixture = prepare(home, config, project, args.harness)
    write_new(root / 'native-context.json', json.dumps(fixture, indent=2) + '\n')
    print(json.dumps({'state': 'prepared', 'harness': args.harness, 'process_home': str(home),
                      'workspace': str(project), 'fixture': str(root / 'native-context.json')}))


if __name__ == '__main__':
    main()
