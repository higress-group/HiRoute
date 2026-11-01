"""Real Qoder -> public Worker CLI -> Gateway rejection and host progress.

Uses an explicitly selected normal native login. Only the model upstream is
controlled; native authentication/settings and unrelated Skills are preserved.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import time

from delegation_product import configure_worker_installation, wait_for_worker_result, worker_cli
from native_context_product import prepare_product, report_failure, selected_installation
from publication_product import Product
import qoder_native_context


def reject_model_request(_fixture, _body):
    raise AssertionError('controlled-worker-model-rejection')


def run(repository, candidate):
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo)
    upstream = None
    stage = 'prepare-qoder-context'
    report = dict(scenario='qoder-worker-failure-diagnostics', candidate=candidate,
                  worker_harness='qoder', cases=[],
                  evidence_limit='Real native Qoder and public CLI; controlled model rejection; no Desktop verdict')
    try:
        binary, adapter, node = selected_installation('qoder')
        fixture, upstream = prepare_product(product, 'qoder')
        configure_worker_installation(product, 'qoder_cli', adapter, binary, node)
        upstream.reply = reject_model_request
        stage = 'native-model-rejection'
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project)
                   + ' --run-timeout 90 --no-wait --submission-key qoder-failure-diagnostics'
                   + ' --file - --output json')
        _, accepted = worker_cli(product, command, 'Reply with OK only.')
        run_id = accepted['data']['run_id']
        report['run_id'] = run_id
        result = wait_for_worker_result(product, run_id, expected='failed', timeout=110)
        assert result.get('result') is None, 'host diagnostics must not become a model Result'
        assert upstream.request_count() > 0, 'native account did not reach the controlled model'
        events = [json.loads(line) for line in
                  (upstream.controls / 'native-context-events.jsonl').read_text().splitlines()]
        assert events and all(event.get('reason') == 'controlled-worker-model-rejection' for event in events), events
        stage = 'owned-cleanup'
        deadline = time.monotonic() + 15
        while True:
            _, listed = worker_cli(product, 'worker list --limit 200 --output json')
            task = next(task for task in listed['data']['tasks'] if task['latest_run_id'] == run_id)
            if task['run']['cleanup'] == 'complete':
                assert task['run']['state'] == 'failed'
                break
            assert time.monotonic() < deadline, 'owned cleanup did not complete'
            time.sleep(.25)
        stage = 'public-failure-explanation'
        _, progress = worker_cli(product, 'worker read --run ' + run_id + ' --max-bytes 32768 --output json')
        text = progress['data']['text']
        assert 'NATIVE_PROMPT_FAILED' in text, 'native error must not be classified as a user cancel'
        assert re.search(r'Last Gateway rejection observed for this run: HTTP [45][0-9]{2} ', text), \
            'the actual Gateway rejection must reach the public progress stream'
        assert json.dumps(fixture['config'], ensure_ascii=False) in text, 'effective Qoder directory is missing'
        assert 'previous directory' in text and 'Existing tasks keep their original directory' in text
        report['cases'].extend([
            {'id': 'worker.failure.native-rejection', 'state': 'green'},
            {'id': 'worker.context.effective-qoder-directory', 'state': 'green'},
        ])
        qoder_native_context.assert_settings_preserved(fixture)
        report.update(state='green', cleanup='complete', native_settings_unchanged=True,
                      controlled_model_requests=upstream.request_count())
        product.stop()
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


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
