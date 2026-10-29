import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Production DesktopApp with synthetic decision reads; drives each state through visible controls.
const driver = `(() => {
  const c = window.productShell, zh = localStorage.getItem('hiroute.language') === 'zh';
  const label = (a, b) => zh ? a : b;
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const all = selector => [...document.querySelectorAll(selector)];
  const until = async (check, what) => { for (let i = 0; i < 200; i++) { if (check()) return; await pause(25); } throw new Error('Timed out: ' + what); };
  const button = name => all('button').find(item => item.textContent.trim() === name && item.offsetParent !== null);
  const click = async name => { await until(() => button(name) && !button(name).disabled, name); button(name).click(); await pause(60); };
  const field = name => all('.decision-connection-form')[0]?.querySelector('[data-native-field="' + name + '"]');
  const choose = (select, value) => { select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); };
  const sourceId = 'source/openai/decisions';
  const shared = { schema: 'hiroute.decision-service/v2', id: 'decision/team', revision: 1, name: 'Team decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c.decisionSource(sourceId).lineage_digest } } };
  function store(services, sources) {
    const state = { services, sources };
    c.handlers.decision_services_v2 = () => ({ services: state.services.map(service => c.decisionView(service, state.sources)) });
    c.handlers.decision_connection_sources = () => ({ sources: structuredClone(state.sources) });
    c.handlers.test_decision_service_v2 = ({ input }) => ({ schema: 'hiroute.decision-service-test-result/v2', outcome: 'passed', failure_code: null, duration_millis: 412, tested_configuration_digest: input.expected_configuration_digest, tested_credential_state_revision: c.decisionView(state.services[0], state.sources).credential_state_revision });
    return state;
  }
  const open = async () => { await until(() => document.querySelector('.sidebar-health .status-dot.good'), 'ready'); await click(label('模型', 'Models')); await click(label('决策模型', 'Decision models')); };
  return {
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
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) for (const state of ['shared', 'independent', 'source-error', 'detour', 'stale']) {
    await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
    await waitFor(client, 'Boolean(window.productShell)');
    await evaluate(client, `(() => { const c = window.productShell; c.reset(); localStorage.setItem('hiroute.language', ${JSON.stringify(language)}); localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)}); })()`);
    const anchor = await evaluate(client, `${driver}[${JSON.stringify(state)}]()`);
    // The stale result sits below the fold; centring keeps it clear of the sticky list header.
    if (state === 'stale') await evaluate(client, `document.querySelector(${JSON.stringify(anchor)}).scrollIntoView({ block: 'center' })`);
    await new Promise(resolve => setTimeout(resolve, 120));
    const geometry = await evaluate(client, `(() => {
      const box = document.querySelector(${JSON.stringify(anchor)}), b = box.getBoundingClientRect();
      const clipped = [...box.querySelectorAll('button, input, select, label, p, strong')].filter(item => item.offsetParent !== null && item.scrollWidth > item.clientWidth + 1 && getComputedStyle(item).overflowX !== 'visible' && !['INPUT', 'SELECT'].includes(item.tagName)).map(item => item.textContent.trim().slice(0, 60));
      const leaks = document.body.innerText.includes('sha256:');
      return { bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1, overflow: box.scrollWidth > box.clientWidth + 1, width: b.width, clipped, digestVisible: leaks };
    })()`);
    if (geometry.bodyOverflow || geometry.overflow || geometry.clipped.length || geometry.digestVisible) throw new Error(`Decision model geometry: ${JSON.stringify({ language, theme, viewport, state, geometry })}`);
    const id = `decision-${state}-${language}-${theme}-${viewport.width}`;
    const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
    results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
  }
  return results;
}
