"""Real Qoder main Agent → installed user Skill → public CLI → real Qoder Worker.

Uses the same Product, managed candidate and native context as the Worker journeys.
Only model upstreams are deterministic. No daily provider/auth settings are edited.
"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys

from delegation_product import configure_worker_installation
from native_context_boundaries import source_events
from native_context_fixture import NativeContextUpstream, assert_preserved, digest, write_new
from native_context_product import prepare_product, report_failure, selected_installation, wait_for_resumable_task
from publication_product import Product, encoded
from qoder_collaboration_fixture import CASES, MainAgentOracle, main_route_settings, worker_decision
import qoder_native_context


AGENT = 'agent_qoder_default'


def check_collaboration(product, key):
    check = {'agent_id': AGENT, 'scope': 'collaboration', 'suite': 'quick', 'allow_model_call': False}
    consent = {'change_digest': 'sha256:' + hashlib.sha256(encoded(check)).hexdigest(),
               'expected_revisions': product.control('GetClientServiceStatus', {})['data']['revisions']}
    capability = product.grant('CheckAgentConnection', consent, key)
    _, checked = product.cli('agents check ' + AGENT + ' --scope collaboration', capability=capability)
    assert checked['data']['skill_loading'] == 'proven' and checked['data']['trusted_cli_execution'] == 'proven', \
        'native collaboration capability check did not complete'


def apply_collaboration(product, context, intent, key):
    spec = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': context, 'collaboration': intent}
    command = 'agents restore' if intent['intent'] == 'restore' else 'agents connect'
    preview = product.preview(command + ' preview', {'spec': spec})
    assert preview['applicable'], preview.get('blockers')
    _, applied = product.cli(command + ' apply', {
        'spec': spec, 'accept_digest': preview['accept_digest'], 'dependency_digest': preview['dependency_digest'],
        'expected_revisions': preview['expected_revisions'], 'idempotency_key': key})
    assert applied['data']['state'] == 'succeeded', applied
    status = product.preview('agents connect status', {
        'schema_version': {'major': 2, 'minor': 0}, 'context_id': context})
    return status['collaboration']


def run_main_agent(product, binary, upstream, fixture):
    native_id, overlay = main_route_settings(upstream.base_url, upstream.model, 'HIROUTE_QODER_MAIN_TOKEN')
    settings = product.root / 'qoder-main-settings.json'
    write_new(settings, json.dumps(overlay))
    temporary = product.root / 'qoder-main-tmp'
    temporary.mkdir(mode=0o700)
    args = [str(binary), '--cwd', str(product.project), '--config-dir', fixture['config'],
            '--setting-sources', 'user', '--settings', str(settings), '--model', native_id,
            '--print', '--no-session-persistence', '--output-format', 'stream-json',
            '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}', '--tools', 'Skill,Read,Bash',
            '--allowed-tools', 'Skill,Bash', '--permission-mode', 'dont_ask',
            '--max-model-request-retries', '0', '--max-output-tokens', '2048', '--max-turns', '32',
            '-p', fixture['main_marker'] + ': Use hiroute-collaboration and explicitly delegate the native receipt task.']
    env = dict(product.env, HIROUTE_QODER_MAIN_TOKEN=upstream.token, TMPDIR=str(temporary))
    process = subprocess.Popen(args, env=env, cwd=product.project, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=190)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            stdout, stderr = process.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate(timeout=3)
        product.outputs.extend((stdout, stderr))
        raise AssertionError('native main Agent exceeded the full delegation deadline') from None
    product.outputs.extend((stdout, stderr))
    assert process.returncode == 0, 'native main Agent exited unsuccessfully; inspect retained private diagnostics'
    assert ('MAIN-AGENT-COMPLETED-' + fixture['artifact']['receipt']).encode() in stdout, \
        'native main Agent did not return the independently verified completion'


def restore_skill(product, context, restore_ref, before, skill):
    # A journey can configure/restore more than once. Each restoration point
    # represents a distinct operation; do not reuse one global idempotency key.
    key = 'qoder-collaboration-disable-' + hashlib.sha256(encoded(restore_ref)).hexdigest()[:16]
    status = apply_collaboration(product, context, {'intent': 'restore', 'restore_point_ref': restore_ref},
                                 key)
    assert (status['state'] == 'restored' and status.get('current_selection') is None
            and status.get('restore_point_ref') is None), \
        'collaboration disable did not restore and release its current selection/reference'
    if before is None:
        assert not skill.exists(), 'product left its owned collaboration Skill installed'
    else:
        assert skill.read_bytes() == before, 'product removed or rewrote a borrowed user Skill'


def finish_collaboration(product, sources, restore, report, stage, primary_failure):
    """Restore before stopping the service; preserve the first business failure."""
    errors = []
    if restore is not None:
        try:
            if product.process is None:
                product.start()
            restore()
            report['restore_after_failure'] = {'state': 'succeeded'}
        except Exception as error:
            errors.append({'step': 'restore_collaboration', 'error_type': type(error).__name__})
            report['restore_after_failure'] = {'state': 'red'}
    if primary_failure or errors:
        report['cleanup_failures'] = list(errors)
        try:
            report_failure(product, report, stage)
        except Exception as error:
            errors.append({'step': 'preserve_failure_report', 'error_type': type(error).__name__})
    # Try every independently owned cleanup even when another fails. Never remove
    # user material directly to compensate for a failed product Restore.
    actions = [('close_product', product.close),
               ('cleanup_owned_receipts', lambda: qoder_native_context.cleanup_context(product))]
    actions.extend(('close_source', source.close) for source in sources)
    for step, action in actions:
        try:
            action()
        except Exception as error:
            errors.append({'step': step, 'error_type': type(error).__name__})
    if errors:
        print(json.dumps({'scenario': 'qoder-collaboration-cleanup', 'state': 'red',
                          'candidate': report['candidate'], 'failures': errors,
                          'primary_failure_preserved': primary_failure}), flush=True)
        if not primary_failure:
            raise AssertionError('Qoder collaboration cleanup failed; inspect the cleanup report')


def run(repository, candidate):
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo)
    sources = []
    fixture = None
    context = restore_ref = None
    restored = False
    skill = None
    before_skill = None
    primary_failure = False
    oracle = None
    stage = 'explicit-qoder-context'
    report = dict(scenario='qoder-main-agent-delegation', candidate=candidate, worker_harness='qoder',
                  required_cases=list(CASES), selected_cases=list(CASES), cases=[],
                  evidence_limit='Real main Agent, user Skill, public CLI, daemon, Gateway and Worker; synthetic models; no Desktop verdict')
    try:
        binary, _, _ = selected_installation('qoder')
        (product.root / 'bin/qodercli').symlink_to(binary)
        fixture, worker_source = prepare_product(product, 'qoder')
        report['source_context_tokens'] = fixture['source_context_tokens']
        sources.append(worker_source)
        configure_worker_installation(product, 'qoder_cli', None, binary, None)
        fixture['main_marker'] = 'QODER-MAIN-' + secrets.token_hex(12)
        fixture['artifact'] = dict(path=str(product.project / 'delegated-native-artifact.txt'),
                                   receipt='QODER-WORKER-' + secrets.token_hex(16))
        (product.root / 'native-context.json').write_text(json.dumps(fixture))
        worker_source.reply = worker_decision
        skill = qoder_native_context.prepare_collaboration_target(product)
        assert not skill.is_symlink(), 'user collaboration Skill is a symlink'
        before_skill = skill.read_bytes() if skill.exists() else None
        stage = 'native-collaboration-preflight'
        check_collaboration(product, 'qoder-preflight')
        scan = product.preview('agents scan')
        agents = [agent for agent in scan['agents'] if agent['agent_id'] == AGENT]
        assert len(agents) == 1, 'Qoder installation/context was not discovered'
        context = agents[0]['context_id']
        stage = 'install-real-user-collaboration-skill'
        status = apply_collaboration(product, context,
                    {'intent': 'configure', 'settings': {'trigger_mode': 'explicit'}}, 'qoder-collaboration-enable')
        assert status['state'] == 'configured' and status['current_selection']['trigger_mode'] == 'explicit', status
        restore_ref = status['restore_point_ref']
        assert restore_ref and skill.is_file(), 'configured collaboration has no owned restore reference/Skill'
        assert not (product.project / '.qoder/skills/hiroute-collaboration').exists(), 'project Skill shadows user target'
        stage = 'verify-installed-user-skill'
        check_collaboration(product, 'qoder-installed-user-skill')
        qoder_native_context.assert_settings_preserved(fixture)
        controls = product.root / 'main-agent-source'
        controls.mkdir(mode=0o700)
        (controls / 'native-context.json').write_text('{}')
        main_source = NativeContextUpstream(controls)
        sources.append(main_source)
        main_source.token, main_source.model = 'synthetic-main-agent-only-token', 'main-agent/receipt:v1'
        oracle = MainAgentOracle(fixture, skill, product.bin / 'hiroute', lambda: source_events(worker_source))
        main_source.reply = oracle.reply
        stage = 'real-main-agent-to-worker'
        run_main_agent(product, binary, main_source, fixture)
        assert oracle.user_skill_proved and oracle.completed, 'main Agent oracle did not complete all stages'
        assert all(event['state'] == 'green' for source in sources for event in source_events(source))
        fixture['task_id'] = oracle.accepted['task_id']
        wait_for_resumable_task(product, oracle.accepted['task_id'], oracle.accepted['run_id'])
        report.update(task_id=oracle.accepted['task_id'], run_id=oracle.accepted['run_id'],
                      native_session_id=qoder_native_context.task_binding(fixture),
                      main_requests=len(source_events(main_source)), worker_requests=len(source_events(worker_source)),
                      actual_user_skill_sha256=digest(skill), artifact_sha256=digest(fixture['artifact']['path']),
                      main_worker_distinct_sources=True, native_context_setup='explicit-borrowed-login-context')
        report['cases'].extend({'id': case, 'state': 'green'} for case in CASES[:2])
        assert_preserved(fixture)
        stage = 'disable-only-owned-collaboration-skill'
        restore_skill(product, context, restore_ref, before_skill, skill)
        restored = True
        qoder_native_context.assert_settings_preserved(fixture)
        report['cases'].append({'id': CASES[2], 'state': 'green'})
        product.stop()
        diagnostics = product.diagnostics_snapshot()
        assert diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug'
        report.update(state='green', user_settings_unchanged=True, diagnostics=diagnostics,
                      binaries={name: digest(product.bin / name) for name in ('hiroute', 'hirouted')},
                      harness_sha256=digest(binary))
    except Exception:
        primary_failure = True
        if oracle is not None:
            report['main_observations'] = {
                'actual_user_skill_proved': oracle.user_skill_proved,
                'requests': oracle.requests,
                'completed': oracle.completed,
                'public_worker_result': ({key: oracle.accepted.get(key)
                                          for key in ('task_id', 'run_id', 'run_state')}
                                         if oracle.accepted is not None else None),
            }
        raise
    finally:
        restore = None
        if restore_ref and not restored:
            restore = lambda: restore_skill(product, context, restore_ref, before_skill, skill)
        finish_collaboration(product, sources, restore, report, stage, primary_failure)
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
