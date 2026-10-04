"""Owned Qoder settings and native reads for the persisted-model product journey.

Only this leaf may write the explicitly selected MODEL context. It never reads
authentication, and it never supplies native provider/settings/credential overrides.
"""
from copy import deepcopy
import json
import os
from pathlib import Path
import secrets
import signal
import stat
import subprocess

from native_context_fixture import native_text
from qoder_native_context import selected_directory


CASES = ('agent.models.persisted-routes', 'agent.models.credential-rotation',
         'agent.models.independent-restore', 'agent.models.default-reference-guard')
LIVE_PROMPT = 'Reply with exactly HIROUTE_LIVE_CHECK_OK. Do not use tools or perform any other action.'


def observe_source_requests(source):
    """Count even rejected requests on the existing SSE server; record no secrets."""
    original = source.server.RequestHandlerClass
    count = [0]

    class CountedHandler(original):
        def do_POST(self):
            with source.lock:
                count[0] += 1
            super().do_POST()

    source.server.RequestHandlerClass = CountedHandler

    def observed():
        with source.lock:
            return count[0]

    source.request_count = observed


def select_model_context(product):
    home = selected_directory('HIROUTE_QODER_MODEL_CONTEXT_HOME')
    config = selected_directory('HIROUTE_QODER_MODEL_CONFIG_DIR')
    assert home != Path.home().resolve() and config.is_relative_to(home), \
        'model acceptance requires a dedicated context, never the daily HOME/config'
    product.env.update(HOME=str(home), QODER_CONFIG_DIR=str(config))
    return config


class OwnedModelSettings:
    """Seed a synthetic user baseline; undo only our unchanged final baseline."""
    def __init__(self, config, foreign_endpoint):
        self.path = Path(config) / 'settings.json'
        assert not self.path.is_symlink(), 'model fixture cannot own a linked settings target'
        self.original = self.path.read_bytes() if self.path.exists() else None
        self.original_mode = stat.S_IMODE(self.path.stat().st_mode) if self.original is not None else None
        self.baseline = json.loads(self.original) if self.original is not None else {}
        assert isinstance(self.baseline, dict), 'dedicated model settings must be a JSON object'
        self.baseline = deepcopy(self.baseline)
        assert not {'hirouteAcceptance', 'hirouteLaterEdit'} & self.baseline.keys(), \
            'another model fixture owns this context'
        providers = self.baseline.setdefault('providers', {})
        assert isinstance(providers, dict) and 'fixture-native' not in providers
        providers['fixture-native'] = {
            'type': 'openai-compatible', 'protocol': 'openai-responses', 'authType': 'bearer',
            'baseUrl': foreign_endpoint, 'apiKey': 'synthetic-unselected-native-key',
            'models': [{'model': 'native-default', 'contextWindow': 100000,
                        'maxOutputTokens': 4096, 'capabilities': {'tools': True}}],
            'fixtureUnknownProviderKey': {'preserve': ['native', 'user']},
        }
        self.native_default = 'fixture-native/native-default'
        self.baseline.setdefault('model', {})['name'] = self.native_default
        self.baseline.setdefault('general', {})['enableAutoUpdate'] = False
        self.baseline.setdefault('hooks', {})
        self.baseline.setdefault('mcpServers', {})
        self.baseline['hirouteAcceptance'] = {'unknown': ['preserve', secrets.token_hex(8)]}
        self.write(self.baseline)

    def read(self):
        assert not self.path.is_symlink() and self.path.is_file(), 'settings target was replaced'
        return json.loads(self.path.read_text())

    def write(self, value):
        assert not self.path.is_symlink(), 'refusing to write linked fixture settings'
        self.path.write_text(json.dumps(value, indent=2) + '\n')
        self.path.chmod(0o600)

    def assert_preserved(self, provider_id, selected_default=None):
        actual = self.read()
        actual.get('providers', {}).pop(provider_id, None)
        expected = deepcopy(self.baseline)
        if selected_default is not None:
            expected['model']['name'] = selected_default
        assert actual == expected, 'model operation changed native/default/unknown user fields'

    def add_user_edit(self, provider_id):
        self.assert_preserved(provider_id)
        text = self.path.read_text()
        value = {'edited_after_apply': secrets.token_hex(8)}
        # Add one root field without reserializing the managed provider. Otherwise
        # a fixture's unrelated edit could manufacture owned-node formatting drift.
        end = text.rfind('}')
        updated = text[:end].rstrip() + ',\n  "hirouteLaterEdit": ' + json.dumps(value) + '\n' + text[end:]
        expected = self.read()
        expected['hirouteLaterEdit'] = value
        assert json.loads(updated) == expected, 'fixture root edit changed another field'
        self.path.write_text(updated)
        self.baseline['hirouteLaterEdit'] = value

    def select_default(self, expected, replacement):
        actual = self.read()
        assert actual['model']['name'] == expected, 'fixture default changed independently'
        actual['model']['name'] = replacement
        text = self.path.read_text()
        token = json.dumps(expected)
        assert text.count(token) == 1, 'fixture default token is ambiguous'
        updated = text.replace(token, json.dumps(replacement), 1)
        assert json.loads(updated) == actual, 'fixture default edit changed another field'
        self.path.write_text(updated)

    def close(self):
        assert self.read() == self.baseline, 'cannot undo fixture baseline while managed/drifted fields remain'
        if self.original is None:
            self.path.unlink()
        else:
            self.path.write_bytes(self.original)
            self.path.chmod(self.original_mode)


class PersistedRouteOracle:
    def __init__(self, model):
        self.model = model
        self.marker = None
        self.receipt = None
        self.calls = []

    def arm(self, label):
        self.marker = 'PERSISTED-INPUT-' + secrets.token_hex(12)
        self.receipt = 'PERSISTED-OUTPUT-' + secrets.token_hex(12)
        self.label = label
        return self.marker

    def reply(self, _fixture, body):
        assert body['model'] == self.model, 'persisted model reached the wrong source'
        text = native_text(body)
        if LIVE_PROMPT in text:
            self.calls.append('live-check')
            return dict(kind='text', text='HIROUTE_LIVE_CHECK_OK', request_kind='live-check')
        assert self.marker is not None and self.marker in text, 'unplanned persisted model request'
        self.calls.append(self.label)
        return dict(kind='text', text=self.receipt, request_kind=self.label)


def native_command(product, binary, selector, prompt):
    # Ordinary settings precedence remains active. In particular, no --settings,
    # --setting-sources or credential environment can hide a persistence defect.
    return [str(binary), '--cwd', str(product.project), '--config-dir', product.env['QODER_CONFIG_DIR'],
            '--model', selector, '--print', '--no-session-persistence', '--output-format', 'stream-json',
            '--tools', 'Read', '--permission-mode', 'dont_ask', '--max-model-request-retries', '0',
            '--max-turns', '2', '-p', prompt]


def successful_native_result(stdout, receipt):
    records = []
    for line in stdout.decode().splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get('type') == 'result':
            records.append(value)
    assert len(records) == 1, 'expected one native Qoder terminal result'
    result = records[0]
    assert result.get('subtype') == 'success' and result.get('is_error') is False, \
        'native Qoder terminal result is an error'
    assert result.get('result', '').strip() == receipt, 'native result lost the independent source receipt'
    assert isinstance(result.get('session_id'), str) and result['session_id'], 'native result lacks session identity'
    return result['session_id']


def read_persisted_route(product, binary, selector, source, oracle, label):
    prompt = oracle.arm(label)
    before = len(oracle.calls)
    command = native_command(product, binary, selector, prompt)
    # Product already strips inherited provider/auth variables. Only its selected
    # native config root and private runtime/receipt roots enter this process.
    process = subprocess.Popen(command, env=dict(product.env), cwd=product.project,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=60)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            stdout, stderr = process.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate(timeout=3)
        product.outputs.extend((stdout, stderr))
        raise AssertionError('ordinary Qoder model invocation exceeded its deadline') from None
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=3)
    product.outputs.extend((stdout, stderr))
    assert process.returncode == 0, 'ordinary Qoder failed; inspect private diagnostics'
    assert oracle.calls[before:] == [label], 'ordinary Qoder did not make one actual persisted-route request'
    native_session_id = successful_native_result(stdout, oracle.receipt)
    return {'label': label, 'model': source.model, 'native_session_id': native_session_id, 'state': 'green'}
