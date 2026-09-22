#!/usr/bin/python3
"""Installed Linux standalone CLI -> daemon -> Gateway -> observation/Worker product loop.

Only external processes are deterministic fixtures. Every HiRoute query and mutation crosses the
installed public CLI; this file never opens Local Control directly and never writes product data.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from model_connections_product import MODEL, NATIVE_TOKEN, NativeUpstream, declared


def wire(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def terminal(value):
    return value in ('succeeded', 'failed', 'cancelled', 'unknown')


class InstalledProduct:
    def __init__(self, repository):
        self.repo = Path(repository).resolve()
        self.sha = subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=self.repo, text=True).strip()
        self.temporary = tempfile.TemporaryDirectory(prefix='hiroute-headless-')
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir(mode=0o700)
        self.state = self.root / 'state'
        self.runtime = self.root / 'runtime'
        self.codex_home = self.home / '.codex'
        self.codex_home.mkdir(mode=0o700)
        self.fixture_bin = self.root / 'fixture-bin'
        self.fixture_bin.mkdir(mode=0o700)
        self.project = self.root / 'project'
        self.project.mkdir(mode=0o700)
        self.package = self.root / 'package'
        self.package.mkdir(mode=0o700)
        self.outputs = []
        self.secrets = {
            NATIVE_TOKEN,
            'headless-user-native-token',
            'headless-access-token',
            'headless-id-token',
            'headless-refresh-token',
        }
        self.env = {
            key: value for key, value in os.environ.items()
            if not key.startswith(('HIROUTE_', 'OPENAI_', 'ANTHROPIC_'))
        }
        self.env.update({
            'HOME': str(self.home),
            'XDG_STATE_HOME': str(self.state),
            'XDG_RUNTIME_DIR': str(self.runtime),
            'CODEX_HOME': str(self.codex_home),
            'PATH': str(self.fixture_bin) + ':/usr/bin:/bin',
        })
        self.cli_path = self.home / '.local/bin/hiroute'
        self.daemon = None
        self.upstream = NativeUpstream()

    def close(self):
        if self.daemon is not None:
            self.daemon.terminate()
            try:
                self.daemon.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.daemon.kill()
                self.daemon.wait(timeout=10)
            self.outputs.extend((
                self.daemon.stdout.read() if self.daemon.stdout else b'',
                self.daemon.stderr.read() if self.daemon.stderr else b'',
            ))
            self.daemon = None
        self.upstream.close()
        leaked = []
        for output in self.outputs:
            text = output.decode(errors='replace') if isinstance(output, bytes) else output
            leaked.extend(secret for secret in self.secrets if secret and secret in text)
        assert not leaked, 'protected input or fixture credential leaked into process output'
        self.temporary.cleanup()

    def make_external_fixtures(self):
        claude = self.fixture_bin / 'claude'
        claude.write_text(r'''#!/usr/bin/python3
import http.client, json, subprocess, sys
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
body = json.dumps({'model': model, 'messages': [
    {'role': 'user', 'content': 'installed headless Agent request'}
], 'max_tokens': 16, 'stream': False}).encode()
client = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=20)
try:
    client.request('POST', endpoint.path.rstrip('/') + '/v1/messages', body=body,
                   headers={'Authorization': 'Bearer ' + token,
                            'Content-Type': 'application/json'})
    response = client.getresponse()
    data = response.read()
    raise SystemExit(0 if response.status == 200 and data else 1)
finally:
    client.close()
''')
        claude.chmod(0o700)
        claude_settings = self.home / '.claude/settings.json'
        claude_settings.parent.mkdir(mode=0o700)
        claude_settings.write_text(json.dumps({
            'theme': 'dark',
            'env': {
                'ANTHROPIC_BASE_URL': 'https://open.bigmodel.cn/api/anthropic',
                'ANTHROPIC_MODEL': 'glm-5.3',
                'ANTHROPIC_AUTH_TOKEN': 'headless-user-native-token',
                'UNRELATED': 'preserve-me',
            },
        }))
        claude_settings.chmod(0o600)

        codex = self.fixture_bin / 'codex'
        codex.write_text(r'''#!/usr/bin/python3
import ast, configparser, http.client, json, os, re, sys
from urllib.parse import urlparse

if '--version' in sys.argv:
    print('codex-cli 0.116.0')
    raise SystemExit(0)

provider = None
model = None
probe = False
for argument in sys.argv:
    if argument.startswith('model_providers.hiroute_native_probe='):
        match = re.search(r'base_url=("(?:[^"\\]|\\.)*")', argument)
        if match:
            provider = {
                'base_url': json.loads(match.group(1)),
                'http_headers': {
                    'Authorization': 'Bearer hiroute-native-probe-no-authority',
                },
            }
            probe = True
    elif argument.startswith('model='):
        model = json.loads(argument[len('model='):])

if provider is None:
    try:
        with open(os.path.join(os.environ['CODEX_HOME'], 'config.toml'), encoding='utf-8') as stream:
            configuration = configparser.ConfigParser(interpolation=None)
            configuration.optionxform = str
            configuration.read_string('[root]\n' + stream.read())
        provider = {
            'base_url': ast.literal_eval(configuration['model_providers.hiroute']['base_url']),
            'wire_api': ast.literal_eval(configuration['model_providers.hiroute']['wire_api']),
            'http_headers': {
                'X-HiRoute-Token': ast.literal_eval(
                    configuration['model_providers.hiroute.http_headers']['X-HiRoute-Token']),
            },
        }
        model = ast.literal_eval(configuration['root']['model'])
    except (KeyError, OSError, ValueError, SyntaxError, configparser.Error):
        raise SystemExit(2)

endpoint = urlparse(provider['base_url'])
body = json.dumps({'model': model, 'input': 'installed headless Agent request',
                   'stream': provider.get('wire_api') is None}).encode()
headers = {'Content-Type': 'application/json'}
headers.update(provider.get('http_headers', {}))
client = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=30)
try:
    client.request('POST', endpoint.path.rstrip('/') + '/responses', body=body,
                   headers=headers)
    response = client.getresponse()
    payload = response.read()
    if response.status != 200:
        raise SystemExit(1)
    if probe:
        raise SystemExit(0)
    document = json.loads(payload)
    answer = ''.join(part.get('text', '') for item in document.get('output', [])
                     for part in item.get('content', []))
    print(answer)
finally:
    client.close()
''')
        codex.chmod(0o700)
        adapter = self.fixture_bin / 'codex-acp'
        adapter.write_text(r'''#!/usr/bin/python3
import http.client, json, os, sys
from urllib.parse import urlparse
config = json.loads(os.environ['CODEX_CONFIG'])
base = urlparse(config['model_providers']['hiroute']['base_url'])
token = os.environ['HIROUTE_RUN_TOKEN']
session = 'headless-acp-session'
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion': 1, 'agentCapabilities': {'loadSession': True}}
    elif method == 'session/new':
        result = {'sessionId': session, '_meta': {'agentSessionId': session}, 'modes': {
            'currentModeId': 'agent-full-access',
            'availableModes': [{'id': 'agent-full-access', 'name': 'Autonomous'}]}}
    elif method == 'session/set_mode':
        result = {}
    elif method == 'session/prompt':
        prompt = request['params']['prompt'][0]['text']
        body = json.dumps({'model': config['model'], 'input': prompt, 'stream': False}).encode()
        client = http.client.HTTPConnection(base.hostname, base.port, timeout=30)
        try:
            client.request('POST', base.path.rstrip('/') + '/responses', body=body, headers={
                'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
            response = client.getresponse()
            payload = json.loads(response.read())
            if response.status != 200:
                raise RuntimeError(payload)
            answer = ''.join(part.get('text', '') for item in payload.get('output', [])
                             for part in item.get('content', []))
        finally:
            client.close()
        print(json.dumps({'jsonrpc': '2.0', 'method': 'session/update', 'params': {
            'sessionId': session, 'update': {'sessionUpdate': 'agent_message_chunk',
                                             'content': {'type': 'text', 'text': answer}}}}),
              flush=True)
        result = {'stopReason': 'end_turn'}
    else:
        raise RuntimeError(method)
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
''')
        adapter.chmod(0o700)
        self.adapter = adapter.resolve()
        self.codex = codex.resolve()

    def install(self):
        self.make_external_fixtures()
        auth = {
            'OPENAI_API_KEY': None,
            'auth_mode': 'chatgpt',
            'last_refresh': 'fixture',
            'tokens': {
                'access_token': 'headless-access-token',
                'id_token': 'headless-id-token',
                'refresh_token': 'headless-refresh-token',
                'account_id': 'headless-cpa-account',
            },
        }
        auth_path = self.codex_home / 'auth.json'
        auth_path.write_bytes(wire(auth))
        auth_path.chmod(0o600)
        codex_config = self.codex_home / 'config.toml'
        codex_config.write_text(
            'model = ' + json.dumps(MODEL) + '\n'
            'model_reasoning_effort = "low"\n'
            'model_provider = "fixture_native"\n'
            '[model_providers.fixture_native]\n'
            'name = "Fixture native source"\n'
            'base_url = ' + json.dumps(self.upstream.base_url) + '\n'
            'wire_api = "responses"\n'
            'experimental_bearer_token = ' + json.dumps(NATIVE_TOKEN) + '\n'
            'requires_openai_auth = false\n')
        codex_config.chmod(0o600)
        # The complete cache is metadata; this account serves only the saved model.
        catalog = json.loads((
            self.repo / 'crates/integrations/src/agents/codex_bundled_catalog.json'
        ).read_text())
        assert any(entry['slug'] == MODEL for entry in catalog['models'])
        model_cache = self.codex_home / 'models_cache.json'
        model_cache.write_bytes(wire(catalog))
        model_cache.chmod(0o600)

        cpa = self.root / 'cpa_upstream.py'
        shutil.copy2(self.repo / 'crates/daemon/tests/support/cpa_upstream.py', cpa)
        cpa.chmod(0o700)
        license_path = self.root / 'CPA-LICENSE'
        license_path.write_text('isolated fixture license\n')
        license_path.chmod(0o600)
        source_bin = Path(os.environ['HIROUTE_HEADLESS_PRODUCT_BIN_DIR']).resolve()
        # The packager deliberately refuses group-writable inputs. CI/worktree umasks can leave
        # tracked read-only payloads at 0664, so stage byte-identical, private package inputs just
        # as a release job does instead of weakening the production check.
        package_source = self.root / 'package-source'
        (package_source / 'scripts').mkdir(parents=True)
        (package_source / 'assets/skills/hiroute-management').mkdir(parents=True)
        (package_source / 'docs').mkdir(parents=True)
        (package_source / 'notices').mkdir(parents=True)
        shutil.copy2(self.repo / 'scripts/package-standalone.py',
                     package_source / 'scripts/package-standalone.py')
        shutil.copy2(self.repo / 'LICENSE', package_source / 'LICENSE')
        shutil.copy2(self.repo / 'assets/skills/hiroute-management/SKILL.md',
                     package_source / 'assets/skills/hiroute-management/SKILL.md')
        shutil.copy2(self.repo / 'docs/standalone-cli.md',
                     package_source / 'docs/standalone-cli.md')
        (package_source / 'notices/THIRD-PARTY-LICENSES.txt').write_text(
            'Isolated product-test dependency notice\n')
        (package_source / 'notices/third-party-licenses.json').write_text(json.dumps({
            'schema': 'hiroute.third-party-licenses/v1',
            'inputs': {'fixture': 'locked'},
            'packages': [{'id': 'fixture:cpa@1'}],
            'documents': [{'sha256': '0' * 64}],
        }, sort_keys=True) + '\n')
        candidate_hiroute = self.root / 'candidate-hiroute'
        candidate_hirouted = self.root / 'candidate-hirouted'
        shutil.copy2(source_bin / 'hiroute', candidate_hiroute)
        shutil.copy2(source_bin / 'hirouted', candidate_hirouted)
        for path in (
            package_source / 'scripts/package-standalone.py',
            package_source / 'LICENSE',
            package_source / 'assets/skills/hiroute-management/SKILL.md',
            package_source / 'docs/standalone-cli.md',
            package_source / 'notices/THIRD-PARTY-LICENSES.txt',
            package_source / 'notices/third-party-licenses.json',
        ):
            path.chmod(0o600)
        candidate_hiroute.chmod(0o700)
        candidate_hirouted.chmod(0o700)
        # Debug candidates contain almost a gigabyte of symbols. Release packages are stripped;
        # doing the same to these private copies keeps this production-path regression bounded.
        stripped = subprocess.run(
            ['strip', '--strip-debug', str(candidate_hiroute), str(candidate_hirouted)],
            capture_output=True, timeout=60)
        self.outputs.extend((stripped.stdout, stripped.stderr))
        assert stripped.returncode == 0, stripped.stderr.decode(errors='replace')
        package = subprocess.run([
            'python3', '-B', str(package_source / 'scripts/package-standalone.py'), 'build',
            '--version', '0.1.0-headless-product', '--revision', self.sha,
            '--hiroute', str(candidate_hiroute),
            '--hirouted', str(candidate_hirouted),
            '--cpa-binary', str(cpa), '--cpa-version', '7.2.140-hiroute.2',
            '--cpa-license', str(license_path),
            '--notices', str(package_source / 'notices'), '--output', str(self.package),
        ], cwd=self.repo, env=self.env, capture_output=True, timeout=90)
        self.outputs.extend((package.stdout, package.stderr))
        assert package.returncode == 0, package.stderr.decode(errors='replace')
        manifest = next(self.package.glob('*.tar.gz.json'))
        archive = manifest.with_suffix('')
        installed = subprocess.run([
            'python3', '-B', str(self.repo / 'scripts/install-standalone.py'), 'install',
            '--manifest', str(manifest), '--archive', str(archive),
        ], cwd=self.repo, env=self.env, capture_output=True, timeout=90)
        self.outputs.extend((installed.stdout, installed.stderr))
        assert installed.returncode == 0, installed.stderr.decode(errors='replace')
        assert self.cli_path.is_symlink()
        assert (self.home / '.agents/skills/hiroute-management/SKILL.md').is_file()
        assert (self.home / '.local/share/hiroute/0.1.0-headless-product/docs/standalone-cli.md').is_file()

    def start(self):
        self.daemon = subprocess.Popen(
            [str(self.cli_path), 'service', 'run'], cwd=self.project, env=self.env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 90
        last = None
        while time.monotonic() < deadline:
            result = subprocess.run(
                [str(self.cli_path), 'service', 'status', '--output', 'json'],
                cwd=self.project, env=self.env, capture_output=True, timeout=5)
            self.outputs.extend((result.stdout, result.stderr))
            if result.returncode == 0:
                last = json.loads(result.stdout)
                if last['data']['local_control_ready']:
                    assert last['data']['runtime_root'] == str(self.runtime)
                    break
            if self.daemon.poll() is not None:
                modes = []
                for path in sorted(self.state.rglob('*')):
                    modes.append((str(path.relative_to(self.root)), oct(path.lstat().st_mode & 0o777)))
                raise AssertionError({
                    'stderr': self.daemon.stderr.read().decode(errors='replace'),
                    'state_modes': modes,
                })
            time.sleep(.1)
        else:
            raise AssertionError(('standalone readiness timeout', last))
        system = self.cli('system', 'status')
        assert system['data']['daemon'] == 'role_all', system
        assert system['data']['gateway'] == 'ready', system
        gateway = self.cli('gateway', 'show')
        assert gateway['data']['ready'] and gateway['data']['connect_address'], gateway

    def cli(self, *arguments, payload=None, expected=0, pass_fds=()):
        command = [str(self.cli_path), *arguments]
        if '--output' not in command:
            command.extend(['--output', 'json'])
        result = subprocess.run(
            command, cwd=self.project, env=self.env,
            input=(None if payload is None else
                   payload.encode() if isinstance(payload, str) else wire(payload)),
            pass_fds=pass_fds, capture_output=True, timeout=90)
        self.outputs.extend((result.stdout, result.stderr))
        try:
            envelope = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError((command, result.returncode, result.stdout, result.stderr)) from error
        daemon_failure = None
        if (result.returncode != expected and self.daemon is not None
                and self.daemon.poll() is not None):
            daemon_stdout = self.daemon.stdout.read() if self.daemon.stdout else b''
            daemon_stderr = self.daemon.stderr.read() if self.daemon.stderr else b''
            self.outputs.extend((daemon_stdout, daemon_stderr))
            daemon_failure = {
                'returncode': self.daemon.returncode,
                'stdout': daemon_stdout.decode(errors='replace'),
                'stderr': daemon_stderr.decode(errors='replace'),
            }
        assert result.returncode == expected, (
            command, expected, result.returncode, envelope, daemon_failure)
        if expected == 0:
            assert envelope['status'] in ('succeeded', 'accepted'), (command, envelope)
        return envelope

    def register_secret(self, candidate, secret):
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, secret.encode())
            os.close(write_fd)
            write_fd = None
            return self.cli('protected-input', 'register', '--candidate', candidate,
                            '--secret-fd', str(read_fd), pass_fds=(read_fd,))
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)


def native_draft(upstream):
    return {
        'inference_model_id': None,
        'candidate_ref': 'candidate/native/headless-product',
        'lineage_ref': 'lineage/native/headless-product',
        'display_name': 'Headless Native API',
        'existing_source_id': None,
        'edit_revision': 1,
        'check_id': 'check/native/headless-product-1',
        'base_url': upstream.base_url,
        'base_kind': 'api_root',
        'request_path_override': None,
        'inventory_path_override': '/v1/models',
        'protocol': 'responses',
        'protocol_profile_id': 'profile/custom/responses',
        'protocol_profile_revision': 1,
        'authentication': {'kind': 'bearer'},
        'configuration_revision': 1,
        'models': [{
            'upstream_model_id': MODEL,
            'display_name': 'Headless Native Model',
            'catalog_configuration_id': None,
            'membership': 'user_declared',
            'capabilities': {
                'tool': declared(True),
                'vision': declared(False),
                'streaming': declared(True),
                'context_tokens': declared(32768),
                'max_output_tokens': declared(4096),
                'native_reasoning': declared({'kind': 'fixed', 'profile': 'provider-default'}),
            },
        }],
    }


def apply_from_preview(product, command, preview, key, extra=None):
    body = {
        'spec': preview['spec'],
        'accept_digest': preview['accept_digest'],
        'expected_revisions': preview['expected_revisions'],
        'idempotency_key': key,
    }
    if extra:
        body.update(extra)
    return product.cli(*command, '--request-stdin', payload=body)


def save_candidate(product, candidate, validation, key):
    snapshot = product.cli('compute', 'list')['data']
    selectable = [model['model_ref'] for model in candidate['models'] if model['selectable']]
    assert selectable, candidate
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': selectable,
        'intent': 'save_ready',
        'key_edits': [],
    }
    if validation is not None:
        change['validation'] = validation
    preview = product.cli(
        'compute', 'connection', 'preview', '--request-stdin', payload={'change': change})['data']
    applied = apply_from_preview(
        product, ('compute', 'connection', 'apply'), preview, key)
    assert applied['operation']['state'] == 'succeeded', applied
    return preview, applied


def plan_change(intent, editor, plan_id=None, head=None, creation_key=None):
    target = ({'intent': 'create', 'creation_key': creation_key}
              if intent == 'create' else
              {'intent': 'update', 'plan_id': plan_id, 'expected_head_revision': head})
    return {'schema': 'hiroute.plan-content-change/v2', 'target': target,
            'editor': editor, 'consumed_draft': None}


def apply_plan(product, change, key):
    preview = product.cli(
        'routing', 'preview', '--request-stdin', payload={'change': change})['data']
    body = {'change': change, 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': key}
    applied = product.cli('routing', 'apply', '--request-stdin', payload=body)
    assert applied['operation']['state'] == 'succeeded', applied
    return preview, body, applied


def run(repository):
    product = InstalledProduct(repository)
    stage = 'install'
    evidence = {}
    try:
        product.install()
        product.start()

        stage = 'released-command-admission'
        schema = product.cli('schema', 'list')['data']
        released = {item['command_id'] for item in schema['commands']}
        required = {
            'operations.find', 'operations.get', 'compute.list', 'compute.show',
            'compute.connection.options', 'compute.connection.preview',
            'compute.connection.apply', 'compute.connection.authorize',
            'compute.connection.test', 'routing.options', 'routing.list', 'routing.show',
            'routing.preview', 'routing.apply', 'models.show', 'agents.scan', 'agents.check',
            'agents.connect.preview', 'agents.connect.apply', 'agents.connect.status',
            'agents.restore.preview', 'agents.restore.apply', 'sessions.list',
            'sessions.show', 'sessions.receipt', 'sessions.status', 'value.show',
        }
        assert required <= released, required - released
        rejected = product.cli('settings', 'show', expected=2)
        assert rejected['error']['code'] == 'UNKNOWN_COMMAND', rejected
        invalid = product.cli('compute', 'connection', 'test', '--request-stdin',
                              payload={'kind': 'native', 'request': {}, 'unknown': True}, expected=2)
        assert invalid['error']['code'] == 'INVALID_ARGUMENTS', invalid

        stage = 'native-source-check-save-recovery'
        candidate_ref = 'candidate/native/headless-product'
        product.register_secret(candidate_ref, NATIVE_TOKEN)
        draft = native_draft(product.upstream)
        checked = product.cli('compute', 'connection', 'test', '--request-stdin', payload={
            'kind': 'native', 'request': {'draft': draft, 'input_candidate': {
                'candidate_ref': candidate_ref, 'candidate_revision': 1}}})['data']
        assert checked['directory'] == 'unavailable' and checked['candidate']['fact_state'] == 'complete'
        native_preview, native_apply = save_candidate(
            product, checked['candidate'], None, 'headless-native-save')
        replay = apply_from_preview(
            product, ('compute', 'connection', 'apply'), native_preview,
            'headless-native-save')
        assert replay['operation']['operation_id'] == native_apply['operation']['operation_id']
        damaged = {
            'spec': native_preview['spec'], 'accept_digest': 'sha256:' + '0' * 64,
            'expected_revisions': native_preview['expected_revisions'],
            'idempotency_key': 'headless-native-save',
        }
        reused = product.cli('compute', 'connection', 'apply', '--request-stdin',
                             payload=damaged, expected=3)
        assert reused['error']['code'] == 'IDEMPOTENCY_KEY_REUSED', reused
        recovered = product.cli('operations', 'find', '--request-stdin', payload={
            'principal_kind': 'interactive_user', 'operation_kind': 'ApplyComputeSave',
            'idempotency_key': 'headless-native-save',
            'accepted_digest': native_preview['accept_digest'],
        })
        assert recovered['data']['digest_matches']
        operation_id = recovered['data']['operation']['operation_id']
        assert operation_id == native_apply['operation']['operation_id']
        assert product.cli('operations', 'get', operation_id)['data']['state'] == 'succeeded'
        snapshot = product.cli('compute', 'list')['data']
        native_source = next(source for source in snapshot['sources']
                             if source['display_name'] == 'Headless Native API')
        shown = product.cli('compute', 'show', native_source['source_id'])['data']
        assert shown['sources'] == [native_source]
        binding_id = native_source['models'][0]['binding_id']
        product.cli('protected-input', 'release', '--candidate', candidate_ref)

        stage = 'subscription-discovery-check-save'
        options = product.cli('compute', 'connection', 'options')['data']
        subscriptions = options['subscriptions']
        assert subscriptions['discovery_state'] == 'complete', subscriptions
        pending = next(item for item in subscriptions['candidates']
                       if item['provenance'] == 'connector_owned')
        sub_preview = product.cli(
            'compute', 'connection', 'preview', '--request-stdin',
            payload={'candidate': pending['candidate']})['data']
        sub_apply = apply_from_preview(
            product, ('compute', 'connection', 'apply'), sub_preview,
            'headless-subscription-check')
        checked_sub = product.cli(
            'compute', 'connection', 'authorize', '--request-stdin',
            payload={'action': 'result', 'operation': sub_apply['operation']})['data']
        assert checked_sub['status'] == 'verified', checked_sub
        save_candidate(product, checked_sub['checked_candidate'],
                       checked_sub['validation'], 'headless-subscription-save')
        snapshot = product.cli('compute', 'list')['data']
        subscription_source = next(source for source in snapshot['sources']
                                   if source['connection_identity']['access_kind'] == 'subscription')
        configuration_id = next(model['catalog_configuration_id']
                                for model in subscription_source['models']
                                if model.get('catalog_configuration_id'))

        stage = 'route-create-update-conflict'
        routing_options = product.cli('routing', 'options', '--request-stdin', payload={})['data']
        assert any(item['binding_id'] == binding_id for item in routing_options['candidates'])
        model = product.cli('models', 'show', '--request-stdin', payload={
            'kind': 'reference', 'model_configuration_id': configuration_id})['data']
        assert model['kind'] == 'reference'
        editor = {
            'schema': 'hiroute.plan-editor/v2',
            'display_name': 'Headless production route',
            'purpose': 'Installed CLI and Agent traffic',
            'mode': 'fixed_model',
            'candidates': [{'binding_id': binding_id}],
            'smart': {'economy': [], 'primary': [], 'primary_fallback': False,
                      'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
            'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
            'delegation_enabled': False,
            'requirements': {},
            'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000,
                       'attempt_timeout_ms': 30000},
        }
        created, _, _ = apply_plan(
            product, plan_change('create', editor, creation_key='headless-product-route'),
            'headless-route-create')
        plan_id = created['plan_head']['reference']['plan_id']
        alias = created['plan_head']['model_alias']
        catalog = product.cli('routing', 'list')['data']
        assert any(plan['head']['reference']['plan_id'] == plan_id for plan in catalog['plans'])
        shown_plan = product.cli('routing', 'show', plan_id)['data']
        head = shown_plan['head']['head_revision']
        stale_change = plan_change('update', dict(editor, purpose='stale edit'), plan_id, head)
        stale_preview = product.cli(
            'routing', 'preview', '--request-stdin', payload={'change': stale_change})['data']
        worker_editor = dict(editor, purpose='Headless delegated work', delegation_enabled=True,
                             work={'harness': 'codex_cli', 'protocol': 'responses'})
        apply_plan(product, plan_change('update', worker_editor, plan_id, head),
                   'headless-route-worker-update')
        stale_body = {'change': stale_change, 'accept_digest': stale_preview['change_digest'],
                      'expected_revisions': stale_preview['expected_revisions'],
                      'idempotency_key': 'headless-route-stale'}
        conflict = product.cli('routing', 'apply', '--request-stdin',
                               payload=stale_body, expected=3)
        assert conflict['error']['code'] in ('CHANGE_PREVIEW_STALE', 'REVISION_CONFLICT'), conflict

        stage = 'agent-check-connect-original-entry'
        scan = product.cli('agents', 'scan')['data']
        codex = next(agent for agent in scan['agents']
                     if agent['agent_id'] == 'agent_codex_default')
        assert codex['supported'], codex
        context = codex['context_id']
        checked_agent = product.cli(
            'agents', 'check', 'agent_codex_default', '--scope', 'native-authentication')['data']
        assert checked_agent['native_authentication'] == 'proven'
        rescanned = next(agent for agent in product.cli('agents', 'scan')['data']['agents']
                         if agent['agent_id'] == 'agent_codex_default')
        assert rescanned['supported'] and rescanned['context_id'] == context, rescanned
        spec = {
            'schema_version': {'major': 2, 'minor': 0},
            'context_id': context,
            'model': {'intent': 'configure', 'settings': {
                'mode': 'codex_default', 'native_model_mode': 'hiroute_only', 'fixed_models': [],
                'allowed_plan_ids': [plan_id],
                'default_selection': {'kind': 'plan', 'plan_id': plan_id},
            }},
        }
        agent_preview = product.cli(
            'agents', 'connect', 'preview', '--request-stdin', payload={'spec': spec})['data']
        assert agent_preview['applicable'], agent_preview
        assert not agent_preview['resident_service']['login_item_required'], agent_preview
        agent_body = {
            'spec': agent_preview['spec'],
            'accept_digest': agent_preview['accept_digest'],
            'dependency_digest': agent_preview['dependency_digest'],
            'expected_revisions': agent_preview['expected_revisions'],
            'idempotency_key': 'headless-agent-connect',
        }
        agent_apply = product.cli(
            'agents', 'connect', 'apply', '--request-stdin', payload=agent_body)
        assert agent_apply['operation']['state'] == 'succeeded'
        agent_status = product.cli('agents', 'connect', 'status', context)['data']
        assert agent_status['state'] == 'configured', agent_status
        managed_config = (product.codex_home / 'config.toml').read_text()
        catalog_pointers = [json.loads(value.strip())
                            for line in managed_config.splitlines()
                            for key, separator, value in [line.partition('=')]
                            if separator and key.strip() == 'model_catalog_json']
        assert len(catalog_pointers) == 1, managed_config
        managed_catalog = json.loads(Path(catalog_pointers[0]).read_text())
        assert [entry['slug'] for entry in managed_catalog['models']] == [alias], managed_catalog
        assert len(json.loads((product.codex_home / 'models_cache.json').read_text())['models']) > 1
        restore_point = agent_status['restore_point_ref']
        launched = subprocess.run(
            [str(product.codex), 'exec', 'headless request'], cwd=product.project,
            env=product.env, capture_output=True, timeout=60)
        product.outputs.extend((launched.stdout, launched.stderr))
        assert launched.returncode == 0 and b'native product answer' in launched.stdout, launched
        assert any(request['path'] == '/v1/responses' for request in product.upstream.requests)

        stage = 'observation-facts-and-usage'
        deadline = time.monotonic() + 30
        while True:
            sessions = product.cli('sessions', 'list', '--include-unlinked', '--limit', '50')['data']
            if sessions['sessions']:
                observed = None
                for summary in sessions['sessions']:
                    detail = product.cli('sessions', 'show', summary['session_id'])['data']
                    for turn in detail['turns']:
                        for candidate_receipt_id in turn['receipt_ids']:
                            candidate_receipt = product.cli(
                                'sessions', 'receipt', candidate_receipt_id)['data']
                            facts = [event['fact']
                                     for event in candidate_receipt['ordered_facts']]
                            route = next((fact for fact in facts
                                          if fact['kind'] == 'route_decision'), None)
                            usage = next((fact for fact in facts
                                          if fact['kind'] == 'usage_and_cache'), None)
                            if (route is not None and route['plan_id'] == plan_id
                                    and usage is not None):
                                observed = (detail, candidate_receipt_id,
                                            candidate_receipt, usage)
                                break
                        if observed is not None:
                            break
                    if observed is not None:
                        break
                if observed is not None:
                    break
            assert time.monotonic() < deadline, sessions
            time.sleep(.1)
        detail, receipt_id, receipt, usage = observed
        session_id = detail['summary']['session_id']
        assert receipt['ordered_facts'] and MODEL in json.dumps(receipt), receipt
        assert usage['input_tokens'] == 4 and usage['output_tokens'] == 3, usage
        observation_status = product.cli('sessions', 'status')['data']
        assert observation_status['facts_completeness'] != 'unavailable', observation_status
        value = product.cli('value', 'show', '--routing', plan_id,
                            '--session', session_id)['data']
        assert len(value['plans']) == 1, value
        plan_value = value['plans'][0]
        assert plan_value['agent_plan_id'] == plan_id, plan_value
        # No price evidence was declared for this user-provided model. The immutable receipt still
        # exposes the reported usage above, while the value ledger must not fabricate a priced row.
        assert plan_value['input_tokens'] == 0 and plan_value['output_tokens'] == 0, plan_value
        assert plan_value['actual_incremental_cost_micros'] is None, plan_value
        assert plan_value['facts_completeness'] == 'unknown', plan_value

        stage = 'worker-selection-submit-wait-result-replay'
        discovered = product.cli(
            'worker', 'dependencies', 'discover', '--harness', 'codex_cli')['data']
        revision = next(item['revision'] for item in discovered['selection_revisions']
                        if item['harness'] == 'codex_cli')
        selected = product.cli(
            'worker', 'dependencies', 'select', '--request-stdin', payload={
                'harness': 'codex_cli', 'adapter_path': str(product.adapter),
                'cli_path': str(product.codex), 'expected_selection_revision': revision,
            })['data']
        assert any(item['adapter_path'] == str(product.adapter) for item in selected['selected'])
        executors = product.cli('worker', 'executors')['data']
        assert any(item['harness'] == 'codex_cli' and item['state'] == 'ready'
                   and item['start_approve_all']['state'] == 'ready'
                   for item in executors['executors']), executors
        plans = product.cli('worker', 'plans')['data']
        assert any(item['agent_plan_id'] == plan_id and item['availability'] == 'ready'
                   for item in plans['plans']), plans
        worker_command = (
            'worker', 'exec', '--plan', plan_id, '--cwd', str(product.project),
            '--no-wait', '--submission-key', 'headless-worker-submit', '--file', '-')
        accepted = product.cli(*worker_command, payload='delegate through installed CLI')
        run_id = accepted['data']['run_id']
        recovered_worker = product.cli(
            'worker', 'status', '--submission-key', 'headless-worker-submit',
            '--operation', 'exec')['data']
        assert recovered_worker['run_id'] == run_id
        product.cli('worker', 'status', '--run', run_id)
        product.cli('worker', 'wait', '--run', run_id, '--wait-timeout', '30')
        deadline = time.monotonic() + 60
        while True:
            result = product.cli('worker', 'result', '--run', run_id)['data']
            if terminal(result['run_state']):
                break
            assert time.monotonic() < deadline, result
            time.sleep(.1)
        assert result['run_state'] == 'succeeded' and 'native product answer' in result['result'], result
        replayed = product.cli(*worker_command, payload='delegate through installed CLI')['data']
        assert replayed['replayed'] and replayed['run_id'] == run_id, replayed
        listed = product.cli('worker', 'list')['data']
        assert any(item['run']['run_id'] == run_id for item in listed['tasks']), listed

        stage = 'agent-restore'
        restore_spec = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': context,
                        'model': {'intent': 'restore', 'restore_point_ref': restore_point}}
        restore_preview = product.cli(
            'agents', 'restore', 'preview', '--request-stdin',
            payload={'spec': restore_spec})['data']
        assert not restore_preview['resident_service']['login_item_removal_required']
        restore_body = {
            'spec': restore_preview['spec'],
            'accept_digest': restore_preview['accept_digest'],
            'dependency_digest': restore_preview['dependency_digest'],
            'expected_revisions': restore_preview['expected_revisions'],
            'idempotency_key': 'headless-agent-restore',
        }
        restored = product.cli(
            'agents', 'restore', 'apply', '--request-stdin', payload=restore_body)
        assert restored['operation']['state'] == 'succeeded'
        restored_status = product.cli('agents', 'connect', 'status', context)['data']
        assert restored_status['state'] in ('restored', 'not_configured'), restored_status
        codex_configuration = (product.codex_home / 'config.toml').read_text()
        assert 'model = ' + json.dumps(MODEL) in codex_configuration
        assert 'model_provider = "fixture_native"' in codex_configuration
        assert 'experimental_bearer_token = ' + json.dumps(NATIVE_TOKEN) in codex_configuration
        assert 'model_providers.hiroute' not in codex_configuration
        assert 'X-HiRoute-Token' not in codex_configuration

        evidence = {
            'scenario': 'installed-standalone-headless-management-loop',
            'state': 'green', 'candidate': product.sha,
            'installed_cli': str(product.cli_path),
            'released_command_count': len(released),
            'native_source': native_source['source_id'],
            'subscription_source': subscription_source['source_id'],
            'plan_id': plan_id, 'model_alias': alias,
            'agent_context': context, 'session_id': session_id,
            'receipt_id': receipt_id, 'worker_run_id': run_id,
            'upstream_request_count': len(product.upstream.requests),
            'protected_input_released_after_save': True,
            'same_uid_local_checks_without_capability': True,
            'standalone_resident_service_reused': True,
            'unknown_result_recovered_by_idempotency': True,
            'duplicate_worker_submission_replayed': True,
        }
        print(json.dumps(evidence, sort_keys=True), flush=True)
    except Exception:
        print(json.dumps({'scenario': 'installed-standalone-headless-management-loop',
                          'state': 'red', 'candidate': product.sha, 'stage': stage,
                          'partial': evidence}, sort_keys=True), flush=True)
        raise
    finally:
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
