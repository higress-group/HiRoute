"""Focused real shared-context concurrency/cancellation and missing-history cases.

Reuse the core native context transport and product setup. No fake ACP, database
edits, deletion API, or private execution port. This entry never builds binaries.
"""
import qoder_native_context
import pi_native_context
import argparse
from concurrent.futures import ThreadPoolExecutor
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time

from delegation_product import configure_worker_installation, wait_for_worker_result, worker_cli
from native_context_fixture import (NativeContextUpstream, assert_preserved, decision, digest,
                                    tool_results, write_new)
from native_context_product import selected_installation
from native_context_product import (exact_history, prepare_product, report_failure, save_context_source,
                                    wait_for_resumable_task)
from publication_process import plan_change
from publication_product import Product


CASES = ('worker.context.concurrent-routing', 'worker.context.cancel-owned-work',
         'worker.context.exact-continue')


def wait_until(check, reason, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        result = check()
        if result:
            return result
        assert time.monotonic() < deadline, reason
        time.sleep(.1)


def prepare_heartbeat(fixture, role):
    fixture = copy.deepcopy(fixture)
    root = Path(fixture['project']) / ('native-context-' + role)
    root.mkdir(mode=0o700)
    receipt = role + '-' + fixture['receipt']
    script = root / 'heartbeat.sh'
    paths = {name: str(root / name) for name in ('pid', 'heartbeat', 'release', 'done')}
    write_new(script, '\n'.join((
        '#!/bin/sh', 'set -eu', 'trap "exit 0" TERM INT',
        'printf "%s\\n" "$$" > ' + shlex.quote(paths['pid']), 'iterations=0',
        'while [ ! -f ' + shlex.quote(paths['release']) + ' ]; do',
        '  printf x >> ' + shlex.quote(paths['heartbeat']),
        '  iterations=$((iterations + 1))',
        '  [ "$iterations" -lt 900 ] || exit 9', '  sleep 0.1', 'done',
        'printf "%s\\n" ' + shlex.quote(receipt) + ' > ' + shlex.quote(paths['done']),
        'printf "%s\\n" ' + shlex.quote(receipt), '',
    )), True)
    fixture['receipt'] = receipt
    fixture['boundary'] = dict(paths, role=role, script=str(script),
                               marker='NATIVE-CONTEXT-WORKER-' + role)
    return fixture


def boundary_decision(fixture, body):
    """Two source oracles reject cross-routing and execute actual native shell children."""
    state = fixture['boundary']
    assert state['marker'] in json.dumps(body), 'task reached the other task source'
    results = tool_results(body, fixture['harness'])
    heartbeat_id = 'native_context_heartbeat_' + state['role']
    if heartbeat_id in results:
        # Codex may return a still-running tool session. The actual script's done
        # receipt, rather than that adapter-specific result spelling, proves completion.
        done = Path(state['done'])
        wait_until(done.exists, 'native tool did not finish after release', timeout=25)
        assert done.read_text().strip() == fixture['receipt'], 'wrong native child receipt'
        return {'kind': 'text', 'text': fixture['receipt'], 'continued': False}
    action = decision(fixture, body)
    if action['kind'] != 'text':
        return action
    names = {tool.get('name') for tool in body.get('tools', [])}
    command = '/bin/sh ' + shlex.quote(state['script'])
    if fixture['harness'] in ('claude', 'qoder'):
        assert 'Bash' in names, 'native Bash tool unavailable'
        name, arguments = 'Bash', {'command': command, 'timeout': 120000}
    elif fixture['harness'] == 'pi':
        assert 'bash' in names
        name, arguments = 'bash', {'command': command, 'timeout': 120}
    elif 'exec_command' in names:
        name, arguments = 'exec_command', {'cmd': command, 'yield_time_ms': 30000,
                                            'max_output_tokens': 1000}
    else:
        assert 'shell_command' in names, 'native shell tool unavailable'
        name, arguments = 'shell_command', {'command': command, 'timeout_ms': 120000}
    return dict(kind='tool', id=heartbeat_id, name=name, arguments=arguments)


def install_oracle(upstream, fixture):
    (upstream.controls / 'native-context.json').write_text(json.dumps(fixture))
    upstream.reply = boundary_decision


def heartbeat_size(fixture):
    path = Path(fixture['boundary']['heartbeat'])
    return path.stat().st_size if path.exists() else 0


def running_child(fixture):
    path = Path(fixture['boundary']['pid'])
    if not path.exists():
        return False
    pid = int(path.read_text())
    try:
        os.kill(pid, 0)  # Observation only; never signal a PID read from a file.
    except ProcessLookupError:
        return False
    return True


def assert_cancelled_child_and_live_neighbor(target, neighbor):
    wait_until(lambda: not running_child(target), 'cancelled native tool child still exists', 15)
    target_size, neighbor_size = heartbeat_size(target), heartbeat_size(neighbor)
    assert target_size > 0 and neighbor_size > 0, 'both real native children must have started'
    wait_until(lambda: heartbeat_size(neighbor) > neighbor_size,
               'neighbor stopped when the other task was cancelled', 5)
    time.sleep(.3)
    assert heartbeat_size(target) == target_size, 'cancelled native tool still produces side effects'
    assert running_child(neighbor), 'neighbor native tool did not survive cancellation'


def source_events(upstream):
    path = upstream.controls / 'native-context-events.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def transcript_snapshot(fixture):
    root = (qoder_native_context.history_directory(fixture) if fixture['harness'] == 'qoder'
            else pi_native_context.history_directory(fixture) if fixture['harness'] == 'pi' else Path(fixture['config']) / ('sessions' if fixture['harness'] == 'codex' else 'projects'))
    paths = list(root.rglob('*.jsonl'))
    assert all(not path.is_symlink() for path in paths), 'unexpected linked fixture history'
    return {str(path): digest(path) for path in paths if path.is_file()}


def missing_history_refuses_new_session(product, fixture, task, upstreams):
    """Move only the exact transcript created by this synthetic task; restore afterward."""
    path, native_id = exact_history(fixture)
    path = Path(path)
    original = path.read_bytes()
    removed = path.with_suffix('.native-context-held')
    assert not removed.exists(), 'missing-history fixture would overwrite a neighboring file'
    before_calls = [upstream.request_count() for upstream in upstreams]
    path.rename(removed)
    before_histories = transcript_snapshot(fixture)
    try:
        command = ('worker continue --task ' + task['task_id'] + ' --expected-latest-run '
                   + task['run_id'] + ' --run-timeout 30 --no-wait'
                   + ' --submission-key native-context-missing-history --file - --output json')
        status, response = worker_cli(product, command, 'Continue the same task.', success=False)
        if status == 0:
            # Codex owns history discovery: an accepted run may fail during native load.
            accepted = response['data']
            result = wait_for_worker_result(product, accepted['run_id'], expected='failed', timeout=45)
            assert not result.get('result'), 'missing native history produced a replacement result'
        else:
            error = response.get('error', {}).get('code', '')
            # The public envelope intentionally projects ResumeUnavailable to this
            # capability code; do not assert an internal domain enum on the CLI wire.
            assert error == 'CAPABILITY_UNAVAILABLE', response
        assert [upstream.request_count() for upstream in upstreams] == before_calls, \
            'missing history sent a model prompt'
        assert transcript_snapshot(fixture) == before_histories, \
            'missing native history created or changed another session'
        assert not path.exists(), 'missing transcript was silently recreated'
        assert_preserved(fixture)
        return {'native_session_id': native_id, 'native_history_restored_after_case': True}
    finally:
        assert not path.exists(), 'refusing to overwrite an unexpected replacement transcript'
        removed.rename(path)
        assert path.read_bytes() == original


def publish_neighbor(product, upstream):
    saved = save_context_source(product, upstream, variant='native-context-neighbor')
    change = plan_change(product, 'create', 'native-context-neighbor-plan',
                         display_name='Native context independent neighbor',
                         candidates=[{'binding_id': saved['binding_id']}])
    preview = product.preview('routing preview', {'change': change})
    product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'neighbor-plan')
    return preview['plan_head']['reference']['plan_id']


def submit_concurrent_workers(product, fixtures, plans):
    """Submit both public requests before waiting for either native tool to start."""
    def submit(item):
        fixture, plan = item
        command = ('worker exec --plan ' + plan + ' --cwd ' + str(product.project)
                   + ' --run-timeout 180 --no-wait --submission-key native-context-'
                   + fixture['boundary']['role'] + ' --file - --output json')
        prompt = fixture['boundary']['marker'] + ': use both native context skills and run the requested receipt tool.'
        _, accepted = worker_cli(product, command, prompt)
        return accepted['data']

    with ThreadPoolExecutor(max_workers=2) as executor:
        return list(executor.map(submit, zip(fixtures, plans)))


def run(repository, candidate, harness):
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo)
    if harness == "pi":
        product.env["PI_CODING_AGENT_DIR"] = str(Path(product.env["HOME"]) / "selected-pi-config")
    upstreams, fixtures, runs = [], [], []
    stage = 'setup'
    report = dict(scenario='worker-native-context-boundaries', candidate=candidate,
                  worker_harness=harness, required_cases=list(CASES), selected_cases=list(CASES), cases=[],
                  evidence_limit='Real native children and borrowed configuration; synthetic sources; no Desktop verdict')
    try:
        base, target_source = prepare_product(product, harness)
        report['source_context_tokens'] = base['source_context_tokens']
        upstreams.append(target_source)
        target = prepare_heartbeat(base, 'target')
        neighbor = prepare_heartbeat(base, 'neighbor')
        fixtures.extend((target, neighbor))
        install_oracle(target_source, target)
        other_controls = product.root / 'neighbor-upstream'
        other_controls.mkdir(mode=0o700)
        neighbor_source = NativeContextUpstream(other_controls)
        neighbor_source.token = 'different-synthetic-neighbor-source-token'
        neighbor_source.model = 'gpt-5.5'
        upstreams.append(neighbor_source)
        install_oracle(neighbor_source, neighbor)
        neighbor_plan = publish_neighbor(product, neighbor_source)
        binary, adapter, node = selected_installation(harness)
        configure_worker_installation(product, product.worker_work['harness'], adapter, binary, node)
        # This standalone journey creates its own native roots. No core journey,
        # native initialization, or completed Worker may warm them before these requests.
        _, initial = worker_cli(product, 'worker list --output json')
        assert initial['data']['tasks'] == [], 'expected the first Workers in this fresh fixture'
        report['native_context_setup'] = ('explicit-borrowed-login-context-concurrent-first-tasks'
                                          if harness == 'qoder' else 'fresh-shared-context-concurrent-first-tasks')
        stage = 'simultaneous-native-tools'
        runs = submit_concurrent_workers(product, fixtures, (product.plan_id, neighbor_plan))
        for fixture, run in zip(fixtures, runs):
            fixture['task_id'] = run['task_id']
        wait_until(lambda: all(heartbeat_size(item) > 0 and running_child(item) for item in fixtures),
                   'both real Workers did not execute their native tools concurrently', 120)
        for fixture, source in zip(fixtures, upstreams):
            attempts = source_events(source)
            assert attempts and all(event['state'] == 'green' and event['model'] == source.model
                                    for event in attempts), attempts
            assert_preserved(fixture)
        assert runs[0]['task_id'] != runs[1]['task_id']
        report['simultaneous_routes'] = [dict(model=source.model, requests_before_cancel=len(source_events(source)))
                                         for source in upstreams]
        report['cases'].append({'id': CASES[0], 'state': 'green'})
        stage = 'cancel-target-with-live-neighbor'
        worker_cli(product, 'worker cancel --run ' + runs[0]['run_id']
                   + ' --idempotency-key native-context-cancel --reason user-requested --output json')
        wait_for_worker_result(product, runs[0]['run_id'], expected='cancelled', timeout=30)
        assert_cancelled_child_and_live_neighbor(target, neighbor)
        Path(neighbor['boundary']['release']).touch()
        result = wait_for_worker_result(product, runs[1]['run_id'], timeout=45)
        assert neighbor['receipt'] in result['result']
        wait_until(lambda: not running_child(neighbor), 'completed neighbor tool still exists', 10)
        for fixture in fixtures:
            assert_preserved(fixture)
        cancelled = dict(target, receipt=target['boundary']['marker'])
        target_history = (qoder_native_context.cancelled_history(cancelled, runs[0]['run_id'])
                          if harness == 'qoder' else exact_history(cancelled))
        wait_for_resumable_task(product, runs[1]['task_id'], runs[1]['run_id'])
        neighbor_history = exact_history(neighbor)
        assert target_history[1] != neighbor_history[1], 'concurrent tasks shared a native session'
        report['native_session_ids'] = [target_history[1], neighbor_history[1]]
        for history in (target_history, neighbor_history):
            text = Path(history[0]).read_text()
            assert all(source.token not in text for source in upstreams), 'source credential entered native history'
        report['cases'].append({'id': CASES[1], 'state': 'green'})
        report['task_ids'] = [run['task_id'] for run in runs]
        report['run_ids'] = [run['run_id'] for run in runs]
        stage = 'missing-native-history'
        report['missing_history'] = missing_history_refuses_new_session(product, neighbor, runs[1], upstreams)
        if harness == 'pi':
            report['corrupt_history'] = pi_native_context.verify_corrupt_history_variants(product, dict(neighbor, plan_id=neighbor_plan), runs[1], upstreams)
        report['cases'].append({'id': CASES[2], 'state': 'green', 'variant': 'missing-history-no-new'})
        report.update(state='green', binaries={name: digest(product.bin / name) for name in ('hiroute', 'hirouted')},
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
        # Let our synthetic scripts finish on a failed assertion. Product.stop retains
        # ownership of actual Worker cancellation; never signal a PID from a fixture file.
        for fixture in fixtures:
            Path(fixture['boundary']['release']).touch()
        try:
            product.close()
        finally:
            try:
                qoder_native_context.cleanup_context(product)
            finally:
                for upstream in upstreams:
                    upstream.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository')
    parser.add_argument('candidate')
    parser.add_argument('--harness', choices=('codex', 'claude', 'qoder', 'pi'), required=True)
    args = parser.parse_args()
    run(args.repository, args.candidate, args.harness)
