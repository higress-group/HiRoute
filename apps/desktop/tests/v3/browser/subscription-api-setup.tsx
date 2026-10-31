import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { ModelManagementPage } from '../../../src/product/ModelManagementPage';
import { PresentationRoot } from '../../../src/ui';
import { readyManagement, idleModelFeedStatus } from './product-fixtures';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const provider = params.get('provider') === 'claude' ? 'claude' : 'codex';
const language = params.get('lang') === 'en' ? 'en' : 'zh';
const entry = params.get('entry') ?? 'managed';
const optionId = provider === 'codex' ? 'openai.platform.global.v1' : 'anthropic.platform.global.v1';
const ids = [`fixture-${provider}-one`, `fixture-${provider}-two`];
const candidateRef = `candidate/cpa/${provider}/${entry === 'native' ? 'native' : 'managed'}/inline`;
const trace: { command: string; payload: any }[] = [];
const state: any = { mode: params.get('mode') ?? '', management: structuredClone(readyManagement), operations: {}, keys: {}, savedSubscription: false, apiCreated: 0 };
state.management.sources = [];
state.management.subscription_api_links = [];
state.management.subscription_modes = [];
const baseSource = readyManagement.sources[0];
const model = (id: string, index = 0) => ({ ...structuredClone(baseSource.models[0]), upstream_model_id: id, model_ref: `model/${id}`, binding_id: `binding/${provider}/${id}`, display_name: `${provider === 'codex' ? 'GPT' : 'Claude'} fixture model ${index + 1}` });
const makeSource = (id: string, native = false) => ({ ...structuredClone(baseSource), source_id: id, revision: 1, display_name: native ? `${provider === 'codex' ? 'OpenAI' : 'Anthropic'} API` : `${provider === 'codex' ? 'Codex' : 'Claude Code'} subscription`, provenance: native ? 'registered' : 'connector_owned', state: 'ready', keys: [], models: [], actions: ['append_models', 'reauthorize', 'delete'], connection_identity: { access_kind: native ? 'native_api' : 'agent_subscription', connection_option_id: native ? optionId : `${provider}.subscription.global.v1`, product_label: native ? 'API' : 'Subscription' } });
const cpaModels = ids.map((id, index) => ({ upstream_model_id: id, model_ref: `model/${id}`, display_name: model(id, index).display_name, membership: 'observed', selectable: true }));
const pending: any = { candidate: { candidate_ref: candidateRef, candidate_revision: 1 }, correlation: { candidate_ref: candidateRef, edit_revision: 1, check_id: 'check/subscription', input_digest: 'sha256:subscription' }, producer: 'cpa', provenance: 'connector_owned', display_name: `${provider === 'codex' ? 'Codex' : 'Claude Code'} subscription`, models: [], input_state: 'provided', fact_state: 'pending_approval', ...(entry === 'append' ? { existing_source_id: 'source/subscription' } : {}) };
const approval = { operation_id: 'operation/subscription-check', state: 'succeeded', sequence: 1, cancellable: false };
const checked = { ...pending, candidate: { ...pending.candidate, candidate_revision: 2 }, models: cpaModels, fact_state: 'complete', validation: { approval_operation: approval, validation_ref: 'validation/subscription', validation_revision: '1' } };
let subscriptionCheck: any = null;
const catalog = { product_release: 'fixture', catalog_binding_id: 'catalog/fixture', release_sequence: 1, connector_registry_digest: 'sha256:fixture', model_data_digest: 'sha256:fixture', cross_reference_digest: 'sha256:fixture' };
function recordSource(source: any) { state.management.sources.push(source); state.management.revisions.target++; }
if (entry === 'append' || entry === 'saved') { const source = makeSource('source/subscription'); source.models = entry === 'saved' ? ids.map(model) : [model(ids[0])]; recordSource(source); state.savedSubscription = true; }
if (params.has('existing')) { const source = makeSource('source/existing-api', true); source.models = ids.map(model); recordSource(source); }
function subSource() { return state.management.sources.find((source: any) => source.source_id === 'source/subscription'); }
Object.assign(window, { inlineSetup: { state, trace, ids, setMode: (mode: string) => { state.mode = mode; } } });
mockIPC(async (command, args: any = {}) => {
  trace.push({ command, payload: command === 'register_protected_model_input' ? { secret: '[redacted-fixture]' } : structuredClone(args) });
  if (command === 'compute_management_snapshot') return structuredClone(state.management);
  if (command === 'compute_subscriptions') return { discovery_state: 'complete', candidates: args.nativeOnly && entry !== 'native' ? [] : [structuredClone(subscriptionCheck ? checked : pending)] };
  if (command === 'compute_scan') return { connections: [], diagnostics: [] };
  if (command === 'recover_subscription_check') return null;
  if (command === 'manage_subscription_login') return { schema: 'hiroute.subscription-login-result/v1', sessions: [{ provider, login_ref: 'login/fixture', status: 'authorized', candidate: pending.candidate }] };
  if (command === 'check_subscription') { state.management.revisions.target++; subscriptionCheck = { candidate: checked.candidate, approval_operation: approval, status: 'verified', checked_candidate: checked, validation: checked.validation }; return subscriptionCheck; }
  if (command === 'get_subscription_check_result') return subscriptionCheck;
  if (command === 'close_subscription_check' || command === 'cancel_model_connection_check' || command === 'release_protected_model_input') return null;
  if (command === 'model_feed_status') return structuredClone(idleModelFeedStatus);
  if (command === 'compute_connection_options') return { schema: 'hiroute.compute-connection-options/v1', catalog, options: [{ connection_option_id: optionId, display_name: 'Official API', origin: 'native_api', registered_check_available: true }] };
  if (command === 'register_protected_model_input') { if (!state.savedSubscription) throw new Error('API preparation preceded subscription save'); return { input_candidate: { candidate_ref: 'input/api', candidate_revision: 1 } }; }
  if (command === 'preview_native_network_route') return { mode: args.input.policy.mode, route: 'direct', reason: 'policy_direct', destination: 'https://api.example.test', state_revision: 'sha256:network', failure: null };
  if (command === 'check_registered_model_connection_v2') {
    if (!state.savedSubscription) throw new Error('API check preceded subscription save');
    const request = args.request.request;
    const models = state.mode === 'unsupported' ? [] : (state.mode === 'partial' ? cpaModels.slice(0, 1) : cpaModels);
    const check = { candidate: { candidate: { candidate_ref: 'candidate/native-api', candidate_revision: 1 }, correlation: { candidate_ref: 'candidate/native-api', edit_revision: request.edit_revision, check_id: request.check_id, input_digest: 'sha256:api' }, producer: 'native', provenance: 'registered', display_name: request.display_name, models, input_state: 'provided', fact_state: 'complete' }, input_digest: 'sha256:api', authentication: state.mode === 'auth' ? 'rejected' : 'verified', directory: models.length ? 'available' : 'empty', inference: 'not_run', observed_model_ids: models.map((item: any) => item.upstream_model_id) };
    return { check, network: { policy: args.request.network_edit.policy, expected_network_revision: 0, route: { mode: 'direct', route: 'direct', reason: 'policy_direct', destination: 'https://api.example.test', state_revision: 'sha256:network', failure: null } }, failure: state.mode === 'auth' ? { phase: 'upstream_auth', code: 'upstream_auth', retryable: false } : null };
  }
  if (['preview_compute_save', 'preview_compute_save_v3', 'preview_compute_save_v4'].includes(command)) {
    const change = args.change;
    const apiLink = args.apiLink ?? null;
    if (apiLink && state.mode === 'link_failure') throw new Error('fixture association unavailable');
    return { spec: { schema_version: { major: 1, minor: 0 }, command_id: 'compute.connection.apply', desired_state: { change, apiLink } }, accept_digest: 'sha256:preview', expected_revisions: state.management.revisions };
  }
  if (command === 'apply_compute_save') {
    const request = args.request;
    if (state.keys[request.idempotency_key]) return { operation: state.keys[request.idempotency_key].operation };
    const { change, apiLink } = request.spec.desired_state;
    const native = change.subject.kind === 'candidate' && change.subject.candidate.candidate_ref === 'candidate/native-api';
    let source: any;
    if (native) {
      source = makeSource('source/api', true);
      source.models = change.selected_model_refs.map((ref: string) => model(ref.slice(6)));
      recordSource(source); state.apiCreated++;
    } else {
      source = subSource();
      if (!source) { source = makeSource('source/subscription'); recordSource(source); }
      else source.revision++;
      if (change.subject.kind === 'candidate') {
        const wanted = new Set([...source.models.map((item: any) => item.model_ref), ...change.selected_model_refs]);
        source.models = ids.map(model).filter((item: any) => wanted.has(item.model_ref));
        state.savedSubscription = true;
      }
      if (apiLink) state.management.subscription_api_links = apiLink.intent === 'remove' ? [] : [{ source_id: source.source_id, api_source_id: apiLink.api_source_id }];
      state.management.revisions.target++;
      state.management.subscription_modes = [{ source_id: source.source_id, source_revision: source.revision, mode: entry === 'native' ? 'native_borrowed' : 'cpa_managed' }];
    }
    const operation = { operation_id: `operation/save-${Object.keys(state.operations).length + 1}`, state: 'succeeded', sequence: 1, cancellable: false };
    const result = { disposition: 'saved', source_id: source.source_id, saved_revision: source.revision, bindings: [], operation };
    state.operations[operation.operation_id] = result; state.keys[request.idempotency_key] = result;
    if (apiLink) state.linkOperation = operation.operation_id;
    if (native && state.mode === 'lost') throw new Error('fixture lost API reply');
    return { operation };
  }
  if (command === 'get_compute_save_result') {
    const result = args.operation ? state.operations[args.operation.operation_id] : state.keys[args.idempotencyKey];
    if (result?.operation.operation_id === state.linkOperation && state.mode === 'link_pending') return { ...result, disposition: 'pending' };
    return result ?? { disposition: 'failed', reason: 'OPERATION_NOT_FOUND', bindings: [] };
  }
  if (command === 'open_external_url') return null;
  if (command === 'compute_key_masks') return { keys: [] };
  if (command === 'native_source_network') {
    const source = state.management.sources.find((item: any) => item.source_id === args.input.source_id);
    return { source_id: source.source_id, source_revision: source.revision, network_revision: 0, policy: { mode: 'direct' }, applied_revision: source.revision, application_state: 'applied', operation: null, routes: [], models: [], decisions: [] };
  }
  return null;
});
createRoot(document.getElementById('root')!).render(<PresentationRoot language={language} theme={params.get('theme') === 'light' ? 'light' : 'dark'} textScale={1}>
  <div className="app-window"><main className="main" style={{ marginLeft: 0 }}>
    <ModelManagementPage language={language} active trustedAuthority startScanning={entry === 'native'} startAdding={entry === 'managed'} scanOnMount={entry === 'native'} startAppending={entry === 'append'} initialSourceId={entry === 'append' || entry === 'saved' ? 'source/subscription' : null} plans={[]} agents={[]} onOperation={() => {}} onRecoveryRefresh={() => {}} onChanged={() => {}} />
  </main></div>
</PresentationRoot>);
