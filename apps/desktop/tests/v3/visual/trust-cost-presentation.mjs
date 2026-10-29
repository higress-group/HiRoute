import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const output = path.resolve(process.argv[2] ?? path.join(desktop, 'test-results/trust-cost-presentation'));
const port = () => new Promise((resolve, reject) => {
  const server = net.createServer(); server.once('error', reject);
  server.listen(0, '127.0.0.1', () => { const address = server.address(); server.close(() => resolve(address.port)); });
});
const ready = async url => {
  for (let i = 0; i < 200; i++) { try { if ((await fetch(url)).ok) return; } catch {} await new Promise(r => setTimeout(r, 50)); }
  throw new Error(`Timed out waiting for ${url}`);
};
const stop = async child => {
  if (!child?.pid) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 1000))]);
  child.stdout?.destroy(); child.stderr?.destroy();
};
const httpPort = await port(), cdpPort = await port();
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-cost-presentation-'));
await mkdir(output, { recursive: true });
let vite, chrome, client;
try {
  vite = spawn(path.join(desktop, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktop, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { vite.once('spawn', resolve); vite.once('error', reject); });
  await ready(`http://127.0.0.1:${httpPort}/`);
  const executable = process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
  chrome = spawn(executable, ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, 'about:blank'], { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { chrome.once('spawn', resolve); chrome.once('error', reject); });
  await ready(`http://127.0.0.1:${cdpPort}/json/version`);
  client = await connectPage(cdpPort);
  await navigate(client, `http://127.0.0.1:${httpPort}/trust-cost-presentation.html`, { width: 755, height: 600 });
  await waitFor(client, `Boolean(document.querySelector('.trust-cost') && document.querySelector('.trust-tile'))`);
  const baseline = await evaluate(client, `({ card: document.querySelector('.trust-cost').innerText, tile: [...document.querySelectorAll('.trust-tile')].at(-1).innerText })`);
  await writeFile(path.join(output, 'observed.json'), JSON.stringify(baseline, null, 2));
  if (process.argv.includes('--baseline')) {
    assert.match(baseline.tile, /\$0\.0028/, 'The small known amount must remain visible in the evidence tile');
    assert.match(baseline.card, /−\$0\.1125 to −\$0\.0896/, 'The full range must be the primary difference result');
  } else {
    const initial = await evaluate(client, `({report: structuredClone(window.trustCostFixture.report), models: structuredClone(window.trustCostFixture.models), editorPlan: structuredClone(window.trustCostFixture.editorPlan)})`);
    const results = [];
    async function fixture(settings = {}) {
      const generation = await evaluate(client, `(() => { const f = window.trustCostFixture; Object.assign(f, ${JSON.stringify({
        report: initial.report, models: initial.models, scored: 4, unrated: 1, language: 'en', theme: 'light', view: 'quality', ...settings,
      })}); f.reads = []; f.refresh(); return f.generation; })()`);
      await waitFor(client, `document.querySelector('[data-fixture-generation="${generation}"] .trust-cost') !== null`);
      await waitFor(client, `document.querySelector('[data-fixture-generation="${generation}"] .quality-scope-meta')?.textContent.includes(${JSON.stringify(String(settings.scored ?? 4))})`);
    }
    async function snapshot(selector, name) {
      const geometry = await evaluate(client, `(() => { const e = document.querySelector(${JSON.stringify(selector)}); e.scrollIntoView({block:'center'});
        const r = e.getBoundingClientRect(); const children = [...e.querySelectorAll('strong, small, summary, p')].filter(x => x.getClientRects().length);
        return {left:r.left, right:r.right, width:r.width, height:r.height, overflow:e.scrollWidth > e.clientWidth + 1,
          escaped:children.some(x => { const b=x.getBoundingClientRect(); return b.left < r.left - 1 || b.right > r.right + 1; }),
          bodyOverflow:document.documentElement.scrollWidth > innerWidth + 1}; })()`);
      assert.ok(!geometry.overflow && !geometry.escaped && !geometry.bodyOverflow, `${name} must fit: ${JSON.stringify(geometry)}`);
      const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      await writeFile(path.join(output, `${name}.png`), Buffer.from(shot.data, 'base64'));
      results.push({ name, geometry });
    }
    async function keyboardToggle(selector, open = true) {
      await evaluate(client, `(() => { const s = document.querySelector(${JSON.stringify(selector)}); s.scrollIntoView({block:'center'}); s.focus(); })()`);
      assert.equal(await evaluate(client, `document.activeElement === document.querySelector(${JSON.stringify(selector)})`), true);
      await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', text: '\r', windowsVirtualKeyCode: 13 });
      await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
      await waitFor(client, `document.querySelector(${JSON.stringify(selector)})?.parentElement.open === ${open} && document.querySelector(${JSON.stringify(selector)}).getAttribute('aria-expanded') === '${open}'`);
    }
    async function editorScope({ bindings, candidates, announcements }) {
      const plan = structuredClone(initial.editorPlan);
      plan.desired.strategy.economy = bindings.map(binding_id => ({ binding_id }));
      const report = structuredClone(initial.report);
      report.plan_id = plan.agent_plan_id;
      report.branches[0].branch_id = 'smart_saving';
      report.branches[0].upward_from_ms = Date.UTC(2026, 9, 11);
      report.rows[0].branch_id = 'smart_saving';
      report.announcements = announcements.map(model_configuration_id => ({ model_configuration_id, included_on: '2026-10-10',
        cohort_from_ms: Date.UTC(2026, 9, 11), before: 0, announcement_day: 0, after: 0, executed_version_verified: false }));
      const generation = await evaluate(client, `(() => { const f=window.trustCostFixture; Object.assign(f, ${JSON.stringify({
        view: 'editor', language: 'en', theme: 'light', report, editorPlan: plan, editorCandidates: candidates, scored: 4, unrated: 1,
      })}); f.refresh(); return f.generation; })()`);
      await waitFor(client, `document.querySelector('[data-fixture-generation="${generation}"] .quality-model-row[data-model-configuration=${JSON.stringify(candidates[0].model_configuration_id)}]') !== null`);
      await keyboardToggle('.trust-evidence-scope > summary');
      return evaluate(client, `({text:document.querySelector('.trust-evidence-scope').innerText,
        models:[...document.querySelectorAll('.quality-model-row')].map(row=>row.dataset.modelConfiguration)})`);
    }
    const candidate = (binding_id, model_configuration_id) => ({ binding_id, model_configuration_id, display_name: model_configuration_id,
      reasoning: { kind: 'fixed', profile: 'provider-default' }, billing_class: 'standard', routable: true, ingress_protocols: ['responses'] });
    const first = candidate('binding/a', 'model/current-a'), second = candidate('binding/b', 'model/current-b');
    const scopeCases = [
      { name: 'missing-binding', input: { bindings: ['binding/a', 'binding/b'], candidates: [first], announcements: ['model/published-b'] } },
      { name: 'changed-source-identity', input: { bindings: ['binding/a'], candidates: [first], announcements: ['model/published-a'] } },
      { name: 'matching-current-identity', input: { bindings: ['binding/a'], candidates: [first], announcements: ['model/current-a'] } },
      { name: 'complete-empty-announcements', input: { bindings: ['binding/a', 'binding/b'], candidates: [first, second], announcements: [] } },
    ];
    const scopes = [];
    for (const item of scopeCases) scopes.push({ name: item.name, ...await editorScope(item.input) });
    await writeFile(path.join(output, 'editor-scope-cases.json'), JSON.stringify(scopes, null, 2));
    const effectiveStart = await evaluate(client, `new Date(Date.UTC(2026, 9, 11)).toLocaleString('en-US')`);
    for (const item of scopes.slice(0, 3)) {
      assert.deepEqual(item.models, ['model/current-a'], `${item.name}: the production editor options projection must be loaded`);
      assert.match(item.text, /Model-update basis is unconfirmed/, `${item.name}: current editor identities do not prove the published cohort basis`);
      assert.doesNotMatch(item.text, /No update announcement|Update announcement dates/);
      assert.ok(item.text.includes(`Effective step-up sample start: ${effectiveStart}.`));
    }
    assert.deepEqual(scopes[3].models, ['model/current-a', 'model/current-b']);
    assert.match(scopes[3].text, /No model-update announcements are present in this report; retention bounds the step-up scope/);
    assert.ok(scopes[3].text.includes(`Effective step-up sample start: ${effectiveStart}.`));
    assert.doesNotMatch(scopes[3].text, /unconfirmed/);
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const width of [755, 1200]) {
      await client.send('Emulation.setDeviceMetricsOverride', { width, height: width === 755 ? 600 : 813, deviceScaleFactor: 1, mobile: false });
      await fixture({ language, theme });
      const state = await evaluate(client, `(() => ({header:document.querySelector('.quality-scope-meta').innerText,
        suggestion:document.querySelector('.trust-suggestion').innerText,
        rows:[...document.querySelectorAll('.trust-cost-row')].map(e=>e.innerText),
        tile:[...document.querySelectorAll('.trust-tile')].at(-1).innerText}))()`);
      assert.equal(state.rows.length, 3);
      assert.ok(state.header.includes(language === 'zh' ? '所选范围：5 个阶段 · 4 已评分 · 1 未评分' : 'Selected scope: 5 stages · 4 scored · 1 unrated'));
      assert.ok(state.suggestion.includes(language === 'zh' ? '升档样本：2 / 30' : 'Step-up samples: 2 / 30'));
      assert.ok(state.suggestion.includes(language === 'zh' ? '当前版本 ·' : 'Active revision ·'));
      assert.ok(state.suggestion.includes(language === 'zh' ? '常规组 · 完整评分' : 'Regular · Fully scored'));
      const separator = language === 'zh' ? ' 至 ' : ' to ';
      assert.ok(state.rows[0].includes('$0.0028') && state.tile.includes('$0.0028'));
      assert.ok(state.rows[1].includes(`$0.0924${separator}$0.1152`) && state.tile.includes(`$0.0924${separator}$0.1152`));
      assert.ok(state.rows[2].includes(`−$0.1125${separator}−$0.0896`) && state.tile.includes(`−$0.1125${separator}−$0.0896`));
      assert.ok(!state.rows.some(row => /Unknown|未知/.test(row)));
      const suffix = `${language}-${theme}-${width}`;
      await snapshot('.trust-suggestion', `suggestion-${suffix}`);
      await snapshot('.trust-cost', `cost-${suffix}`);
      await keyboardToggle('.trust-cost-details > summary');
      const details = await evaluate(client, `document.querySelector('.trust-cost-details').innerText`);
      assert.ok(details.includes(language === 'zh' ? '未上报缓存写入量' : 'unreported cache-write'));
      assert.ok(!details.includes(language === 'zh' ? '排除 0' : '0 excluded'));
      assert.ok(details.includes('-112436') && details.includes('-89636'));
      await snapshot('.trust-cost-details', `calculation-${suffix}`);
      await keyboardToggle('.trust-cost-details > summary', false);
      await keyboardToggle('.trust-evidence-scope > summary');
      const scope = await evaluate(client, `document.querySelector('.trust-evidence-scope').innerText`);
      assert.ok(scope.includes(language === 'zh' ? '此报告没有模型更新公告' : 'No model-update announcements are present in this report'));
      assert.ok(scope.includes(language === 'zh' ? '与页面展示的时间筛选无关' : 'independently of the page’s display period'));
      await snapshot('.trust-evidence-scope', `scope-${suffix}`);
      await keyboardToggle('.trust-evidence-scope > summary', false);
    }

    const unconfirmed = structuredClone(initial.report);
    unconfirmed.announcements = [{ model_configuration_id: 'model/published', included_on: '2026-10-10',
      cohort_from_ms: Date.UTC(2026, 9, 11), before: 0, announcement_day: 0, after: 0, executed_version_verified: false }];
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const width of [755, 1200]) {
      await client.send('Emulation.setDeviceMetricsOverride', { width, height: width === 755 ? 600 : 813, deviceScaleFactor: 1, mobile: false });
      await fixture({ language, theme, report: unconfirmed });
      await keyboardToggle('.trust-evidence-scope > summary');
      const scope = await evaluate(client, `document.querySelector('.trust-evidence-scope').innerText`);
      assert.ok(scope.includes(language === 'zh' ? '模型更新是否适用于本分类尚未确认' : 'Model-update basis is unconfirmed'));
      assert.ok(!scope.includes('2026-10-10'));
      await snapshot('.trust-evidence-scope', `scope-unconfirmed-${language}-${theme}-${width}`);
    }

    const withCost = (changes, difference = null) => ({ ...initial.report, cost: { kind: 'available', difference_micros: difference, comparison: { ...initial.report.cost.comparison, ...changes } } });
    async function costState(report) {
      await fixture({ report });
      return evaluate(client, `(() => ({rows:[...document.querySelectorAll('.trust-cost-row')].map(e=>e.innerText),
        card:document.querySelector('.trust-cost').innerText, tile:[...document.querySelectorAll('.trust-tile')].at(-1).innerText,
        tone:document.querySelector('[data-cost-metric="difference"] strong')?.className}))()`);
    }
    let state = await costState(withCost({ compared_request_count: 1, missing_request_count: 0, reference_estimate_micros: 3000 }, -236));
    assert.equal(state.rows.length, 3); assert.ok(state.rows[1].includes('$0.0030') && state.rows[2].includes('−$0.0002'));
    assert.ok(!state.card.includes('Range') && state.tone === 'less');
    state = await costState(withCost({ reference_estimate_micros: null, same_usage_bounds: null }));
    assert.ok(state.rows[0].includes('$0.0028') && state.rows[1].includes('Unknown') && state.rows[2].includes('Difference unknown'));
    state = await costState(withCost({ same_usage_bounds: { ...initial.report.cost.comparison.same_usage_bounds, difference_min_micros: 1 } }));
    assert.ok(state.rows[0].includes('$0.0028') && state.rows[1].includes('Unknown') && state.card.includes('cannot be verified'));
    state = await costState(withCost({ compared_request_count: 2 }));
    assert.ok(state.rows.every(row => /Unknown|unknown/.test(row)) && state.card.includes('Cost evidence is inconsistent'));
    await snapshot('.trust-cost', 'cost-inconsistent-en-light-1200');
    for (const flag of ['retention_boundary_partial', 'arithmetic_overflow']) {
      state = await costState(withCost({ request_count: 2, compared_request_count: 1, missing_request_count: 1,
        compared_routed_subtotal_micros: 100, compared_reference_subtotal_micros: 200, [flag]: true }));
      assert.ok(state.rows.every(row => /Unknown|unknown/.test(row)));
      assert.ok(state.card.includes(flag === 'retention_boundary_partial' ? 'outside retention' : 'exceeds the arithmetic range'));
      assert.ok(!state.card.includes('subtotals'));
      await snapshot('.trust-cost', `cost-${flag}-en-light-1200`);
      await keyboardToggle('.trust-cost-details > summary');
      assert.match(await evaluate(client, `document.querySelector('.trust-cost-details').innerText`), /Known paired-request subtotals only/);
    }
    state = await costState(withCost({ request_count: 2, compared_request_count: 1, missing_request_count: 1,
      excluded_request_count: 1, same_usage_bounds: null, compared_routed_subtotal_micros: 100, compared_reference_subtotal_micros: 200 }));
    assert.ok(state.card.includes('cannot be estimated completely') && !state.card.includes('excluded requests'));
    await keyboardToggle('.trust-cost-details > summary');
    assert.match(await evaluate(client, `document.querySelector('.trust-cost-details').innerText`), /1 excluded request is included in the missing count/);
    for (const [difference, tone] of [[-100, 'less'], [100, 'more'], [0, '']]) {
      state = await costState(withCost({ reference_estimate_micros: 3000 }, difference)); assert.equal(state.tone, tone);
    }
    state = await costState(withCost({ routed_estimate_micros: 200000, same_usage_bounds: { reference_min_micros: 100000,
      reference_max_micros: 300000, difference_min_micros: -100000, difference_max_micros: 100000 } }));
    assert.equal(state.tone, ''); assert.ok(state.rows[2].includes('−$0.1000 to +$0.1000'));
    state = await costState({ ...initial.report, cost: { kind: 'unavailable', reason: 'primary_not_unique' } });
    assert.ok(state.card.includes('unique published Primary') && state.tile.includes('unique published Primary'));

    const longRange = withCost({ routed_estimate_micros: 1, same_usage_bounds: { reference_min_micros: 9000000000000000,
      reference_max_micros: 9007199254740990, difference_min_micros: -9007199254740989, difference_max_micros: -8999999999999999 } });
    await client.send('Emulation.setDeviceMetricsOverride', { width: 755, height: 600, deviceScaleFactor: 1, mobile: false });
    for (const [language, theme] of [['zh', 'dark'], ['en', 'light']]) {
      await fixture({ language, theme, report: longRange });
      const text = await evaluate(client, `document.querySelector('[data-cost-metric="difference"]').innerText`);
      assert.ok(text.includes(language === 'zh' ? ' 至 ' : ' to ') && (text.match(/−/g) ?? []).length === 2);
      await snapshot('.trust-cost', `long-range-${language}-${theme}-755`);
    }

    const larger = structuredClone(initial.report); larger.branches[0].upward_cohort.complete = 41;
    larger.branches[0].suggestion = { kind: 'step_up', threshold_millis: 650 };
    larger.announcements = [{ model_configuration_id: 'other-branch', included_on: '2026-10-03', before: 0, announcement_day: 0, after: 0, executed_version_verified: false }];
    await fixture({ report: larger, scored: 58 });
    assert.equal(await evaluate(client, `document.querySelector('.trust-suggestion-actions button') !== null`), true);
    assert.match(await evaluate(client, `document.querySelector('.quality-scope-meta').innerText`), /58 scored/);
    assert.match(await evaluate(client, `document.querySelector('.trust-suggestion').innerText`), /Step-up samples: 41 \/ 30/);
    await keyboardToggle('.trust-evidence-scope > summary');
    const unrelated = await evaluate(client, `document.querySelector('.trust-evidence-scope').innerText`);
    assert.match(unrelated, /Model-update basis is unconfirmed/);
    assert.doesNotMatch(unrelated, /2026-10-03|No model-update announcements/);
    await evaluate(client, `(() => { const s=[...document.querySelectorAll('.quality-filters select')].find(e=>e.querySelector('option[value="seven_days"]')); s.value='all'; s.dispatchEvent(new Event('change',{bubbles:true})); })()`);
    await waitFor(client, `document.querySelector('.trust-suggestion')?.innerText.includes('Step-up samples: 41 / 30')`);
    const timeReads = await evaluate(client, `window.trustCostFixture.reads.filter(x=>x.view==='plan_trust').map(x=>x.query.from_ms)`);
    assert.ok(timeReads.length >= 2 && timeReads[0] !== timeReads.at(-1), 'Period changes must reach the real report wiring');
    await evaluate(client, `(() => { const s=[...document.querySelectorAll('.quality-filters select')].find(e=>e.querySelector('option[value="current"]')); s.value='all'; s.dispatchEvent(new Event('change',{bubbles:true})); })()`);
    await waitFor(client, `document.querySelector('.trust-suggestion')?.innerText.includes('Suggestions cover only the active version')`);
    assert.equal(await evaluate(client, `document.querySelector('.trust-suggestion-actions button') !== null`), false);

    larger.announcements.push({ model_configuration_id: 'model/regular', included_on: '2026-10-10', before: 0, announcement_day: 0, after: 0, executed_version_verified: false });
    await fixture({ report: larger }); await keyboardToggle('.trust-evidence-scope > summary');
    const scope = await evaluate(client, `document.querySelector('.trust-evidence-scope').innerText`);
    assert.ok(scope.includes('Model-update basis is unconfirmed') && !scope.includes('2026-10-10') && !scope.includes('2026-10-03') && scope.includes('Effective step-up sample start'));
    await fixture({ report: larger, models: [] }); await keyboardToggle('.trust-evidence-scope > summary');
    assert.match(await evaluate(client, `document.querySelector('.trust-evidence-scope').innerText`), /Model-update basis is unconfirmed/);

    const multiple = structuredClone(larger);
    multiple.branches.push({ ...structuredClone(multiple.branches[0]), branch_id: 'other', upward_cohort: { ...multiple.branches[0].upward_cohort, complete: 7 } });
    const models = [...structuredClone(initial.models), { ...initial.models[0], branch_id: 'other', branch_name: 'Other category', model_configuration_id: 'other-branch' }];
    await fixture({ report: multiple, models });
    const labels = await evaluate(client, `[...document.querySelectorAll('.trust-suggestion-sample')].map(e=>e.innerText)`);
    assert.equal(labels.length, 2); assert.match(labels[0], /41 \/ 30/); assert.match(labels[1], /7 \/ 30/);
    assert.ok(labels[0].includes('A long category name') && labels[1].includes('Other category'));

    const difficulty = structuredClone(initial.report); difficulty.branches[0].branch_id = 'smart_saving';
    const difficultyModels = [{ ...initial.models[0], branch_id: 'smart_saving', branch_name: undefined }];
    await fixture({ report: difficulty, models: difficultyModels });
    assert.match(await evaluate(client, `document.querySelector('.trust-suggestion-sample').innerText`), /Active revision · Difficulty routing · Regular · Fully scored/);

    for (const [micros, expected] of [[0, '$0.00'], [1, '$0.000001'], [49, '$0.000049'], [2764, '$0.0028'], [1000000, '$1.00'], [null, 'Amount unknown']]) {
      const generation = await evaluate(client, `(() => { const f=window.trustCostFixture; Object.assign(f, {view:'timeline', language:'en', timelineMicros:${JSON.stringify(micros)}, timelineCurrency:'USD'}); f.refresh(); return f.generation; })()`);
      await waitFor(client, `document.querySelector('[data-fixture-generation="${generation}"] .routing-tile strong') !== null`);
      const timeline = await evaluate(client, `({value:document.querySelector('.routing-tile strong').innerText, tile:document.querySelector('.routing-tile').innerText, detail:document.querySelector('.routing-turn-detail').innerText})`);
      assert.equal(timeline.value, expected);
      assert.ok(timeline.tile.includes(micros === null ? 'Not enough evidence to estimate' : 'Plus an unknown portion'));
      assert.ok(timeline.detail.includes(expected) && timeline.detail.includes('+ unknown'));
    }
    await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2));
    console.log(`Passed cost/evidence browser scenarios and ${results.length} production-style layout captures.`);
  }
} finally {
  client?.close(); await stop(chrome); await stop(vite); await rm(profile, { recursive: true, force: true });
}
