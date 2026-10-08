"""Save additional native models, then read them through ordinary Qoder or Pi.

Uses production Local Control settings, publication and Gateway. Native starts
read only the persisted provider in an explicitly selected native context.
The original four Worker/collaboration journeys remain separate.
"""
from publication_process import judgment_fixture
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys

from model_connections_product import save_native_source
from agent_product_support import apply_settings, settings_status, expose_native_installation
from native_context_boundaries import source_events
from native_context_fixture import NativeContextUpstream, NativeProxyTrap, digest
from native_context_product import report_failure, selected_installation
from publication_process import plan_change
from publication_product import Product, encoded
from collaboration_product import apply_collaboration, check_collaboration as _check_collaboration, restore_skill
from additional_model_fixture import (CASES, OwnedModelSettings, PersistedRouteOracle,
                                 read_persisted_route, select_model_context, persisted_plan_provider)
import qoder_native_context

HARNESS = "qoder"
AGENT = "agent_qoder_default"
MODE = "qoder_additional"
TAIL = "/_hiroute/qoder/v1"
PLAN_PROTOCOLS = {}

def check_collaboration(product, key):
    return _check_collaboration(product, key, agent=AGENT)


def model_spec(context, plans=None, restore_ref=None, regenerate=False):
    model = ({'intent': 'restore', 'restore_point_ref': restore_ref} if restore_ref else
             {'intent': 'configure', 'settings': {'mode': MODE, 'allowed_plan_ids': plans, 'plan_protocols': {p: PLAN_PROTOCOLS.get(p, 'responses') for p in plans}}})
    return {'schema_version': {'major': 2, 'minor': 0}, 'context_id': context, 'model': model,
            'collaboration': {'intent': 'keep'},
            'access_token': {'intent': 'regenerate' if regenerate else 'keep'}}


def apply_models(product, spec, key):
    return apply_settings(product, spec, key, 'model')


def publish_source_plan(product, source, index):
    saved = save_native_source(product, source, token=source.token, upstream_model_id=source.model,
                               variant=HARNESS + '-model-' + str(index), context_tokens=100000, protocol='responses' if index == 0 else 'messages')
    product.editor = {
        'schema': 'hiroute.plan-editor/v2', 'display_name': 'Additional ' + HARNESS + ' route ' + str(index),
        'purpose': 'Read a persisted additional model route', 'mode': 'fixed_model',
        'candidates': [{'binding_id': saved['binding_id']}],
        'smart': {'economy': [], 'primary': [], 'judgment': judgment_fixture(),
                  'reselect_on_user_message': False, 'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
        'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
        'delegation_enabled': False, 'requirements': {},
        'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000, 'attempt_timeout_ms': 30000},
    }
    change = plan_change(product, 'create', HARNESS + '-model-plan-' + str(index))
    preview = product.preview('routing preview', {'change': change})
    product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'model-plan-' + str(index))
    product.plan_id = preview['plan_head']['reference']['plan_id']
    PLAN_PROTOCOLS[product.plan_id] = 'responses' if index == 0 else 'messages'
    source.expected_protocol = PLAN_PROTOCOLS[product.plan_id]
    return {'plan_id': product.plan_id, 'alias': preview['plan_head']['model_alias']}


def assert_configured(status, plans):
    assert status['state'] == 'configured' and status.get('restore_point_ref'), 'model route is not configured'
    assert status['current_selection'] == {'mode': MODE, 'allowed_plan_ids': sorted(plans), 'plan_protocols': {p: PLAN_PROTOCOLS.get(p, 'responses') for p in plans}}, \
        'model selection changed the selected Plan set'


def assert_saved_aliases(settings, provider, aliases):
    models = [model for name, value in settings.read()['providers'].items() if name == provider or name.startswith(provider + '-') for model in value['models']]
    for alias in aliases:
        assert len(settings.read()['providers'][persisted_plan_provider(settings, provider, alias)]['models']) == 1
    assert len(models) == len(aliases) and {item['id' if HARNESS in ('pi','dsh') else 'model'] for item in models} == set(aliases), \
        'persisted native choices do not match the selected routes'


def live_check(product, context, aliases, oracles):
    status = settings_status(product, context)
    targets = [target for target in status.get('live_check_targets', []) if target['surface'] == 'qoder_cli']
    assert len(targets) == 1 and set(targets[0]['client_model_ids']) == set(aliases), 'live scope lost persisted routes'
    check = {'agent_id': AGENT, 'scope': 'live', 'suite': 'quick', 'allow_model_call': True, 'target': targets[0]}
    consent = {'change_digest': 'sha256:' + hashlib.sha256(encoded(check)).hexdigest(),
               'expected_revisions': product.control('GetClientServiceStatus', {})['data']['revisions']}
    capability = product.grant('CheckAgentConnection', consent, 'qoder-persisted-live')
    before = [len(oracle.calls) for oracle in oracles]
    product.cli('agents check', check, capability)
    status = settings_status(product, context)
    assert status['model_verified'] is True and any(
        row['surface'] == 'qoder_cli' and row['state'] == 'passed'
        and row['applied_revision'] == status['applied_revision'] for row in status['surface_results']), \
        'real persisted-route Live check did not verify the current publication'
    assert all(oracle.calls[index:] == ['live-check'] for oracle, index in zip(oracles, before)), \
        'Live verification must actually request each selected persisted route once'


def reject_gateway_request(product, token, alias, protocol, sources, expected_codes):
    before = [source.request_count() for source in sources]
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=10)
    try:
        body = ({'model': alias, 'input': 'Untrusted removed route', 'max_output_tokens': 16, 'stream': True}
                if protocol == 'responses' else {'model': alias, 'messages': [{'role':'user','content':'Untrusted removed route'}],
                    'max_tokens':16, 'stream':True})
        client.request('POST', TAIL + '/' + protocol, body=encoded(body),
            headers={'Content-Type': 'application/json', **({'X-HiRoute-Token':token} if HARNESS in ('pi','dsh')
                else {'Authorization':'Bearer ' + token})})
        response = client.getresponse()
        error = json.loads(response.read())
        assert response.status in (401, 403, 404, 422) and error.get('code') in expected_codes, \
            f'obsolete authority or removed alias rejection: {response.status} {error.get("code")}'
    finally:
        client.close()
    assert [source.request_count() for source in sources] == before, 'rejected request reached a source'


def assert_default_blocks_removal(product, context, settings, provider, plans, skill):
    selected = persisted_plan_provider(settings, provider, plans[0]['alias']) + '/' + plans[0]['alias']
    settings.select_default(settings.native_default, selected)
    try:
        status = settings_status(product, context)
        before, skill_before = digest(settings.path), digest(skill)
        original_token = product.bearer(product.agent_connection)
        for spec in (model_spec(context, [plans[1]['plan_id']]),
                     model_spec(context, restore_ref=status['restore_point_ref'])):
            command = 'agents restore' if spec['model']['intent'] == 'restore' else 'agents connect'
            preview = product.preview(command + ' preview', {'spec': spec})
            assert not preview['applicable'] and any(
                item['reason'] == ('additional_default_in_use' if HARNESS in ('pi','dsh') else 'qoder_default_in_use') for item in preview['blockers']), \
                'removing the selected native default must fail closed'
            assert digest(settings.path) == before and digest(skill) == skill_before, 'blocked removal changed files'
            assert settings_status(product, context) == status, 'blocked removal changed settings authority'
            assert product.bearer(product.agent_connection) == original_token, 'blocked removal changed the model token'
        # Keeping the default-selected alias remains a legal user adjustment.
        preview = product.preview('agents connect preview', {'spec': model_spec(context, [p['plan_id'] for p in plans])})
        assert preview['applicable'], 'retaining the selected default was incorrectly rejected'
        settings.assert_preserved(provider, selected_default=selected)
    finally:
        settings.select_default(selected, settings.native_default)


def restore_models(product, context):
    current = settings_status(product, context)
    _, restored = apply_models(product, model_spec(context, restore_ref=current['restore_point_ref']),
                               'qoder-model-restore')
    assert restored['state'] == 'not_configured' and restored.get('current_selection') is None \
        and restored.get('restore_point_ref') is None, 'model Restore did not release its selection/reference'


def run(repository, candidate):
    global HARNESS, AGENT, MODE, TAIL
    PLAN_PROTOCOLS.clear()
    HARNESS = os.environ.get('HIROUTE_PRODUCT_WORKER_HARNESS', 'qoder')
    assert HARNESS in ('qoder', 'pi', 'dsh')
    AGENT = 'agent_' + HARNESS + '_default'
    MODE = HARNESS + '_additional'
    TAIL = '/v1' if HARNESS in ('pi','dsh') else '/_hiroute/qoder/v1'
    repo = Path(repository).resolve()
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() == candidate
    product = Product(repo)
    sources, oracles, settings, context, skill = [], [], None, None, None
    foreign = None
    model_active = skill_active = False
    before_skill = None
    stage = 'dedicated-model-context'
    report = dict(scenario=HARNESS + '-persisted-model-routes', candidate=candidate, worker_harness=HARNESS,
                  required_cases=list(CASES), selected_cases=list(CASES), cases=[], native_reads=[],
                  evidence_limit=f'Real persisted settings/native {HARNESS}/Gateway; controlled sources; no Desktop verdict')
    failed = False
    cleanup_errors = []
    try:
        config = select_model_context(product, HARNESS)
        binary, _, node = selected_installation(HARNESS)
        expose_native_installation(product, HARNESS, binary, node)
        foreign = NativeProxyTrap(product.root / 'unselected-native.log')
        settings = OwnedModelSettings(config, foreign.url + '/v1', HARNESS)
        product.enable_debug_diagnostics()
        product.start()
        plans = []
        for index, model in enumerate(('gpt-5.4', 'gpt-5.5')):
            controls = product.root / ('persisted-source-' + str(index))
            controls.mkdir(mode=0o700)
            (controls / 'native-context.json').write_text('{}')
            source = NativeContextUpstream(controls)
            source.model, source.token = model, 'synthetic-persisted-source-' + secrets.token_hex(12)
            oracle = PersistedRouteOracle(model)
            source.reply = oracle.reply
            sources.append(source)
            oracles.append(oracle)
            plans.append(publish_source_plan(product, source, index))
        agent = next(item for item in product.preview('agents scan')['agents'] if item['agent_id'] == AGENT)
        context = agent['context_id']
        product.agent_connection = 'agent-connection/' + context
        stage = 'persist-model-routes'
        plan_ids = [plan['plan_id'] for plan in plans]
        preview, status = apply_models(product, model_spec(context, plan_ids), 'qoder-model-enable')
        model_active = True
        product.bearer(product.agent_connection)  # Register the actual secret for output-leak checks immediately.
        provider = preview['model_effect']['provider_id']
        assert preview['model_effect']['endpoint'] == 'http://127.0.0.1:' + str(product.port) + TAIL
        assert_configured(status, plan_ids)
        assert_saved_aliases(settings, provider, [plan['alias'] for plan in plans])
        settings.assert_preserved(provider)
        settings.add_user_edit(provider)
        if HARNESS == 'pi':
            settings.install_conflicting_auth([persisted_plan_provider(settings, provider, plan['alias']) for plan in plans])
        for index, plan in enumerate(plans):
            report['native_reads'].append(read_persisted_route(product, binary, persisted_plan_provider(settings, provider, plan['alias']) + '/' + plan['alias'],
                sources[index], oracles[index], 'persisted-' + str(index)))
        product.stop()
        product.start()
        report['native_reads'].append(read_persisted_route(product, binary, persisted_plan_provider(settings, provider, plans[0]['alias']) + '/' + plans[0]['alias'],
            sources[0], oracles[0], 'after-daemon-restart'))
        if HARNESS == 'qoder':
            live_check(product, context, [plan['alias'] for plan in plans], oracles)
        report['cases'].append({'id': CASES[0], 'state': 'green'})

        stage = 'rotate-only-model-authority'
        old_token = product.bearer(product.agent_connection)
        _, status = apply_models(product, model_spec(context, plan_ids, regenerate=True), 'qoder-model-rotate')
        assert_configured(status, plan_ids)
        new_token = product.bearer(product.agent_connection)
        assert old_token != new_token, 'model token rotation reused the prior credential'
        reject_gateway_request(product, old_token, plans[0]['alias'], PLAN_PROTOCOLS[plans[0]['plan_id']], sources, ('GATEWAY_GRANT_UNAUTHORIZED',))
        report['native_reads'].append(read_persisted_route(product, binary, persisted_plan_provider(settings, provider, plans[1]['alias']) + '/' + plans[1]['alias'],
            sources[1], oracles[1], 'after-token-rotation'))
        settings.assert_preserved(provider)
        report['cases'].append({'id': CASES[1], 'state': 'green'})

        stage = 'independent-model-and-skill'
        (config / 'skills').mkdir(mode=0o700, exist_ok=True)
        skill = (config / 'skills/hiroute-collaboration/SKILL.md' if HARNESS in ('pi','dsh') else
                 qoder_native_context.prepare_collaboration_target(product))
        before_skill = skill.read_bytes() if skill.exists() else None
        check_collaboration(product, 'model-skill-preflight')
        collaboration = apply_collaboration(product, context,
            {'intent': 'configure', 'settings': {'trigger_mode': 'explicit'}}, 'model-skill-enable')
        skill_active = True
        check_collaboration(product, 'model-skill-installed')
        restore_skill(product, context, collaboration['restore_point_ref'], before_skill, skill)
        skill_active = False
        assert_configured(settings_status(product, context), plan_ids)
        report['native_reads'].append(read_persisted_route(product, binary, persisted_plan_provider(settings, provider, plans[0]['alias']) + '/' + plans[0]['alias'],
            sources[0], oracles[0], 'after-skill-restore'))
        # Restore invalidates evidence about the formerly installed Skill. A new
        # configuration must prove the current capability rather than reuse it.
        check_collaboration(product, 'model-skill-reenable-preflight')
        collaboration = apply_collaboration(product, context,
            {'intent': 'configure', 'settings': {'trigger_mode': 'explicit'}}, 'model-skill-enable-again')
        skill_active = True
        skill_digest = digest(skill)
        assert_default_blocks_removal(product, context, settings, provider, plans, skill)
        report['cases'].append({'id': CASES[3], 'state': 'green'})
        _, status = apply_models(product, model_spec(context, [plan_ids[0]]), 'qoder-model-adjust')
        assert_configured(status, [plan_ids[0]])
        assert_saved_aliases(settings, provider, [plans[0]['alias']])
        reject_gateway_request(product, product.bearer(product.agent_connection), plans[1]['alias'], PLAN_PROTOCOLS[plans[1]['plan_id']], sources,
            ('AGENT_MODEL_NOT_GRANTED', 'AGENT_PLAN_NOT_AVAILABLE', 'AGENT_PROTOCOL_UNSUPPORTED'))
        report['native_reads'].append(read_persisted_route(product, binary, persisted_plan_provider(settings, provider, plans[0]['alias']) + '/' + plans[0]['alias'],
            sources[0], oracles[0], 'after-route-adjustment'))
        restore_models(product, context)
        model_active = False
        assert settings.read() == settings.baseline, 'model Restore did not preserve unrelated user edits'
        assert digest(skill) == skill_digest, 'model Restore changed the independent Skill'
        remaining = settings_status(product, context)['collaboration']
        assert remaining['state'] == 'configured' and remaining['restore_point_ref'] == collaboration['restore_point_ref']
        check_collaboration(product, 'skill-survives-model-restore')
        report['cases'].append({'id': CASES[2], 'state': 'green'})
        restore_skill(product, context, remaining['restore_point_ref'], before_skill, skill)
        skill_active = False
        assert not foreign.path.read_text().strip(), 'native defaults or providers unexpectedly received requests'
        assert all(event['state'] == 'green' for source in sources for event in source_events(source))
        assert all(source.request_count() == len(source_events(source)) for source in sources), \
            'an unrecorded rejected request reached the source'
        diagnostics = product.diagnostics_snapshot()
        assert diagnostics['state'] == 'complete' and diagnostics['level_applied']['level'] == 'debug'
        report.update(state='green', diagnostics=diagnostics, persisted_aliases=[p['alias'] for p in plans],
                      native_default_preserved=True, unknown_user_fields_preserved=True, foreign_requests=0,
                      source_requests=[source.request_count() for source in sources], live_requests=2 if HARNESS == 'qoder' else 0,
                      source_context_tokens=100000, source_output_tokens=4096,
                      binaries={name: digest(product.bin / name) for name in ('hiroute', 'hirouted')},
                      harness_sha256=digest(binary))
    except Exception:
        failed = True
        raise
    finally:
        # Restore through production while its daemon is alive. A failed operation
        # never authorizes deleting a managed provider or installed Skill directly.
        for label, active, action in (
            ('restore_model', model_active, lambda: restore_models(product, context)),
            ('restore_skill', skill_active, lambda: restore_skill(product, context,
                settings_status(product, context)['collaboration']['restore_point_ref'], before_skill, skill)),
        ):
            if active:
                try:
                    if product.process is None:
                        product.start()
                    action()
                except Exception as error:
                    cleanup_errors.append({'step': label, 'error_type': type(error).__name__})
        if failed or cleanup_errors:
            report['cleanup_failures'] = list(cleanup_errors)
            try:
                report_failure(product, report, stage)
            except Exception as error:
                cleanup_errors.append({'step': 'preserve_failure_report', 'error_type': type(error).__name__})
        actions = []
        if settings is not None:
            actions.append(('owned_settings_baseline', settings.close))
        # Pi owns its fixture HOME under Product's temporary root; release native
        # leaves before Product.close deletes that root. External Qoder roots
        # follow the same ownership order.
        actions.extend([('owned_skill_directory', lambda: qoder_native_context.cleanup_context(product)),
                        ('close_product', product.close)])
        actions.extend(('close_source', source.close) for source in sources)
        if foreign is not None:
            actions.append(('close_foreign', foreign.close))
        for label, action in actions:
            try:
                action()
            except Exception as error:
                cleanup_errors.append({'step': label, 'error_type': type(error).__name__})
        if cleanup_errors:
            print(json.dumps({'scenario': 'qoder-model-cleanup', 'state': 'red', 'candidate': candidate,
                              'primary_failure_preserved': failed, 'failures': cleanup_errors}), flush=True)
            if not failed:
                raise AssertionError('model acceptance cleanup failed')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
