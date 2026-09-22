const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const assert = (condition, message) => { if (!condition) throw new Error(message); };

async function until(predicate, label) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
    await pause(20);
  }
}

export async function runModelToRouteScenarios() {
  const name = 'creating a route from a model selects that model, not the first catalog candidate';
  try {
    await until(() => [...document.querySelectorAll('button')].some(item => item.textContent?.includes('GPT-6-Astra')), 'Astra model row');
    const model = [...document.querySelectorAll('button')].find(item => item.textContent?.includes('GPT-6-Astra'));
    model.click();
    await until(() => [...document.querySelectorAll('button')].some(item => item.textContent?.trim() === '创建智能路由'), 'create route from selected model');
    [...document.querySelectorAll('button')].find(item => item.textContent?.trim() === '创建智能路由').click();
    await until(() => document.querySelector('.route-candidate'), 'new route candidate');
    const bindings = [...document.querySelectorAll('.route-candidate')].map(item => item.dataset.bindingId);
    assert(bindings.length === 1 && bindings[0] === 'binding/codex/gpt-6-astra', `Selected Astra produced route candidates ${bindings.join(', ')}`);
    return { evidence: 'Rendered model detail → route editor with mock IPC; not native/Tauri', tests: 1, passed: 1, failed: 0, results: [{ name, state: 'green' }] };
  } catch (error) {
    return { evidence: 'Rendered model detail → route editor with mock IPC; not native/Tauri', tests: 1, passed: 0, failed: 1, results: [{ name, state: 'red', error: error instanceof Error ? error.message : String(error) }] };
  }
}
