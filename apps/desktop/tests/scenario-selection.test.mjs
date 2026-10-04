import assert from 'node:assert/strict';
import test from 'node:test';
import { selectScenarios, assertScenarioResults, runScenarios } from './v3/browser/scenario-selection.mjs';
import { agentTrustScenarioCatalog } from './v3/browser/agent-trust-scenarios.mjs';
import { routingWorkerScenarioCatalog } from './v3/browser/routing-worker-scenarios.mjs';
import { productShellScenarioCatalog } from './v3/browser/product-shell-scenarios.mjs';
import { agentTrustRequired, routingWorkerRequired, productShellRequired, focusedRequirements } from './v3/browser/scenario-requirements.mjs';

const catalogs = [
  [agentTrustScenarioCatalog, agentTrustRequired],
  [routingWorkerScenarioCatalog, routingWorkerRequired],
  [productShellScenarioCatalog, productShellRequired],
];

test('full mode discovers every current scenario and checks the independent required set', () => {
  for (const [catalog, requiredIds] of catalogs) {
    assert.deepEqual(selectScenarios(catalog, { requiredIds }), catalog);
    const extra = { id: 'future.capability', name: 'A newly added scenario', capabilities: ['future'] };
    assert.equal(selectScenarios([...catalog, extra], { requiredIds }).at(-1), extra);
    for (const missing of requiredIds) {
      assert.throws(() => selectScenarios(catalog.filter(item => item.id !== missing), { requiredIds }), /Required scenario is missing/);
    }
  }
});

test('focused capabilities survive reordering and renamed display copy', () => {
  const all = catalogs.flatMap(([catalog]) => catalog);
  for (const [capability, requiredIds] of Object.entries(focusedRequirements)) {
    const reordered = all.toReversed().map(item => ({ ...item, name: 'Reworded user-facing scenario' }));
    const actual = selectScenarios(reordered, { capability, requiredIds }).map(item => item.id);
    assert.deepEqual(actual.toSorted(), requiredIds.toSorted());
    assert.throws(() => selectScenarios(reordered, { capability: 'other', requiredIds }), /No scenarios selected/);
  }
});

test('empty, unknown, duplicate and silently excluded selections fail before execution', () => {
  const catalog = agentTrustScenarioCatalog;
  assert.throws(() => selectScenarios(catalog, 3), /options object/);
  assert.throws(() => selectScenarios(catalog, { requiredIds: 'not-a-list' }), /must be a list/);
  assert.throws(() => selectScenarios(catalog, { capability: '' }), /nonempty string/);
  assert.throws(() => selectScenarios([]), /catalog is empty/);
  assert.throws(() => selectScenarios(catalog, { ids: [] }), /nonempty unique/);
  assert.throws(() => selectScenarios(catalog, { ids: ['missing.scenario'] }), /missing/);
  assert.throws(() => selectScenarios([...catalog, catalog[0]]), /duplicate scenario/);
  for (const id of [undefined, null, { toString: () => 'coerced.id' }]) {
    assert.throws(() => selectScenarios([{ ...catalog[0], id }]), /Invalid or duplicate scenario/);
  }
  assert.throws(() => selectScenarios(catalog, { ids: [catalog[0].id, catalog[0].id] }), /nonempty unique/);
  assert.throws(() => selectScenarios(catalog, { ids: [catalog[0].id], requiredIds: [catalog[1].id] }), /not selected/);
});

test('count equality alone cannot hide duplicate, missing, foreign or unexecuted results', () => {
  const selected = agentTrustScenarioCatalog.slice(0, 2);
  const green = selected.map(({ id }) => ({ id, state: 'green' }));
  assert.doesNotThrow(() => assertScenarioResults(selected, green));
  for (const results of [[], [green[0], green[0]], [green[0], { id: 'foreign.scenario', state: 'green' }],
    [green[0], { ...green[1], state: 'not-executed' }]]) {
    assert.throws(() => assertScenarioResults(selected, results));
  }
});

test('execution reports each selected ID and preserves red outcomes', async () => {
  const fixture = [
    { id: 'fixture.pass', name: 'Pass', capabilities: ['fixture'], run: async () => {} },
    { id: 'fixture.fail', name: 'Fail', capabilities: ['fixture'], run: async () => { throw new Error('expected fixture failure'); } },
  ];
  const report = await runScenarios(fixture);
  assert.equal(report.tests, 2);
  assert.equal(report.passed, 1);
  assert.equal(report.failed, 1);
  assert.deepEqual(report.results.map(item => item.id), fixture.map(item => item.id));
  await assert.rejects(runScenarios(fixture, { ids: [] }), /nonempty unique/);
});
