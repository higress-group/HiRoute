"""Drive the real daemon with client-bundled release facts and Local Control.

Fixtures are user-owned Agent files only; no test writes business database rows.
"""
import hashlib
import http.client
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import tempfile
import time

V1 = {'major': 1, 'minor': 0}
V2 = {'major': 2, 'minor': 0}
SENTINEL = 'mvp01-process-upstream-secret-sentinel'
CONNECTION = 'agent-connection/agent_claude_default/claude-messages-v1'


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


class Product:
    def __init__(self, repository, project_source=False, root=None):
        self.repo = Path(repository)
        self.project_source = project_source
        self.bin = Path(os.environ.get('HIROUTE_VALIDATION_PRODUCT_BIN_DIR',
                                       str(self.repo / 'target/debug')))
        self.command_descriptors = json.loads(
            (self.repo / 'contracts/cli/planned-manifest.v1.json').read_text()
        )['commands']
        # Leave room for the nested Local Control socket under macOS' longer per-user
        # TMPDIR while retaining the runner-provided private temporary-directory boundary.
        self.temporary = tempfile.TemporaryDirectory(prefix='h') if root is None else None
        self.root = (Path(self.temporary.name).resolve() if self.temporary is not None
                     else Path(root).resolve())
        if self.temporary is None:
            self.root.mkdir(mode=0o700)
        home = self.root / 'home'
        (home / '.claude').mkdir(parents=True)
        home.chmod(0o700)
        (home / '.claude').chmod(0o700)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        agent = bin_dir / 'claude'
        agent.write_text(r'''#!/usr/bin/python3
import http.client
import json
import subprocess
import sys
from urllib.parse import urlparse

if '--version' in sys.argv:
    print('Claude Code 2.1.231')
    raise SystemExit(0)

try:
    settings_path = sys.argv[sys.argv.index('--settings') + 1]
    settings = json.loads(open(settings_path, encoding='utf-8').read())
    endpoint = urlparse(settings['env']['ANTHROPIC_BASE_URL'])
    model = settings['env']['ANTHROPIC_MODEL']
    token = subprocess.check_output(
        settings['apiKeyHelper'], shell=True, text=True,
        env={'PATH': '/usr/bin:/bin'}).strip()
except (IndexError, KeyError, OSError, subprocess.SubprocessError, ValueError):
    raise SystemExit(2)

body = json.dumps({
    'model': model,
    'messages': [{'role': 'user', 'content': 'Reply with OK only.'}],
    'max_tokens': 8,
    'stream': True,
}).encode()
client = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=5)
try:
    client.request('POST', endpoint.path.rstrip('/') + '/v1/messages', body=body, headers={
        'Authorization': 'Bearer ' + token,
        'Content-Type': 'application/json',
    })
    response = client.getresponse()
    response.read()
    raise SystemExit(0 if response.status == 200 else 1)
finally:
    client.close()
''')
        agent.chmod(0o700)
        self.settings = home / '.claude/settings.json'
        self.settings.write_bytes(encoded({'env': {
            'ANTHROPIC_BASE_URL': 'https://open.bigmodel.cn/api/anthropic',
            'ANTHROPIC_MODEL': 'opus',
            'ANTHROPIC_DEFAULT_OPUS_MODEL': 'glm-5.3',
            'ANTHROPIC_AUTH_TOKEN': SENTINEL}}))
        self.settings.chmod(0o600)
        self.project = self.root / 'project'
        self.project.mkdir(mode=0o700)
        if project_source:
            # Managed launch isolates project auth from Agent writes while discovery can
            # independently import the original upstream configuration.
            self.project_settings = self.project / '.claude/settings.json'
            self.project_settings.parent.mkdir(mode=0o700)
            self.project_settings.write_bytes(self.settings.read_bytes())
            self.project_settings.chmod(0o600)
            self.settings.write_bytes(encoded({}))
        self.storage = self.root / 'storage'
        self.storage.mkdir(mode=0o700)
        self.diagnostics_root = self.storage / 'diagnostics'
        self.diagnostics_args = []
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(('ANTHROPIC_', 'HIROUTE_', 'OPENAI_'))}
        self.env.update(HOME=str(home), CODEX_HOME=str(home / '.codex'),
                        PATH=str(bin_dir) + ':/usr/bin:/bin',
                        HIROUTE_RUNTIME_DIR=str(self.root / 'runtime'),
                        HIROUTE_REPLAY_ROOT=str(self.root / 'replay'))
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.process = None
        self.outputs = []
        self.secrets = {SENTINEL, 'fixture.id.token', 'fixture-refresh-sentinel'}
        self.cpa_args = []

    def enable_debug_diagnostics(self):
        """Make the headless product run's diagnostic level and root explicit."""
        self.diagnostics_args = [
            '--diagnostics-root', str(self.diagnostics_root),
            '--diagnostic-level-override', 'debug',
        ]

    def install_codex_fixture(self, model=None, reasoning=None,
                              provider_base_url=None, provider_token=None):
        """Install a bounded native-check fixture; model traffic still uses the real Gateway."""
        codex = self.root / 'bin/codex'
        if codex.is_symlink():
            raise RuntimeError(f'refusing to replace a linked Codex binary: {codex}')
        codex.write_text(r'''#!/usr/bin/python3
import http.client
import json
import re
import sys
from urllib.parse import urlparse

if '--version' in sys.argv:
    print('codex-cli 0.116.0')
    raise SystemExit(0)

provider = next((arg for arg in sys.argv[1:]
                 if arg.startswith('model_providers.hiroute_native_probe=')), None)
model_arg = next((arg for arg in sys.argv[1:] if arg.startswith('model=')), None)
if provider is None or model_arg is None:
    raise SystemExit(2)
match = re.search(r'base_url=("(?:[^"\\]|\\.)*")', provider)
if match is None:
    raise SystemExit(2)
endpoint = urlparse(json.loads(match.group(1)))
model = json.loads(model_arg.split('=', 1)[1])
body = json.dumps({'model': model, 'input': 'native fixture check', 'stream': True}).encode()
client = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=5)
try:
    client.request('POST', endpoint.path.rstrip('/') + '/responses', body=body, headers={
        'Authorization': 'Bearer hiroute-native-probe-no-authority',
        'Content-Type': 'application/json',
    })
    response = client.getresponse()
    response.read()
    raise SystemExit(0 if response.status == 200 else 1)
finally:
    client.close()
''')
        codex.chmod(0o700)
        codex_home = Path(self.env.get(
            'CODEX_HOME', str(self.root / 'home/.codex')))
        codex_home.mkdir(mode=0o700, exist_ok=True)
        self.codex_settings = codex_home / 'config.toml'
        fixture_model = model or 'gpt-5.4'
        if (not self.codex_settings.exists() or model is not None
                or reasoning is not None or provider_base_url is not None):
            fixture_reasoning = reasoning or 'medium'
            settings = [
                'model = ' + json.dumps(fixture_model),
                'model_reasoning_effort = ' + json.dumps(fixture_reasoning),
            ]
            if provider_base_url is not None:
                assert provider_token, 'custom Codex fixture providers require an exact protected token'
                settings.extend([
                    'model_provider = "fixture_native"',
                    '[model_providers.fixture_native]',
                    'name = "Fixture native source"',
                    'base_url = ' + json.dumps(provider_base_url),
                    'wire_api = "responses"',
                    'experimental_bearer_token = ' + json.dumps(provider_token),
                    'requires_openai_auth = false',
                ])
                self.secrets.add(provider_token)
            self.codex_settings.write_text('\n'.join(settings) + '\n')
            self.codex_settings.chmod(0o600)
        # Keep the full client metadata catalog: names in this file are not account rights.
        # Production resolution never falls back to the repository copy used here.
        model_cache = codex_home / 'models_cache.json'
        if not model_cache.exists():
            catalog = json.loads((
                self.repo / 'crates/integrations/src/agents/codex_bundled_catalog.json'
            ).read_text())
            assert any(entry['slug'] == fixture_model for entry in catalog['models'])
            model_cache.write_bytes(encoded(catalog))
            model_cache.chmod(0o644)
        return codex

    def enable_cpa(self, model=None):
        fixture = self.root / 'cpa-fixture'
        fixture.mkdir(mode=0o700)
        binary = fixture / 'cpa_upstream.py'
        shutil.copy2(self.repo / 'crates/daemon/tests/support/cpa_upstream.py', binary)
        binary.chmod(0o700)
        auth = fixture / 'auth.json'
        auth.write_bytes(encoded({'OPENAI_API_KEY': None, 'auth_mode': 'chatgpt',
                         'last_refresh': 'fixture', 'tokens': {'access_token': SENTINEL,
                         'id_token': 'fixture.id.token', 'refresh_token': 'fixture-refresh-sentinel',
                         'account_id': 'mvp01-cpa-account'}}))
        auth.chmod(0o600)
        native_model = model or 'gpt-5.5'
        model_path = fixture / 'model-id'
        model_path.write_text(native_model)
        model_path.chmod(0o600)
        self.cpa_args = ['--cpa-binary', str(binary), '--cpa-sha256',
                         hashlib.sha256(binary.read_bytes()).hexdigest()]
        self.env['CODEX_HOME'] = str(fixture)
        self.cpa_fixture = fixture
        # The selected CPA target is now this exact CODEX_HOME. Install the bounded native
        # engine/config/cache fixture there as well; a cache under the previous target must not
        # satisfy strict target-bound catalog resolution.
        self.install_codex_fixture(model=native_model)

    def start(self, fault=None, expected_failure=False):
        # Native Agent capability evidence is deliberately process-local and must be
        # re-established after every successful daemon restart.
        self.native_model_checked_agents = set()
        self.shutdown_r, self.shutdown_w = os.pipe()
        self.cap_r, self.cap_w = os.pipe()
        self.cap_ack_r, self.cap_ack_w = os.pipe()
        env = dict(self.env)
        if fault:
            env['HIROUTE_TEST_PUBLICATION_CRASH_AT'] = fault
        self.process = subprocess.Popen([
            str(self.bin / 'hirouted'), '--role', 'all', '--storage-root', str(self.storage),
            '--runtime-root', str(self.root / 'runtime'), '--listen', f'127.0.0.1:{self.port}',
            '--lkg', str(self.root / 'gateway-lkg'), '--shutdown-fd', str(self.shutdown_r),
            '--capability-fd', str(self.cap_r), '--capability-ack-fd', str(self.cap_ack_w),
            *self.cpa_args, *self.diagnostics_args, *getattr(self, 'worker_args', [])],
            env=env, cwd=self.project,
            pass_fds=(self.shutdown_r, self.cap_r, self.cap_ack_w), stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        assert select.select([self.process.stdout], [], [], getattr(self, 'startup_timeout', 45))[0], 'daemon readiness timeout'
        ready = self.process.stdout.readline()
        if expected_failure:
            assert not ready, 'unsafe recovery opened product service'
            status = self.process.wait(timeout=40)
            self.outputs.append(self.process.stderr.read())
            for fd in (self.shutdown_r, self.shutdown_w, self.cap_r, self.cap_w,
                       self.cap_ack_r, self.cap_ack_w):
                os.close(fd)
            self.process = None
            assert status != 0, 'failed recovery must not report successful startup'
            return status
        assert ready, 'daemon did not become ready: ' + self.process.stderr.read().decode()
        assert json.loads(ready)['role'] == 'all'
        self.outputs.append(ready)

    def stop(self, crash=False):
        if self.process is None:
            return
        os.close(self.shutdown_w)
        try:
            status = self.process.wait(timeout=40)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise
        self.outputs.append(self.process.stderr.read())
        for fd in (self.shutdown_r, self.cap_r, self.cap_w,
                   self.cap_ack_r, self.cap_ack_w):
            os.close(fd)
        self.process = None
        assert status == (86 if crash else 0), ('daemon exit', status)

    def cli(self, command, payload=None, capability=None, success=True):
        """Use command notation to exercise Planned operations over internal Local Control.

        Public CLI rejection of Planned descriptors is covered separately. These publication
        scenarios need the non-standalone daemon's internal surface, not a test-only CLI bypass.
        """
        operation, body = self._control_request(command, payload)
        grant = None if capability is None else {
            'principal_kind': 'interactive_user',
            'capability': capability,
        }
        try:
            envelope = self.control(operation, body, grant, success=False)
        except (OSError, json.JSONDecodeError):
            if success or self.process is None:
                raise
            status = self.process.wait(timeout=5)
            assert status == 86, ('unexpected Local Control transport failure', status)
            envelope = {
                'schema_version': V2,
                'status': 'unavailable',
                'warnings': [],
                'next_actions': [],
                'error': {'code': 'DAEMON_UNAVAILABLE'},
            }
        exit_code = {
            'succeeded': 0,
            'accepted': 0,
            'internal_error': 1,
            'usage_error': 2,
            'conflict': 3,
            'denied': 4,
            'not_found': 5,
            'unavailable': 6,
            'action_required': 7,
            'needs_attention': 8,
        }[envelope['status']]
        if success:
            assert exit_code == 0, (command, envelope)
        return exit_code, envelope

    def _control_request(self, command, payload):
        tokens = command.split()
        matches = [descriptor for descriptor in self.command_descriptors
                   if tokens[:len(descriptor['path'])] == descriptor['path']]
        assert matches, ('unregistered test command', command)
        descriptor = max(matches, key=lambda item: len(item['path']))
        options = tokens[len(descriptor['path']):]
        if payload is not None:
            assert not options, ('payload command also supplied positional options', command)
            return descriptor['operation_id'], payload
        if not options:
            return descriptor['operation_id'], {}
        if descriptor['command_id'] == 'routing.show' and len(options) == 1:
            return descriptor['operation_id'], {'agent_plan_id': options[0]}
        if descriptor['command_id'] == 'operations.get' and len(options) == 1:
            return descriptor['operation_id'], {
                'operation_id': options[0],
                'after_sequence': 0,
            }
        if descriptor['command_id'] == 'agents.connect.status' and len(options) == 1:
            if options[0].startswith('agent-context/'):
                body = {'schema_version': V2, 'context_id': options[0]}
            else:
                body = {'connection_id': options[0]}
            return descriptor['operation_id'], body
        if descriptor['command_id'] == 'agents.check':
            body = {
                'agent_id': options[0],
                'scope': 'configuration',
                'suite': 'quick',
                'allow_model_call': False,
            }
            index = 1
            while index < len(options):
                option = options[index]
                if option == '--allow-model-call':
                    body['allow_model_call'] = True
                    index += 1
                    continue
                assert option in ('--scope', '--suite') and index + 1 < len(options), command
                body[option[2:]] = options[index + 1].replace('-', '_')
                index += 2
            return descriptor['operation_id'], body
        raise AssertionError(('test command needs an explicit typed payload mapping', command))

    def preview(self, command, payload=None):
        return self.cli(command, payload)[1]['data']

    def control(self, operation, payload, protected_grant=None, success=True):
        """Call the current typed Local Control surface over the real daemon socket."""
        request_id = 'publication-product-' + hashlib.sha256(
            encoded((operation, time.monotonic_ns()))).hexdigest()[:24]
        request = {
            'schema_version': V2,
            'request_id': request_id,
            'operation_id': operation,
            'payload': payload,
        }
        if protected_grant is not None:
            request['protected_grant'] = protected_grant
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(90)
            sock.connect(str(self.root / 'runtime/hiroute/control.sock'))
            stream = sock.makefile('rwb')
            stream.write(encoded({
                'api_version': V2,
                'machine_schema_version': V2,
                'client_name': 'publication-product-test',
                'client_version': '0.1.0',
            }) + b'\n')
            stream.flush()
            hello = json.loads(stream.readline())
            assert 'local-control-v2' in hello['capabilities'], hello
            stream.write(encoded(request) + b'\n')
            stream.flush()
            envelope_bytes = stream.readline()
        self.outputs.append(envelope_bytes)
        envelope = json.loads(envelope_bytes)
        assert envelope.get('request_id') == request_id, envelope
        if success:
            assert envelope.get('error') is None, envelope
        return envelope

    def desktop_grant(self, operation, accepted_digest, expected_revisions, key):
        capability = ('mvp01-desktop-capability-' + key
                      + '-01234567890123456789')
        self.secrets.add(capability)
        registration = hashlib.sha256(
            encoded((operation, key, time.monotonic_ns()))).hexdigest()
        frame = {
            'schema': 'hiroute.protected-apply-grant/v2',
            'registration_id': registration,
            'capability': capability,
            'principal_kind': 'desktop',
            'workspace_id': 'personal/default',
            'operation_kind': operation,
            'accepted_digest': accepted_digest,
            'expected_revisions': expected_revisions,
            'expires_at_unix': int(time.time()) + 120,
        }
        self.register_protected_frame(frame)
        return {'principal_kind': 'desktop', 'capability': capability}

    def grant(self, operation, preview, key):
        capability = 'mvp01-protected-capability-' + key + '-01234567890123456789'
        self.secrets.add(capability)
        registration = hashlib.sha256(encoded((operation, key, time.monotonic_ns()))).hexdigest()
        frame = {'schema': 'hiroute.protected-apply-grant/v2',
                 'registration_id': registration,
                 'capability': capability, 'principal_kind': 'interactive_user',
                 'workspace_id': 'personal/default', 'operation_kind': operation,
                 'accepted_digest': preview['change_digest'],
                 'expected_revisions': preview['expected_revisions'],
                 'expires_at_unix': int(time.time()) + 120}
        self.register_protected_frame(frame)
        return capability

    def register_protected_frame(self, frame):
        os.write(self.cap_w, encoded(frame) + b'\n')
        assert select.select([self.cap_ack_r], [], [], 2)[0], 'protected grant acknowledgement timeout'
        response = b''
        while not response.endswith(b'\n'):
            byte = os.read(self.cap_ack_r, 1)
            assert byte, 'protected grant acknowledgement channel closed'
            response += byte
        assert json.loads(response) == {
            'schema': 'hiroute.protected-apply-ack/v2',
            'registration_id': frame['registration_id'],
            'registered': True,
        }

    def apply(self, command, operation, preview, body, key, crash=False):
        body = dict(body, accept_digest=preview['change_digest'],
                    expected_revisions=preview['expected_revisions'], idempotency_key=key)
        status, result = self.cli(command, body, success=not crash)
        if crash:
            assert status != 0, 'crash must not be reported as success'
        else:
            assert result['data']['state'] == 'succeeded', result
        return result, body, None

    def bearer(self, connection_id=CONNECTION):
        connection = connection_id.encode()
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(10)
            sock.connect(str(self.root / 'runtime/hiroute/agent-grant-v1.sock'))
            sock.sendall(b'HIRAGENT' + (1).to_bytes(2, 'big') + len(connection).to_bytes(2, 'big') + connection)
            sock.shutdown(socket.SHUT_WR)
            value = sock.makefile('rb').read().strip().decode()
        assert value, 'protected Agent grant unavailable'
        self.secrets.add(value)
        return value

    def catalog(self):
        client = http.client.HTTPConnection('127.0.0.1', self.port, timeout=10)
        try:
            connection = getattr(self, 'agent_connection', CONNECTION)
            client.request('GET', '/v1/models', headers={
                'Authorization': 'Bearer ' + self.bearer(connection)})
            response = client.getresponse()
            body = response.read()
            self.outputs.append(body)
            assert response.status == 200, (response.status, body)
            catalog = json.loads(body)
            assert catalog['data'], catalog
            return catalog, response.getheader('ETag')
        finally:
            client.close()

    def diagnostics_snapshot(self, root=None):
        """Summarize safe product JSONL without exposing event payloads."""
        root = Path(root or self.diagnostics_root)
        files = sorted(root.rglob('*.jsonl')) if root.is_dir() else []
        records = []
        invalid_records = 0
        for path in files:
            if path.is_symlink() or not path.is_file():
                invalid_records += 1
                continue
            for line in path.read_text().splitlines():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    invalid_records += 1
                    continue
                if isinstance(record, dict):
                    records.append(record)
                else:
                    invalid_records += 1
        current = root / 'daemon/current.jsonl'
        current_records = []
        if current.is_file() and not current.is_symlink():
            for line in current.read_text().splitlines():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(record, dict):
                    current_records.append(record)
        current_boot = next(
            (record.get('boot_id') for record in reversed(current_records)
             if isinstance(record.get('boot_id'), str)), None)
        level_applied = next(
            (record['event']['level_applied'] for record in reversed(current_records)
             if record.get('boot_id') == current_boot
             and isinstance(record.get('event'), dict)
             and isinstance(record['event'].get('level_applied'), dict)), None)
        complete = bool(current_boot and level_applied and not invalid_records)
        state = 'complete' if complete else ('partial' if files else 'unavailable')
        return {
            'state': state,
            'path': str(root),
            'current_boot': current_boot,
            'level_applied': level_applied,
            'files': [str(path.relative_to(root)) for path in files],
            'record_count': len(records),
            'invalid_record_count': invalid_records,
        }

    def preserve_diagnostics(self, destination):
        """Copy only safe diagnostic JSONL into a retained product-test artifact."""
        destination = Path(destination)
        for source in sorted(self.diagnostics_root.rglob('*.jsonl')):
            if source.is_symlink() or not source.is_file():
                continue
            relative = source.relative_to(self.diagnostics_root)
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        return self.diagnostics_snapshot(destination)

    def close(self):
        try:
            self.stop()
            assert all(secret.encode() not in output for secret in self.secrets
                       for output in self.outputs), 'public secret leak'
        finally:
            if self.temporary is not None:
                self.temporary.cleanup()
