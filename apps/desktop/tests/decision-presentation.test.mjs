import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decisionFailureHelp, initialAuthMode, protectedInput, sameEndpointOrigin, testKey, decisionReferences } from '../src/features/decision-services/presentation.ts';
import { protocolExamples, protocolCurl } from '../src/features/decision-services/protocol-examples.ts';
import { defaultJudgment, judgmentIssue } from '../src/features/decision-services/types.ts';
import { readFileSync } from 'node:fs';

test('long judgment prompts remain editable and the wire schema has no per-prompt quota', () => {
  const judgment = structuredClone(defaultJudgment);
  judgment.degree.instructions = '决策条件\n'.repeat(5000);
  judgment.competence.criteria[2] = 'Competence standard\n'.repeat(1000);
  assert.equal(judgmentIssue(judgment, 'global', true, 'zh'), null);
  judgment.degree.instructions = '   ';
  assert.match(judgmentIssue(judgment, 'global', true, 'zh').selector, /degree-instructions/);
  const { schemas } = JSON.parse(readFileSync(new URL('../../../decision-extensions/api/decision.openapi.json', import.meta.url), 'utf8')).components;
  assert.equal(schemas.OrdinalDefinition, undefined);
  const category = schemas.Route.oneOf[1].properties.categories.items;
  assert.deepEqual(category.properties.condition, { type: 'string', minLength: 1 });
  assert.equal(category.properties.id.maxLength, 128);
});

test('custom auth does not infer stored schemes, and only new bearer input is prefixed', () => {
  const header = { kind: 'custom', auth_header: { name: 'Authorization', value_secret_ref: 'protected/r1' } };
  assert.equal(initialAuthMode(header), 'header');
  assert.equal(protectedInput(header, 'header', ''), null);
  assert.equal(protectedInput(header, 'header', 'Basic example'), 'Basic example');
  assert.equal(protectedInput(header, 'bearer', 'example'), 'Bearer example');
  assert.equal(protectedInput({ ...header, kind: 'system_one' }, 'bearer', 'example'), 'example');
  assert.equal(protectedInput({ kind: 'custom' }, 'none', 'abandoned'), null);
});
test('credentials are origin scoped and test status is revision scoped', () => {
  assert.equal(sameEndpointOrigin('https://example.test/a', 'https://example.test/b'), true);
  for (const target of ['https://elsewhere.test/b', 'http://example.test/a', 'https://example.test:444/a', 'incomplete']) assert.equal(sameEndpointOrigin('https://example.test/a', target), false);
  assert.notEqual(testKey({ id: 'same', revision: 1 }), testKey({ id: 'same', revision: 2 }));
});
test('references retain each published and draft pin, excluding inactive routing modes', () => {
  const classifier = revision => ({ kind: 'decision_service', service: { id: 'decision/one', revision } });
  const plans = [{ agent_plan_id: 'plan', desired: { display_name: 'Published', strategy: { routing: { classifier: classifier(1) } } } }];
  const drafts = [{ draft_id: 'draft', editor: { display_name: 'Draft', mode: 'custom_branches', branch_routing: { classifier: classifier(2) } } }, { draft_id: 'inactive', editor: { mode: 'fixed_model', branch_routing: { classifier: classifier(2) } } }];
  assert.deepEqual(decisionReferences('decision/one', plans, drafts).map(ref => [ref.key, ref.revision]), [['plan', 1], ['draft', 2]]);
});
test('protocol examples separate the new task decision from its prior assessment target', () => {
  const { first, assessment } = protocolExamples;
  assert.equal(first.request.preceding_stage, null);
  assert.equal(first.response.assessment, undefined);
  assert.equal(first.request.route.kind, 'smart_saving');
  assert.equal(assessment.request.preceding_stage.from, 0);
  assert.equal(assessment.request.visible_conversation[0].branch_id, undefined);
  assert.equal(assessment.response.category, 'review');
  for (const key of ['first', 'assessment']) {
    assert.doesNotMatch(protocolCurl(key), /\n\+/);
    const request = JSON.parse(protocolCurl(key).split("--data-raw '")[1].slice(0, -1));
    assert.deepEqual(Object.keys(request).sort(), ['history_partial', 'latest_user', 'preceding_stage', 'route', 'visible_conversation']);
    assert.deepEqual(request, protocolExamples[key].request);
  }
});

test('decision diagnostics distinguish confirmed causes without exposing arbitrary service text', () => {
  assert.match(decisionFailureHelp('CLASSIFIER_AUTH_REJECTED', 'en'), /Authentication was rejected/);
  assert.match(decisionFailureHelp('CLASSIFIER_ENDPOINT_REJECTED', 'en'), /workspace endpoint/);
  assert.match(decisionFailureHelp('CLASSIFIER_RATE_LIMITED', 'en'), /rate limit/);
  assert.match(decisionFailureHelp('CLASSIFIER_INPUT_REJECTED', 'zh'), /协议兼容性/);
  assert.doesNotMatch(decisionFailureHelp('private upstream details', 'en'), /private upstream details/);
});

import {
  applySourceDefault, chooseMode, chooseProvider, chooseSource, comparable, currentBundle, draftFromRecord, editBundle, expectedSourceRevisions,
  lineageChanged, nativeIssues, nativeService, newNativeDraft, repairActions, resetToSuggested, retainedIndependentAuth, serviceChoiceLabels,
  sourceChoiceLabels, sourceFor, testPresentation, testRecord, displayName, endpointHost,
} from '../src/features/decision-services/form-state.ts';
import { classifierFor, isCustomExtension, isNativeDecision } from '../src/features/decision-services/types.ts';

const contract = JSON.parse(readFileSync(new URL('./decision-services-v2-contract.json', import.meta.url), 'utf8'));
const [legacyView, sharedView] = contract.decision_services_v2.result.services;
const exampleSource = contract.decision_connection_sources.result.sources[0];

test('host warnings compare the same hostname projection as source summaries without relaxing credential origins', () => {
  assert.equal(endpointHost('http://127.0.0.1:50038/v1/decisions'), '127.0.0.1');
  assert.equal(endpointHost('https://API.example.test:8443/v1/decisions'), 'api.example.test');
  assert.equal(endpointHost('http://[::1]:50038/v1/decisions'), '[::1]');
  assert.notEqual(endpointHost('https://other.test/v1/decisions'), 'api.example.test');
  assert.equal(sameEndpointOrigin('https://api.example.test:8443/a', 'https://api.example.test/b'), false);
});
const sourceB = { ...exampleSource, source_id: 'source/b', lineage_digest: 'sha256:b', revision: 3, display_name: 'Second API', endpoint_host: 'second.test', suggestion: { model: 'model-b', endpoint: 'https://second.test/v1/decisions', wire_protocol: 'system_one' } };
const ready = sources => ({ status: 'ready', sources });

test('first successful read defaults the mode only once and never selects an account', () => {
  const fresh = newNativeDraft('decision/new');
  assert.equal(fresh.mode, null);
  const shared = applySourceDefault(fresh, [exampleSource]);
  assert.equal(shared.mode, 'shared');
  assert.equal(shared.sourceId, null);
  assert.equal(applySourceDefault(fresh, []).mode, 'independent');
  assert.equal(applySourceDefault(fresh, [{ ...exampleSource, state: 'disabled' }]).mode, 'independent');
  // Explicit intent made while loading wins over a late result.
  const explicit = chooseMode(fresh, 'independent', null);
  assert.equal(applySourceDefault(explicit, [exampleSource]).mode, 'independent');
  assert.equal(newNativeDraft('decision/compatible', 'compatible').provider, 'compatible');
  assert.equal(currentBundle(newNativeDraft('x', 'compatible'), null).wire_protocol, null);
});

test('each source context keeps its own non-secret bundle and refresh never overwrites it', () => {
  let draft = chooseSource(newNativeDraft('decision/new'), exampleSource, exampleSource.source_id);
  assert.deepEqual(currentBundle(draft, exampleSource), exampleSource.suggestion);
  draft = editBundle(draft, exampleSource, { model: 'edited-a' });
  draft = chooseSource(draft, sourceB, sourceB.source_id);
  assert.equal(currentBundle(draft, sourceB).model, 'model-b');
  draft = chooseSource(draft, exampleSource, exampleSource.source_id);
  const refreshed = { ...exampleSource, suggestion: { ...exampleSource.suggestion, model: 'newer-suggestion' } };
  assert.equal(currentBundle(draft, refreshed).model, 'edited-a');
  assert.equal(currentBundle(resetToSuggested(draft, refreshed), refreshed).model, 'newer-suggestion');
});

test('default names follow model and connection until the user edits the name', () => {
  let draft = chooseSource(newNativeDraft('decision/new'), exampleSource, exampleSource.source_id);
  assert.equal(displayName(draft, exampleSource, 'en'), 'gpt-6-luna · Example API');
  draft = chooseProvider(draft, 'openrouter');
  assert.match(displayName(draft, null, 'en'), /OpenRouter$/);
  draft = { ...draft, name: 'Mine', nameEdited: true };
  assert.equal(displayName(resetToSuggested(draft, null), null, 'en'), 'Mine');
});

test('saved independent references survive mode roundtrips but not provider or origin changes', () => {
  const saved = draftFromRecord(legacyView.service);
  const auth = legacyView.service.connection.auth_header;
  assert.deepEqual(retainedIndependentAuth(saved), auth);
  const roundtrip = chooseMode(chooseMode(saved, 'shared', exampleSource), 'independent', null);
  assert.deepEqual(retainedIndependentAuth(roundtrip), auth);
  assert.equal(retainedIndependentAuth(chooseProvider(saved, 'openrouter')), null);
  assert.equal(retainedIndependentAuth(editBundle(saved, null, { endpoint: 'https://elsewhere.test/systemone' })), null);
  assert.deepEqual(retainedIndependentAuth(editBundle(saved, null, { endpoint: 'https://example.test/other' })), auth);
  assert.equal(nativeIssues(editBundle(saved, null, { endpoint: 'https://elsewhere.test/x' }), null, false, false, 'en').credential, 'Enter an API key.');
});

test('an untouched V1 or V2 record opened in the V2 form is not dirty', () => {
  for (const view of [legacyView, sharedView]) {
    const draft = draftFromRecord(view.service);
    const source = sourceFor(draft, ready([exampleSource]), view);
    assert.equal(comparable(draft, source, 'en'), comparable(structuredClone(draft), source, 'en'));
    assert.deepEqual(nativeIssues(draft, source, false, false, 'en'), {});
    assert.notEqual(comparable(editBundle(draft, source, { model: 'changed' }), source, 'en'), comparable(draft, source, 'en'));
  }
});

test('shared services bind the current lineage, and replaced or missing sources need an explicit choice', () => {
  const draft = draftFromRecord(sharedView.service);
  const service = nativeService(draft, exampleSource, 3, false, 'en');
  assert.equal(service.schema, 'hiroute.decision-service/v2');
  assert.deepEqual(service.connection.authentication, sharedView.service.connection.authentication);
  assert.equal(service.connection.provider, 'openai');
  const replaced = { ...exampleSource, lineage_digest: 'sha256:replaced' };
  assert.equal(lineageChanged(draft, replaced), true);
  assert.match(nativeIssues(draft, replaced, false, false, 'en').source, /replaced/);
  assert.equal(sourceFor(draft, ready([]), sharedView).state, 'missing');
  assert.equal(sourceFor(draft, { status: 'error' }, sharedView).state, 'unavailable');
  assert.match(nativeIssues(draft, sourceFor(draft, { status: 'error' }, sharedView), false, false, 'en').source, /could not be read/);
  assert.equal(lineageChanged(chooseSource(draft, replaced, replaced.source_id), replaced), false);
});

test('unknown codecs need an explicit choice and URL edits keep the chosen codec', () => {
  let draft = newNativeDraft('decision/compatible', 'compatible');
  draft = editBundle(draft, null, { model: 'm', endpoint: 'https://compatible.test/v1' });
  assert.ok(nativeIssues(draft, null, true, false, 'en').wire_protocol);
  draft = editBundle(draft, null, { wire_protocol: 'openai_decisions' });
  draft = editBundle(draft, null, { endpoint: 'https://compatible.test/v2' });
  assert.equal(currentBundle(draft, null).wire_protocol, 'openai_decisions');
  assert.deepEqual(nativeIssues(draft, null, true, false, 'en'), {});
  for (const endpoint of ['ftp://x.test', 'https://user:pass@x.test', 'https://x.test/#a', 'x.test']) assert.ok(nativeIssues(editBundle(draft, null, { endpoint }), null, true, false, 'en').endpoint, endpoint);
});

test('expected source revisions are deduplicated and use the freshest observation', () => {
  const read = ready([{ ...exampleSource, revision: 9 }]);
  assert.deepEqual(expectedSourceRevisions(['source/example', 'source/example', null, 'source/unknown'], read, sharedView), [{ source_id: 'source/example', revision: 9 }]);
  assert.deepEqual(expectedSourceRevisions(['source/example'], { status: 'error' }, sharedView), [{ source_id: 'source/example', revision: 7 }]);
});

test('a previous pass becomes stale when configuration or credential state changes', () => {
  const result = contract.test_decision_service_v2.result;
  const record = testRecord({ ...result, outcome: 'passed', failure_code: null, tested_configuration_digest: sharedView.configuration_digest, tested_credential_state_revision: sharedView.credential_state_revision }, 1);
  assert.equal(testPresentation(record, sharedView, false), 'passed');
  assert.equal(testPresentation(record, { ...sharedView, credential_state_revision: 'sha256:rotated' }, false), 'stale');
  assert.equal(testPresentation(record, { ...sharedView, configuration_digest: 'sha256:edited' }, false), 'stale');
  assert.equal(testPresentation(undefined, sharedView, false), 'untested');
  assert.equal(testPresentation(record, sharedView, true), 'testing');
  assert.equal(testPresentation({ ...record, outcome: 'failed' }, sharedView, false), 'failed');
  // Without a successful readback a pass is never current; a proven change still reads as stale.
  assert.equal(testPresentation(record, sharedView, false, false), 'unverified');
  assert.equal(testPresentation(record, { ...sharedView, credential_state_revision: 'sha256:rotated' }, false, false), 'stale');
  assert.equal(testPresentation({ ...record, outcome: 'failed' }, sharedView, false, false), 'failed');
});

test('repair actions follow the failure fact and credential ownership', () => {
  assert.deepEqual(repairActions('DECISION_SOURCE_DISABLED', true), ['manage_source']);
  assert.deepEqual(repairActions('DECISION_NO_USABLE_CREDENTIALS', true), ['manage_source']);
  assert.deepEqual(repairActions('DECISION_SOURCE_NOT_FOUND', true), ['choose_source']);
  assert.deepEqual(repairActions('DECISION_SOURCE_LINEAGE_MISMATCH', true), ['choose_source']);
  assert.deepEqual(repairActions('DECISION_AUTHENTICATION_REJECTED', true), ['manage_source', 'edit']);
  assert.deepEqual(repairActions('DECISION_AUTHENTICATION_REJECTED', false), ['edit']);
  for (const code of ['DECISION_INVALID_CONFIGURATION', 'DECISION_MALFORMED_OUTPUT']) assert.deepEqual(repairActions(code, false), ['edit']);
  for (const code of ['DECISION_TIMEOUT', 'DECISION_RATE_LIMITED', 'DECISION_TRANSPORT_UNAVAILABLE']) assert.deepEqual(repairActions(code, true), ['retry']);
  assert.deepEqual(repairActions('DECISION_REVISION_CONFLICT', true), ['refresh']);
  for (const code of ['DECISION_AUTHENTICATION_REJECTED', 'DECISION_SOURCE_DISABLED', 'DECISION_TIMEOUT']) {
    assert.doesNotMatch(decisionFailureHelp(code, 'en'), /^Testing failed|DECISION_/);
    assert.notEqual(decisionFailureHelp(code, 'zh'), decisionFailureHelp('unknown', 'zh'));
  }
});

test('same-name choices add model, source and host before a short ID', () => {
  const view = (id, model, sourceId, host) => ({ ...sharedView, service: { ...sharedView.service, id, name: 'Same', connection: { ...sharedView.service.connection, model } }, source: { ...exampleSource, source_id: sourceId, endpoint_host: host } });
  const labels = serviceChoiceLabels([view('d/1', 'a', 'source/one', 'h1'), view('d/2', 'b', 'source/one', 'h1'), { ...legacyView, service: { ...legacyView.service, name: 'Unique' } }], 'en');
  assert.equal(labels.get('d/1'), 'Same · a · Example API');
  assert.equal(labels.get('decision/legacy'), 'Unique');
  const hosts = serviceChoiceLabels([view('d/1', 'a', 'source/one', 'h1'), view('d/2', 'a', 'source/one', 'h2')], 'en');
  assert.equal(hosts.get('d/2'), 'Same · a · Example API · h2');
  const ids = serviceChoiceLabels([view('d/1', 'a', 'source/aaaaaaaa1', 'h1'), view('d/2', 'a', 'source/bbbbbbbb2', 'h1')], 'en');
  assert.match(ids.get('d/2'), /bbbbbbb2$/);
  const sources = sourceChoiceLabels([exampleSource, { ...exampleSource, source_id: 'source/other-one' }], 'en');
  assert.match(sources.get('source/other-one'), /· ther-one — Ready locally$/);
});

test('route classifiers keep V1 and V2 records distinguishable', () => {
  assert.equal(classifierFor(legacyView.service).kind, 'decision_service');
  const v2 = classifierFor(sharedView.service);
  assert.equal(v2.kind, 'decision_service_v2');
  assert.equal(isNativeDecision(v2), true);
  assert.equal(isCustomExtension(v2), false);
  const extension = classifierFor({ ...sharedView.service, connection: { kind: 'custom', endpoint: 'https://x.test', timeout_ms: 1000, auth_header: null } });
  assert.equal(isCustomExtension(extension), true);
  const plans = [{ agent_plan_id: 'plan', desired: { display_name: 'Published', strategy: { routing: { classifier: v2 } } } }];
  assert.deepEqual(decisionReferences(sharedView.service.id, plans, []).map(ref => ref.revision), [2]);
});
