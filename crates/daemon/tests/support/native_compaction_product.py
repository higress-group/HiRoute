"""Automatic native compaction stays on the real Worker's frozen Gateway route.

The synthetic usage receipt triggers Qoder's own compactor inside one public
Worker execution. This is separate from the no-probe Continue/Skill journey.
"""
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import sys

from delegation_product import configure_worker_installation, wait_for_worker_result, worker_cli
from native_context_boundaries import source_events
from native_context_fixture import (assert_preserved, digest, is_native_compaction, native_text,
                                    protect_configuration, tool_results, write_new)
from native_context_product import (exact_history, prepare_product, report_failure,
                                    selected_installation, wait_for_resumable_task)
from publication_product import Product
import qoder_native_context


CASES = ('worker.context.tool-summary-route', 'worker.context.compaction-route')
CONTEXT_TOKENS = 100_000
PRESSURE_TOKENS = 90_000
PI_PRESSURE_TOKENS = 31_000


class CompactionOracle:
    def __init__(self, fixture, upstream_model):
        self.fixture, self.model = fixture, upstream_model
        self.main_requests = self.compact_requests = self.tool_summary_requests = 0
        self.tool_receipt = 'NATIVE-TOOL-' + secrets.token_hex(12)
        self.summary_receipt = 'NATIVE-COMPACT-' + secrets.token_hex(12)
        self.tool_summary_receipt = 'NATIVE-TOOL-SUMMARY-' + secrets.token_hex(12)

    def reply(self, _fixture, body):
        assert body.get('model') == self.model, 'compaction or main request changed the frozen upstream model'
        results = tool_results(body, 'qoder')
        items = body.get('input', [])
        last = native_text(items[-1]) if isinstance(items, list) and items else native_text(items)
        if 'tool output' in last.lower() and 'summar' in last.lower() and self.tool_receipt in last:
            assert self.main_requests == 1 and self.tool_summary_requests == 0, 'unexpected native tool summarization'
            self.tool_summary_requests += 1
            return dict(kind='text', request_kind='tool_summary', input_tokens=30,
                        text=self.tool_summary_receipt + ': The read-only tool returned ' + self.tool_receipt)
        if is_native_compaction(body):
            assert self.main_requests == 1 and self.compact_requests == 0, 'unexpected or repeated native compaction'
            assert self.tool_summary_requests == 1 and self.tool_summary_receipt in results.get('native_compaction_tool', ''), \
                'native compaction omitted the correlated summarized tool receipt'
            assert self.summary_receipt not in native_text(body), 'summary receipt was present before compaction'
            self.compact_requests += 1
            return dict(kind='text', request_kind='compact', input_tokens=30,
                        text='<analysis>Local routing acceptance only.</analysis><summary>' +
                             self.summary_receipt + ': The read-only tool returned ' + self.tool_summary_receipt +
                             '. Finish the requested native routing receipt.</summary>')
        self.main_requests += 1
        assert self.main_requests <= 3, 'unexpected native main request'
        if self.main_requests == 1:
            assert self.fixture['compaction_prompt'] in native_text(body), 'unknown native task'
            return dict(kind='tool', request_kind='main', id='native_compaction_tool', name='Bash',
                        arguments={'command': '/bin/sh ' + shlex.quote(self.fixture['compaction_script']),
                                   'timeout': 10000}, input_tokens=PRESSURE_TOKENS)
        assert (self.compact_requests == 1 and self.summary_receipt in native_text(body)
                and self.tool_summary_receipt in native_text(body)), \
            'main request did not consume the actual native summary'
        return dict(kind='text', request_kind='main', input_tokens=30,
                    text='compacted-' + self.fixture['receipt'])


def assert_native_compaction(path, summary_receipt):
    rows = [json.loads(line) for line in Path(path).read_text().splitlines() if line]
    boundaries = [row.get('compactMetadata', {}) for row in rows if row.get('subtype') == 'compact_boundary']
    assert any(item.get('trigger') == 'auto' and item.get('preTokens', 0) >= PRESSURE_TOKENS
               for item in boundaries), 'native history has no actual automatic compaction boundary'
    assert any(row.get('isCompactSummary') and summary_receipt in json.dumps(row) for row in rows), \
        'exact native session did not persist the independently issued summary'
    return [{'trigger': item.get('trigger'), 'pre_tokens': item.get('preTokens'),
             'post_tokens': item.get('postTokens')} for item in boundaries]


class PiCompactionOracle(CompactionOracle):
    def reply(self, _fixture, body):
        assert body.get('model') == self.model
        text = native_text(body)
        if any(signature in text.lower() for signature in ('conversation to summarize',
                'structured summary with new information', 'earlier context from an ongoing conversation')):
            assert self.main_requests == 2 and self.compact_requests == 0, 'unexpected Pi summarization order'
            assert self.tool_receipt in text, 'native summary omitted the actual tool receipt'
            self.compact_requests += 1
            return dict(kind='text',request_kind='compact',input_tokens=30,
                        text=self.summary_receipt + ': Executed tool receipt ' + self.tool_receipt)
        self.main_requests += 1
        assert self.main_requests <= 4
        if self.main_requests <= 2:
            assert self.fixture['compaction_prompt'] in text
            if self.main_requests == 2:
                assert self.tool_receipt in tool_results(body, 'pi').get('native_compaction_tool', ''), \
                    'pressure turn lacks the independently executed first tool receipt'
            return dict(kind='tool',request_kind='main',id='native_compaction_tool' + ('' if self.main_requests == 1 else '_pressure'),name='bash',
                arguments={'command':'/bin/sh ' + shlex.quote(self.fixture['compaction_script']),'timeout':10},
                input_tokens=1 if self.main_requests == 1 else PI_PRESSURE_TOKENS)
        assert self.compact_requests == 1 and self.summary_receipt in text, \
            f'Pi main request lacked native summary: main={self.main_requests}, compact={self.compact_requests}'
        return dict(kind='text',request_kind='main',input_tokens=30,text='compacted-' + self.fixture['receipt'])


def assert_pi_compaction(path, receipt):
    rows = [json.loads(line) for line in Path(path).read_text().splitlines() if line]
    compact = [row for row in rows if row['type'] == 'compaction']
    assert len(compact) == 1 and compact[0]['tokensBefore'] >= PI_PRESSURE_TOKENS
    assert receipt in compact[0]['summary'], 'native history lost the independent summary'
    return [{'pre_tokens':compact[0]['tokensBefore'],'summary_preserved':True}]


def run(repository, candidate):
    harness = os.environ.get('HIROUTE_PRODUCT_WORKER_HARNESS','qoder')
    assert harness in ('qoder','pi')
    cases = CASES if harness == 'qoder' else (CASES[1],)
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product, upstream = Product(repo), None
    if harness == 'pi':
        product.env['PI_CODING_AGENT_DIR'] = str(Path(product.env['HOME']) / 'selected-pi-config')
    stage = 'prepare-frozen-worker-source'
    report = dict(scenario=harness + '-worker-compaction-route', candidate=candidate, worker_harness=harness,
                  required_cases=list(cases), selected_cases=list(cases), cases=[],
                  evidence_limit='Real Plan/admission/profile/native Worker/Gateway; controlled upstream; no Desktop verdict')
    try:
        fixture, upstream = prepare_product(product, harness, context_tokens=32768 if harness == 'pi' else CONTEXT_TOKENS)
        report['source_context_tokens'] = fixture['source_context_tokens']
        binary, _, node = selected_installation(harness)
        configure_worker_installation(product, 'pi' if harness == 'pi' else 'qoder_cli', None, binary, node)
        if harness == 'qoder':
            project_settings = product.project / '.qoder/settings.json'
            settings = json.loads(project_settings.read_text())
            settings['experimental'] = {'contextManagement': True}
            settings['model']['summarizeToolOutput'] = {'Bash': {'tokenBudget': 2000}}
            project_settings.write_text(json.dumps(settings))
            protect_configuration(fixture, [project_settings])
        fixture['compaction_prompt'] = 'QODER-COMPACTION-' + secrets.token_hex(12)
        fixture['compaction_script'] = str(product.project / 'native-compaction-receipt.sh')
        oracle = (PiCompactionOracle if harness == 'pi' else CompactionOracle)(fixture, upstream.model)
        script = Path(fixture['compaction_script'])
        # Pi's compactor estimates actual recent messages as well as provider usage.
        # Two genuine large tool outputs leave an older receipt to summarize and
        # a recent one to retain; fabricated usage alone cannot prove compaction.
        lines = 1200 if harness == 'pi' else 300
        write_new(script, '#!/bin/sh\ni=0\nwhile [ "$i" -lt ' + str(lines) + ' ]; do\n' +
                  '  /usr/bin/printf "%s\\n" ' + shlex.quote(oracle.tool_receipt) +
                  '\n  i=$((i + 1))\ndone\n', executable=True)
        protect_configuration(fixture, [script])
        (product.root / 'native-context.json').write_text(json.dumps(fixture))
        upstream.reply = oracle.reply
        stage = 'automatic-compaction-in-real-worker'
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(product.project) +
                   ' --run-timeout 120 --no-wait --submission-key qoder-compaction --file - --output json')
        _, accepted = worker_cli(product, command,
            fixture['compaction_prompt'] + ': Execute the requested read-only receipt tool, then finish the task.')
        first = accepted['data']
        fixture['task_id'] = first['task_id']
        result = wait_for_worker_result(product, first['run_id'], timeout=150)
        assert 'compacted-' + fixture['receipt'] in result['result'], 'missing compacted Worker result'
        wait_for_resumable_task(product, first['task_id'], first['run_id'])
        history = exact_history(fixture)
        report['native_compaction_boundaries'] = (assert_pi_compaction if harness == 'pi' else assert_native_compaction)(history[0], oracle.summary_receipt)
        stage = 'continue-native-compacted-history'
        command = ('worker continue --task ' + first['task_id'] + ' --expected-latest-run ' + first['run_id'] +
                   ' --run-timeout 120 --no-wait --submission-key qoder-compaction-continue --file - --output json')
        _, accepted = worker_cli(product, command, 'Continue the same native task using its compacted summary.')
        continued = accepted['data']
        result = wait_for_worker_result(product, continued['run_id'], timeout=150)
        assert 'compacted-' + fixture['receipt'] in result['result']
        wait_for_resumable_task(product, first['task_id'], continued['run_id'])
        assert exact_history(fixture) == history, 'compacted Continue changed the exact native session'
        assert oracle.main_requests == (4 if harness == 'pi' else 3) and oracle.compact_requests == 1 and oracle.tool_summary_requests == (0 if harness == "pi" else 1), \
            'required native request kinds missing'
        attempts = source_events(upstream)
        assert [event.get('request_kind') for event in attempts] == (['main','main','compact','main','main'] if harness == 'pi' else ['main', 'tool_summary', 'compact', 'main', 'main']), attempts
        assert all(event['state'] == 'green' and event['model'] == upstream.model for event in attempts)
        assert_preserved(fixture)
        if harness == 'qoder':
            qoder_native_context.assert_settings_preserved(fixture)
        product.stop()
        diagnostics = product.diagnostics_snapshot()
        assert diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug'
        report.update(state='green', cases=[{'id': case, 'state': 'green'} for case in cases], task_id=first['task_id'],
                      run_ids=[first['run_id'], continued['run_id']], native_session_id=history[1],
                      main_requests=oracle.main_requests, compact_requests=oracle.compact_requests,
                      tool_summary_requests=oracle.tool_summary_requests,
                      upstream_model=upstream.model, foreign_requests=0, diagnostics=diagnostics,
                      binaries={name: digest(product.bin / name) for name in ('hiroute', 'hirouted')},
                      harness_sha256=digest(binary))
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
