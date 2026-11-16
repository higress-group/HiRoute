import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const transport = process.env.HIROUTE_BROWSER_TRANSPORT ?? 'tauri';
const shot = async (name, settle = true) => {
  if (settle) await evaluate(client, `Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>true)`);
  const value = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(value.data, 'base64'));
};
async function open(query, width, height, theme, lang) {
  const url = `${base}new-model-trial.html?polish=1&lang=${lang}&theme=${theme}&${query}${transport === 'console' ? '&transport=console' : ''}`;
  await client.send('Page.enable');
  await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url });
  await waitFor(client, `location.href===${JSON.stringify(url)} && !!document.querySelector('.plan-editor,.new-models,.sessions-page')`);
  await evaluate(client, 'document.fonts.ready.then(()=>true)');
  if (transport === 'console') assert.equal(await evaluate(client, 'window.fixtureTransport'), 'console');
  if (width === 755 && query.startsWith('view=timeline')) {
    await click('.toolbar-actions button', lang === 'zh' ? '紧凑视图' : 'Compact view');
    await waitFor(client, `document.querySelector('.sessions-page').classList.contains('is-compact')`);
  }
}
async function click(selector, text) {
  const expression = `(()=>{const el=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&e.textContent.includes(${JSON.stringify(text)}));if(!el)return false;el.click();return true;})()`;
  await waitFor(client, expression);
}
async function check(id, run) {
  if (process.env.POLISH_CASE && !id.startsWith(process.env.POLISH_CASE)) return;
  try { await run(); results.push({ id, status: 'passed' }); }
  catch (error) {
    await shot(`failure-${id}`);
    const geometry = await evaluate(client, `({width:innerWidth,document:document.documentElement.scrollWidth,visibility:document.visibilityState,replay:document.querySelector('.trust-replay')?.textContent,reads:window.trialTrace?.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='plan_trust_replay'),lanes:[...document.querySelectorAll('.routing-lanes-scroll')].map(e=>({width:e.clientWidth,scroll:e.scrollWidth,visible:e.getClientRects().length}))})`).catch(() => null);
    await writeFile(path.join(output, `failure-${id}.json`), JSON.stringify({ error: String(error.stack ?? error), geometry }, null, 2));
    results.push({ id, status: 'failed', error: String(error.stack ?? error) });
    await client.send('Emulation.setEmulatedMedia', { features: [] });
  }
}
const reads = `trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='plan_trust_replay')`;
const matrix = process.env.POLISH_BASELINE ? [['en', 'light', process.env.POLISH_BASELINE === '755' ? 755 : 1200, process.env.POLISH_BASELINE === '755' ? 600 : 813]]
  : ['en', 'zh'].flatMap(lang => ['light', 'dark'].flatMap(theme => [[lang, theme, 1200, 813], [lang, theme, 755, 600]]));
try {
  for (const [lang, theme, width, height] of matrix) {
    const suffix = `${lang}-${theme}-${width}`;
    await check(`home-current-announcement-${suffix}`, async () => {
      await open('view=home&current-announcement=1', width, height, theme, lang);
      await waitFor(client, `!!document.querySelector('.new-model-card')`);
      assert.deepEqual(await evaluate(client, `[...document.querySelectorAll('.new-model-card h3')].map(e=>e.textContent)`), ['DeepSeek V4.1 Flash 1017']);
      const text = await evaluate(client, `document.querySelector('.new-models').textContent`);
      assert.ok(!text.includes('Older directory model'));
      assert.ok(!text.includes(lang === 'zh' ? '签名已校验' : 'Signature verified'));
      assert.ok(await evaluate(client, `!!document.querySelector('.new-models .model-feed-disclosure')`));
      const card = await evaluate(client, `document.querySelector('.new-model-card').innerText`);
      assert.ok(card.includes(lang === 'zh' ? '从范围明确的小任务开始试用，并观察实际表现。' : 'Start with a well-scoped task and observe the results.'));
      assert.ok(!card.includes(lang === 'zh' ? '接入服务' : 'connection provider'));
      assert.ok(!card.includes(lang === 'zh' ? '同名模型可能被供应商更新' : 'Providers may update a model behind the same name'));
      const addedDate = await evaluate(client, `new Date('2026-10-19T00:00:00Z').toLocaleDateString(${JSON.stringify(lang === 'zh' ? 'zh-CN' : 'en-US')},{month:${JSON.stringify(lang === 'zh' ? 'numeric' : 'short')},day:'numeric',timeZone:'UTC'})`);
      assert.ok(card.includes(lang === 'zh' ? `公告收录 ${addedDate}` : `Announcement added ${addedDate}`));
      const disclosure = `.new-models > details.disclosure`;
      assert.equal(await evaluate(client, `document.querySelectorAll(${JSON.stringify(disclosure)}).length`), 1);
      assert.equal(await evaluate(client, `document.querySelector(${JSON.stringify(disclosure)}).open`), false);
      assert.equal(await evaluate(client, `document.querySelector(${JSON.stringify(disclosure)}+' summary').getAttribute('aria-expanded')`), 'false');
      assert.equal(await evaluate(client, `document.querySelector(${JSON.stringify(disclosure)}+' .disclosure-label').textContent`), lang === 'zh' ? '版本与来源' : 'Version and source');
      const uncertainty = lang === 'zh' ? '同名模型可能被供应商更新' : 'Providers may update a model behind the same name';
      assert.equal(await evaluate(client, `document.querySelector('.new-models').innerText.includes(${JSON.stringify(uncertainty)})`), false);
      assert.ok(await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(disclosure)}+' summary');e.focus();return document.activeElement===e;})()`));
      await client.send('Input.dispatchKeyEvent', { type:'keyDown', key:'Enter', code:'Enter', windowsVirtualKeyCode:13, text:'\r', unmodifiedText:'\r' });
      await client.send('Input.dispatchKeyEvent', { type:'keyUp', key:'Enter', code:'Enter', windowsVirtualKeyCode:13 });
      await waitFor(client, `document.querySelector(${JSON.stringify(disclosure)}).open && document.querySelector(${JSON.stringify(disclosure)}+' summary').getAttribute('aria-expanded')==='true'`);
      const expanded = await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(disclosure)}+' .disclosure-body');return {visible:document.querySelector('.new-models').innerText.includes(${JSON.stringify(uncertainty)}),text:e.innerText,overflow:e.scrollWidth>e.clientWidth+1};})()`);
      assert.equal(expanded.visible, true);
      for (const phrase of lang === 'zh' ? ['同名模型', '供应商更新', '收录日期', '不代表实际执行的版本'] : ['Providers may update a model behind the same name', 'date added', 'does not identify the version that runs']) assert.ok(expanded.text.includes(phrase), phrase);
      assert.equal(expanded.overflow, false);
      await shot(`home-version-source-expanded-${suffix}`);
      await client.send('Input.dispatchKeyEvent', { type:'keyDown', key:'Enter', code:'Enter', windowsVirtualKeyCode:13, text:'\r', unmodifiedText:'\r' });
      await client.send('Input.dispatchKeyEvent', { type:'keyUp', key:'Enter', code:'Enter', windowsVirtualKeyCode:13 });
      await waitFor(client, `!document.querySelector(${JSON.stringify(disclosure)}).open && document.querySelector(${JSON.stringify(disclosure)}+' summary').getAttribute('aria-expanded')==='false'`);
      assert.equal(await evaluate(client, `document.querySelector('.new-models').innerText.includes(${JSON.stringify(uncertainty)})`), false);
      assert.ok(await evaluate(client, `document.documentElement.scrollWidth<=innerWidth+1`));
      assert.ok(await evaluate(client, `[...document.querySelectorAll('.new-model-card,.new-model-card h3,.new-model-note')].every(e=>e.scrollWidth<=e.clientWidth+1)`));
      await shot(`home-current-announcement-${suffix}`);
      await click('.new-model-card button', lang === 'zh' ? '查看模型表现' : 'View model performance');
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='open-plan').length`), 1);
    });
    await check(`home-empty-recommendations-${suffix}`, async () => {
      await open('view=home&no-recommendations=1', width, height, theme, lang);
      await waitFor(client, `!!document.querySelector('.new-models > .field-help')`);
      assert.equal(await evaluate(client, `document.querySelectorAll('.new-model-card').length`), 0);
      const text = await evaluate(client, `document.querySelector('.new-models').innerText`);
      assert.ok(text.includes(lang === 'zh' ? '暂无推荐' : 'No featured models'));
      assert.ok(!text.includes('DeepSeek V4.1 Flash 1017'));
      assert.equal(await evaluate(client, `document.querySelector('.new-models .model-feed-disclosure')`), null);
      assert.ok(await evaluate(client, `!document.querySelector('.new-models header button').disabled`));
      assert.ok(await evaluate(client, `document.documentElement.scrollWidth<=innerWidth+1 && [...document.querySelectorAll('.new-models,.new-models > .field-help')].every(e=>e.scrollWidth<=e.clientWidth+1)`));
      await shot(`home-empty-recommendations-${suffix}`);
    });
    for (const [scenario, from] of [['unrelated', Date.UTC(2026, 9, 20, 12, 34, 56)], ['retention', Date.UTC(2026, 9, 24, 12, 34, 56)], ['no-announcement', Date.UTC(2026, 9, 20, 12, 34, 56)]]) {
      await check(`home-scope-${scenario}-${suffix}`, async () => {
        await open(`view=home&home-scope=${scenario}`, width, height, theme, lang);
        await waitFor(client, `!!document.querySelector('.trust-attention-item')`);
        const row = await evaluate(client, `(()=>{const e=document.querySelector('.trust-attention-item');const s=e.querySelector('.trust-attention-scope');return {text:e.textContent,scope:s?.textContent??'',buttons:[...e.querySelectorAll('button')].map(b=>b.textContent),overflow:e.scrollWidth>e.clientWidth+1};})()`);
        const expectedStart = await evaluate(client, `new Date(${from}).toLocaleString(${JSON.stringify(lang === 'zh' ? 'zh-CN' : 'en-US')},{dateStyle:'medium',timeStyle:'medium'})`);
        assert.ok(row.scope.includes(expectedStart), `effective start missing: ${row.scope}`);
        assert.ok(row.scope.includes(lang === 'zh' ? '保留记录' : 'retained records'), row.scope);
        assert.ok(row.scope.includes(lang === 'zh' ? '可能' : 'may'), row.scope);
        assert.ok(!row.scope.includes(lang === 'zh' ? '模型更新公告之后' : 'model update announcement'), row.scope);
        for (const value of ['90%', '37', '41', '9', '2']) assert.ok(row.text.includes(value), value);
        assert.ok(row.text.includes(lang === 'zh' ? '自定义 0.72' : 'Custom 0.72'));
        assert.deepEqual(row.buttons, lang === 'zh' ? ['查看依据', '打开调整草稿'] : ['View evidence', 'Open change draft']);
        assert.equal(row.overflow, false);
        await evaluate(client, `document.querySelector('.trust-attention-item').scrollIntoView({block:'center'});true`);
        await shot(`home-scope-${scenario}-${suffix}`);
        await click('.trust-attention-item button', lang === 'zh' ? '查看依据' : 'View evidence');
        await click('.trust-attention-item button', lang === 'zh' ? '打开调整草稿' : 'Open change draft');
        const actions = await evaluate(client, `trialTrace.filter(x=>x.command==='review-trust'||x.command==='review-trust-action').map(({command,payload})=>({command,payload}))`);
        assert.deepEqual(actions, [
          {command:'review-trust',payload:{planId:'plan/daily-coding'}},
          {command:'review-trust-action',payload:{planId:'plan/daily-coding',branchId:'smart_saving'}},
        ]);
      });
    }
    await check(`replay-cache-${suffix}`, async () => {
      await open('view=editor&native=1', width, height, theme, lang);
      await waitFor(client, `!!document.querySelector('.trust-replay-bar')`);
      const first = await evaluate(client, `${reads}[0].payload.request.intent.query.thresholds_millis`);
      assert.deepEqual(first, [720]);
      const count = await evaluate(client, `${reads}.length`);
      const levels = lang === 'zh' ? ['试用', '熟悉', '信任', '放手'] : ['Trial', 'Familiar', 'Trusted', 'Hands-off'];
      for (const [index, level] of levels.entries()) {
        await click('.trust-levels button', level);
        await waitFor(client, `${reads}.length===${count + index + 1} && !document.querySelector('.trust-replay-state')`);
        assert.deepEqual(await evaluate(client, `${reads}.at(-1).payload.request.intent.query.thresholds_millis`), [[900], [800], [650], [500]][index]);
        assert.equal(await evaluate(client, `document.querySelector('.trust-replay-state')`), null);
      }
      await click('.trust-levels button', lang === 'zh' ? '试用' : 'Trial');
      assert.equal(await evaluate(client, `${reads}.length`), count + 4);
      await waitFor(client, `document.querySelector('.trust-replay-bar').getAnimations({subtree:true}).some(a=>a.effect.getComputedTiming().duration===250)`);
      await click('.trust-exact summary', lang === 'zh' ? '精确值' : 'Exact value');
      await evaluate(client, `(()=>{const el=document.querySelector('[data-decision-field="smart-simple-threshold"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,'0.777');el.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
      await waitFor(client, `${reads}.length===${count + 5} && document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.777')`);
      assert.deepEqual(await evaluate(client, `${reads}.at(-1).payload.request.intent.query.thresholds_millis`), [777]);
      await click('.trust-levels button', lang === 'zh' ? '试用' : 'Trial');
      assert.equal(await evaluate(client, `${reads}.length`), count + 5);
      const label = lang === 'zh' ? '内置规则' : 'Built-in rules';
      assert.ok(await evaluate(client, `document.querySelector('.trust-replay').textContent.includes(${JSON.stringify(label)})`));
      await evaluate(client, `document.querySelector('.trust-replay').scrollIntoView({block:'center'});true`);
      await shot(`replay-${suffix}`);
      await client.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
      await click('.trust-levels button', lang === 'zh' ? '熟悉' : 'Familiar');
      assert.equal(await evaluate(client, `document.querySelector('.trust-replay-bar').getAnimations({subtree:true}).length`), 0);
      await open('view=timeline&reason=heuristic', width, height, theme, lang);
      await waitFor(client, `!!document.querySelector('.routing-turn-reason')`);
      assert.equal(await evaluate(client, `document.querySelector('.routing-turn-reason').textContent`), label);
      await client.send('Emulation.setEmulatedMedia', { features: [] });
    });
    await check(`bounded-lanes-${suffix}`, async () => {
      await open('view=timeline', width, height, theme, lang);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===2 && document.querySelector('.routing-lanes-scroll').clientWidth>0`);
      const widths = await evaluate(client, `[...document.querySelectorAll('.routing-turn')].map(e=>e.getBoundingClientRect().width)`);
      assert.ok(widths.every(value => value <= 232.5), JSON.stringify(widths));
      assert.equal(await evaluate(client, `[...document.querySelectorAll('.routing-cell-empty')].some(e=>getComputedStyle(e).borderStyle.includes('dashed'))`), false);
      await evaluate(client, `document.querySelector('.routing-lanes-card').scrollIntoView({block:'center'});true`);
      await shot(`lanes-two-${suffix}`);
      await open('view=timeline&turns=40', width, height, theme, lang);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===40 && document.querySelector('.routing-lanes-scroll').clientWidth>0`);
      assert.ok(await evaluate(client, `(()=>{const e=document.querySelector('.routing-lanes-scroll');return e.scrollWidth>e.clientWidth && document.documentElement.scrollWidth<=innerWidth+1;})()`));
      const before = await evaluate(client, `document.querySelector('.routing-lane-label').getBoundingClientRect().left`);
      await evaluate(client, `document.querySelector('.routing-lanes-scroll').scrollLeft=500;true`);
      assert.ok(Math.abs(await evaluate(client, `document.querySelector('.routing-lane-label').getBoundingClientRect().left`) - before) <= 1);
      await evaluate(client, `document.querySelector('.routing-lanes-card').scrollIntoView({block:'center'});true`);
      await shot(`lanes-forty-${suffix}`);
    });
    await check(`visible-appends-${suffix}`, async () => {
      await open('view=timeline', width, height, theme, lang);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===2`);
      assert.equal(await evaluate(client, `document.querySelectorAll('.routing-turn[data-new=true]').length`), 0);
      await evaluate(client, `document.querySelector('.routing-lanes-card').scrollIntoView({block:'center'});true`);
      await click('.routing-turn', lang === 'zh' ? '轮次 1' : 'Turn 1');
      await evaluate(client, `document.querySelector('.routing-lanes-scroll').scrollLeft=0;appendTrialTurns(6);true`);
      await waitFor(client, `document.querySelectorAll('.routing-turn[data-new=true]').length===6`);
      assert.ok(await evaluate(client, `document.querySelector('.routing-turn[data-new=true]').getAnimations().some(a=>a.effect.getComputedTiming().duration===600)`));
      await waitFor(client, `!!document.querySelector('.routing-new-turns')`);
      await waitFor(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll'),r=s.getBoundingClientRect(),left=r.left+s.querySelector('.routing-lane-label').getBoundingClientRect().width;const outside=[...s.querySelectorAll('.routing-turn[data-new=true]')].filter(e=>{const b=e.getBoundingClientRect();return b.right<=left||b.left>=r.right;}).length;return Number(document.querySelector('.routing-new-turns')?.textContent.trim().split(' ')[0])===outside;})()`);
      await shot(`timeline-arrival-${suffix}`, false);
      await click('.routing-new-turns', lang === 'zh' ? '新轮次' : 'new turn');
      await waitFor(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll');return s.scrollLeft>=s.scrollWidth-s.clientWidth-2;})()`);
      assert.equal(await evaluate(client, `document.querySelector('.routing-new-turns')`), null);
      await client.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
      await evaluate(client, 'appendTrialTurns(1);true');
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===9`);
      assert.equal(await evaluate(client, `[...document.querySelectorAll('.routing-turn')].at(-1)?.getAnimations().length ?? 0`), 0);
      await client.send('Emulation.setEmulatedMedia', { features: [] });
    });
    await check(`auto-follow-${suffix}`, async () => {
      await open('view=timeline&turns=8', width, height, theme, lang);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===8 && document.querySelector('.routing-lanes-scroll').clientWidth>0`);
      await evaluate(client, 'appendTrialTurns(3);true');
      await waitFor(client, `document.querySelectorAll('.routing-turn').length===11`);
      await waitFor(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll');return s.scrollLeft>=s.scrollWidth-s.clientWidth-2;})()`);
      assert.equal(await evaluate(client, `document.querySelector('.routing-new-turns')`), null);
      assert.equal(await evaluate(client, `document.querySelector('.routing-turn[aria-pressed=true]').dataset.turnId`), 'request-11');
    });
    await check(`suggestion-hierarchy-${suffix}`, async () => {
      await open('view=editor', width, height, theme, lang);
      await click('[role=tab]', lang === 'zh' ? '模型表现' : 'Model performance');
      await waitFor(client, `!!document.querySelector('.trust-suggestion.info')`);
      const title = await evaluate(client, `document.querySelector('.trust-suggestion-line strong').textContent`);
      assert.equal(title.includes('≥'), false);
      assert.ok(title.includes(lang === 'zh' ? '信任' : 'Trusted'));
      const banner = await evaluate(client, `document.querySelector('.trust-suggestion.info').textContent`);
      for (const value of ['41', '30', '90%', '85%', '71%', '60%', '9', '3', '0.65']) assert.ok(banner.includes(value), value);
      assert.ok(await evaluate(client, `document.querySelectorAll('.trust-suggestion-replay,.trust-suggestion-newly-regular').length===2`));
      assert.ok(await evaluate(client, `(()=>{const e=document.querySelector('.trust-suggestion.info');return e.scrollWidth<=e.clientWidth+1;})()`));
      await evaluate(client, `document.querySelector('.trust-suggestion.info').scrollIntoView({block:'center'});true`);
      await shot(`suggestion-${suffix}`);
    });
    await check(`trial-cards-and-clearance-${suffix}`, async () => {
      await open('view=quickstart', width, height, theme, lang);
      await click('.new-models button', lang === 'zh' ? '试用' : 'Try');
      await waitFor(client, `!!document.querySelector('.trial-model-card[aria-pressed=true]')`);
      assert.ok(await evaluate(client, `document.querySelector('.trial-model-card[aria-pressed=true] .model-fact-tags .badge')!==null`));
      assert.ok(await evaluate(client, `document.querySelector('.trial-model-card[aria-pressed=true] .trial-model-note')?.textContent.trim().length>0`));
      assert.ok(await evaluate(client, `document.querySelector('.trial-model-card[aria-pressed=true] .trial-model-check')!==null`));
      await shot(`trial-cards-${suffix}`);
      const clearance = await evaluate(client, `(()=>{const s=document.querySelector('.trial-page .page-content');s.scrollTop=s.scrollHeight;const cards=[...document.querySelectorAll('.trial-setup>.trial-card')];const last=cards.at(-1).getBoundingClientRect(),footer=document.querySelector('.trial-footer').getBoundingClientRect();return {last:last.bottom,top:footer.top,height:footer.height,bottom:footer.bottom};})()`);
      assert.ok(clearance.top - clearance.last >= clearance.height - 1, JSON.stringify(clearance));
      assert.ok(clearance.bottom <= height + 1, JSON.stringify(clearance));
      await shot(`trial-bottom-${suffix}`);
    });
  }
} finally {
  client.close();
  await writeFile(path.join(output, 'report.json'), JSON.stringify({ proof: 'Production components with synthetic IPC', results }, null, 2));
}
console.log(JSON.stringify({ passed: results.filter(r => r.status === 'passed').length, failed: results.filter(r => r.status !== 'passed').length, output }));
if (!results.length || results.some(result => result.status !== 'passed')) process.exitCode = 1;
