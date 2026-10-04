// Stable IDs select user capabilities; display names are report copy only.
export function scenario(id, capabilities, name, run) {
  return { id, capabilities, name, run };
}

export function scenarioCatalog(scenarios) {
  return scenarios.map(({ id, capabilities, name }) => ({ id, capabilities, name }));
}

export function selectScenarios(catalog, selection = {}) {
  if (!selection || typeof selection !== 'object' || Array.isArray(selection)) throw new Error('Scenario selection must be an options object');
  const { ids, capability, requiredIds = [] } = selection;
  if (!Array.isArray(requiredIds) || requiredIds.some(id => typeof id !== 'string' || !id)) throw new Error('Required scenario IDs must be a list of IDs');
  if (capability !== undefined && (typeof capability !== 'string' || !capability)) throw new Error('Capability must be a nonempty string');
  if (!Array.isArray(catalog) || catalog.length === 0) throw new Error('Scenario catalog is empty');
  if (ids !== undefined && (!Array.isArray(ids) || ids.length === 0 || new Set(ids).size !== ids.length)) {
    throw new Error('Scenario IDs must be a nonempty unique list');
  }
  const byId = new Map();
  for (const item of catalog) {
    if (!item || typeof item.id !== 'string' || !/^[a-z][a-z0-9.-]+$/.test(item.id) || byId.has(item.id)
      || typeof item.name !== 'string' || !item.name.trim()
      || !Array.isArray(item.capabilities) || item.capabilities.length === 0
      || item.capabilities.some(value => typeof value !== 'string' || !value)) {
      throw new Error(`Invalid or duplicate scenario: ${item?.id}`);
    }
    byId.set(item.id, item);
  }
  for (const id of [...(ids ?? []), ...requiredIds]) {
    if (!byId.has(id)) throw new Error(`Required scenario is missing: ${id}`);
  }
  const selected = catalog.filter(item => (!ids || ids.includes(item.id))
    && (!capability || item.capabilities.includes(capability)));
  if (selected.length === 0) throw new Error('No scenarios selected');
  for (const id of requiredIds) {
    if (!selected.some(item => item.id === id)) throw new Error(`Required scenario was not selected: ${id}`);
  }
  return selected;
}

export function assertScenarioResults(selected, results) {
  if (!Array.isArray(results) || results.length !== selected.length) throw new Error('Not all selected scenarios executed');
  const expected = new Set(selected.map(item => item.id));
  const seen = new Set();
  for (const result of results) {
    if (!expected.has(result.id) || seen.has(result.id) || !['green', 'red'].includes(result.state)) {
      throw new Error(`Unexpected, duplicate or unexecuted scenario result: ${result.id}`);
    }
    seen.add(result.id);
  }
}

export async function runScenarios(scenarios, selection) {
  const selected = selectScenarios(scenarios, selection);
  const results = [];
  for (const { id, capabilities, name, run } of selected) {
    try { await run(); results.push({ id, capabilities, name, state: 'green' }); }
    catch (error) { results.push({ id, capabilities, name, state: 'red', error: error instanceof Error ? error.message : String(error) }); }
  }
  assertScenarioResults(selected, results);
  return { evidence: 'React components + mock IPC only; not native/Tauri and not the real daemon', tests: results.length, passed: results.filter(item => item.state === 'green').length, failed: results.filter(item => item.state === 'red').length, results };
}
