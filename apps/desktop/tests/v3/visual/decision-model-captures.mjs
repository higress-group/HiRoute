import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Production DesktopApp with synthetic decision reads; drives each state through visible controls.
const driver = `(() => {
  const c = window.productShell, zh = localStorage.getItem('hiroute.language') === 'zh';
  const label = (a, b) => zh ? a : b;
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const all = selector => [...document.querySelectorAll(selector)];
  const until = async (check, what) => { for (let i = 0; i < 400; i++) { if (check()) return; await pause(25); } throw new Error('Timed out: ' + what); };
  const button = name => all('button').find(item => item.textContent.trim() === name && item.offsetParent !== null);
  // A return refresh can disable the enclosing fieldset while button.disabled stays false.
  const click = async name => { await until(() => button(name) && !button(name).matches(':disabled'), name); button(name).click(); await pause(60); };
  const field = name => all('.decision-connection-form')[0]?.querySelector('[data-native-field="' + name + '"]');
  const choose = (select, value) => { select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); };
  const sourceId = 'source/openai/decisions';
  const shared = { schema: 'hiroute.decision-service/v2', id: 'decision/team', revision: 1, name: 'Team decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c.decisionSource(sourceId).lineage_digest } } };
  function store(services, sources) {
    const state = { services, sources };
    for (const source of sources) {
      if (c.management.sources.some(item => item.source_id === source.source_id)) continue;
      const native = structuredClone(c.management.sources.find(item => item.authentication.kind === 'bearer'));
      c.management.sources.push({ ...native, source_id: source.source_id, display_name: source.display_name,
      models: native.models.map((model, index) => ({ ...model, model_ref: source.source_id + '/model/' + index, binding_id: source.source_id + '/binding/' + index })),
      keys: native.keys.map((key, index) => ({ ...key, key_id: source.source_id + '/key/' + index, model_statuses: [] })),
        target: { ...native.target, authority: source.endpoint_host } });
    }
    c.handlers.decision_services_v2 = () => ({ services: state.services.map(service => c.decisionView(service, state.sources)) });
    c.handlers.decision_connection_sources = () => ({ sources: structuredClone(state.sources) });
    c.handlers.save_decision_service_v2 = ({ input }) => {
      state.services = state.services.filter(item => item.id !== input.change.id);
      if (input.change.service) state.services.push(structuredClone(input.change.service));
      return { operation_id: 'operation/decision', accepted_digest: 'sha256:' + '5'.repeat(64), state: 'succeeded' };
    };
    c.handlers.test_decision_service_v3 = ({ input }) => c.decisionNetworkTest({ schema: 'hiroute.decision-service-test-result/v2', outcome: 'passed', failure_code: null, duration_millis: 412, tested_configuration_digest: input.expected_configuration_digest, tested_credential_state_revision: c.decisionView(state.services[0], state.sources).credential_state_revision });
    return state;
  }
  const setInput = (element, value) => { Object.getOwnPropertyDescriptor(element.constructor.prototype, 'value').set.call(element, value); element.dispatchEvent(new Event('input', { bubbles: true })); };
  // Route -> decision model -> API connection -> decision model -> route, stopping at the requested step.
  async function nested(step) {
    store([], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
    await until(() => document.querySelector('.sidebar-health .status-dot.good'), 'ready');
    const routeRow = () => all('.master-list .list-row').find(item => item.offsetParent !== null && !item.closest('.decision-models-page'));
    await until(() => { if (routeRow()) return true; button(label('智能路由', 'Smart routing'))?.click(); return false; }, 'routes');
    routeRow().click();
    await until(() => document.querySelector('.plan-identity-fields input'), 'route editor');
    setInput(document.querySelector('.plan-identity-fields input'), label('嵌套路由草稿', 'Nested route draft')); await pause(40);
    await click(label('决策模型', 'Decision model')); await click(label('添加决策模型', 'Add decision model'));
    await until(() => all('.decision-mode')[0]?.getAttribute('aria-pressed') === 'true', 'reuse default');
    choose(field('source'), sourceId); await pause(40);
    setInput(field('model'), 'gpt-6-luna-mini'); setInput(field('name'), label('嵌套决策模型', 'Nested decisions')); await pause(40);
    if (step === 'route') return '.decision-connection-form';
    await click(label('管理 API 接入', 'Manage API connection'));
    await until(() => document.querySelector('.decision-detour') && all('.model-category-tabs').some(item => item.offsetParent !== null), 'detour');
    await until(() => all('.detail-identity').some(item => item.offsetParent !== null && item.textContent.includes('Team OpenAI')), 'selected API connection');
    if (step === 'connection') return '.decision-detour';
    document.querySelector('.decision-detour button').click();
    await until(() => field('name')?.value === label('嵌套决策模型', 'Nested decisions'), 'decision draft after detour');
    if (step === 'return') return '.decision-connection-form';
    await click(label('仅保存', 'Save only'));
    await until(() => document.querySelector('.decision-selector select')?.value, 'route with saved selection');
    return '.decision-selector';
  }
  // A remount can replace the tree after the first ready signal; repeat navigation until the models page holds.
  const open = async () => {
    await until(() => document.querySelector('.sidebar-health .status-dot.good'), 'ready');
    await until(() => { if (button(label('决策模型', 'Decision models'))) return true; button(label('模型', 'Models'))?.click(); return false; }, 'models page');
    await click(label('决策模型', 'Decision models'));
  };
  return {
    async detail() {
      store([structuredClone(shared)], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      await open(); await click(label('测试连接', 'Test connection'));
      await until(() => document.querySelector('.decision-test-result.passed'), 'passed detail');
      return '.decision-models-page .detail-pane';
    },
    async 'removal-entry'() {
      store([structuredClone(shared)], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      await open(); document.querySelector('.decision-removal').scrollIntoView({ block: 'center' });
      return '.decision-removal';
    },
    async removal() {
      store([structuredClone(shared)], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      await open(); await click(label('移除模型', 'Remove model'));
      await until(() => document.querySelector('[role="dialog"]'), 'removal confirmation');
      return '[role="dialog"]';
    },
    async 'removal-blocked'() {
      store([structuredClone(shared)], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      c.handlers.save_decision_service_v2 = () => { throw { code: 'DECISION_IN_USE' }; };
      await open(); await click(label('移除模型', 'Remove model'));
      await until(() => document.querySelector('[role="dialog"] .btn-danger'), 'removal confirmation');
      document.querySelector('[role="dialog"] .btn-danger').click();
      await until(() => document.querySelector('[role="dialog"] .callout.warn'), 'retained reference rejection');
      return '[role="dialog"]';
    },
    async 'shared-edit'() {
      store([structuredClone(shared)], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      await open(); await click(label('编辑', 'Edit'));
      await until(() => field('model'), 'saved shared editor');
      return '.decision-connection-form';
    },
    async shared() {
      store([], [c.decisionSource(sourceId, { display_name: 'Team OpenAI' })]); c.remount();
      await open(); await click(label('添加决策模型', 'Add decision model'));
      await until(() => all('.decision-mode')[0]?.getAttribute('aria-pressed') === 'true', 'reuse default');
      choose(field('source'), sourceId); await pause(60);
      return '.decision-connection-form';
    },
    async independent() {
      store([], [c.decisionSource(sourceId)]); c.remount();
      await open(); await click(label('添加决策模型', 'Add decision model'));
      await until(() => all('.decision-mode')[1], 'mode choice'); all('.decision-mode')[1].click();
      await until(() => field('credential'), 'independent form');
      return '.decision-connection-form';
    },
    async 'source-error'() {
      store([], []); c.handlers.decision_connection_sources = () => { throw { code: 'DAEMON_UNAVAILABLE' }; }; c.remount();
      await open(); await click(label('添加决策模型', 'Add decision model'));
      await until(() => button(label('重试', 'Retry')), 'source read error');
      return '.decision-connection-form';
    },
    async detour() {
      store([structuredClone(shared)], [c.decisionSource(sourceId)]); c.remount();
      await open(); await click(label('编辑', 'Edit'));
      await click(label('管理 API 接入', 'Manage API connection'));
      await until(() => document.querySelector('.decision-detour') && all('.model-category-tabs').some(item => item.offsetParent !== null), 'detour banner over API connection management');
      return '.decision-detour';
    },
    'nested-route': () => nested('route'),
    'nested-connection': () => nested('connection'),
    'nested-return': () => nested('return'),
    'nested-parent': () => nested('parent'),
    async unverified() {
      store([structuredClone(shared)], [c.decisionSource(sourceId)]); c.remount();
      await open(); await click(label('测试连接', 'Test connection'));
      await until(() => document.querySelector('.decision-test-result.passed'), 'passed');
      await click(label('编辑', 'Edit')); await click(label('管理 API 接入', 'Manage API connection'));
      await until(() => document.querySelector('.decision-detour button'), 'detour');
      c.handlers.decision_services_v2 = () => { throw { code: 'DAEMON_UNAVAILABLE' }; };
      document.querySelector('.decision-detour button').click(); await pause(120);
      await click(label('取消', 'Cancel'));
      await until(() => document.querySelector('.decision-test-result[data-test-state="unverified"]'), 'unverified detail');
      return '.decision-test-result';
    },
    async stale() {
      const state = store([structuredClone(shared)], [c.decisionSource(sourceId)]); c.remount();
      await open(); await click(label('测试连接', 'Test connection'));
      await until(() => document.querySelector('.decision-test-result.passed'), 'passed');
      await click(label('编辑', 'Edit')); await click(label('管理 API 接入', 'Manage API connection'));
      await until(() => document.querySelector('.decision-detour button'), 'detour');
      state.sources[0] = { ...state.sources[0], credential_state_revision: 'sha256:' + '9'.repeat(64) };
      document.querySelector('.decision-detour button').click();
      await until(() => document.querySelector('.decision-test-state.stale'), 'stale row');
      await click(label('取消', 'Cancel'));
      await until(() => document.querySelector('.decision-test-result.stale'), 'stale detail');
      return '.decision-test-result';
    },
  };
})()`;

export async function decisionModelCaptures(client, baseUrl, output) {
  const results = [];
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const viewport of [{ width: 1200, height: 813 }, { width: 840, height: 975 }, { width: 755, height: 600 }]) for (const state of ['detail', 'removal-entry', 'removal', 'removal-blocked', 'shared-edit', 'shared', 'independent', 'source-error', 'detour', 'stale', 'unverified', 'nested-route', 'nested-connection', 'nested-return', 'nested-parent']) {
    if (viewport.width === 840 && state !== 'shared-edit') continue;
    await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
    await waitFor(client, 'Boolean(window.productShell)');
    await evaluate(client, `(() => { const c = window.productShell; c.reset(); localStorage.setItem('hiroute.language', ${JSON.stringify(language)}); localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)}); })()`);
    const anchor = await evaluate(client, `${driver}[${JSON.stringify(state)}]()`).catch(async cause => {
      const shot = await client.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(path.join(output, `failed-${state}.png`), Buffer.from(shot.data, 'base64'));
      await writeFile(path.join(output, `failed-${state}.txt`), await evaluate(client, `document.body.innerText`));
      throw new Error(`Decision model capture ${state} (${language}/${theme}/${viewport.width}): ${cause.message}`);
    });
    // Test results and the route selector sit below the fold; centring keeps it clear of the sticky list header.
    if (['stale', 'unverified', 'nested-parent'].includes(state)) await evaluate(client, `document.querySelector(${JSON.stringify(anchor)}).scrollIntoView({ block: 'center' })`);
    await new Promise(resolve => setTimeout(resolve, 120));
    const geometry = await evaluate(client, `(() => {
      const box = document.querySelector(${JSON.stringify(anchor)}), b = box.getBoundingClientRect();
      const clipped = [...box.querySelectorAll('button, input, select, label, p, strong')].filter(item => item.offsetParent !== null && item.scrollWidth > item.clientWidth + 1 && getComputedStyle(item).overflowX !== 'visible' && !['INPUT', 'SELECT'].includes(item.tagName)).map(item => item.textContent.trim().slice(0, 60));
      const leaks = document.body.innerText.includes('sha256:');
      const endpoint = box.querySelector('.decision-endpoint');
      const endpointBox = endpoint?.getBoundingClientRect(), endpointLabel = endpoint?.previousElementSibling?.getBoundingClientRect();
      const footer = box.querySelector('.decision-form-actions')?.getBoundingClientRect();
      const fields = box.querySelector('.decision-form-fields')?.getBoundingClientRect();
      const pane = box.closest('.detail-pane'), split = pane?.parentElement;
      const paneFillsGrid = !pane || !split || pane.getBoundingClientRect().height >= split.getBoundingClientRect().height - 1;
      const removal = box.matches('.decision-removal') ? box.querySelector('button') : box.querySelector('.decision-removal button');
      const removalBox = removal?.getBoundingClientRect();
      return { bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1, overflow: box.scrollWidth > box.clientWidth + 1, width: b.width, clipped, digestVisible: leaks,
        endpointFontSize: endpoint ? parseFloat(getComputedStyle(endpoint).fontSize) : null,
        endpointAligned: !endpointBox || !endpointLabel || Math.abs(endpointBox.left - endpointLabel.left) < 1,
        footerClearance: !footer || !fields || fields.bottom <= footer.top + 1,
        footerVisible: !footer || (footer.top >= 0 && footer.bottom <= innerHeight + 1), paneFillsGrid,
        removalVisible: !!removalBox && removalBox.width > 0 && removalBox.left >= 0 && removalBox.right <= innerWidth + 1 && removalBox.top >= 0 && removalBox.bottom <= innerHeight + 1 };
    })()`);
    if (geometry.bodyOverflow || geometry.overflow || geometry.clipped.length || geometry.digestVisible) throw new Error(`Decision model geometry: ${JSON.stringify({ language, theme, viewport, state, geometry })}`);
    if (state === 'detail' && (geometry.endpointFontSize < 13 || !geometry.endpointAligned)) throw new Error(`Decision endpoint readability: ${JSON.stringify(geometry)}`);
    if (state === 'removal-entry' && !geometry.removalVisible) throw new Error(`The standalone decision removal action cannot be brought into view: ${JSON.stringify(geometry)}`);
    if (['shared', 'shared-edit', 'independent'].includes(state) && (!geometry.footerClearance || !geometry.footerVisible)) throw new Error(`Decision editor footer is covered or outside the viewport: ${JSON.stringify(geometry)}`);
    if (['shared', 'shared-edit', 'independent'].includes(state) && !geometry.paneFillsGrid) throw new Error(`Decision editor is compressed by an extra grid row: ${JSON.stringify({ viewport, geometry })}`);
    const id = `decision-${state}-${language}-${theme}-${viewport.width}`;
    const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
    if (language === 'zh' && theme === 'light' && viewport.width === 1200 && ['detail', 'shared-edit'].includes(state)) {
      // An inert copy of the rendered production DOM/CSS for independent style inspection.
      const capture = await evaluate(client, `(() => {
        const clone = document.documentElement.cloneNode(true), assets = [];
        clone.querySelectorAll('script').forEach(node => node.remove());
        clone.querySelectorAll('input').forEach((node, index) => {
          const original = document.querySelectorAll('input')[index];
          if (node.type !== 'password') node.setAttribute('value', original.value);
        });
        clone.querySelectorAll('select').forEach((node, index) => [...node.options].forEach((option, i) => option.toggleAttribute('selected', i === document.querySelectorAll('select')[index].selectedIndex)));
        clone.querySelectorAll('img').forEach((node, index) => {
          const url = document.querySelectorAll('img')[index].src;
          if (url.startsWith('data:')) return;
          assets.push(url); node.setAttribute('src', 'capture-asset-' + (assets.length - 1));
        });
        return { html: '<!doctype html>' + clone.outerHTML, assets };
      })()`);
      for (const [index, url] of capture.assets.entries()) {
        if (new URL(url).origin !== new URL(baseUrl).origin) throw new Error('Unexpected external capture asset');
        const response = await fetch(url);
        if (!response.ok) throw new Error(`Capture asset failed: ${response.status}`);
        const asset = `data:${response.headers.get('content-type')};base64,${Buffer.from(await response.arrayBuffer()).toString('base64')}`;
        capture.html = capture.html.replace(`capture-asset-${index}`, asset);
      }
      await writeFile(path.join(output, `${id}.html`), capture.html);
    }
    results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
  }
  return results;
}
