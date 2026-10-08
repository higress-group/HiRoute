// Documentation only: current components, synthetic reads, no daemon/provider calls.
// node tests/decision-docs-capture.mjs CDP_PORT VITE_URL [--capture]
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { connectPage, navigate, evaluate, waitFor } from './v3/visual/cdp-client.mjs';
const [port, base] = process.argv.slice(2);
assert(port && base, 'Provide an isolated Chrome CDP port and Vite URL');
const capture = process.argv.includes('--capture');
const assets = new URL('../../../decision-extensions/assets/', import.meta.url);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
let clockScript;
const clickText = async text => evaluate(client, `[...document.querySelectorAll('button')].find(b=>b.innerText.trim()===${JSON.stringify(text)}).click()`);
try {
  await client.send('Page.enable');
  await client.send('Emulation.setTimezoneOverride', { timezoneId: 'Asia/Shanghai' });
  clockScript = await client.send('Page.addScriptToEvaluateOnNewDocument', { source: 'Date.now = () => Date.UTC(2026, 9, 8, 9, 40);' });
  for (const language of ['zh', 'en']) for (const view of ['decision-models', 'config', 'custom-branches', 'quality', 'session', 'quality-session']) {
    const compact = ['session', 'quality-session'].includes(view);
    const width = compact ? 1200 : 1600;
    const height = compact ? (view === 'session' ? 1180 : 813) : view === 'custom-branches' ? 1840 : view === 'config' ? 1580 : view === 'quality' ? 1420 : 1100;
    await navigate(client, `${base}/decision-docs.html?lang=${language}&view=${view}`, {width,height});
    await waitFor(client, `location.search.includes('lang=${language}&view=${view}') && !!window.__DECISION_DOCS__`);
    if (view === 'decision-models') {
      await waitFor(client, `document.querySelectorAll('.decision-models-page .list-row').length===3`);
      await clickText(language === 'zh' ? '编辑' : 'Edit');
      await waitFor(client, `!!document.querySelector('input[value="decision-model-preview"]')`);
    } else if (!compact) {
      await waitFor(client, `document.querySelector('.candidate-main strong')?.innerText.includes('Qwen')`);
      if (view === 'quality') {
        await clickText(language === 'zh' ? '模型表现' : 'Model performance');
        await waitFor(client, `document.querySelectorAll('.quality-model-row').length===4 && document.body.innerText.includes('0.88')`);
        assert.equal(await evaluate(client, `document.querySelectorAll('.quality-model-row').length`), 4);
        await evaluate(client, `document.querySelector('.quality-view-stages').click()`);
        await waitFor(client, `document.querySelectorAll('.quality-row').length===2`);
        for (const [filter, count] of [['unrated', 1], ['low', 0], ['high', 1], ['all', 2]]) {
          await evaluate(client, `(() => {const s=document.querySelector('.quality-score-filter');s.value='${filter}';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
          await waitFor(client, `document.querySelectorAll('.quality-row').length===${count}`);
        }
        await evaluate(client, `document.querySelector('.quality-view-stages').click()`);
        await waitFor(client, `document.querySelectorAll('.quality-row').length===0`);
      } else {
        // Scroll the real pane to the mode/decision controls; do not hide form fields.
        await evaluate(client, `(() => {const s=document.querySelector('${view === 'custom-branches' ? '.branch-routing-card' : '.plan-mode-switcher'}').closest('.editor-section');const p=document.querySelector('.detail-pane');p.scrollTop+=s.getBoundingClientRect().top-p.getBoundingClientRect().top-16;})()`);
      }
    } else {
      await waitFor(client, `document.querySelectorAll('.quality-row').length===2`);
      if (view === 'session') {
        // The taller guide excerpt exposes the actual stage's decision details.
        await evaluate(client, `document.querySelector('.quality-assessment-details summary').click()`);
        await waitFor(client, `document.body.innerText.includes('${language === 'zh' ? '本次简单概率' : 'Simple probability'}')`);
      }
      assert(await evaluate(client, `document.querySelector('.quality-list').getBoundingClientRect().bottom < innerHeight`), 'Whole stage cards must fit');
      assert.equal(await evaluate(client, `document.querySelectorAll('.quality-evidence-actions button:not(:disabled)').length`), 0);
    }
    await evaluate(client, 'document.activeElement?.blur();document.fonts.ready');
    // Wait for the product's resize observers and disclosure transitions.
    await evaluate(client, 'new Promise(r=>setTimeout(r,350))');
    const state = await evaluate(client, `({overflow:document.documentElement.scrollWidth>innerWidth,errors:[...document.querySelectorAll('[role=alert]')].filter(e=>e.checkVisibility()).map(e=>e.innerText),calls:__DECISION_DOCS__.captureCalls})`);
    assert.equal(state.overflow, false);assert.deepEqual(state.errors, []);
    assert(state.calls.every(c=>['observation_read','compute_management_snapshot','plan_editor_options','decision_services'].includes(c)));
    if (capture) {
      const shot=await client.send('Page.captureScreenshot',{format:'png'});
      fs.writeFileSync(new URL(`${view}-${language==='zh'?'zh-CN':'en'}.png`,assets),Buffer.from(shot.data,'base64'));
    }
    results.push({language,view,status:'green',readOnlyCalls:state.calls});
  }
  console.log(JSON.stringify({browser:'Isolated macOS Chromium; current components, not native Desktop',results},null,2));
} finally {
  if(clockScript) await client.send('Page.removeScriptToEvaluateOnNewDocument',{identifier:clockScript.identifier});
  client.close();clearInterval(keepAlive);
}
