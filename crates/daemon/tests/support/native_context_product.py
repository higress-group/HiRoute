"""Real installed Worker/native context journey with a deterministic model upstream.

Requires the same exact-candidate binaries and selected native installations as
delegation_product.py. No business SQL, private execution port, or replacement ACP.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from delegation_product import configure_worker_installation
from delegation_product import wait_for_worker_result, worker_cli
from native_context_fixture import (BUSINESS_CASES, CORE_CASES, CONTINUE_PROMPT, NativeContextUpstream,
                                    assert_preserved, install_proxy_conflict, prepare, protect_configuration)
from model_connections_product import save_native_source
from publication_product import Product
from publication_process import plan_change
import qoder_native_context


def save_context_source(product, upstream, variant='', context_tokens=None):
    """Declare a source budget supported by the selected native client contract."""
    protocol = product.worker_work['protocol']
    return save_native_source(product, upstream, token=upstream.token, protocol=protocol,
                              upstream_model_id=upstream.model, variant=variant,
                              context_tokens=context_tokens if context_tokens is not None else
                              native_context_budget(product))


def native_context_budget(product):
    if product.worker_work.get('harness') == 'qoder_cli':
        # Ordinary collaboration/cancellation use a roomy budget. The core
        # journey separately supplies 32768/4096 to test native budget projection.
        return 100_000
    return 200_000 if product.worker_work['protocol'] == 'messages' else 32_768


def selected_installation(harness):
    binary = Path(os.environ['HIROUTE_WORKER_' + harness.upper() + '_BINARY']).resolve(strict=True)
    if harness == 'qoder':
        return binary, None, None
    adapter = Path(os.environ['HIROUTE_WORKER_' + harness.upper() + '_ACP_ADAPTER']).resolve(strict=True)
    node = Path(os.environ['HIROUTE_WORKER_NODE']).resolve(strict=True)
    return binary, adapter, node


def prepare_product(product, harness, context_tokens=None):
    """Prepare a normal API source/Plan; Desktop can take over after product.stop()."""
    product.enable_debug_diagnostics()
    roots = {'codex': 'CODEX_HOME', 'claude': 'CLAUDE_CONFIG_DIR', 'qoder': 'QODER_CONFIG_DIR'}
    if harness == 'qoder':
        qoder_native_context.select_context(product)
        fixture = qoder_native_context.prepare_context(product)
    else:
        fixture = prepare(Path(product.env['HOME']), Path(product.env[roots[harness]]),
                          product.project, harness)
    native = Path(product.env[roots[harness]])
    product.worker_work = {'harness': {'codex': 'codex_cli', 'claude': 'claude_code',
                                       'qoder': 'qoder_cli'}[harness],
                           'protocol': 'messages' if harness == 'claude' else 'responses'}
    fixture['source_context_tokens'] = (context_tokens if context_tokens is not None
                                        else native_context_budget(product))
    upstream = NativeContextUpstream(product.root)
    try:
        product.start()
        saved = save_context_source(product, upstream, context_tokens=context_tokens)
        product.editor = {
            'schema': 'hiroute.plan-editor/v2', 'display_name': 'Native Worker context',
            'purpose': 'Read native user and project skills', 'mode': 'fixed_model',
            'candidates': [{'binding_id': saved['binding_id']}],
            'smart': {'economy': [], 'primary': [], 'primary_fallback': False,
                      'reselect_on_user_message': False, 'classifier': {'kind': 'local_rules'},
                      'complex_keywords': []},
            'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
            'delegation_enabled': True, 'work': product.worker_work, 'requirements': {},
            'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000, 'attempt_timeout_ms': 30000},
        }
        change = plan_change(product, 'create', 'native-context-plan')
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'native-worker-plan')
        product.plan_id = preview['plan_head']['reference']['plan_id']
        product.model_alias = preview['plan_head']['model_alias']
        configuration = native / ('config.toml' if harness == 'codex' else 'settings.json')
        if harness == 'codex':
            configuration.write_text('model = "gpt-5.5"\nmodel_provider = "native_context_ambient"\n'
                '[model_providers.native_context_ambient]\nname = "Unreachable synthetic source"\n'
                'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
                'env_key = "NATIVE_CONTEXT_UNUSED_TOKEN"\nrequires_openai_auth = false\n')
        elif harness == 'qoder':
            qoder_native_context.install_project_conflict(fixture, upstream)
        elif harness == 'claude':
            configuration.write_text(json.dumps({'env': {'ANTHROPIC_BASE_URL': 'http://127.0.0.1:9',
                'ANTHROPIC_AUTH_TOKEN': 'synthetic-ambient-must-not-authorize-worker',
                'ANTHROPIC_MODEL': 'synthetic-wrong-model',
                **install_proxy_conflict(fixture, upstream)}}))
        if harness != 'qoder':
            configuration.chmod(0o600)
            protect_configuration(fixture, [configuration])
        fixture['plan_id'] = product.plan_id
        fixture['model_alias'] = product.model_alias
        (product.root / 'native-context.json').write_text(json.dumps(fixture))
        return fixture, upstream
    except Exception:
        upstream.close()
        raise


def publish_replacement_route(product, fixture):
    """Publish a distinguishable route; existing tasks must retain their original route."""
    controls = product.root / 'replacement-native-context-source'
    controls.mkdir(mode=0o700)
    (controls / 'native-context.json').write_text(json.dumps(fixture))
    upstream = NativeContextUpstream(controls)
    upstream.model = 'gpt-5.5'
    upstream.token = 'synthetic-replacement-native-context-source-token'
    try:
        saved = save_context_source(product, upstream, variant='native-context-replacement')
        change = plan_change(product, 'update', display_name='Published replacement Worker route',
                             candidates=[{'binding_id': saved['binding_id']}],
                             delegation_enabled=True, work=product.worker_work)
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                      {'change': change}, 'native-plan-update')
        return upstream
    except Exception:
        upstream.close()
        raise


def assert_frozen_route(original_events, before, replacement):
    """Require actual requests to the old route and no requests to the new route."""
    requests = original_events[before:]
    assert requests and all(event['state'] == 'green' for event in requests), \
        'Continue did not use the original source'
    assert any(event.get('continued') is True for event in requests), \
        'original source did not receive the resumed native history'
    replacement_log = replacement.controls / 'native-context-events.jsonl'
    assert not replacement_log.exists() or not replacement_log.read_text().strip(), \
        'existing task used the newly published route'


def exact_history(fixture):
    """Inspect only this synthetic context's native transcript directory, never auth."""
    if fixture['harness'] == 'qoder':
        return qoder_native_context.exact_history(fixture)
    folder = Path(fixture['config']) / ('sessions' if fixture['harness'] == 'codex' else 'projects')
    matches = []
    for path in folder.rglob('*.jsonl'):
        assert not path.is_symlink(), 'unexpected linked fixture history'
        text = path.read_text()
        if fixture['receipt'] not in text:
            continue
        identities = set()
        for line in text.splitlines():
            row = json.loads(line)
            identity = (row.get('payload', {}).get('id') if row.get('type') == 'session_meta'
                        else row.get('sessionId') if fixture['harness'] == 'claude' else None)
            if identity:
                identities.add(identity)
        assert len(identities) == 1, 'native transcript has ambiguous session identity'
        matches.append((str(path), identities.pop()))
    assert len(matches) == 1, 'expected exactly the task native transcript in borrowed context'
    return matches[0]


def events(product):
    return [json.loads(line) for line in
            (product.root / 'native-context-events.jsonl').read_text().splitlines()]


def task_plan_revision(product, task_id):
    _, response = worker_cli(product, 'worker list --output json')
    task = next(task for task in response['data']['tasks'] if task['task_id'] == task_id)
    return task['plan_revision']


def wait_for_resumable_task(product, task_id, run_id, timeout=10):
    """Read public finalization readiness before inspecting native disk evidence."""
    deadline = time.monotonic() + timeout
    while True:
        _, response = worker_cli(product, 'worker list --output json')
        task = next((item for item in response['data']['tasks'] if item['task_id'] == task_id), None)
        if task:
            assert task['latest_run_id'] == run_id, 'another run replaced the expected latest run'
            if (task['run']['state'] == 'succeeded' and task['run']['cleanup'] == 'complete'
                    and (task['resumable_until_ms'] or 0) > int(time.time() * 1000)):
                return task
        assert time.monotonic() < deadline, ('native continuation did not become ready', task)
        time.sleep(.05)


def report_failure(product, report, stage):
    """Preserve this journey's current daemon diagnostics before fixture cleanup."""
    try:
        product.stop()
        stopped = 'stopped'
    except Exception:
        stopped = 'failed'
    evidence = product.repo / 'target/product-e2e-evidence' / (
        report['scenario'] + '-' + report['worker_harness'] + '-' + str(os.getpid()))
    report.update(state='red', stage=stage, daemon_stop_state=stopped)
    try:
        report['diagnostics'] = product.preserve_diagnostics(evidence)
    except OSError:
        # Diagnostics are secondary evidence; never replace the original failure or
        # emit raw paths/exception content that could contain sensitive material.
        report['diagnostics'] = {'state': 'unavailable', 'reason': 'diagnostics_io'}
    print(json.dumps(report), flush=True)


def run(repository, candidate):
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    harness = os.environ['HIROUTE_PRODUCT_WORKER_HARNESS']
    assert harness in ('codex', 'claude', 'qoder')
    product = Product(repo)
    # Headless services may explicitly select a non-default native configuration.
    # Mac Pilot intentionally uses its documented HOME/.codex and HOME/.claude roots.
    if harness != 'qoder':
        product.env['CODEX_HOME' if harness == 'codex' else 'CLAUDE_CONFIG_DIR'] = str(
            Path(product.env['HOME']) / ('selected-' + harness + '-config'))
    upstream = replacement = None
    stage = 'prepare-native-context'
    report = dict(scenario='worker-native-context-core', candidate=candidate, worker_harness=harness,
                  required_cases=list(CORE_CASES), selected_cases=list(CORE_CASES), cases=[],
                  evidence_limit='Real native Harness and daemon; controlled upstream; no Desktop verdict',
                  related_contracts=[{'id': case, 'state': 'not_executed'}
                                     for case in BUSINESS_CASES if case not in CORE_CASES])
    try:
        binary, adapter, node = selected_installation(harness)
        stage = 'real-daemon-and-plan'
        fixture, upstream = prepare_product(product, harness,
                                             context_tokens=32_768 if harness == 'qoder' else None)
        report['source_context_tokens'] = fixture['source_context_tokens']
        if harness == 'qoder':
            fixture['expected_max_output_tokens'] = 4096
            report['source_output_tokens'] = 4096
            (product.root / 'native-context.json').write_text(json.dumps(fixture))
        configure_worker_installation(product, product.worker_work['harness'], adapter, binary, node)
        if harness == 'qoder':
            stage = 'unsupported-qoder-permissions'
            report['rejected_permission_modes'] = qoder_native_context.assert_restricted_modes_unavailable(product, upstream)
        stage = 'native-skills'
        prompt = 'Use ' + ' and '.join(skill['name'] for skill in fixture['skills']) + ' to produce their read-only receipts.'
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project)
                   + ' --run-timeout 120 --no-wait --submission-key native-context-start --file - --output json')
        _, accepted = worker_cli(product, command, prompt)
        first = accepted['data']
        fixture['task_id'] = first['task_id']
        result = wait_for_worker_result(product, first['run_id'], timeout=150)
        assert fixture['receipt'] in result['result'], 'missing native skill execution receipt'
        history = None
        report['task_id'] = first['task_id']
        report['run_ids'] = [first['run_id']]
        # Submit the first Continue immediately after result: no readiness probe,
        # native file wait or retry may hide the product's finalization contract.
        for restart in (False, True):
            stage = 'continue-after-restart' if restart else 'immediate-continue'
            if restart:
                replacement = publish_replacement_route(product, fixture)
                product.stop()
                if harness == 'qoder':
                    qoder_native_context.replace_current_context(product)
                product.start()
                before_requests = len(events(product))
            command = ('worker continue --task ' + first['task_id'] + ' --expected-latest-run '
                       + report['run_ids'][-1] + ' --run-timeout 120 --no-wait --submission-key '
                       + ('native-context-reopen' if restart else 'native-context-continue')
                       + ' --file - --output json')
            _, accepted = worker_cli(product, command, CONTINUE_PROMPT)
            continued = accepted['data']
            assert continued['task_id'] == first['task_id']
            assert continued['run_id'] not in report['run_ids'] and not continued['replayed']
            result = wait_for_worker_result(product, continued['run_id'], timeout=150)
            assert 'continued-' + fixture['receipt'] in result['result']
            ready = wait_for_resumable_task(product, first['task_id'], continued['run_id'])
            current_history = exact_history(fixture)
            if restart:
                assert current_history == history, 'Continue changed native session or transcript'
                assert ready['plan_revision'] == report['frozen_plan_revision']
                assert_frozen_route(events(product), before_requests, replacement)
                report['continued_after_route_replacement'] = True
            else:
                # The oracle required the first run's real tool history. Both turns
                # must now occupy one native transcript; a replacement is ambiguous.
                history = current_history
                report['native_session_id'] = history[1]
                report['frozen_plan_revision'] = ready['plan_revision']
                report['cases'].append({'id': CORE_CASES[0], 'state': 'green'})
            assert_preserved(fixture)
            report['run_ids'].append(continued['run_id'])
            before = len(events(product))
            _, replay = worker_cli(product, command, CONTINUE_PROMPT)
            assert replay['data']['replayed'] and replay['data']['run_id'] == continued['run_id']
            assert len(events(product)) == before, 'Continue replay sent another model request'
        report['cases'].append({'id': CORE_CASES[1], 'state': 'green'})
        attempts = events(product)
        if fixture.get('proxy_trap_events'):
            report['native_proxy_requests'] = len(Path(fixture['proxy_trap_events']).read_text().splitlines())
            assert report['native_proxy_requests'] == 0, 'native Worker used the user-configured proxy'
        assert attempts and all(event['state'] == 'green' for event in attempts)
        assert sum(event.get('continued') is True for event in attempts) == 2
        if harness == 'qoder':
            qoder_native_context.assert_settings_preserved(fixture)
            report['foreign_project_requests'] = len(Path(fixture['foreign_route_events']).read_text().splitlines())
        if harness != 'qoder':
            report['conflicting_settings_unchanged'] = True
        report.update(state='green', native_settings_unchanged=True,
                      neighboring_material_preserved=True, upstream_requests=len(attempts),
                      binaries={name: hashlib.sha256((product.bin / name).read_bytes()).hexdigest()
                                for name in ('hiroute', 'hirouted')},
                      harness_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
        product.stop()
        diagnostics = product.diagnostics_snapshot()
        assert diagnostics['state'] == 'complete', diagnostics
        assert diagnostics['level_applied']['level'] == 'debug', diagnostics
        print(json.dumps(report), flush=True)
    except Exception:
        report_failure(product, report, stage)
        raise
    finally:
        try:
            product.close()
        finally:
            try:
                qoder_native_context.cleanup_context(product)
            finally:
                if upstream:
                    upstream.close()
                if replacement:
                    replacement.close()


def prepare_desktop(repository, candidate, root, harness):
    """Keep only the independent model server alive for a real Pilot takeover."""
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo, root=root)
    upstream = None
    try:
        fixture, upstream = prepare_product(product, harness)
        product.stop()
        print(json.dumps({'state': 'prepared', 'candidate': candidate, 'harness': harness,
                          'data_root': str(product.root), 'process_home': fixture['home'],
                          'workspace': fixture['project'], 'plan_id': product.plan_id,
                          'base_url': upstream.base_url, 'fixture': str(product.root / 'native-context.json'),
                          'proxy_trap_events': fixture.get('proxy_trap_events'),
                          'configuration_saved_through_ui': False}), flush=True)
        # The caller owns this helper process. Stop it only after the Desktop journey ends.
        def stop_helper(*_):
            raise KeyboardInterrupt()
        signal.signal(signal.SIGTERM, stop_helper)
        signal.pause()
    except KeyboardInterrupt:
        pass
    finally:
        try:
            product.close()
        finally:
            try:
                qoder_native_context.cleanup_context(product)
            finally:
                if upstream:
                    upstream.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository')
    parser.add_argument('candidate')
    parser.add_argument('--desktop-root', type=Path)
    parser.add_argument('--harness', choices=('codex', 'claude', 'qoder'))
    args = parser.parse_args()
    if args.desktop_root:
        if not args.harness:
            parser.error('--harness is required with --desktop-root')
        prepare_desktop(args.repository, args.candidate, args.desktop_root, args.harness)
    else:
        run(args.repository, args.candidate)
