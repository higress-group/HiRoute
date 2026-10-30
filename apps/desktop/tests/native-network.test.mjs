import test from 'node:test';
import assert from 'node:assert/strict';
import { networkDraft, networkEdit, networkPolicyEqual, networkRepairable, networkFailureText, manualProxyError, reconcileNetworkDraft } from '../src/features/model-connections/native-network.ts';
import { ConnectionEditingSession } from '../src/features/model-connections/connection-session.ts';
import { buildCheckDraft } from '../src/features/model-connections/state.ts';

const draft = () => ({ entry_kind: 'custom_api', display_name: 'Team', base_url: 'https://origin.invalid', base_kind: 'api_root', request_path_override: null, inventory_path_override: null, protocol: 'responses', authentication: { kind: 'bearer' }, models: [], additional_endpoints: [], provenance: { kind: 'user_configured', configuration_revision: 1 }, network: networkDraft() });

test('inactive proxy input is retained in the session and omitted from direct/inherit wire payloads', () => {
  const value = { ...networkDraft(), mode: 'manual', proxy_url: 'http://proxy.invalid:8080', no_proxy: 'private.invalid' };
  const direct = { ...value, mode: 'direct' }, inherit = { ...value, mode: 'inherit' };
  assert.deepEqual(networkEdit(direct).policy, { mode: 'direct' });
  assert.deepEqual(networkEdit(inherit).policy, { mode: 'inherit' });
  assert.equal(networkEdit({ ...direct, mode: 'manual' }).policy.proxy_url, value.proxy_url);
  assert.equal(value.no_proxy, 'private.invalid');
});

test('source prefill is pristine, route edits are dirty, and parked credentials stay owned by their source context', () => {
  const session = new ConnectionEditingSession(), initial = draft();
  session.begin('source', initial);
  const configured = networkDraft({ mode: 'manual', proxy_url: 'http://proxy.invalid:8080/', no_proxy: '' }, 4);
  session.applyNetworkBaseline('source', configured);
  const loaded = { ...initial, network: configured };
  assert.equal(session.changed('source', loaded), false);
  const changed = { ...loaded, network: { ...configured, no_proxy: 'private.invalid' } };
  assert.equal(session.changed('source', changed), true);
  const input = { candidate_ref: 'protected/ref', candidate_revision: 1 };
  session.stash('source', { draft: changed, custom: true, metadataPrefillKey: '', secret: 'retained-key', protectedInput: input, credentialReentry: false, selection: null });
  const restored = session.take('source');
  assert.equal(restored.secret, 'retained-key');
  assert.deepEqual(restored.protectedInput, input);
  assert.equal(restored.draft.provenance.kind, 'user_configured');
  assert.equal(restored.draft.network.no_proxy, 'private.invalid');
  assert.equal('network' in buildCheckDraft(restored.draft), false);
});

test('proxy authentication failures offer network repair without provider credential blame', () => {
  const failure = { phase: 'proxy_auth', code: 'opaque', retryable: true };
  assert.equal(networkRepairable(failure), true);
  assert.match(networkFailureText(failure, false), /proxy requires authentication/);
  assert.doesNotMatch(networkFailureText(failure, false), /rejected authentication|re-enter/);
  assert.equal(networkRepairable({ ...failure, phase: 'upstream_auth' }), false);
  assert.match(networkFailureText({ ...failure, phase: 'transport_unknown' }, false), /unknown/);
});

test('manual URL validation rejects credentials and endpoint suffixes; equivalent normalized policies do not add an impact step', () => {
  for (const proxy_url of ['http://user:secret@proxy.invalid', 'http://@proxy.invalid', 'socks5://proxy.invalid', 'https://proxy.invalid/api', 'http://proxy.invalid?key=secret']) assert.equal(manualProxyError({ ...networkDraft(), mode: 'manual', proxy_url }), 'proxy_url');
  assert.equal(manualProxyError({ ...networkDraft(), mode: 'manual', proxy_url: 'https://proxy.invalid:8443' }), null);
  assert.equal(networkPolicyEqual({ mode: 'manual', proxy_url: 'http://proxy.invalid:80', no_proxy: ' private.invalid, ' }, { mode: 'manual', proxy_url: 'http://proxy.invalid/', no_proxy: 'private.invalid' }), true);
});

test('refresh after a source conflict retains policy edits and rebases the next guarded preview', () => {
  const before = { source_id: 'source/a', policy: { mode: 'direct' } };
  const current = { source_id: 'source/a', policy: { mode: 'manual', proxy_url: 'https://peer.invalid:8443/', no_proxy: 'peer.invalid' }, network_revision: 8 };
  const edited = { ...networkDraft(), mode: 'manual', proxy_url: 'http://local.invalid:7890', no_proxy: 'local.invalid', expected_network_revision: 3 };
  const refreshed = reconcileNetworkDraft(before, current, edited);
  assert.deepEqual(refreshed, { ...edited, expected_network_revision: 8 });
  assert.deepEqual(networkEdit(refreshed), { expected_network_revision: 8, policy: { mode: 'manual', proxy_url: edited.proxy_url, no_proxy: edited.no_proxy } });
  assert.equal(networkPolicyEqual(current.policy, networkEdit(refreshed).policy), false);
});

test('refresh adopts external changes for a pristine draft while retaining inactive manual input', () => {
  const before = { source_id: 'source/a', policy: { mode: 'direct' } };
  const parked = { ...networkDraft(), proxy_url: 'unfinished manual input', no_proxy: 'saved.local' };
  const refreshed = reconcileNetworkDraft(before, { ...before, policy: { mode: 'inherit' }, network_revision: 4 }, parked);
  assert.deepEqual(refreshed, { ...parked, mode: 'inherit', expected_network_revision: 4 });
  assert.deepEqual(networkEdit(refreshed).policy, { mode: 'inherit' });
  const unchanged = reconcileNetworkDraft(before, { ...before, network_revision: 5 }, parked);
  assert.equal(unchanged.proxy_url, parked.proxy_url);
});

test('refresh of a different source never transfers another source policy draft', () => {
  const before = { source_id: 'source/a', policy: { mode: 'direct' } };
  const current = { source_id: 'source/b', policy: { mode: 'manual', proxy_url: 'https://new.invalid/', no_proxy: '' }, network_revision: 2 };
  assert.deepEqual(reconcileNetworkDraft(before, current, { ...networkDraft(), mode: 'manual', proxy_url: 'http://old.invalid/' }), networkDraft(current.policy, 2));
});

const { networkTestRecord, testPresentation } = await import('../src/features/decision-services/form-state.ts');
const { readFileSync } = await import('node:fs');
const decisionContract = JSON.parse(readFileSync(new URL('./decision-services-v2-contract.json', import.meta.url), 'utf8'));
const shared = decisionContract.decision_services_v2.result.services[1];
const checkedDecision = () => ({ schema: 'hiroute.decision-service-test-result/v3', check: {
  schema: 'hiroute.decision-service-test-result/v2', outcome: 'passed', failure_code: null, duration_millis: 4,
  tested_configuration_digest: shared.configuration_digest, tested_credential_state_revision: shared.credential_state_revision,
}, network: { policy: { mode: 'direct' }, expected_network_revision: 2, route: {
  mode: 'direct', route: 'direct', reason: 'policy_direct', destination: 'https://decision.invalid:443',
  state_revision: 'network/2/epoch/1', failure: null,
} }, failure: null });

test('decision passes are invalidated by a source route or service environment change without altering credential identity', () => {
  const checked = checkedDecision(), record = networkTestRecord(checked, 100);
  assert.equal(testPresentation(record, shared, false, true, checked.network.route), 'passed');
  assert.equal(testPresentation(record, shared, false, true, { ...checked.network.route, state_revision: 'network/3/epoch/1' }), 'stale');
  assert.equal(testPresentation(record, shared, false, true, { ...checked.network.route, state_revision: 'network/2/epoch/2' }), 'stale');
  assert.equal(record.tested_credential_state_revision, shared.credential_state_revision);
});

test('unreadable shared network state cannot retain a current pass, while independent decisions need no Native policy', () => {
  const checked = checkedDecision(), record = networkTestRecord(checked, 100);
  assert.equal(testPresentation(record, shared, false, true, null), 'unverified');
  assert.equal(testPresentation(record, shared, false, false, checked.network.route), 'unverified');
  const independent = structuredClone(shared);
  independent.service.connection.authentication = { kind: 'independent', auth_header: { name: 'Authorization', value_secret_ref: 'fixture/ref' } };
  assert.equal(testPresentation(networkTestRecord({ ...checked, network: null }, 100), independent, false), 'passed');
});

test('decision network failures retain a typed proxy repair cause alongside the saved configuration result', () => {
  const checked = checkedDecision();
  checked.check.outcome = 'failed'; checked.check.failure_code = 'DECISION_TRANSPORT_UNAVAILABLE';
  checked.failure = { phase: 'proxy_auth', code: 'NATIVE_DECISION_NETWORK_FAILED', retryable: false };
  const record = networkTestRecord(checked, 100);
  assert.equal(record.code, 'DECISION_TRANSPORT_UNAVAILABLE');
  assert.equal(record.network_failure.phase, 'proxy_auth');
  assert.equal(networkRepairable(record.network_failure), true);
  assert.equal(record.tested_configuration_digest, shared.configuration_digest);
});
