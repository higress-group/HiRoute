"""Actual native tool interruption, durable stop and exact Continue across daemon restart."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from delegation_product import configure_worker_installation, worker_cli, wait_for_worker_result
from native_context_boundaries import (prepare_heartbeat, install_oracle, heartbeat_size,
                                      running_child, wait_until)
from native_context_fixture import CONTINUE_PROMPT, assert_preserved, decision
from native_context_product import (prepare_product, selected_installation, exact_history,
                                    wait_for_resumable_task, report_failure)
from publication_product import Product

CASES = ('worker.lifecycle.running-restart', 'worker.lifecycle.ended-cancel-idempotency',
         'worker.context.exact-continue')


def task_view(product, task_id):
    _, response = worker_cli(product, 'worker list --limit 200 --output json')
    return next(item for item in response['data']['tasks'] if item['task_id'] == task_id)


def run(repository, candidate):
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    harness = os.environ['HIROUTE_PRODUCT_WORKER_HARNESS']
    assert harness in ('codex', 'claude')
    product = Product(repo)
    source = active = None
    stage = 'prepare'
    report = dict(scenario='worker-native-shutdown', candidate=candidate, worker_harness=harness,
                  required_cases=list(CASES), selected_cases=list(CASES), cases=[],
                  evidence_limit='Real native CLI, public Worker commands and daemon; controlled upstream')
    try:
        fixture, source = prepare_product(product, harness)
        binary, adapter, node = selected_installation(harness)
        configure_worker_installation(product, product.worker_work['harness'], adapter, binary, node)
        stage = 'completed-native-task'
        prompt = 'Use ' + ' and '.join(skill['name'] for skill in fixture['skills']) + ' to produce their read-only receipts.'
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project)
                   + ' --run-timeout 120 --no-wait --submission-key before-restart --file - --output json')
        _, accepted = worker_cli(product, command, prompt)
        original = accepted['data']
        completed = wait_for_worker_result(product, original['run_id'])
        assert fixture['receipt'] in completed['result']
        original_view = wait_for_resumable_task(product, original['task_id'], original['run_id'])
        history_path, native_id = exact_history(fixture)
        history_before = Path(history_path).read_bytes()

        stage = 'running-native-tool'
        active = prepare_heartbeat(fixture, 'shutdown')
        install_oracle(source, active)
        active_prompt = active['boundary']['marker'] + ': execute the owned heartbeat tool.'
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project)
                   + ' --run-timeout 120 --no-wait --submission-key running-at-restart --file - --output json')
        _, accepted = worker_cli(product, command, active_prompt)
        interrupted = accepted['data']
        wait_until(lambda: heartbeat_size(active) > 0 and running_child(active),
                   'native child never started', 120)
        stage = 'normal-daemon-shutdown'
        started = time.monotonic()
        product.stop()
        report['shutdown_ms'] = (time.monotonic() - started) * 1000
        assert report['shutdown_ms'] < 30_000, 'Worker drain exceeded the existing role deadline'
        size = heartbeat_size(active)
        wait_until(lambda: not running_child(active), 'native child survived daemon shutdown', 5)
        time.sleep(.2)
        assert heartbeat_size(active) == size, 'native child kept writing after shutdown'
        product.start()
        closed = task_view(product, interrupted['task_id'])['run']
        assert closed['state'] in ('unknown', 'failed'), closed
        assert closed['cleanup'] == 'complete', closed
        assert closed['failure']['error'] != 'cancelled', closed
        assert closed['failure']['stage'] == 'prompt', closed
        report['interrupted_outcome'] = closed['state']
        report['cases'].append(dict(id=CASES[0], state='green'))

        stage = 'cancel-ended-run-and-replay'
        for _ in range(2):
            worker_cli(product, 'worker cancel --run ' + interrupted['run_id']
                       + ' --idempotency-key cancel-ended --reason user-requested --output json')
            assert task_view(product, interrupted['task_id'])['run'] == closed
        _, replay = worker_cli(product, command, active_prompt, success=False)
        assert replay['data']['replayed'] and replay['data']['run_id'] == interrupted['run_id']
        assert task_view(product, interrupted['task_id'])['run'] == closed
        report['cases'].append(dict(id=CASES[1], state='green'))

        stage = 'continue-original-completed-session'
        (source.controls / 'native-context.json').write_text(json.dumps(fixture))
        source.reply = decision
        command = ('worker continue --task ' + original['task_id'] + ' --expected-latest-run '
                   + original['run_id'] + ' --run-timeout 120 --no-wait --submission-key after-restart'
                   + ' --file - --output json')
        _, accepted = worker_cli(product, command, CONTINUE_PROMPT)
        continued = accepted['data']
        assert continued['task_id'] == original['task_id'] and not continued['replayed']
        result = wait_for_worker_result(product, continued['run_id'])
        assert 'continued-' + fixture['receipt'] in result['result']
        current = wait_for_resumable_task(product, original['task_id'], continued['run_id'])
        assert current['plan_revision'] == original_view['plan_revision']
        assert Path(history_path).read_bytes().startswith(history_before), 'native history was replaced'
        assert len(Path(history_path).read_bytes()) > len(history_before)
        assert_preserved(fixture)
        report['native_session_id'] = native_id
        report['run_ids'] = [original['run_id'], interrupted['run_id'], continued['run_id']]
        report['cases'].append(dict(id=CASES[2], state='green'))
        product.stop()
        report.update(state='green', binaries={name: hashlib.sha256((product.bin / name).read_bytes()).hexdigest()
                                              for name in ('hiroute', 'hirouted')})
        print(json.dumps(report), flush=True)
    except Exception:
        report_failure(product, report, stage)
        raise
    finally:
        if active:
            Path(active['boundary']['release']).touch()
        try:
            product.close()
        finally:
            if source:
                source.close()


if __name__ == '__main__':
    run(*sys.argv[1:])
