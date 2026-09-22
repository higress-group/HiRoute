"""Operation recovery assertions over the real CLI, daemon and Gateway HTTP catalog."""
import hashlib
import json
import sys
from publication_product import Product, encoded


def plan_change(product, intent, creation_key=None, **editor_changes):
    target = {'intent': intent, 'creation_key': creation_key} if intent == 'create' else {
        'intent': 'update', 'plan_id': product.plan_id,
        'expected_head_revision': product.cli('routing show ' + product.plan_id)[1]['data']['head']['head_revision']}
    return {'schema': 'hiroute.plan-content-change/v2', 'target': target,
            'editor': dict(product.editor, **editor_changes), 'consumed_draft': None}


def ensure_native_model_evidence(product, agent_id):
    """Refresh process-local native ingress evidence without making an upstream model call."""
    checked = getattr(product, 'native_model_checked_agents', set())
    if agent_id in checked:
        return
    check = {'agent_id': agent_id, 'scope': 'native_authentication',
             'suite': 'quick', 'allow_model_call': False}
    revisions = product.preview(
        'routing preview', {'change': plan_change(product, 'update')})['expected_revisions']
    consent = {'change_digest': 'sha256:' + hashlib.sha256(encoded(check)).hexdigest(),
               'expected_revisions': revisions}
    sequence = getattr(product, 'native_model_check_sequence', 0) + 1
    product.native_model_check_sequence = sequence
    capability = product.grant(
        'CheckAgentConnection', consent, 'native-model-check-' + str(sequence))
    product.cli('agents check ' + agent_id + ' --scope native-authentication',
                capability=capability)
    checked.add(agent_id)
    product.native_model_checked_agents = checked


def configure_model_settings_v2(product, allowed_plan_ids, key, default_plan_id=None,
                                agent_id='agent_codex_default',
                                native_model_mode='hiroute_only'):
    """Use the current typed settings transaction through same-user Local Control."""
    context = getattr(product, 'agent_context_id', None)
    if context is None:
        scan = product.preview('agents scan')
        context = next(agent['context_id'] for agent in scan['agents']
                       if agent['agent_id'] == agent_id)
        product.agent_context_id = context
    product.agent_settings_agent_id = agent_id
    product.agent_connection = 'agent-connection/' + context
    ensure_native_model_evidence(product, agent_id)
    spec = model_settings_spec_v2(product, allowed_plan_ids, default_plan_id,
                                  native_model_mode)
    preview, body, capability = prepare_agent_settings_v2(product, spec, key)
    _, applied = product.cli('agents connect apply', body, capability)
    if applied['data']['state'] != 'succeeded':
        operation = product.cli(
            'operations get ' + applied['data']['operation_id'])[1]
        raise AssertionError({'apply': applied, 'operation': operation})
    product.agent_settings_path = (
        product.codex_settings if agent_id == 'agent_codex_default' else product.settings)
    status = product.cli('agents connect status ' + context)[1]
    assert status['status'] == 'succeeded', status
    assert status['data']['current_selection'] == preview['spec']['model']['settings'], status
    # Desktop seeds every later full-state edit from this authoritative projection. Keep the
    # normalized spec (including initially adopted native models), not the caller's sparse input.
    product.agent_settings_spec = preview['spec']
    return applied


def model_settings_spec_v2(product, allowed_plan_ids, default_plan_id=None,
                           native_model_mode='hiroute_only'):
    default_plan_id = default_plan_id or allowed_plan_ids[0]
    agent_id = getattr(product, 'agent_settings_agent_id', 'agent_codex_default')
    if agent_id == 'agent_claude_default':
        ordered = [default_plan_id] + [
            plan_id for plan_id in allowed_plan_ids if plan_id != default_plan_id]
        assert len(ordered) <= 3, 'Claude exposes exactly three managed launcher presets'
        presets = {}
        for index, preset in enumerate(('opus', 'sonnet', 'haiku')):
            presets[preset] = ({'kind': 'plan', 'plan_id': ordered[index]}
                               if index < len(ordered)
                               else {'kind': 'preserve_native'})
        return {
            'schema_version': {'major': 2, 'minor': 0},
            'context_id': product.agent_context_id,
            'model': {'intent': 'configure', 'settings': {
                'mode': 'claude_launcher',
                'surfaces': ['claude_cli'],
                'fixed_models': [],
                'preset_mappings': presets,
            }},
        }
    previous = getattr(product, 'agent_settings_spec', None)
    fixed_models = []
    if (previous is not None
            and previous.get('context_id') == product.agent_context_id
            and previous.get('model', {}).get('intent') == 'configure'
            and previous['model'].get('settings', {}).get('mode') == 'codex_default'):
        fixed_models = previous['model']['settings']['fixed_models']
    return {
        'schema_version': {'major': 2, 'minor': 0},
        'context_id': product.agent_context_id,
        'model': {'intent': 'configure', 'settings': {
            'mode': 'codex_default',
            'native_model_mode': native_model_mode,
            'fixed_models': fixed_models,
            'allowed_plan_ids': allowed_plan_ids,
            'default_selection': {'kind': 'plan', 'plan_id': default_plan_id},
        }},
    }


def prepare_agent_settings_v2(product, spec, key):
    agent_id = getattr(product, 'agent_settings_agent_id', None)
    if agent_id is not None:
        ensure_native_model_evidence(product, agent_id)
    preview = product.preview('agents connect preview', {'spec': spec})
    assert preview['applicable'], preview.get('blockers')
    body = {
        # Seal the exact normalized full-state selection shown by Preview. This mirrors the
        # Desktop confirmation payload and prevents a sparse fixture input from dropping models.
        'spec': preview['spec'],
        'accept_digest': preview['accept_digest'],
        'dependency_digest': preview['dependency_digest'],
        'expected_revisions': preview['expected_revisions'],
        'idempotency_key': key,
    }
    # This harness plays the Desktop host: the first resident-service connection carries the
    # host's login-item declaration exactly as the native confirmation flow would provide it.
    if preview.get('resident_service', {}).get('login_item_required'):
        body['login_item'] = {
            'before': 'not_registered', 'after': 'enabled', 'created': True}
    return preview, body, None


def prepare_control_apply(product, operation, preview, key):
    body = {
        'spec': preview['spec'],
        'accept_digest': preview['accept_digest'],
        'expected_revisions': preview['expected_revisions'],
        'idempotency_key': key,
    }
    capability = (product.desktop_grant(
        operation, preview['accept_digest'], preview['expected_revisions'], key)
        if operation == 'ApplySubscriptionCheck' else None)
    return body, capability


def apply_control(product, operation, preview, key):
    body, capability = prepare_control_apply(product, operation, preview, key)
    applied = product.control(operation, body, capability)
    assert applied['data']['state'] == 'succeeded', applied
    return applied


def save_compute_candidate(product, candidate, key, validation=None):
    snapshot = product.control('ListCompute', {})['data']
    selectable = [model['model_ref'] for model in candidate['models']
                  if model['selectable']]
    assert selectable, candidate
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': selectable,
        'intent': 'save_ready',
        'key_edits': [],
    }
    if validation is not None:
        change['validation'] = validation
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    applied = apply_control(product, 'ApplyComputeSave', preview, key)
    saved = product.control(
        'GetComputeSaveResult', {'operation': applied['operation']})['data']
    assert saved['disposition'] == 'saved', saved
    assert saved['management_state'] == 'ready', saved
    assert saved['source_id'] and saved['saved_revision'] == 1, saved
    assert saved['bindings'], saved
    return saved


def prepare_discovered_source(product):
    item = next(item for item in product.preview('compute scan')['items']
                if item['inventory_eligible'] and item.get('discovery'))
    request = {
        'discovery': item['discovery'],
        'prepare_id': 'prepare/publication-product/source',
    }
    revisions = product.control('GetClientServiceStatus', {})['data']['revisions']
    digest = 'sha256:' + hashlib.sha256(encoded(request)).hexdigest()
    capability = product.desktop_grant(
        'PrepareDiscoveredModelConnection', digest, revisions,
        'prepare-discovered-source')
    candidate = product.control(
        'PrepareDiscoveredModelConnection', request, capability)['data']
    assert candidate['provenance'] == 'registered', candidate
    assert candidate['fact_state'] == 'complete', candidate
    return candidate, None


def prepare_subscription_source(product):
    candidates = product.control('ListComputeSubscriptions', {})['data']
    assert candidates['discovery_state'] == 'complete', candidates
    pending = next(candidate for candidate in candidates['candidates']
                   if candidate['provenance'] == 'connector_owned')
    preview = product.control(
        'PreviewSubscriptionCheck', {'candidate': pending['candidate']})['data']
    applied = apply_control(
        product, 'ApplySubscriptionCheck', preview, 'subscription-check')
    checked = product.control(
        'GetSubscriptionCheckResult', {'operation': applied['operation']})['data']
    assert checked['status'] == 'verified', checked
    assert checked['checked_candidate']['fact_state'] == 'complete', checked
    return checked['checked_candidate'], checked['validation']


def prepare_saved_source_update(product, key):
    snapshot = product.control(
        'ListCompute', {'source_id': product.source_id})['data']
    assert len(snapshot['sources']) == 1, snapshot
    source = snapshot['sources'][0]
    change = {
        'schema': 'hiroute.compute-management-change/v2',
        'subject': {'kind': 'saved_source', 'source_id': product.source_id},
        'expected_revisions': snapshot['revisions'],
        'selected_model_refs': [product.binding['model_ref']],
        'intent': 'save_ready',
        'key_edits': [],
    }
    preview = product.control('PreviewComputeSave', {'change': change})['data']
    body, capability = prepare_control_apply(
        product, 'ApplyComputeSave', preview, key)
    return body, capability


def bootstrap(product):
    if not getattr(product, 'collaboration_only', False):
        # Default production fixtures use the current V2 settings transaction and an isolated
        # Codex native-ingress probe. The probe reaches only the daemon loopback challenge and
        # never uses a developer's daily Agent config or a paid upstream.
        product.install_codex_fixture()
    product.start()
    if product.cpa_args:
        candidate, validation = prepare_subscription_source(product)
    else:
        candidate, validation = prepare_discovered_source(product)
    saved = save_compute_candidate(product, candidate, 'source-save', validation)
    identity = dict(saved['bindings'][0], source_id=saved['source_id'],
                    source_revision=saved['saved_revision'])
    product.saved_source = saved
    product.source_id = saved['source_id']
    product.binding = identity
    if product.project_source:
        # Importing a project source does not authorize HiRoute's user-level Agent settings
        # to override that project's native auth. The fixture owner removes its project
        # override before explicitly enabling the ordinary Claude connection.
        product.project_settings.unlink()
    selection = {'binding_id': identity['binding_id']}
    if product.cpa_args:
        selection['reasoning'] = {'kind': 'profile', 'profile': 'low'}
    product.editor = {'schema': 'hiroute.plan-editor/v2', 'display_name': '\u65e5\u5e38\u7f16\u7801',
                      'purpose': 'Production recovery', 'mode': 'fixed_model', 'candidates': [selection],
                      'smart': {'economy': [], 'primary': [], 'primary_fallback': False, 'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
                      'free': {'candidates': [], 'primary': [], 'primary_fallback': False},
                      'delegation_enabled': False,
                      'requirements': {}, 'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000, 'attempt_timeout_ms': 30000}}
    change = plan_change(product, 'create', 'production-editor')
    rp = product.preview('routing preview', {'change': change})
    product.apply('routing apply', 'ApplyAgentPlanChange', rp, {'change': change}, 'plan')
    product.plan_id = rp['plan_head']['reference']['plan_id']
    product.model_alias = rp['plan_head']['model_alias']
    if getattr(product, 'collaboration_only', False):
        return
    configure_model_settings_v2(
        product, [product.plan_id], 'agent-model',
        agent_id=getattr(
            product,
            'model_settings_agent_id',
            'agent_codex_default' if product.cpa_args else 'agent_claude_default'))
    product.catalog()


def recovery(repository, boundary):
    product = Product(repository)
    try:
        bootstrap(product)
        before, old_etag = product.catalog()
        product.stop()
        product.start(boundary)
        change = plan_change(product, 'update', display_name='Updated MVP plan')
        preview = product.preview('routing preview', {'change': change})
        _, body, cap = product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                                     {'change': change}, 'interrupted-plan', crash=True)
        product.stop(crash=True)
        product.start()
        _, replay = product.cli('routing apply', body, cap)
        assert replay['data']['state'] == 'succeeded', replay
        _, again = product.cli('routing apply', body, cap)
        assert again['data'] == replay['data'], 'replay created another Operation'
        operation = product.cli('operations get ' + replay['data']['operation_id'])[1]
        assert operation['data']['state'] == 'succeeded', operation
        after, new_etag = product.catalog()
        assert [x['id'] for x in before['data']] == [x['id'] for x in after['data']], 'alias changed'
        assert old_etag and new_etag and old_etag != new_etag, 'catalog did not advance publication'
        print(json.dumps({'scenario': boundary, 'state': 'green', 'daemon_fault_exit': 86,
                          'replay_cli_exit': 0, 'operation_id': replay['data']['operation_id']}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    for boundary in ('after_prepare', 'before_install', 'after_gateway_durable',
                     'after_target', 'after_active', 'after_terminal'):
        recovery(sys.argv[1], boundary)
