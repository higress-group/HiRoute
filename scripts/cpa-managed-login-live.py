#!/usr/bin/env python3
"""Private, resumable real CPA login acceptance through HiRoute's production CLI.

Prepare starts an isolated role=all daemon, but does not start OAuth. Authorize
requires a real controlling terminal: the URL goes only to that terminal/browser,
and callback text is read without echo and registered through an inherited FD.
No native credential is needed for a managed login. CPA alone writes its new auth.
"""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import socket
import stat
import subprocess
import sys
import termios
import tempfile
import time
import webbrowser


def support_module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


evidence = support_module('cpa-managed-login-evidence')
runtime_support = support_module('cpa-managed-login-runtime')


class LiveFailure(Exception):
    pass


def require(condition, code):
    if not condition:
        raise LiveFailure(code)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def sha256(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as handle:
        while data := handle.read(1024 * 1024):
            value.update(data)
    return value.hexdigest()


def private_directory(path, create=False):
    path = Path(path)
    if create:
        path.mkdir(mode=0o700)
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid()
            and stat.S_IMODE(info.st_mode) == 0o700, 'private_directory_required')
    return path


def private_read(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
                and not info.st_mode & 0o077 and info.st_size <= 4 * 1024 * 1024,
                'private_file_required')
        raw = b''
        while chunk := os.read(fd, 65536):
            raw += chunk
            require(len(raw) <= 4 * 1024 * 1024, 'private_file_too_large')
        return json.loads(raw)
    finally:
        os.close(fd)


def private_write(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.next')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, 'wb') as handle:
            handle.write(encoded(value) + b'\n')
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def safe_code(error):
    if isinstance(error, (LiveFailure, evidence.EvidenceFailure, runtime_support.RuntimeFailure)) and re.fullmatch(r'[A-Za-z0-9_.:-]{1,160}', str(error)):
        return str(error)
    return type(error).__name__


def caller_harness_sha():
    return runtime_support.caller_harness_sha(Path(__file__).resolve().parents[1])


def isolated_environment(root):
    # Construct an allowlist instead of inheriting alternative client auth/config.
    env = {key: os.environ[key] for key in (
        'LANG', 'LC_ALL', 'LC_CTYPE', 'TZ', 'SSL_CERT_FILE', 'SSL_CERT_DIR',
        'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
        'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy') if key in os.environ}
    home = Path(root) / 'home'
    env.update(HOME=str(home), CODEX_HOME=str(home / '.codex'),
               CLAUDE_CONFIG_DIR=str(home / '.claude'),
               CLAUDE_SECURESTORAGE_CONFIG_DIR=str(home / '.claude'),
               QODER_CONFIG_DIR=str(home / '.qoder'),
               PI_CODING_AGENT_DIR=str(home / '.pi/agent'), DSH_HOME=str(home / '.dsh'),
               XDG_CONFIG_HOME=str(home / '.config'), XDG_CACHE_HOME=str(home / '.cache'),
               XDG_DATA_HOME=str(home / '.local/share'), TMPDIR=str(Path(root) / 'tmp'),
               PATH=str(Path(root) / 'bin') + ':/usr/local/bin:/usr/bin:/bin',
               HIROUTE_RUNTIME_DIR=str(Path(root) / 'runtime'),
               HIROUTE_WORKER_RECEIPT_DIR=str(Path(root) / 'worker-receipts'),
               HIROUTE_REPLAY_ROOT=str(Path(root) / 'replay'))
    return env


def validate_candidate(repository, candidate, binaries, cpa_binary):
    repository, binaries, cpa_binary = map(lambda p: Path(p).resolve(),
                                           (repository, binaries, cpa_binary))
    actual = subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'],
                                     text=True).strip()
    require(actual == candidate and re.fullmatch(r'[0-9a-f]{40}', candidate),
            'candidate_checkout_mismatch')
    dirty = subprocess.check_output(['git', '-C', str(repository), 'status', '--porcelain',
                                     '--untracked-files=no'])
    require(not dirty, 'candidate_checkout_dirty')
    require(binaries == repository / 'target/debug', 'retained_default_debug_target_required')
    pin = json.loads((repository / 'vendor/cpa/source.json').read_bytes())
    provenance = json.loads(cpa_binary.with_suffix('.provenance.json').read_bytes())
    require(all(provenance.get(key) == value for key, value in pin.items()), 'cpa_pin_mismatch')
    cpa_digest = sha256(cpa_binary)
    require(cpa_digest == provenance.get('sha256'), 'cpa_binary_digest_mismatch')
    return {'repository': str(repository), 'candidate_sha': candidate,
            'product_bin': str(binaries), 'cpa_binary': str(cpa_binary),
            'binaries': {name: sha256(binaries / name) for name in ('hiroute', 'hirouted')},
            'cpa': {key: pin[key] for key in ('version', 'commit', 'patch_sha256')}
                   | {'sha256': cpa_digest}}


def register_callback(configuration, input_candidate, secret, runner=subprocess.run):
    require(isinstance(secret, str) and 0 < len(secret.encode()) <= 4096,
            'callback_input_invalid')
    reader, writer = os.pipe()
    try:
        data = secret.encode()
        require(os.write(writer, data) == len(data), 'callback_pipe_write_failed')
        os.close(writer)
        writer = None
        result = runner([
            str(Path(configuration['product_bin']) / 'hiroute'), 'protected-input', 'register',
            '--candidate', input_candidate['candidate_ref'], '--secret-fd', str(reader),
            '--output', 'json'], env=isolated_environment(configuration['product_root']),
            cwd=Path(configuration['product_root']) / 'project', capture_output=True,
            timeout=30, pass_fds=(reader,))
        require(data not in result.stdout and data not in result.stderr,
                'protected_callback_public_output_leak')
        require(result.returncode == 0, 'protected_callback_register_failed')
        envelope = json.loads(result.stdout)
        require(envelope.get('data', {}).get('registered') is True,
                'protected_callback_not_registered')
    finally:
        os.close(reader)
        if writer is not None:
            os.close(writer)


def hidden_tty_input(fd):
    require(os.isatty(fd), 'controlling_terminal_required')
    previous = termios.tcgetattr(fd)
    current = list(previous)
    current[3] &= ~(termios.ECHO | termios.ECHONL)
    try:
        termios.tcsetattr(fd, termios.TCSAFLUSH, current)
        os.write(fd, b'Paste the complete callback URL/code here (input is hidden), then Enter: ')
        value = bytearray()
        while True:
            byte = os.read(fd, 1)
            require(bool(byte), 'callback_input_closed')
            if byte in (b'\n', b'\r'):
                break
            value.extend(byte)
            require(len(value) <= 4096, 'callback_input_too_large')
        require(bool(value), 'callback_input_empty')
        return value.decode()
    finally:
        termios.tcsetattr(fd, termios.TCSAFLUSH, previous)
        os.write(fd, b'\n')


def credential_projection(path, provider):
    value = private_read(path)
    require(value.get('type') == provider and isinstance(value.get('access_token'), str)
            and bool(value['access_token']) and isinstance(value.get('refresh_token'), str)
            and bool(value['refresh_token']), 'managed_credential_invalid')
    expiry = value.get('expired')
    require(isinstance(expiry, str), 'original_access_expiry_absent')
    try:
        expiry_unix = int(datetime.fromisoformat(expiry.replace('Z', '+00:00')).timestamp())
    except ValueError:
        raise LiveFailure('original_access_expiry_invalid') from None
    return {'file_sha256': sha256(path), 'access_sha256': hashlib.sha256(
        value['access_token'].encode()).hexdigest(), 'refresh_sha256': hashlib.sha256(
        value['refresh_token'].encode()).hexdigest(), 'access_expires_at_unix': expiry_unix,
        'private_mode': '0o600', 'refresh_authority': 'cpa_managed'}


def managed_auth_directory(storage, login_ref):
    require(re.fullmatch(r'login-[A-Za-z0-9_-]+', login_ref) is not None, 'login_reference_invalid')
    # role=all gives the first CPA runtime storage/cpa/state; runtime_set owns
    # the registry at that state directory's parent, shared by both providers.
    return Path(storage) / 'cpa/managed-logins' / login_ref / 'auth'


def prove_forget_removed(directory, before):
    require(before.get('file_sha256') is not None, 'forget_existing_credential_required')
    require(not os.path.lexists(directory), 'forgotten_credential_directory_present')


def native_roundtrip(product, provider, source):
    """One real client turn through its already saved HiRoute connection."""
    source_identity = runtime_support.source_fingerprint(source)
    marker = 'NATIVE_OK'
    prompt = 'Reply with exactly NATIVE_OK. Do not use tools.'
    if provider == 'codex':
        command = [str(product.root / 'bin/codex'), 'exec', '--json', '--skip-git-repo-check',
                   '--sandbox', 'read-only', '-m', source['alias'],
                   '-c', 'model_reasoning_effort="low"', prompt]
    else:
        command = [str(product.bin / 'hiroute'), 'agent', 'launch', '--agent', 'claude-code',
                   '--context', source['context'], '--', '--print', '--verbose',
                   '--output-format', 'stream-json', '--no-session-persistence',
                   '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
                   '--tools', '', '--max-turns', '1', '--model', source['alias'], prompt]
    _, _, native, *_ = modules(product.repo)
    wires_before = native.wire_request_count(product)
    started = time.monotonic()
    try:
        result = subprocess.run(command, env=product.env, cwd=product.project,
                                capture_output=True, timeout=180)
    except subprocess.TimeoutExpired as error:
        product.native_client_evidence = evidence.native_projection(
            error.stdout or b'', error.stderr or b'', provider, None)
        raise LiveFailure(provider + ':native_client_timeout') from None
    product.outputs.extend((result.stdout, result.stderr))
    product.native_client_evidence = evidence.native_projection(result.stdout, result.stderr, provider, result.returncode)
    require(result.returncode == 0, provider + ':native_client_exit_' + str(result.returncode))
    facts = evidence.native_success(result.stdout, provider, marker)
    require(not facts['turn_failed'], provider + ':native_client_turn_failed')
    require(facts['terminal_present'] and facts['answer_verified'],
            provider + ':native_client_answer_invalid')
    usage = native.usage_numbers(facts['usage'])
    time.sleep(.3)
    sends = native.wire_request_count(product) - wires_before
    require(sends > 0, provider + ':native_client_no_production_upstream_send')
    return {'scenario': provider + '-real-native-client-dialogue', 'provider': provider,
            'state': 'green', 'model': source['model'], 'usage': usage, 'answer_verified': True,
            'source_identity_sha256': source_identity,
            'production_upstream_sends': sends, 'duration_seconds': round(time.monotonic() - started, 3),
            'client_evidence': product.native_client_evidence}


def native_borrowed(arguments):
    """Reuse host-side access-only filtering and add actual native business turns."""
    caller_harness_sha()
    configuration = validate_candidate(arguments.repository, arguments.candidate_sha,
                                        arguments.product_bin, arguments.cpa_binary)
    if getattr(arguments, 'provider', None) is not None:
        return native_borrowed_selected(arguments, configuration)
    arguments.cpa_sha256 = configuration['cpa']['sha256']
    arguments.prepare_only = arguments.control_only = arguments.claude_roundtrip_only = False
    arguments.isolated_source_failure = True
    _, _, native, *_ = modules(arguments.repository)
    publish = native.publish_plan
    observe = native.observed_usage
    completed = []

    def publish_and_turn(product, provider, source, judgment, configure):
        publish(product, provider, source, judgment, configure)
        completed.append(native_roundtrip(product, provider, source)
                         | {'candidate_sha': arguments.candidate_sha})

    native.publish_plan = publish_and_turn
    native.observed_usage = lambda product, results, encoder: observe(product, [*results, *completed], encoder)
    try:
        exit_code = native.run(arguments)
    finally:
        native.publish_plan = publish
        native.observed_usage = observe
    report = json.loads(arguments.report.read_bytes())
    report['scenarios'].extend(completed)
    report['paid_inference_requests'] = sum('usage' in item for item in report['scenarios'])
    report['native_dialogue_count'] = len(completed)
    report['native_dialogue_upstream_sends'] = sum(item['production_upstream_sends'] for item in completed)
    if len(completed) != 2:
        report['state'] = 'red'
    # Every native process has already ended and the old harness has audited the
    # native owner files and removed its access-only copies before this write.
    private_write(arguments.report, report)
    return {'state': report['state'], 'candidate_sha': arguments.candidate_sha,
            'native_dialogue_count': len(completed), 'paid_inference_requests': report['paid_inference_requests'],
            'report': str(arguments.report), 'process_exit': exit_code}


def gateway_roundtrip(product, provider, source, stream, label):
    _, _, native, *_ = modules(product.repo)
    first_tool = provider == 'claude' and not source.get('tool_history')
    if first_tool:
        require(not stream, 'claude:initial_tool_use_requires_nonstream')
        # The existing real-provider helper starts the bounded forced tool turn
        # through this exact label. Keep the caller's evidence label independent.
        helper_label = 'claude-initial-inference'
    else:
        helper_label = label
    result = native.gateway(product, provider, source, stream, helper_label)
    if provider == 'claude':
        require(result.get('tool_round') == ('tool_use' if first_tool else 'tool_result'),
                'claude:required_tool_round_missing')
        require(bool(source.get('tool_use_id')) and bool(source.get('tool_history')),
                'claude:matching_tool_history_missing')
        if not first_tool:
            require(result.get('multi_turn_verified') is True, 'claude:tool_result_not_verified')
    result['scenario'] = label
    return result


def access_only_cpa_audit(product, provider):
    root = product.storage / ('cpa' if provider == 'codex' else 'cpa-claude')
    configurations = list(root.rglob('config.yaml'))
    require(len(configurations) == 1, provider + ':cpa_config_count_invalid')
    line = next((line for line in configurations[0].read_text().splitlines()
                 if line.startswith('auth-dir:')), None)
    require(line is not None, provider + ':cpa_auth_directory_missing')
    directory = private_directory(Path(line.split(':', 1)[1].strip().strip('\"\'')))
    files = list(directory.glob('hiroute-managed-*.json'))
    require(len(files) == 1, provider + ':access_only_lease_count_invalid')
    _, _, native, *_ = modules(product.repo)
    value = private_read(files[0])
    require(not native.forbidden_refresh(value), provider + ':refresh_authority_in_cpa')
    require(not any('expir' in key.lower() for key in value), provider + ':expiry_in_cpa')
    return {'auth_file_count': 1, 'sha256': sha256(files[0]), 'mode': '0o600',
            'refresh_authority': False, 'expiry_in_cpa': False}


def native_borrowed_selected(arguments, configuration):
    """Run one real provider independently, retaining every ownership assertion."""
    provider = arguments.provider
    require(provider in ('codex', 'claude'), 'provider_invalid')
    _, _, native, apply_control, configure, judgment, prepare = modules(arguments.repository)
    host = getattr(arguments, provider + '_host')
    caller_sha = caller_harness_sha()
    report = {'schema': 'hiroute.cpa-native-selected/v1', 'state': 'red',
              'candidate_sha': arguments.candidate_sha, 'caller_harness_sha': caller_sha,
              'binaries': configuration['binaries'], 'cpa': configuration['cpa'],
              'selected_providers': [provider], 'scenarios': [], 'native_sources': {},
              'started_at_utc': datetime.now(timezone.utc).isoformat(),
              'limitations': ['This provider-only run does not prove the other provider.',
                              'Borrowed access never proves fresh managed OAuth or automatic refresh.']}
    temporary = tempfile.TemporaryDirectory(prefix='hiroute-native-selected-')
    configuration['product_root'] = str(Path(temporary.name) / 'p')
    for kind in ('codex', 'claude'):
        configuration[kind + '_cli'] = str(getattr(arguments, kind + '_cli').resolve())
    product = make_product(configuration, fresh=True)
    before, stage = {}, 'credentials'

    def record(scenario):
        report['scenarios'].append(scenario | {'candidate_sha': arguments.candidate_sha})

    try:
        before[provider] = native.remote_source(host, provider, 'borrow')
        credential = before[provider].pop('credential')
        require(not native.forbidden_refresh(credential), 'refresh_authority_in_borrow')
        if provider == 'codex':
            credential['last_refresh'] = datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z')
            before[provider]['synthetic_last_refresh_metadata'] = True
            destination = Path(product.env['CODEX_HOME']) / 'auth.json'
        else:
            destination = Path(product.env['CLAUDE_CONFIG_DIR']) / '.credentials.json'
        native.private_json(destination, credential)
        product.secrets.update(value for value in native.string_values(credential) if len(value) > 8)
        del credential
        other = ('home/.claude/.credentials.json' if provider == 'codex' else 'home/.codex/auth.json')
        require(not (product.root / other).exists(), 'unselected_native_auth_present')
        report['native_sources'] = before
        executable = getattr(arguments, provider + '_cli').resolve()
        version = subprocess.run([str(executable), '--version'], env=product.env,
                                 cwd=product.project, capture_output=True, timeout=15)
        match = re.search(rb'\d+\.\d+\.\d+', version.stdout)
        require(version.returncode == 0 and match is not None, 'native_client_version_unavailable')
        report['native_client'] = {'version': match.group().decode(), 'sha256': sha256(executable)}
        stage = 'daemon-start'
        product.start()
        diagnostics = product.diagnostics_snapshot()
        require(diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug',
                'daemon_diagnostics_not_debug')
        report['diagnostics_initial'] = diagnostics
        stage = provider + '-check-save-publish-connect'
        source = native.subscription_save(product, provider, apply_control, prepare)
        native.publish_plan(product, provider, source, judgment, configure)
        record({'scenario': stage, 'provider': provider, 'state': 'green', 'model': source['model']})
        report['cpa_auth_initial'] = access_only_cpa_audit(product, provider)
        for stream in (False, True):
            stage = provider + '-initial-' + ('stream' if stream else 'nonstream')
            record(gateway_roundtrip(product, provider, source, stream, stage))
        stage = provider + '-real-native-client-dialogue'
        record(native_roundtrip(product, provider, source))
        stage = provider + '-disable'
        native.save_enabled(product, provider, source, False, apply_control, prepare, stage, report['scenarios'])
        record(native.gateway(product, provider, source, False, provider + '-disabled-rejection', allowed=False))
        stage = provider + '-reenable'
        native.save_enabled(product, provider, source, True, apply_control, prepare, stage, report['scenarios'])
        stage = provider + '-restart-restore'
        product.stop()
        product.start()
        snapshot = product.control('ListCompute', {})['data']
        restored = next(item for item in snapshot['sources'] if item['source_id'] == source['source_id'])
        require(restored['state'] == 'ready', 'restored_source_not_ready')
        require([item['model_ref'] for item in restored['models']] == [source['model_ref']],
                'restored_model_changed')
        plan = product.public_cli('routing show ' + source['plan_id'])[1]['data']
        require(plan['head']['model_alias'] == source['alias'], 'restored_alias_changed')
        connection = product.cli('agents connect status ' + source['context'])[1]['data']
        require(connection.get('current_selection') is not None, 'restored_connection_missing')
        record({'scenario': stage, 'provider': provider, 'state': 'green',
                'source_identity_stable': True, 'saved_connection_restored': True})
        record(gateway_roundtrip(product, provider, source, False, provider + '-after-restart'))
        stage = 'observation'
        report['observation'] = native.observed_usage(product, report['scenarios'], encoded)
        report['cpa_auth_final'] = access_only_cpa_audit(product, provider)
        report['diagnostics_final'] = product.diagnostics_snapshot()
        report['state'] = 'green'
    except BaseException as error:
        report['failure'] = {'stage': stage, 'class': type(error).__name__, 'code': safe_code(error)}
        report['failure'].update(evidence.failure_projection(product, stage))
        record({'scenario': stage, 'provider': provider, 'state': 'red'})
        report['diagnostics_final'] = product.diagnostics_snapshot()
    finally:
        for item in before.values():
            item.pop('credential', None)
        report['native_sources'] = before
        try:
            process = product.process
            product.stop(diagnostic_failure=True)
            report['daemon_process_exit'] = process.returncode if process is not None else None
            report['upstream_wire_requests'] = native.wire_request_count(product)
            report['public_output_secret_scan'] = all(secret.encode() not in output
                for secret in product.secrets for output in product.outputs)
            require(report['public_output_secret_scan'], 'public_output_secret_scan_failed')
            logs = list(product.diagnostics_root.rglob('*.jsonl'))
            require(all(secret.encode() not in path.read_bytes() for secret in product.secrets for path in logs),
                    'diagnostics_secret_scan_failed')
            diagnostic_directory = arguments.report.parent / (arguments.report.stem + '-diagnostics')
            product.preserve_diagnostics(diagnostic_directory)
            report['diagnostics_evidence'] = str(diagnostic_directory)
        except BaseException as error:
            report['state'] = 'red'
            report['cleanup_error'] = type(error).__name__
        native.audit_native_sources(report, before, {provider: host})
        product.listener_port_lease.close()
        temporary.cleanup()
        report['temporary_secrets_removed'] = not Path(temporary.name).exists()
    report['finished_at_utc'] = datetime.now(timezone.utc).isoformat()
    report['paid_inference_requests'] = sum('usage' in item for item in report['scenarios'])
    report['native_dialogue_count'] = sum(item['scenario'] == provider + '-real-native-client-dialogue'
                                          and item['state'] == 'green' for item in report['scenarios'])
    arguments.report.parent.mkdir(parents=True, exist_ok=True)
    private_write(arguments.report, report)
    return {'state': report['state'], 'candidate_sha': arguments.candidate_sha,
            'caller_harness_sha': caller_sha, 'selected_providers': [provider],
            'native_dialogue_count': report['native_dialogue_count'],
            'paid_inference_requests': report['paid_inference_requests'], 'report': str(arguments.report)}


def refresh_verdict(original, current, now, provider_native_processes):
    crossed = now >= original['access_expires_at_unix']
    rotated = current['access_sha256'] != original['access_sha256']
    extended = current['access_expires_at_unix'] > original['access_expires_at_unix']
    ready = crossed and rotated and extended and provider_native_processes == 0
    return {'state': 'green' if ready else 'not_executed', 'original_expiry_crossed': crossed,
            'access_rotated': rotated, 'expiry_extended': extended,
            'native_processes_in_isolated_home': provider_native_processes,
            'expiry_was_modified': False, 'forced_refresh': False}


def request(run_dir, value, timeout=180):
    private_directory(run_dir)
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(timeout)
        connection.connect(str(Path(run_dir) / 'supervisor.sock'))
        connection.sendall(encoded(value) + b'\n')
        stream = connection.makefile('rb')
        response = json.loads(stream.readline(262145))
    require(response.get('ok') is True, response.get('code', 'supervisor_request_failed'))
    return response['result']


def modules(repository):
    sys.path.insert(0, str(Path(repository) / 'crates/daemon/tests/support'))
    from publication_product import Product, fixture_listener_port
    from publication_process import apply_control, configure_model_settings_v2, judgment_fixture, prepare_control_apply
    spec = importlib.util.spec_from_file_location('access_only_acceptance',
                Path(repository) / 'scripts/test-cpa-real-subscriptions.py')
    native = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(native)
    return Product, fixture_listener_port, native, apply_control, configure_model_settings_v2, judgment_fixture, prepare_control_apply


def make_product(configuration, fresh):
    Product, listener, native, *_ = modules(configuration['repository'])

    class LiveProduct(runtime_support.StableApplyMixin, runtime_support.FreshCapabilityMixin, Product):
        def __init__(self):
            self.repo = Path(configuration['repository'])
            self.bin = Path(configuration['product_bin'])
            self.release_version = json.loads((self.repo / 'contracts/cli/local-control-hello.v2.schema.json').read_text())['properties']['client_version']['const']
            self.command_descriptors = json.loads((self.repo / 'contracts/cli/planned-manifest.v1.json').read_text())['commands']
            self.root = Path(configuration['product_root'])
            self.temporary, self.project_source = None, False
            self.env = isolated_environment(self.root)
            self.storage, self.project = self.root / 'storage', self.root / 'project'
            self.diagnostics_root = self.storage / 'diagnostics'
            self.settings = self.root / 'home/.claude/settings.json'
            self.codex_settings = self.root / 'home/.codex/config.toml'
            if fresh:
                private_directory(self.root, create=True)
                for relative in ('home', 'home/.codex', 'home/.claude', 'bin', 'project', 'storage', 'tmp'):
                    private_directory(self.root / relative, create=True)
                private_write(self.settings, {})
                self.codex_settings.write_text('model = "gpt-5.4"\nmodel_reasoning_effort = "low"\n')
                self.codex_settings.chmod(0o600)
                cache = self.root / 'home/.codex/models_cache.json'
                cache.write_bytes((self.repo / 'crates/integrations/src/agents/codex_bundled_catalog.json').read_bytes())
                cache.chmod(0o600)
                for provider in ('codex', 'claude'):
                    (self.root / 'bin' / provider).symlink_to(configuration[provider + '_cli'])
            self.listener_port_lease, chosen = listener()
            self.port = configuration.get('port', chosen)
            # Keep the same gateway address across a supervised daemon restart.
            if self.port != chosen:
                self.listener_port_lease.close()
                lock_path = Path('/tmp') / f'hiroute-product-listener-ports-{os.getuid()}' / f'{self.port}.lock'
                fd = os.open(lock_path, os.O_RDWR | os.O_NOFOLLOW)
                self.listener_port_lease = os.fdopen(fd, 'r+b')
                fcntl.flock(self.listener_port_lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.process, self.outputs, self.secrets = None, [], set()
            self.cpa_args = ['--cpa-binary', configuration['cpa_binary'],
                             '--cpa-sha256', configuration['cpa']['sha256']]
            self.diagnostics_args = []
            self.enable_debug_diagnostics()
            self.startup_timeout = 90

        def control(self, operation, payload, protected_grant=None, success=True):
            envelope = super().control(operation, payload, protected_grant, success=False)
            if success:
                require(envelope.get('error') is None,
                        operation + ':' + (envelope.get('error') or {}).get('code', 'failed'))
            return envelope

        def public_cli(self, command, payload=None, secret=None, success=True):
            # OAuth Start must never enter Product.outputs or an assertion traceback.
            require(secret is None, 'protected_input_requires_dedicated_fd')
            arguments = [str(self.bin / 'hiroute'), *command.split(), '--output', 'json']
            if payload is not None:
                arguments.append('--request-stdin')
            result = subprocess.run(arguments, input=None if payload is None else encoded(payload),
                env=self.env, cwd=self.project, capture_output=True, timeout=90)
            try:
                envelope = json.loads(result.stdout)
            except ValueError:
                raise LiveFailure('public_cli_response_invalid') from None
            if not (command == 'compute connection login' and payload.get('action') == 'start'):
                self.outputs.extend((result.stdout, result.stderr))
            if success:
                require(result.returncode == 0, 'public_cli:' + (envelope.get('error') or {}).get('code', 'failed'))
            return result.returncode, envelope

    return LiveProduct()


class Supervisor:
    def __init__(self, run_dir):
        self.run_dir = private_directory(run_dir)
        self.caller_sha, self.stage = caller_harness_sha(), 'daemon-start'
        self.active_smoke = None
        self.configuration = private_read(self.run_dir / 'configuration.json')
        fresh = not Path(self.configuration['product_root']).exists()
        self.report = {'schema': 'hiroute.cpa-managed-login-live/v1',
            'candidate_sha': self.configuration['candidate_sha'], 'state': 'not_executed',
            'started_at_utc': datetime.now(timezone.utc).isoformat(),
            'binaries': self.configuration['binaries'], 'cpa': self.configuration['cpa'],
            'sessions': {}, 'sources': {}, 'scenarios': [],
            'limitations': ['Automatic original expiry is pending until its unchanged timestamp passes.',
                            'Synthetic recovery checks remain separate from real provider acceptance.']}
        if not fresh:
            self.report = private_read(self.run_dir / 'report.json')
        self.product = make_product(self.configuration, fresh=fresh)
        self.configuration['port'] = self.product.port
        private_write(self.run_dir / 'configuration.json', self.configuration)
        self.product.start()
        self.verify_debug()
        rebound = runtime_support.rebind_managed_claude(self.product, self.report['sources'], private_read,
            previous_cli=Path(self.configuration['previous_product_bin']) / 'hiroute'
                if self.configuration.get('previous_product_bin') else None)
        if rebound is not None:
            self.record(rebound)
        self.write_report()

    def write_report(self):
        self.report['updated_at_utc'] = datetime.now(timezone.utc).isoformat()
        private_write(self.run_dir / 'report.json', self.report)

    def verify_debug(self):
        snapshot = self.product.diagnostics_snapshot()
        require(snapshot['state'] == 'complete' and snapshot['level_applied']['level'] == 'debug',
                'daemon_diagnostics_not_debug')
        self.report.setdefault('diagnostic_boots', []).append({key: snapshot[key] for key in (
            'state', 'current_boot', 'level_applied', 'record_count', 'invalid_record_count')})

    def login(self, value):
        return self.product.public_cli('compute connection login', value)[1]['data']['sessions']

    def refresh_session(self, provider):
        saved = self.report['sessions'][provider]
        current = self.login({'action': 'status', 'login_ref': saved['login_ref']})[0]
        # A fresh status must not overwrite the original, immutable expiry evidence.
        saved.update(current)
        self.write_report()
        return saved

    def audit(self, provider):
        session = self.report['sessions'][provider]
        directory = managed_auth_directory(self.product.storage, session['login_ref'])
        private_directory(directory)
        files = [path for path in directory.iterdir() if path.suffix == '.json']
        require(len(files) == 1, 'managed_credential_count_invalid')
        current = credential_projection(files[0], provider)
        session.setdefault('original_credential', current)
        session['current_credential'] = current
        require(not (self.product.root / 'home/.codex/auth.json').exists(), 'native_codex_auth_present')
        require(not (self.product.root / 'home/.claude/.credentials.json').exists(), 'native_claude_auth_present')
        session['native_auth_files_absent'] = True
        return current

    def record(self, scenario):
        scenario['candidate_sha'] = self.configuration['candidate_sha']
        scenario['caller_harness_sha'] = self.caller_sha
        self.report['scenarios'].append(scenario)
        self.write_report()

    def smoke(self, providers, lifecycle, native_only=False, continue_run=None, reuse_native=False, evidence_manifest=None):
        require(bool(providers) and all(provider in ('codex', 'claude') for provider in providers),
                'required_providers_missing')
        _, _, native, apply_control, configure, judgment, prepare = modules(self.configuration['repository'])
        self.stage = 'observation-baseline'
        baseline = evidence.read_observation(self.product)
        carried, carried_rows, original_baseline = None, [], None
        if continue_run is not None:
            require(lifecycle and not native_only and not reuse_native, 'continuation_mode_invalid')
            carried, carried_rows, original_baseline = runtime_support.continuation(
                self, providers, continue_run, baseline, evidence, evidence_manifest, private_read)
        first = len(self.report['scenarios'])
        run = {'selected_providers': providers, 'native_only': native_only, 'state': 'not_executed',
               'scenario_start': first, 'baseline': baseline, 'candidate_sha': self.configuration['candidate_sha'],
               'caller_harness_sha': self.caller_sha, 'source_snapshots': {}}
        if carried is not None:
            run['continuation'] = carried
        self.report.setdefault('smoke_runs', []).append(run)
        self.active_smoke = run
        self.write_report()
        run_sources = {}
        for provider in providers:
            self.stage = provider + '-fresh-check-save-publish-connect'
            session = self.report['sessions'].get(provider)
            require(session is not None and session['status'] == 'authorized', provider + ':fresh_login_required')
            self.audit(provider)
            if provider not in self.report['sources']:
                source = native.subscription_save(self.product, provider, apply_control, prepare)
                native.publish_plan(self.product, provider, source, judgment, configure)
                self.report['sources'][provider] = source
                self.record({'scenario': provider + '-fresh-check-save-publish-connect',
                             'provider': provider, 'state': 'green', 'model': source['model']})
            source = dict(self.report['sources'][provider])
            source.pop('tool_history', None)
            source.pop('tool_use_id', None)
            run_sources[provider] = source
            run['source_snapshots'][provider] = runtime_support.source_fingerprint(source)
            reused = runtime_support.reused_native_evidence(self, provider, source, evidence_manifest, private_read) if reuse_native else None
            self.write_report()
            for stream in (() if native_only or carried is not None else (False, True)):
                self.stage = provider + '-managed-' + ('stream' if stream else 'nonstream')
                scenario = gateway_roundtrip(self.product, provider, source, stream,
                                              provider + '-managed-' + ('stream' if stream else 'nonstream'))
                self.record(scenario)
            self.stage = provider + '-real-native-client-dialogue'
            if carried is None:
                self.record(reused if reuse_native else native_roundtrip(self.product, provider, source))
            self.audit(provider)
        if lifecycle:
            runtime_support.lifecycle(self, providers, run_sources, native, apply_control, prepare, gateway_roundtrip)
        self.stage = 'observation'
        for row in self.report['scenarios'][first:]:
            row.setdefault('candidate_sha', self.configuration['candidate_sha'])
            row.setdefault('caller_harness_sha', self.caller_sha)
        run['observation'] = evidence.reconcile_observation(self.product, baseline, self.report['scenarios'][first:])
        self.write_report()
        require(run['observation']['state'] == 'green', 'managed_observation_usage_mismatch')
        if carried is not None:
            run['cumulative_observation'] = evidence.observation_delta(original_baseline,
                run['observation']['current'], carried_rows + self.report['scenarios'][first:])
            require(run['cumulative_observation']['state'] == 'green', 'continuation_cumulative_usage_mismatch')
        run['state'], run['scenario_end'] = 'green', len(self.report['scenarios'])
        self.report['state'] = 'green' if all(s['state'] == 'green' for s in self.report['scenarios']) else 'red'
        self.report['paid_inference_requests'] = sum('usage' in item for item in self.report['scenarios'])
        self.write_report()
        return {'state': self.report['state'], 'paid_inference_requests': self.report['paid_inference_requests'],
                'current_run_state': run['state'], 'selected_providers': providers,
                'automatic_original_expiry': 'not_executed', 'report': str(self.run_dir / 'report.json')}

    def action(self, value):
        action = value.get('action')
        if action == 'status':
            readiness = runtime_support.authorization_readiness(self.product)
            if readiness['public_control_ready']:
                for provider in list(self.report['sessions']):
                    self.refresh_session(provider)
            return {'state': self.report['state'], 'candidate_sha': self.configuration['candidate_sha'],
                    'sessions': {kind: session['status'] for kind, session in self.report['sessions'].items()},
                    'daemon_exit_code': self.product.process.poll(),
                    'report': str(self.run_dir / 'report.json'), **readiness}
        if action == 'start':
            provider = value['provider']
            require(provider in ('codex', 'claude'), 'provider_invalid')
            runtime_support.require_authorization_readiness(runtime_support.authorization_readiness(self.product))
            session = self.login({'action': 'start', 'provider': provider})[0]
            url = session.pop('authorization_url')
            require(session['status'] == 'pending', 'login_did_not_start_pending')
            self.report['sessions'][provider] = session
            self.write_report()
            return {'session': session, 'authorization_url': url}
        if action == 'callback':
            provider = value['provider']
            session = self.report['sessions'][provider]
            callback = {'action': 'callback', 'login_ref': session['login_ref'],
                        'input_candidate': session['callback_input_candidate']}
            try:
                self.login(callback)
            except BaseException:
                # The one-use code may already be accepted by CPA. Record its actual
                # public status, preserving the original submission error and no replay.
                try:
                    self.refresh_session(provider)
                except BaseException:
                    pass
                raise
            deadline = time.monotonic() + 90
            while True:
                current = self.refresh_session(provider)
                require(current['status'] in ('pending', 'authorized'), 'oauth_' + current['status'])
                if current['status'] == 'authorized':
                    self.audit(provider)
                    self.record({'scenario': provider + '-fresh-oauth', 'provider': provider,
                                 'state': 'green', 'native_auth_files_absent': True})
                    return {'state': 'green', 'provider': provider, 'status': 'authorized'}
                require(time.monotonic() < deadline, 'oauth_status_timeout')
                time.sleep(.5)
        if action == 'cancel':
            provider = value['provider']
            session = self.report['sessions'][provider]
            current = self.login({'action': 'cancel', 'login_ref': session['login_ref']})[0]
            self.report['sessions'][provider] = current
            self.write_report()
            return {'state': current['status'], 'provider': provider}
        if action == 'smoke':
            return self.smoke(value['providers'], value.get('lifecycle', False),
                              reuse_native=value.get('reuse_native', False), evidence_manifest=value.get('evidence_manifest'))
        if action == 'native':
            return self.smoke([value['provider']], False, native_only=True)
        if action == 'lifecycle':
            return self.smoke(['codex'], True, continue_run=value['run_index'], evidence_manifest=value.get('evidence_manifest'))
        if action == 'expiry':
            results = {}
            for provider, session in self.report['sessions'].items():
                if session['status'] != 'authorized':
                    continue
                current = self.audit(provider)
                # No client process is ever launched under this HOME except bounded
                # native ingress checks; expiry evidence requires those to have ended.
                native_processes = isolated_native_processes(self.product.env['HOME'])
                results[provider] = refresh_verdict(session['original_credential'], current,
                                                    int(time.time()), native_processes)
                if results[provider]['original_expiry_crossed'] and results[provider]['state'] != 'green':
                    results[provider]['state'] = 'red'
                    results[provider]['code'] = 'original_expiry_without_independent_rotation'
                    self.report['state'] = 'red'
                if results[provider]['state'] == 'green':
                    _, _, native, *_ = modules(self.configuration['repository'])
                    require(provider in self.report['sources'], 'expiry_saved_source_required')
                    inference = gateway_roundtrip(self.product, provider, self.report['sources'][provider], False,
                                                  provider + '-original-expiry-crossing')
                    self.record(inference | results[provider])
            self.report['automatic_original_expiry'] = results
            self.write_report()
            return results
        if action == 'forget':
            provider = value['provider']
            session = self.report['sessions'][provider]
            before = self.audit(provider)
            directory = managed_auth_directory(self.product.storage, session['login_ref'])
            forgotten = self.login({'action': 'forget', 'login_ref': session['login_ref']})[0]
            require(forgotten['status'] == 'forgotten', 'login_forget_failed')
            prove_forget_removed(directory, before)
            if provider in self.report['sources']:
                _, _, native, *_ = modules(self.configuration['repository'])
                self.record(native.gateway(self.product, provider, self.report['sources'][provider], False,
                                           provider + '-forgotten-rejection', allowed=False))
            self.report['sessions'][provider] = forgotten
            self.record({'scenario': provider + '-forget-withdraw-delete', 'provider': provider,
                         'state': 'green', 'credential_removed': True})
            return {'state': 'green', 'provider': provider, 'status': 'forgotten'}
        if action == 'stop':
            self.product.stop()
            self.product.listener_port_lease.close()
            self.report['stopped_at_utc'] = datetime.now(timezone.utc).isoformat()
            self.write_report()
            return {'state': 'stopped', 'credentials_preserved': True}
        if action == 'retarget':
            changed = validate_candidate(value['repository'], value['candidate_sha'],
                                         value['product_bin'], self.configuration['cpa_binary'])
            require(changed['cpa'] == self.configuration['cpa'], 'retarget_cpa_pin_changed')
            before = {provider: self.audit(provider) for provider, session in self.report['sessions'].items()
                      if session['status'] == 'authorized'}
            old_sha = self.configuration['candidate_sha']
            self.configuration['previous_product_bin'] = str(self.product.bin)
            self.product.stop()
            self.product.listener_port_lease.close()
            self.configuration.update(changed)
            private_write(self.run_dir / 'configuration.json', self.configuration)
            self.product = make_product(self.configuration, fresh=False)
            self.product.start()
            self.verify_debug()
            rebound = runtime_support.rebind_managed_claude(self.product, self.report['sources'], private_read,
                previous_cli=Path(self.configuration['previous_product_bin']) / 'hiroute')
            if rebound is not None:
                self.record(rebound)
            for provider, credential in before.items():
                require(self.audit(provider)['file_sha256'] == credential['file_sha256'],
                        'retarget_changed_authorization')
            self.report['candidate_sha'] = self.configuration['candidate_sha']
            self.report['binaries'] = self.configuration['binaries']
            self.report['state'] = 'not_executed'
            self.record({'scenario': 'retarget_preserves_original_authorization', 'state': 'green',
                         'previous_candidate_sha': old_sha, 'original_expiry_preserved': True})
            return self.action({'action': 'status'})
        raise LiveFailure('action_invalid')


def isolated_native_processes(home):
    found = 0
    for directory in Path('/proc').iterdir():
        if not directory.name.isdecimal():
            continue
        try:
            environment = (directory / 'environ').read_bytes().split(b'\0')
            command = (directory / 'cmdline').read_bytes().split(b'\0')
            if b'HOME=' + str(home).encode() not in environment or not command:
                continue
            name = Path(os.fsdecode(command[0])).name
            if name in ('codex', 'claude'):
                found += 1
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    return found


def serve(run_dir):
    supervisor = None
    try:
        supervisor = Supervisor(run_dir)
        socket_path = Path(run_dir) / 'supervisor.sock'
        with socket.socket(socket.AF_UNIX) as listener:
            listener.bind(str(socket_path))
            socket_path.chmod(0o600)
            listener.listen(2)
            while True:
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(180)
                    stream = connection.makefile('rb')
                    action = None
                    supervisor.stage, supervisor.active_smoke = 'request', None
                    failure_start = len(getattr(supervisor.product, 'gateway_failures', []))
                    supervisor.product.native_client_evidence = None
                    try:
                        raw = stream.readline(8193)
                        require(len(raw) <= 8192, 'supervisor_request_too_large')
                        value = json.loads(raw)
                        require(not any(key in value for key in ('secret', 'code', 'callback', 'callback_url', 'refresh_token')), 'raw_secret_request_forbidden')
                        action = value.get('action')
                        supervisor.stage = evidence.stage(action)
                        result = supervisor.action(value)
                        response = {'ok': True, 'result': result}
                    except BaseException as error:
                        code = safe_code(error)
                        response = {'ok': False, 'code': code}
                        failure = {'scenario': 'live_' + evidence.stage(action), 'state': 'red', 'code': code}
                        failure.update(evidence.failure_projection(supervisor.product, supervisor.stage,
                                                                  failure_start))
                        supervisor.record(failure)
                        if supervisor.active_smoke is not None:
                            supervisor.active_smoke.update(state='red', failure=failure,
                                                          scenario_end=len(supervisor.report['scenarios']))
                            for row in supervisor.report['scenarios'][supervisor.active_smoke['scenario_start']:]:
                                row.setdefault('candidate_sha', supervisor.configuration['candidate_sha'])
                                row.setdefault('caller_harness_sha', supervisor.caller_sha)
                        supervisor.report['state'] = 'red'
                        supervisor.write_report()
                    connection.sendall(encoded(response) + b'\n')
                    if action == 'stop' and response['ok']:
                        break
        socket_path.unlink()
    except BaseException as error:
        private_write(Path(run_dir) / 'startup-error.json', {'state': 'red', 'code': safe_code(error)})
        if supervisor is not None:
            supervisor.product.stop(diagnostic_failure=True)
        return 1
    return 0


def prepare(arguments):
    run_dir = Path(arguments.run_dir).absolute()
    require(len(str(run_dir / 'product/runtime/hiroute/control.sock').encode()) < 100,
            'runtime_path_too_long')
    configuration = validate_candidate(arguments.repository, arguments.candidate_sha,
                                        arguments.product_bin, arguments.cpa_binary)
    configuration['product_root'] = str(run_dir / 'product')
    for provider in ('codex', 'claude'):
        binary = Path(getattr(arguments, provider + '_cli')).resolve()
        require(binary.is_file() and os.access(binary, os.X_OK), provider + ':real_executable_required')
        configuration[provider + '_cli'] = str(binary)
    private_directory(run_dir, create=True)
    private_write(run_dir / 'configuration.json', configuration)
    subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_serve', '--run-dir', str(run_dir)],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True, env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
    deadline = time.monotonic() + 100
    while True:
        if (run_dir / 'startup-error.json').exists():
            raise LiveFailure(private_read(run_dir / 'startup-error.json')['code'])
        if (run_dir / 'supervisor.sock').exists():
            return runtime_support.require_authorization_readiness(request(run_dir, {'action': 'status'})) | {
                'interactive_entry_ready': True, 'oauth_started': False}
        require(time.monotonic() < deadline, 'supervisor_startup_timeout')
        time.sleep(.2)


def resume(arguments):
    run_dir = private_directory(arguments.run_dir)
    require(not (run_dir / 'supervisor.sock').exists(), 'supervisor_already_running')
    configuration = private_read(run_dir / 'configuration.json')
    validate_candidate(configuration['repository'], configuration['candidate_sha'],
                       configuration['product_bin'], configuration['cpa_binary'])
    subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_serve', '--run-dir', str(run_dir)],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True, env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
    deadline = time.monotonic() + 100
    while True:
        if (run_dir / 'supervisor.sock').exists():
            return runtime_support.require_authorization_readiness(request(run_dir, {'action': 'status'}))
        require(time.monotonic() < deadline, 'supervisor_resume_timeout')
        time.sleep(.2)


def authorize(arguments):
    tty = os.open('/dev/tty', os.O_RDWR | os.O_NOCTTY)
    try:
        require(os.isatty(tty), 'controlling_terminal_required')
        configuration = private_read(Path(arguments.run_dir) / 'configuration.json')
        runtime_support.require_authorization_readiness(request(arguments.run_dir, {'action': 'status'}))
        os.write(tty, b'All preparation is ready. OAuth has a five-minute exchange window.\n'
                      b'The callback stays in this private terminal; never paste it into chat.\n')
        started = request(arguments.run_dir, {'action': 'start', 'provider': arguments.provider})
        url = started.pop('authorization_url')
        require(url.startswith('https://') and '\n' not in url and '\r' not in url, 'oauth_url_invalid')
        if not arguments.no_browser:
            try:
                webbrowser.open(url, new=2)
            except Exception:
                pass
        # This write bypasses stdout and tool logs, even on a headless host.
        os.write(tty, b'Open this URL in your browser if it did not open automatically:\n' + url.encode() + b'\n')
        secret = hidden_tty_input(tty)
        register_callback(configuration, started['session']['callback_input_candidate'], secret)
        del secret, url
        result = request(arguments.run_dir, {'action': 'callback', 'provider': arguments.provider})
        os.write(tty, b'Authorization completed. The new credential belongs to the isolated CPA runtime.\n')
        return result
    finally:
        os.close(tty)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    prepared = commands.add_parser('prepare')
    prepared.add_argument('--run-dir', required=True, type=Path)
    prepared.add_argument('--repository', required=True, type=Path)
    prepared.add_argument('--candidate-sha', required=True)
    prepared.add_argument('--product-bin', required=True, type=Path)
    prepared.add_argument('--cpa-binary', required=True, type=Path)
    prepared.add_argument('--codex-cli', required=True, type=Path)
    prepared.add_argument('--claude-cli', required=True, type=Path)
    borrowed = commands.add_parser('native-borrowed')
    borrowed.add_argument('--repository', required=True, type=Path)
    borrowed.add_argument('--candidate-sha', required=True)
    borrowed.add_argument('--product-bin', required=True, type=Path)
    borrowed.add_argument('--cpa-binary', required=True, type=Path)
    borrowed.add_argument('--codex-cli', required=True, type=Path)
    borrowed.add_argument('--claude-cli', required=True, type=Path)
    borrowed.add_argument('--codex-host', required=True)
    borrowed.add_argument('--claude-host', required=True)
    borrowed.add_argument('--provider', choices=('codex', 'claude'))
    borrowed.add_argument('--report', required=True, type=Path)
    authorized = commands.add_parser('authorize')
    authorized.add_argument('--run-dir', required=True, type=Path)
    authorized.add_argument('--provider', required=True, choices=('codex', 'claude'))
    authorized.add_argument('--no-browser', action='store_true')
    for command in ('status', 'stop', 'expiry', 'resume', '_serve'):
        commands.add_parser(command).add_argument('--run-dir', required=True, type=Path)
    retarget = commands.add_parser('retarget')
    retarget.add_argument('--run-dir', required=True, type=Path)
    retarget.add_argument('--repository', required=True, type=Path)
    retarget.add_argument('--candidate-sha', required=True)
    retarget.add_argument('--product-bin', required=True, type=Path)
    smoke = commands.add_parser('smoke')
    smoke.add_argument('--run-dir', required=True, type=Path)
    smoke.add_argument('--provider', action='append', dest='providers', choices=('codex', 'claude'))
    smoke.add_argument('--lifecycle', action='store_true')
    smoke.add_argument('--reuse-native', action='store_true')
    smoke.add_argument('--evidence-manifest', type=Path)
    continued = commands.add_parser('lifecycle')
    continued.add_argument('--run-dir', required=True, type=Path)
    continued.add_argument('--run-index', required=True, type=int)
    continued.add_argument('--evidence-manifest', type=Path)
    for command in ('cancel', 'forget', 'native'):
        selected = commands.add_parser(command)
        selected.add_argument('--run-dir', required=True, type=Path)
        selected.add_argument('--provider', required=True, choices=('codex', 'claude'))
    arguments = parser.parse_args()
    try:
        if arguments.command == '_serve':
            return serve(arguments.run_dir)
        if arguments.command == 'prepare':
            result = prepare(arguments)
        elif arguments.command == 'native-borrowed':
            result = native_borrowed(arguments)
        elif arguments.command == 'authorize':
            result = authorize(arguments)
        elif arguments.command == 'resume':
            result = resume(arguments)
        else:
            value = {'action': arguments.command}
            if arguments.command == 'smoke':
                value.update(providers=arguments.providers or ['claude', 'codex'], lifecycle=arguments.lifecycle,
                             reuse_native=arguments.reuse_native)
            if arguments.command == 'lifecycle':
                value['run_index'] = arguments.run_index
            if arguments.command in ('smoke', 'lifecycle') and arguments.evidence_manifest is not None:
                value['evidence_manifest'] = str(arguments.evidence_manifest)
            if arguments.command in ('cancel', 'forget', 'native'):
                value['provider'] = arguments.provider
            if arguments.command == 'retarget':
                value.update(repository=str(arguments.repository), candidate_sha=arguments.candidate_sha,
                             product_bin=str(arguments.product_bin))
            result = request(arguments.run_dir, value, timeout=600)
        print(json.dumps(result, sort_keys=True), flush=True)
        return 1 if result.get('state') == 'red' else 0
    except BaseException as error:
        print(json.dumps({'state': 'red', 'code': safe_code(error)}), flush=True)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
