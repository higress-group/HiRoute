"""Owned native settings and reads for Qoder/Pi persisted-model product journeys.

Only this leaf may write the explicitly selected MODEL context. It never reads
authentication, and it never supplies native provider/settings/credential overrides.
"""
from copy import deepcopy
import json
import os
from pathlib import Path
import secrets
import stat

from agent_product_support import run_native_command
from native_context_fixture import native_text
from qoder_native_context import selected_directory


CASES = ('agent.models.persisted-routes', 'agent.models.credential-rotation',
         'agent.models.independent-restore', 'agent.models.default-reference-guard')
LIVE_PROMPT = 'Reply with exactly HIROUTE_LIVE_CHECK_OK. Do not use tools or perform any other action.'


def select_model_context(product, harness="qoder"):
    product.additional_harness = harness
    if harness == "pi":
        config = Path(product.env["HOME"]) / ".pi/agent"
        config.mkdir(mode=0o700, parents=True)
        product.env["PI_CODING_AGENT_DIR"] = str(config)
        return config
    home = selected_directory('HIROUTE_QODER_MODEL_CONTEXT_HOME')
    config = selected_directory('HIROUTE_QODER_MODEL_CONFIG_DIR')
    assert home != Path.home().resolve() and config.is_relative_to(home), \
        'model acceptance requires a dedicated context, never the daily HOME/config'
    product.env.update(HOME=str(home), QODER_CONFIG_DIR=str(config))
    return config


class OwnedModelSettings:
    """Seed a synthetic user baseline; undo only our unchanged final baseline."""
    def __init__(self, config, foreign_endpoint, harness="qoder"):
        self.harness = harness
        self.default_path = Path(config) / "settings.json"
        self.default_original = self.default_path.read_bytes() if harness == "pi" and self.default_path.exists() else None
        self.default_baseline = None
        self.auth_path = Path(config) / "auth.json"
        self.auth_owned = None
        self.path = Path(config) / ('models.json' if harness == 'pi' else 'settings.json')
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
        if harness == 'pi':
            providers['fixture-native'] = {'api':'openai-responses','baseUrl':foreign_endpoint,
                'apiKey':'synthetic-unselected-native-key','models':[{'id':'native-default','name':'Native default',
                'reasoning':False,'input':['text'],'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0},
                'contextWindow':100000,'maxTokens':4096}]}
            self.default_baseline = {'defaultProvider':'fixture-native','defaultModel':'native-default',
                'retry':{'enabled':False},'compaction':{'enabled':False},'cacheWarming':'off'}
            self.default_path.write_text(json.dumps(self.default_baseline))
            self.default_path.chmod(0o600)
        else:
            self.baseline.setdefault('model', {})['name'] = self.native_default
            self.baseline.setdefault('general', {})['enableAutoUpdate'] = False
            self.baseline.setdefault('hooks', {})
            self.baseline.setdefault('mcpServers', {})
        self.baseline['hirouteAcceptance'] = {'unknown': ['preserve', secrets.token_hex(8)]}
        self.write(self.baseline)

    def install_conflicting_auth(self, providers):
        assert self.harness == "pi" and not self.auth_path.exists()
        self.auth_owned = json.dumps({provider:{"type":"api_key","key":"saved-key-must-not-override-route"} for provider in providers}).encode()
        self.auth_path.write_bytes(self.auth_owned)
        self.auth_path.chmod(0o600)

    def read(self):
        assert not self.path.is_symlink() and self.path.is_file(), 'settings target was replaced'
        return json.loads(self.path.read_text())

    def write(self, value):
        assert not self.path.is_symlink(), 'refusing to write linked fixture settings'
        self.path.write_text(json.dumps(value, indent=2) + '\n')
        self.path.chmod(0o600)

    def assert_preserved(self, provider_id, selected_default=None):
        actual = self.read()
        for name in list(actual.get('providers', {})):
            if name == provider_id or name.startswith(provider_id + '-'):
                actual['providers'].pop(name)
        expected = deepcopy(self.baseline)
        if selected_default is not None:
            if self.harness == 'qoder':
                expected['model']['name'] = selected_default
        if self.harness == 'pi':
            default = dict(self.default_baseline)
            if selected_default is not None:
                default['defaultProvider'], default['defaultModel'] = selected_default.split('/',1)
            assert json.loads(self.default_path.read_text()) == default, 'changed separate native defaults'
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
        if self.harness == 'pi':
            actual = json.loads(self.default_path.read_text())
            assert actual['defaultProvider'] + '/' + actual['defaultModel'] == expected
            actual['defaultProvider'], actual['defaultModel'] = replacement.split('/',1)
            self.default_path.write_text(json.dumps(actual))
            return
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
        if self.auth_owned is not None:
            assert self.auth_path.read_bytes() == self.auth_owned
            self.auth_path.unlink()
        assert self.read() == self.baseline, 'cannot undo fixture baseline while managed/drifted fields remain'
        if self.harness == 'pi':
            assert json.loads(self.default_path.read_text()) == self.default_baseline
            if self.default_original is None: self.default_path.unlink()
            else: self.default_path.write_bytes(self.default_original)
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
    if getattr(product, 'additional_harness', 'qoder') == 'pi':
        provider, model = selector.split('/',1)
        return [os.environ['HIROUTE_WORKER_NODE'], str(binary), '--provider', provider, '--model', model,
                '--mode', 'json', '--print', '--no-session', '--no-extensions', '--offline', '--tools', 'read', prompt]
    return [str(binary), '--cwd', str(product.project), '--config-dir', product.env['QODER_CONFIG_DIR'],
            '--model', selector, '--print', '--no-session-persistence', '--output-format', 'stream-json',
            '--tools', 'Read', '--permission-mode', 'dont_ask', '--max-model-request-retries', '0',
            '--max-turns', '2', '-p', prompt]


def successful_native_result(stdout, receipt, harness='qoder'):
    if harness == 'pi':
        rows = [json.loads(line) for line in stdout.decode().splitlines() if line.startswith('{')]
        headers = [r for r in rows if r.get('type') == 'session']
        endings = [r for r in rows if r.get('type') == 'agent_end']
        assert len(headers) == len(endings) == 1, 'missing ordinary Pi terminal result/session'
        last = endings[0]['messages'][-1]
        assert last['role'] == 'assistant' and last['stopReason'] == 'stop'
        assert ''.join(p.get('text','') for p in last['content']).strip() == receipt
        return headers[0]['id']
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
    harness = getattr(product, 'additional_harness', 'qoder')
    stdout = run_native_command(product, command, timeout=60, label=f'ordinary {harness} model invocation')
    assert oracle.calls[before:] == [label], f'ordinary {harness} did not make one actual persisted-route request'
    native_session_id = successful_native_result(stdout, oracle.receipt, harness)
    from native_context_boundaries import source_events
    observed_protocol = source_events(source)[-1]['protocol']
    if hasattr(source, 'expected_protocol'):
        assert observed_protocol == source.expected_protocol, 'native route did not use its chosen upstream protocol'
    return {'label': label, 'model': source.model, 'provider': selector.split('/', 1)[0],
            'protocol': observed_protocol, 'native_session_id': native_session_id, 'state': 'green'}


def persisted_plan_provider(settings, namespace, alias):
    """Select the native entry by its declared route, never by list ordering."""
    matches = [name for name, provider in settings.read().get('providers', {}).items()
               if (name == namespace or name.startswith(namespace + '-'))
               and any(model.get('id', model.get('model')) == alias for model in provider.get('models', []))]
    assert len(matches) == 1, 'persisted route must have exactly one provider'
    return matches[0]
