import { test } from 'node:test';
import assert from 'node:assert/strict';
import { AGENT_DISCOVERY_STORAGE_KEY, loadAgentDiscoveryTargets, saveAgentDiscoveryTargets, agentDiscoveryState } from '../src/features/agents/discovery.ts';
import { agentPageEntries } from '../src/features/agents/ecosystems.ts';

test('custom targets survive a new reader but confer no discovered context or write authority', () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const target = { kind: 'hermes', config_root: '/selected/custom-profile' };
  assert.equal(saveAgentDiscoveryTargets([target], storage), true);
  assert.deepEqual(loadAgentDiscoveryTargets(storage), [target]);
  const rows = agentPageEntries([]);
  assert.equal(rows.length, 8);
  assert.ok(rows.some(row => row.agent_id === "agent_opencode_default"));
  assert.ok(rows.some(row => row.agent_id === "agent_hermes_default"));
  for (const row of rows) {
    assert.equal(agentDiscoveryState(row), 'not_discovered');
    assert.equal(row.context_id, null); assert.equal(row.settings, null);
  }
});

test('damaged or unsafe discovery preferences cannot become targets and denied storage does not prevent startup', () => {
  for (const value of ['invalid json', JSON.stringify({ version: 2, targets: [] }),
    JSON.stringify({ version: 1, targets: [{ kind: 'hermes', config_root: 'relative' }] }),
    JSON.stringify({ version: 1, targets: [{ kind: 'hermes', config_root: '/root/../other' }] })]) {
    assert.deepEqual(loadAgentDiscoveryTargets({ getItem: () => value }), []);
  }
  const denied = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
  assert.deepEqual(loadAgentDiscoveryTargets(denied), []);
  assert.equal(saveAgentDiscoveryTargets([{ kind: 'hermes', config_root: '/selected' }], denied), false);
});

test('an unreadable selected target remains distinct from absence and never selects another profile', () => {
  const target = { kind: 'hermes', config_root: '/selected/unavailable' };
  const selected = { agent_id: 'agent_hermes_default', context_id: null, native_target: target, settings: null,
    configuration_state: 'profile_unavailable', status_error: 'AGENT_STATUS_UNAVAILABLE' };
  const rows = agentPageEntries([selected]);
  assert.equal(rows.filter(row => row.agent_id === selected.agent_id).length, 1);
  assert.equal(agentDiscoveryState(rows[0]), 'unavailable');
  assert.deepEqual(rows[0].native_target, target);
});
