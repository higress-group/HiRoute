#!/usr/bin/env python3
"""Opt-in real Codex acceptance using the production profile-creation entry.

Uses an explicitly supplied subscription through real HiRoute/CPA and a real Codex
CLI with no client login. All created configuration and services are disposable.
Requires consent to use the supplied subscription. Never prints credentials or
raw provider/client errors. A failure exits nonzero and leaves a bounded report.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import subprocess
import sys
import urllib.parse

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / 'crates/daemon/tests/support'))
from publication_product import Product  # noqa: E402
from publication_process import prepare_subscription_source, apply_control  # noqa: E402
from cpa_luna_gateway_live import (  # noqa: E402
    configure_isolated_codex, publish_luna_plan,
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def codex_completed(stdout, returncode):
    events = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return (returncode == 0
            and any(event.get('type') == 'turn.completed' for event in events)
            and not any(event.get('type') in ('turn.failed', 'error') for event in events)
            and any(event.get('type') == 'item.completed'
                    and event.get('item', {}).get('type') == 'agent_message'
                    and 'PROFILE_OK' in event['item'].get('text', '') for event in events))


def safe_cpa_summary(product):
    """Read only our disposable CPA; emit no account IDs, tokens or response text."""
    summaries = []
    for config in product.root.rglob('config.yaml'):
        capability = config.parent / 'capability.private'
        if not capability.is_file():
            continue
        # The rendered managed config has a numeric top-level port. The config's
        # secret-key may already be hashed by CPA; use its private instance secret.
        port = next(int(line.split(':', 1)[1]) for line in config.read_text().splitlines()
                    if line.startswith('port:'))
        secret = capability.read_text().splitlines()[1]
        client = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
        def get(path):
            client.request('GET', path, headers={'Authorization': 'Bearer ' + secret})
            response = client.getresponse()
            return response.status, json.loads(response.read())
        try:
            status, listing = get('/v0/management/auth-files')
            accounts = []
            for account in listing.get('files', []):
                model_status, models = get('/v0/management/auth-files/models?name='
                                          + urllib.parse.quote(account['id'], safe=''))
                accounts.append({
                    'active': account.get('status') == 'active',
                    'disabled': account.get('disabled'),
                    'unavailable': account.get('unavailable'),
                    'retry_zero': account.get('request_retry') == 0,
                    'models_http': model_status,
                    'model_count': len(models.get('models', [])),
                })
            summaries.append({'management_http': status, 'accounts': accounts})
        finally:
            client.close()
    return summaries


def save_source(product, model):
    candidate, validation = prepare_subscription_source(product)
    selected = next((item for item in candidate['models']
                     if item.get('upstream_model_id') == model and item.get('selectable')), None)
    if selected is None:
        raise ValueError('requested_model_not_selectable')
    snapshot = product.control('ListCompute', {})['data']
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [selected['model_ref']],
        'intent': 'save_ready', 'key_edits': [], 'validation': validation,
    }
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, 'profile-preflight-source')
    saved = product.control('GetComputeSaveResult', {'operation': applied['operation']})['data']
    if saved['disposition'] != 'saved' or saved['management_state'] != 'ready':
        raise ValueError('source_not_ready')
    if len(saved['bindings']) != 1:
        raise ValueError('source_binding_ambiguous')
    return saved['bindings'][0]['binding_id']


def run(args):
    source_digest = digest(args.auth_source)
    product = Product(REPO)
    product.bin = args.bin_dir.resolve()
    report = {
        'schema': 'hiroute.codex-profile-preflight/v1',
        'scenario': 'not_executed', 'phase': 'prepare',
        'binary_sha256': digest(product.bin / 'hirouted'),
        'codex_version': subprocess.check_output(
            [str(args.codex_cli), '--version'], text=True, timeout=10).strip(),
        'cpa_sha256': digest(args.cpa_binary),
        'checks': {}, 'limitations': [
            'Real creation and no-login model request; edit/restore covered by daemon tests.',
            'Desktop and cross-provider history are not verified.',
        ],
    }
    try:
        configure_isolated_codex(product, args.auth_source, args.codex_cli)
        client_home = product.root / "client codex ' $home"
        original_home = product.codex_settings.parent
        original_home.rename(client_home)
        product.codex_settings = client_home / 'config.toml'
        product.env['CODEX_HOME'] = str(client_home)
        product.codex_access_mode = 'profile'
        product.enable_debug_diagnostics()
        product.cpa_args = ['--cpa-binary', str(args.cpa_binary),
                            '--cpa-sha256', report['cpa_sha256']]
        product.startup_timeout = 90
        product.start()
        report['phase'] = 'subscription_check'
        binding = save_source(product, args.model)
        report['checks']['upstream_source_ready'] = True
        # CPA owns its authorized upstream copy. Remove only the isolated client copy;
        # the caller's source login is read-only and checked again in finally.
        (client_home / 'auth.json').unlink()
        root = product.codex_settings
        baseline = root.read_bytes()
        report['phase'] = 'publication'
        product.native_model_checked_agents = {'agent_codex_default'}
        publish_luna_plan(product, binding)
        profile = client_home / 'hiroute.config.toml'
        if 'requires_openai_auth = false' not in profile.read_text():
            raise ValueError('profile_writer_shape_changed')
        env = dict(product.env, CODEX_HOME=str(client_home))
        # Only the source side has copied subscription credentials. Both ordinary
        # and profile CLI entrypoints resolve this same never-logged-in client home.
        if (client_home / 'auth.json').exists():
            raise ValueError('unexpected_client_auth')
        report['phase'] = 'real_codex_request'
        completed = subprocess.run([
            str(args.codex_cli), '--profile', 'hiroute', 'exec', '--skip-git-repo-check',
            '--ephemeral', '--json', '--sandbox', 'read-only',
            'Reply with PROFILE_OK only. Do not use tools.',
        ], env=env, cwd=product.project, stdin=subprocess.DEVNULL,
            capture_output=True, timeout=150)
        # Do not preserve raw stdout/stderr: provider diagnostics can contain secrets.
        report['client_exit'] = completed.returncode
        report['checks'].update(
            client_completed=codex_completed(completed.stdout, completed.returncode),
            root_unchanged=root.read_bytes() == baseline,
            client_auth_absent=not (client_home / 'auth.json').exists())
        report['scenario'] = 'green' if all(report['checks'].values()) else 'red'
        report['phase'] = 'complete'
    except Exception as error:
        report.update(scenario='red', error_type=type(error).__name__)
        try:
            report['cpa'] = safe_cpa_summary(product)
        except Exception as diagnostic_error:
            report['diagnostic_error_type'] = type(diagnostic_error).__name__
    finally:
        try:
            report['diagnostics'] = product.preserve_diagnostics(args.output / 'diagnostics')
        except Exception as error:
            report['diagnostic_error_type'] = type(error).__name__
        try:
            product.close()
        except Exception as error:
            report.update(scenario='red', cleanup_error_type=type(error).__name__)
        report['checks']['original_auth_unchanged'] = digest(args.auth_source) == source_digest
        if not report['checks']['original_auth_unchanged']:
            report['scenario'] = 'red'
        (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report['scenario'] == 'green' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--codex-cli', type=Path, required=True)
    parser.add_argument('--cpa-binary', type=Path, required=True)
    parser.add_argument('--auth-source', type=Path, required=True)
    parser.add_argument('--model', default='gpt-5.6-luna')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    for key in ('bin_dir', 'codex_cli', 'cpa_binary', 'auth_source', 'output'):
        setattr(args, key, getattr(args, key).resolve())
    # Refuse to overwrite prior evidence, especially a prior failed gate.
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    return run(args)


if __name__ == '__main__':
    sys.exit(main())
