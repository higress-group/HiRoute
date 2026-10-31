import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';
const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const visible = `e=>e.getClientRects().length && !e.matches(':disabled')`;
async function click(selector, text = '') {
  await waitFor(client, `(()=>{const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>(${visible})(e)&&e.textContent.includes(${JSON.stringify(text)}));if(!e)return false;e.click();return true;})()`);
}
async function change(selector, value) {
  await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(${selector.includes('choice') ? 'HTMLSelectElement' : 'HTMLInputElement'}.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(${selector.includes('choice') ? "'change'" : "'input'"},{bubbles:true}));return true;})()`);
}
async function screenshot(name) {
  await evaluate(client, 'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(true))))');
  const data = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(data.data, 'base64'));
}
async function scrollTo(selector, end = false) {
  await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(selector)}), b=e.closest('.modal-body');if(!b)return false;const r=e.getBoundingClientRect(),br=b.getBoundingClientRect();b.scrollTop+=${end ? 'r.bottom-br.bottom' : 'r.top-br.top'};return true;})()`);
}
async function open({ provider = 'codex', entry = 'managed', mode = '', language = 'zh', theme = 'dark', width = 755, height = 600, existing = false } = {}) {
  const query = new URLSearchParams({ provider, entry, mode, lang: language, theme, ...(existing ? { existing: '1' } : {}) });
  await navigate(client, `${base}subscription-api-setup.html?${query}`, { width, height });
  await waitFor(client, 'Boolean(window.inlineSetup) && !!document.querySelector(".modal")');
  if (entry === 'managed') {
    await click('.add-choice', language === 'zh' ? '连接订阅' : 'Connect subscription');
    await click('button', language === 'zh' ? '检查并选择模型' : 'Check and choose models');
  } else if (entry === 'native') {
    await click('button', language === 'zh' ? '接入' : 'Connect');
  }
  await click('.modal-foot .btn-primary', language === 'zh' ? '检查订阅' : 'Check subscription');
  await waitFor(client, 'document.querySelector("[data-subscription-api-choice]") && document.querySelectorAll(".v3-catalog input[type=checkbox]").length===2 && !document.querySelector(".modal-foot .btn-primary").disabled');
  if (entry === 'append') await click('.v3-catalog label', 'fixture model 2');
  assert.equal(await evaluate(client, 'document.querySelectorAll("[role=dialog]").length'), 1);
}
async function selectNew() {
  await change('[data-subscription-api-choice]', '@new-provider-api');
  await waitFor(client, '!!document.querySelector("[data-subscription-api-key]")');
  await change('[data-subscription-api-key]', 'fixture-inline-api-key');
}
async function save(language = 'zh') { await click('.modal-foot .btn-primary', language === 'zh' ? '保存并接入' : 'Save and connect'); }
async function linked() { await waitFor(client, 'inlineSetup.state.management.subscription_api_links.length===1 && !document.querySelector("[role=dialog]")'); }
async function run(name, fn) {
  try { await fn(); results.push({ name, state: 'passed' }); }
  catch (error) {
    await screenshot(`failure-${name}`).catch(() => {});
    results.push({ name, state: 'failed', error: String(error.stack ?? error), page: await evaluate(client, '({text:document.body.innerText,trace:inlineSetup?.trace})').catch(() => null) });
  }
}
try {
  for (const provider of ['codex', 'claude']) {
    for (const entry of ['managed', 'native', 'append']) await run(`${provider}-${entry}-single-save`, async () => {
      await open({ provider, entry }); await selectNew();
      assert.equal(await evaluate(client, 'document.querySelectorAll(".modal-foot .btn-primary").length'), 1);
      assert.equal(await evaluate(client, 'document.querySelectorAll("[data-subscription-api-setup] .btn-primary").length'), 0);
      assert.equal(await evaluate(client, 'document.querySelectorAll("[role=dialog]").length'), 1);
      await save(); await linked();
      const actual = await evaluate(client, '({state:inlineSetup.state,trace:inlineSetup.trace,ids:inlineSetup.ids})');
      assert.equal(actual.state.apiCreated, 1);
      const api = actual.state.management.sources.find(s => s.source_id === 'source/api');
      assert.deepEqual(api.models.map(m => m.upstream_model_id), actual.ids);
      const calls = actual.trace.map(x => x.command);
      assert.ok(calls.indexOf('register_protected_model_input') > calls.indexOf('get_compute_save_result'), 'subscription save confirmed before API input/check');
      const check = actual.trace.find(x => x.command === 'check_registered_model_connection_v2');
      assert.equal(check.payload.request.request.connection_option_id, provider === 'codex' ? 'openai.platform.global.v1' : 'anthropic.platform.global.v1');
      assert.equal(check.payload.request.request.inference_model_id, null);
      const lastApi = calls.lastIndexOf('apply_compute_save');
      assert.equal(calls.slice(lastApi + 1).includes('compute_scan'), false, 'completion does not wait on another device scan');
    });
  }
  await run('authentication-failure-retries-only-api', async () => {
    await open({ mode: 'auth' }); await selectNew(); await save();
    await waitFor(client, '!!document.querySelector("[data-subscription-saved]") && !document.querySelector(".modal-foot .btn-primary").disabled');
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 0);
    assert.equal(await evaluate(client, 'document.querySelector("[data-subscription-api-key]").value'), '');
    await waitFor(client, 'document.activeElement===document.querySelector("[data-subscription-api-key]")');
    await scrollTo('[data-subscription-api-setup]');
    await screenshot('single-save-api-auth-failure');
    await evaluate(client, 'inlineSetup.setMode("")');
    await click('.modal-foot .btn-primary', '完成 API 设置'); await linked();
    const count = await evaluate(client, 'inlineSetup.trace.filter(x=>x.command==="apply_compute_save" && x.payload.request.spec.desired_state.change.subject.kind==="candidate" && x.payload.request.spec.desired_state.change.subject.candidate.candidate_ref.startsWith("candidate/cpa")).length');
    assert.equal(count, 1, 'retry never resaves retained subscription');
  });
  for (const mode of ['lost', 'link_failure', 'link_pending']) await run(`${mode}-reuses-saved-api`, async () => {
    await open({ mode }); await selectNew(); await save();
    await waitFor(client, '!!document.querySelector("[data-subscription-saved]") && !document.querySelector(".modal-foot .btn-primary").disabled');
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 1);
    if (mode === 'link_pending') {
      assert.equal(await evaluate(client, 'inlineSetup.state.management.subscription_api_links.length'), 1, 'active metadata alone cannot finish a pending operation');
      await screenshot('single-save-link-pending');
    }
    await evaluate(client, 'inlineSetup.setMode("")');
    await click('.modal-foot .btn-primary'); await linked();
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 1);
    if (mode === 'link_pending') assert.equal(await evaluate(client, 'inlineSetup.trace.filter(x=>x.command==="preview_compute_save_v3"&&x.payload.apiLink).length'), 1, 'retry observes the original association save');
  });
  for (const provider of ['codex', 'claude']) await run(`${provider}-existing-api-single-save`, async () => {
    await open({ provider, existing: true });
    await change('[data-subscription-api-choice]', 'source/existing-api');
    await save(); await linked();
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 0);
    assert.equal(await evaluate(client, 'inlineSetup.trace.some(x=>x.command==="register_protected_model_input")'), false);
    assert.equal(await evaluate(client, 'inlineSetup.state.management.subscription_api_links[0].api_source_id'), 'source/existing-api');
  });
  await run('saved-subscription-adds-api-inline', async () => {
    await navigate(client, `${base}subscription-api-setup.html?entry=saved`, { width: 1200, height: 813 });
    await waitFor(client, 'Boolean(window.inlineSetup)');
    await click('button', '关联 API 接入'); await selectNew();
    await click('button.btn-primary', '保存设置');
    await waitFor(client, 'inlineSetup.state.management.subscription_api_links.length===1 && !document.querySelector("[data-subscription-api-choice]")');
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 1);
    assert.equal(await evaluate(client, 'inlineSetup.trace.some(x=>x.command==="check_subscription")'), false);
    assert.equal(await evaluate(client, 'inlineSetup.trace.filter(x=>x.command==="apply_compute_save"&&x.payload.request.spec.desired_state.change.subject.kind==="candidate"&&x.payload.request.spec.desired_state.change.subject.candidate.candidate_ref.startsWith("candidate/cpa")).length'), 0);
  });
  await run('empty-api-keeps-subscription-and-key-editor', async () => {
    await open({ mode: 'unsupported' }); await selectNew(); await save();
    await waitFor(client, '!!document.querySelector("[data-subscription-saved]") && document.body.innerText.includes("同 ID")');
    assert.equal(await evaluate(client, 'inlineSetup.state.savedSubscription'), true);
    assert.equal(await evaluate(client, 'inlineSetup.state.apiCreated'), 0);
    assert.equal(await evaluate(client, 'document.querySelectorAll(".v3-catalog input[type=checkbox]").length'), 0, 'selection already saved');
    await click('.modal-foot .btn', '稍后完成');
    assert.equal(await evaluate(client, 'inlineSetup.state.management.sources.some(s=>s.source_id==="source/subscription")'), true);
  });
  await run('missing-key-focuses-inline-field-before-saving', async () => {
    await open(); await change('[data-subscription-api-choice]', '@new-provider-api');
    await save();
    assert.equal(await evaluate(client, 'document.activeElement===document.querySelector("[data-subscription-api-key]")'), true);
    assert.equal(await evaluate(client, 'inlineSetup.state.savedSubscription'), false);
  });
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[755, 600], [1200, 813]]) await run(`layout-${language}-${theme}-${width}`, async () => {
    await open({ provider: language === 'en' ? 'claude' : 'codex', language, theme, width, height }); await selectNew();
    await click('.subscription-api-credits summary');
    const geometry = await evaluate(client, `(()=>{const d=document.querySelector('[role=dialog]'), f=d.querySelector('.modal-foot'), b=f.querySelector('.btn-primary'), form=d.querySelector('[data-subscription-api-setup]');const rect=e=>{const r=e.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height}};return {dialog:rect(d),footer:rect(f),button:rect(b),bodyScroll:d.scrollWidth,bodyClient:d.clientWidth,fields:[...form.querySelectorAll('.field-label,input,select')].map(rect),text:form.innerText}})()`);
    assert.ok(geometry.button.top >= 0 && geometry.button.bottom <= height + 1, 'main action stays visible');
    assert.ok(geometry.bodyScroll <= geometry.bodyClient + 1, 'no horizontal overflow');
    for (const field of geometry.fields) assert.ok(field.width > 100 && field.left >= geometry.dialog.left - 1 && field.right <= geometry.dialog.right + 1, 'labels/controls stay readable within dialog');
    assert.match(geometry.text, language === 'zh' ? /赠金/ : /credits/);
    assert.match(geometry.text, language === 'zh' ? /官方/ : /official/);
    await screenshot(`single-save-${language}-${theme}-${width}-selection`);
    await scrollTo('[data-subscription-api-setup]');
    await screenshot(`single-save-${language}-${theme}-${width}`);
    await scrollTo('.subscription-api-credits details', true);
    await screenshot(`single-save-${language}-${theme}-${width}-credits`);
  });
} finally {
  await writeFile(path.join(output, 'subscription-api-setup-report.json'), JSON.stringify({ tests: results.length, failed: results.filter(x => x.state === 'failed').length, results }, null, 2));
  for (const item of results) process.stdout.write(`${item.state}: ${item.name}${item.error ? ` ${item.error.split('\n')[0]}` : ''}\n`);
  client.close();
  if (results.length !== 23 || results.some(x => x.state === 'failed')) process.exitCode = 1;
}
