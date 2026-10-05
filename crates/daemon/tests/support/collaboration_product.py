"""Real main Agent → installed user Skill → public CLI → real Qoder or Pi Worker.

Uses the same Product, managed candidate and native context as the Worker journeys.
Only model upstreams are deterministic. No daily provider/auth settings are edited.
"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys

from agent_product_support import apply_settings, run_native_command, expose_native_installation
from delegation_product import configure_worker_installation
from native_context_boundaries import source_events
from native_context_fixture import NativeContextUpstream, assert_preserved, digest, write_new
from native_context_product import prepare_product, report_failure, selected_installation, wait_for_resumable_task
from publication_product import Product, encoded
from collaboration_fixture import CASES, MainAgentOracle, main_route_settings, worker_decision
import qoder_native_context


HARNESS = 'qoder'
AGENT = 'agent_qoder_default'


def check_collaboration(product, key, agent=None):
    agent = AGENT if agent is None else agent
    check = {'agent_id': agent, 'scope': 'collaboration', 'suite': 'quick', 'allow_model_call': False}
    consent = {'change_digest': 'sha256:' + hashlib.sha256(encoded(check)).hexdigest(),
               'expected_revisions': product.control('GetClientServiceStatus', {})['data']['revisions']}
    capability = product.grant('CheckAgentConnection', consent, key)
    _, checked = product.cli('agents check ' + agent + ' --scope collaboration', capability=capability)
    assert checked['data']['skill_loading'] == 'proven' and checked['data']['trusted_cli_execution'] == 'proven', \
        'native collaboration capability check did not complete'


def apply_collaboration(product, context, intent, key):
    spec = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': context, 'collaboration': intent}
    _, status = apply_settings(product, spec, key, 'collaboration')
    return status['collaboration']


def run_main_agent(product, binary, upstream, fixture):
    if HARNESS == 'pi':
        config = Path(fixture['config'])
        models = json.loads((config / 'models.json').read_text())
        models['providers']['hiroute-main-acceptance'] = {'api':'openai-responses',
            'baseUrl':upstream.base_url,'apiKey':upstream.token,
            'models':[{'id':upstream.model,'name':'Main acceptance','reasoning':False,'input':['text'],
                'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0},'contextWindow':100000,'maxTokens':2048}]}
        (config / 'models.json').write_text(json.dumps(models))
        from native_context_fixture import protect_configuration
        protect_configuration(fixture, [config / 'models.json'])
        args = [os.environ['HIROUTE_WORKER_NODE'],str(binary),'--provider','hiroute-main-acceptance',
                '--model',upstream.model,'--print','--mode','json','--no-session','--no-extensions','--offline',
                '--tools','read,bash',fixture['main_marker'] + ': Read hiroute-collaboration and delegate the native receipt task.']
        stdout = run_native_command(product,args,timeout=190,label='native Pi main Agent')
        assert ('MAIN-AGENT-COMPLETED-' + fixture['artifact']['receipt']).encode() in stdout
        return
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
    stdout = run_native_command(product, args, env=env, timeout=190, label='native main Agent')
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
    global HARNESS, AGENT
    HARNESS = os.environ.get('HIROUTE_PRODUCT_WORKER_HARNESS','qoder')
    assert HARNESS in ('qoder','pi')
    AGENT = 'agent_' + HARNESS + '_default'
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
    stage = 'explicit-native-context'
    report = dict(scenario=HARNESS + '-main-agent-delegation', candidate=candidate, worker_harness=HARNESS,
                  required_cases=list(CASES), selected_cases=list(CASES), cases=[],
                  evidence_limit='Real main Agent, user Skill, public CLI, daemon, Gateway and Worker; synthetic models; no Desktop verdict')
    try:
        binary, _, node = selected_installation(HARNESS)
        expose_native_installation(product, HARNESS, binary, node)
        if HARNESS == 'pi':
            product.env['PI_CODING_AGENT_DIR'] = str(Path(product.env['HOME']) / 'selected-pi-config')
        fixture, worker_source = prepare_product(product, HARNESS)
        report['source_context_tokens'] = fixture['source_context_tokens']
        sources.append(worker_source)
        configure_worker_installation(product, 'pi' if HARNESS == 'pi' else 'qoder_cli', None, binary, node)
        if HARNESS == 'pi':
            # Main CLI follows native auth behavior; the Worker-only helper trap is
            # covered by the core journey, not executed by the main Agent fixture.
            auth = Path(fixture['config']) / 'auth.json'
            auth.write_text('{}')
            from native_context_fixture import protect_configuration
            protect_configuration(fixture, [auth])
        fixture['main_marker'] = 'QODER-MAIN-' + secrets.token_hex(12)
        fixture['artifact'] = dict(path=str(product.project / 'delegated-native-artifact.txt'),
                                   receipt='QODER-WORKER-' + secrets.token_hex(16))
        (product.root / 'native-context.json').write_text(json.dumps(fixture))
        worker_source.reply = worker_decision
        skill = (Path(fixture['config']) / 'skills/hiroute-collaboration/SKILL.md' if HARNESS == 'pi' else
                 qoder_native_context.prepare_collaboration_target(product))
        assert not skill.is_symlink(), 'user collaboration Skill is a symlink'
        before_skill = skill.read_bytes() if skill.exists() else None
        stage = 'native-collaboration-preflight'
        check_collaboration(product, 'qoder-preflight')
        scan = product.preview('agents scan')
        agents = [agent for agent in scan['agents'] if agent['agent_id'] == AGENT]
        assert len(agents) == 1, HARNESS + ' installation/context was not discovered'
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
        if HARNESS == 'qoder':
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
                      native_session_id=(__import__('pi_native_context').exact_history(dict(fixture, receipt=fixture['artifact']['receipt']))[1] if HARNESS == 'pi'
                                         else qoder_native_context.task_binding(fixture)),
                      main_requests=len(source_events(main_source)), worker_requests=len(source_events(worker_source)),
                      actual_user_skill_sha256=digest(skill), artifact_sha256=digest(fixture['artifact']['path']),
                      main_worker_distinct_sources=True,
                      native_context_setup=fixture.get('native_context_setup', 'fresh-synthetic-context'))
        report['cases'].extend({'id': case, 'state': 'green'} for case in CASES[:2])
        assert_preserved(fixture)
        stage = 'disable-only-owned-collaboration-skill'
        restore_skill(product, context, restore_ref, before_skill, skill)
        restored = True
        if HARNESS == 'qoder':
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
