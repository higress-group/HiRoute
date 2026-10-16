"""Secure user PTY -> installed CLI -> protected socket -> saved decision -> routed requests.

The external decider chooses controlled probabilities, not semantic quality. No live credentials.
"""
import json
import os
import pty
import select
import signal
import subprocess
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def user_prompt(product, candidate, value, expected=0, format='bearer'):
    stdout = product.project / (candidate.rsplit('/', 1)[-1] + '.stdout')
    stderr = stdout.with_suffix('.stderr')
    gate_read, gate_write = os.pipe()
    pid, terminal = pty.fork()
    if pid == 0:
        os.close(gate_write)
        os.read(gate_read, 1)
        os.close(gate_read)
        for descriptor, path in [(1, stdout), (2, stderr)]:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.dup2(fd, descriptor)
            os.close(fd)
        os.chdir(product.project)
        os.execvpe(str(product.cli_path), [str(product.cli_path), 'protected-input', 'register',
            '--candidate', candidate, '--prompt', '--format', format], product.env)
    transcript = bytearray()
    original = termios.tcgetattr(terminal)
    os.close(gate_read)
    os.write(gate_write, b'1')
    os.close(gate_write)
    sent = False
    status = None
    try:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            readable, _, _ = select.select([terminal], [], [], .05)
            if readable:
                try:
                    transcript.extend(os.read(terminal, 4096))
                except OSError:
                    pass
            if not sent and b'(hidden' in transcript:
                hidden = termios.tcgetattr(terminal)
                assert not hidden[3] & termios.ECHO, 'secret entry was echoed'
                assert not hidden[3] & termios.ISIG, 'cancel would bypass cleanup'
                os.write(terminal, value)
                sent = True
            finished, child_status = os.waitpid(pid, os.WNOHANG)
            if finished:
                status = child_status
                break
        else:
            raise AssertionError('protected user prompt did not finish')
        assert sent
        assert termios.tcgetattr(terminal) == original, 'terminal state was not restored'
        assert os.waitstatus_to_exitcode(status) == expected
        output = stdout.read_bytes()
        errors = stderr.read_bytes()
        product.outputs.extend((bytes(transcript), output, errors))
        # Every printable probe is secret; never include it in failure messages.
        if len(value.rstrip(b'\n')) > 3:
            assert value.rstrip(b'\n') not in transcript + output + errors, 'secret leaked'
        return json.loads(output)
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(terminal)


class Decisions(BaseHTTPRequestHandler):
    complex = False
    requests = []
    credential = 'Bearer user-terminal-decision-probe'

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert self.headers.get('Authorization') == self.credential, 'protected header not resolved'
        self.requests.append(body)
        result = ({'answers': {key: (
            {'type': 'choice', 'choice': next(iter(question['criteria']))}
            if question['type'] == 'choice' else
            {'type': 'score', 'score': 2.0}
            if len(question['criteria']) == 3 else
            {'type': 'score', 'probabilities': {
                '0': .1 if self.complex else .9, '1': .9 if self.complex else .1}})
            for key, question in body['questions'].items()}} if 'questions' in body else
            {'decision': {'kind':'categorical', 'choice':body['decision']['options'][0]['id'],
                'refinement':{'kind':'ordinal', 'probabilities':{'simple':.9, 'complex':.1}}}}
            if body['decision']['kind'] == 'categorical' else
            {'decision': {'kind':'ordinal', 'probabilities':{'simple':.9, 'complex':.1}}})
        raw = json.dumps(result).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def run(product, plan, submit):
    options = product.cli('decision', 'services', 'options')['data']
    assert options['saved_services_state'] == 'known' and options['saved_services'] == []
    assert not Decisions.requests, 'options made an implicit probe'
    for control, suffix in [(b'\x03', 'cancel'), (b'\x04', 'eof')]:
        result = user_prompt(product, 'candidate/decision-' + suffix, control, expected=7)
        assert result['error']['code'] == 'ACTION_REQUIRED'
    result = user_prompt(product, 'candidate/decision-invalid', b'Bearer duplicate-key\n', expected=2)
    assert result['error']['code'] == 'INVALID_ARGUMENTS'
    # Piped input must never become a substitute for the user's controlling terminal.
    no_tty = subprocess.run([str(product.cli_path), 'protected-input', 'register', '--candidate',
        'candidate/no-tty', '--prompt', '--format', 'bearer'], env=product.env,
        input=b'must-not-be-read\n', capture_output=True, timeout=10, start_new_session=True)
    product.outputs.extend((no_tty.stdout, no_tty.stderr))
    assert no_tty.returncode == 7
    assert product.cli('decision', 'services', 'list')['data']['services'] == []
    product.secrets.update(('user-terminal-decision-probe', Decisions.credential))
    registered = user_prompt(product, 'candidate/decision-user', b'user-terminal-decision-probe\n')
    assert registered['data']['registered'] is True
    # Raw custom values use the same prompt but do not add a second scheme.
    raw_value = 'Basic protected-custom-probe'
    product.secrets.add(raw_value)
    raw_registered = user_prompt(product, 'candidate/decision-raw', raw_value.encode() + b'\n', format='raw')
    assert raw_registered['data']['registered'] is True

    endpoint = ThreadingHTTPServer(('127.0.0.1', 0), Decisions)
    worker = threading.Thread(target=endpoint.serve_forever, daemon=True)
    worker.start()
    try:
        path = product.project / 'decision-apply.json'
        args = ('decision', 'services', 'apply', '--id', 'decision-quickstart', '--name', 'User decision',
                '--provider', 'compatible', '--endpoint', f'http://127.0.0.1:{endpoint.server_port}/systemone',
                '--model', 'controlled-decision', '--candidate', 'candidate/decision-user')
        prepared = product.cli(*args, '--apply-file', str(path))['data']
        assert product.cli('decision', 'services', 'list')['data']['services'] == []
        assert 'user-terminal-decision-probe' not in path.read_text()
        submit(product, prepared)
        product.cli(*args, '--apply-file', str(path), expected=2)
        # An Apply wrapper plus --apply-file must not perform a write before rejecting input.
        product.cli('decision', 'services', 'apply', '--request-file', str(path), '--apply-file',
                    str(product.project / 'not-a-preview.json'), expected=2)
        assert not (product.project / 'not-a-preview.json').exists()
        current = product.cli('decision', 'services', 'options')['data']['saved_services']
        assert len(current) == 1 and current[0]['revision'] == 1
        assert not Decisions.requests, 'save or options made a probe'
        tested = product.cli('decision', 'services', 'test', '--service', 'decision-quickstart@1')['data']
        assert tested['outcome'] == 'passed', tested
        assert len(Decisions.requests) == 1
        product.cli('decision', 'services', 'test', '--service', 'decision-quickstart@2', expected=2)
        product.cli('decision', 'services', 'apply', '--id', 'decision-quickstart',
                    '--endpoint', 'https://another.example/systemone', expected=2)
        # Rename preserves the connection and credential reference; a concurrent edit goes stale.
        stale_file = product.project / 'decision-stale.json'
        stale = product.cli('decision', 'services', 'apply', '--id', 'decision-quickstart',
                            '--name', 'Stale name', '--apply-file', str(stale_file))['data']
        rename_file = product.project / 'decision-rename.json'
        rename = product.cli('decision', 'services', 'apply', '--id', 'decision-quickstart',
                             '--name', 'Renamed decision', '--apply-file', str(rename_file))['data']
        submit(product, rename)
        product.cli(*stale['submission']['arguments'], expected=3)
        saved = product.cli('decision', 'services', 'list')['data']['services']
        assert len(saved) == 1 and saved[0]['revision'] == 2
        assert saved[0]['connection'] == current[0]['connection']
        route_file = product.project / 'decision-route.json'
        route = product.cli('routing', 'preview', '--plan', plan, '--decision-service',
                            'decision-quickstart@2', '--apply-file', str(route_file))['data']
        editor = json.loads(route_file.read_text())['change']['editor']
        assert editor['smart']['classifier']['service'] == saved[0]
        submit(product, route)
        # The external CLI double must send distinct user turns, not replay the same input.
        native = product.codex
        text = native.read_text()
        old = "'input': 'installed headless Agent request'"
        assert text.count(old) == 1
        native.write_text(text.replace(old, "'input': sys.argv[-1]"))
        observed = {}
        for complex in [False, True]:
            Decisions.complex = complex
            launched = subprocess.run([str(product.codex), 'exec', 'controlled primary task' if complex else 'controlled economy task'],
                cwd=product.project, env=product.env, capture_output=True, timeout=60)
            product.outputs.extend((launched.stdout, launched.stderr))
            assert launched.returncode == 0 and b'native product answer' in launched.stdout, {
                'exit': launched.returncode, 'stdout': launched.stdout.decode(errors='replace'),
                'stderr': launched.stderr.decode(errors='replace')}
            group = 'primary' if complex else 'regular'
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                sessions = product.cli('sessions', 'list', '--include-unlinked', '--limit', '50')['data']['sessions']
                for session in sessions:
                    detail = product.cli('sessions', 'show', session['session_id'])['data']
                    for turn in detail['turns']:
                        for rid in turn['receipt_ids']:
                            receipt = product.cli('sessions', 'receipt', rid)['data']
                            facts = [event['fact'] for event in receipt['ordered_facts']]
                            decisions = [f for f in facts if f['kind'] == 'route_decision'
                                and f.get('plan_id') == plan and f.get('complexity', {}).get('execution_group') == group
                                and f['complexity']['decision_source'] == 'external_classifier']
                            completed = [f for f in facts if f['kind'] == 'request_finished' and f['outcome'] == 'accepted']
                            executions = [f for f in facts if f['kind'] == 'agent_turn_finished'
                                and f.get('branch_execution', {}).get('group') == group]
                            if decisions and completed and executions:
                                observed[group] = rid
                if group in observed:
                    break
                time.sleep(.1)
            assert group in observed, ('missing routed request evidence', group)
        assert len(set(observed.values())) == 2
        Decisions.credential = raw_value
        raw_path = product.project / 'custom-decision.json'
        custom = product.cli('decision', 'services', 'apply', '--id', 'raw-custom', '--name', 'Raw custom',
            '--provider', 'custom', '--endpoint', f'http://127.0.0.1:{endpoint.server_port}/custom',
            '--candidate', 'candidate/decision-raw', '--apply-file', str(raw_path))['data']
        submit(product, custom)
        tested = product.cli('decision', 'services', 'test', '--service', 'raw-custom@1')['data']
        assert tested['outcome'] == 'passed', tested
        product.cli('protected-input', 'release', '--candidate', 'candidate/decision-raw')
        print(json.dumps({'scenario':'installed-decision-onboarding', 'state':'green',
            'protected_prompt':'PTY hidden/cancel/error/nonTTY', 'execution_groups':list(observed),
            'provider_quality':'controlled protocol only'}), flush=True)
    finally:
        endpoint.shutdown()
        endpoint.server_close()
        worker.join(timeout=5)
