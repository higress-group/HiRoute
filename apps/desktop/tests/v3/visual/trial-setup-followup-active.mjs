import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
const capturedApi = JSON.parse(await readFile(new URL('../../../../../contracts/routing/trial-retained-api-capture.v1.json', import.meta.url),'utf8'));
await client.send('Page.addScriptToEvaluateOnNewDocument', {source:`window.trialRetainedCapture=${JSON.stringify(capturedApi)};`});
await client.send('Page.addScriptToEvaluateOnNewDocument', {source:`window.setupErrors=[];addEventListener('error',e=>window.setupErrors.push({message:e.message,stack:e.error?.stack}));addEventListener('unhandledrejection',e=>window.setupErrors.push({message:String(e.reason),stack:e.reason?.stack}));`});
async function open(query, language='en', width=755, height=600) {
  const url = `${base}new-model-trial.html?view=quickstart&native=1&lang=${language}&${query}`;
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', {width,height,deviceScaleFactor:1,mobile:false});
  await client.send('Page.navigate',{url});
  await waitFor(client, `location.href===${JSON.stringify(url)} && !!document.querySelector('.new-models button')`);
  await click('.new-models button', language==='zh'?'试用':'Try');
  await waitFor(client, `!!document.querySelector('.trial-agent-choice') && !!document.querySelector('.trial-footer')`);
}
async function click(selector, text = '') {
  const expression = `[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&e.textContent.includes(${JSON.stringify(text)}))`;
  // Check inherited fieldset disability and click in the same browser turn,
  // rather than selecting just before a coherent refresh disables the form.
  await waitFor(client, `(()=>{const element=${expression};if(!element)return false;element.click();return true;})()`);
}
async function field(selector, value) {
  await waitFor(client, `!!document.querySelector(${JSON.stringify(selector)})`);
  await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
}
// The combined trial skips the chooser when exactly one service is executable.
// Preserve the saved-binding assertions while following that production journey.
async function connectedService() {
  await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false && document.querySelector('#mc-connection-form .connection-config-heading h3')?.textContent.includes('Kimi')`);
}
async function saveConnection(multiple=false) {
  await click('.trial-model-card','Kimi K3');
  await click('.trial-connect-card button','Connect here');
  await waitFor(client, `!!document.querySelector('.trial-inline-connection .connection-flow')`);
  await connectedService();
  await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
  await field('#mc-connection-form input[type=password]', 'synthetic-fixture-key');
  await click('.modal-foot button', 'Check and continue');
  await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
  await click('.model-selection input[type=checkbox]');
  if (multiple) await evaluate(client, `document.querySelectorAll('.model-selection input[type=checkbox]')[1].click();true`);
  await click('.modal-foot button', 'Save connection');
}
async function shot(name) {
  await evaluate(client, `Promise.all(document.getAnimations().filter(animation=>animation.effect?.getComputedTiming().iterations!==Infinity).map(animation=>animation.finished.catch(()=>{}))).then(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true)))))`);
  const image = await client.send('Page.captureScreenshot',{format:'png'});
  await writeFile(path.join(output,`${name}.png`),Buffer.from(image.data,'base64'));
}
async function run(id, test) {
  if (process.env.TRIAL_SETUP_CASE && !process.env.TRIAL_SETUP_CASE.split(',').includes(id)) return;
  try { await test(); results.push({id,state:'passed'}); }
  catch (error) {
    await shot(`failure-${id}`).catch(()=>{});
    const page = await evaluate(client, `({text:document.body.innerText,trace:window.trialTrace,errors:window.setupErrors})`).catch(()=>null);
    results.push({id,state:'failed',error:String(error.stack??error),page});
  }
}
try {
  await run('review-r2.retained-api-selection', async () => {
    await open('retained-api=1');
    await waitFor(client, `document.querySelector('[data-trial-target=trial]')?.textContent.includes('Retained fixture API')`);
    await click('.trial-choice-list .trial-choice','GPT');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review')`);
    await click('.trial-review > .actions button','Back to setup');
    await click('[data-trial-connected-model]');
    const source=await evaluate(client, `(()=>{const s=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Retained fixture API'));return s&&{text:s.textContent,buttons:[...s.querySelectorAll('button')].map(b=>b.textContent)};})()`);
    assert.ok(source && source.text.includes('Qwen'));
    assert.ok(source.buttons.every(text=>!/Add model|Review connection|edit/i.test(text)),JSON.stringify(source));
    await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await click('.trial-model-card','Kimi');
    await click('[data-trial-target=primary] button','Choose a connected model');
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Retained fixture API'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector('[data-trial-target=primary]')?.textContent.includes('Retained fixture API')`);
  });
  await run('review-r2.production-captured-api-selection', async () => {
    await open('production-capture=1');
    await click('[data-trial-connected-model]');
    const row=await evaluate(client, `(()=>{const s=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Zhipu · Coding Plan'));return s&&{text:s.textContent,buttons:[...s.querySelectorAll('button')].map(b=>b.textContent)};})()`);
    assert.ok(row && row.text.includes('GLM-5.3'));
    assert.ok(row.buttons.every(text=>!/Add model|Review connection|edit/i.test(text)),JSON.stringify(row));
    await evaluate(client, `(()=>{const s=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Zhipu · Coding Plan'));s.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector('[data-trial-target=trial]')?.textContent.includes('Zhipu · Coding Plan')`);
    await click('.trial-choice-list .trial-choice','GPT');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review')`);
  });
  await run('review-r2.retained-api-cannot-bypass-owner-or-identity', async () => {
    for (const query of ['retained-api=1&retained-mismatch=1','retained-api=1&retained-v2-owner=1&disabled-trial=1']) {
      await open(query); await click('.trial-choice-list .trial-choice','GPT');
      await click('.trial-footer .btn-primary','Review changes');
      await waitFor(client, `document.querySelector('.trial-validation-summary')?.textContent.includes('trial connection')`);
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
      await click('[data-trial-connected-model]');
      const retained=await evaluate(client, `(()=>{const s=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Retained fixture API'));return s?.querySelector('.trial-choice')?.disabled;})()`);
      assert.equal(retained,true,'a mismatched or V2-owned retained description cannot grant selection');
    }
  });
  await run('review-r2.recommendations-retry', async () => {
    await open('recommendations-fail=1');
    await waitFor(client, `document.querySelector('[role=alert]')?.textContent.includes('Recommended models')`);
    await click('.trial-choice-list .trial-choice','GPT');
    await click('[role=alert] button','Retry');
    await waitFor(client, `!![...document.querySelectorAll('.trial-model-card')].find(e=>e.textContent.includes('Kimi'))`);
    assert.equal(await evaluate(client, `trialSetupControl.recommendationReads`),3);
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review')`);
  });
  await run('review-r2.recommendations-fail-and-single-flight', async () => {
    await open('recommendations-fail=1');
    await waitFor(client, `document.querySelector('[role=alert]')?.textContent.includes('Recommended models')`);
    await click('.trial-choice-list .trial-choice','GPT'); await field('[data-trial-target=name] input','Kept trial name');
    await evaluate(client, `trialSetupControl.recommendationFailures=1;true`);
    await click('[role=alert] button','Retry');
    await waitFor(client, `trialSetupControl.recommendationReads===3 && document.querySelector('[role=alert]')?.textContent.includes('Recommended models')`);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'Kept trial name');
    await evaluate(client, `trialSetupControl.holdNextRecommendation=true;true`);
    await click('[role=alert] button','Retry');
    await waitFor(client, `trialSetupControl.pendingRecommendations.length===1`);
    await click('.trial-footer button','Refresh availability');
    await waitFor(client, `!document.querySelector('.trial-setup')?.disabled`);
    assert.equal(await evaluate(client, `trialSetupControl.recommendationReads`),4,'page refresh must not start a concurrent recommendation read');
    const selectedBefore=await evaluate(client, `document.querySelector('[data-trial-connected-model]')?.textContent`);
    await evaluate(client, `trialSetupControl.releaseRecommendation();true`);
    await waitFor(client, `!![...document.querySelectorAll('.trial-model-card')].find(e=>e.textContent.includes('Kimi'))`);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'Kept trial name');
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-connected-model]')?.textContent`),selectedBefore);
  });
  await run('review-r2.recommendation-error-layout', async () => {
    for (const [language,theme,width,height] of [['en','light',755,600],['zh','dark',1200,813]]) {
      await open(`recommendations-fail=1&theme=${theme}`,language,width,height);
      await waitFor(client, `!!document.querySelector('[data-trial-target=trial] [role=alert]')`);
      const bounds=await evaluate(client, `(()=>{const e=document.querySelector('[data-trial-target=trial] [role=alert]'),r=e.getBoundingClientRect();return {left:r.left,right:r.right,width:innerWidth,overflow:e.scrollWidth>e.clientWidth+1};})()`);
      assert.ok(bounds.left>=0 && bounds.right<=bounds.width && !bounds.overflow,JSON.stringify(bounds));
      await shot(`review-r2-recommendations-${language}-${theme}-${width}`);
    }
  });
  await run('review-r2.old-recommendations-after-reopen', async () => {
    await open('recommendations-fail=1');
    await waitFor(client, `!!document.querySelector('[data-trial-target=trial] [role=alert]')`);
    await evaluate(client, `trialSetupControl.holdNextRecommendation=true;true`);
    await click('[data-trial-target=trial] [role=alert] button','Retry');
    await waitFor(client, `trialSetupControl.pendingRecommendations.length===1`);
    await click('.trial-footer button','Cancel');
    await waitFor(client, `!!document.querySelector('.new-models button')`);
    await evaluate(client, `trialModel.display_name='Fresh fixture model';true`);
    await click('.new-models button','Try');
    await waitFor(client, `document.querySelector('[data-trial-connected-model]')?.dataset.trialBinding==='binding/bailian/qwen-coder'`);
    const currentLabel = await evaluate(client, `document.querySelector('[data-trial-connected-model]').textContent`);
    await evaluate(client, `trialSetupControl.releaseRecommendation();true`);
    await waitFor(client, `trialSetupControl.pendingRecommendations.length===0`);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-connected-model]')?.textContent`),currentLabel);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-connected-model]')?.dataset.trialBinding`),'binding/bailian/qwen-coder');
  });
  await run('review-r2.same-model-selection-wins', async () => {
    await open('additional-connection=1&same-names=1&saved-registered=1&saved-read-failure=1&same-model-save=1');
    await click('.trial-choice-list .trial-choice','GPT');
    await click('[data-trial-target=trial] button','Connect another source'); await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[type=password]','synthetic-fixture-key');
    await click('.modal-foot button','Check and continue'); await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button','Save connection');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('could not be verified')`);
    await click('.trial-recovery button','Back to trial');
    await waitFor(client, `!!document.querySelector('[data-trial-target=trial] .trial-connection-field select') && !document.querySelector('.trial-setup')?.disabled`);
    await evaluate(client, `(()=>{const e=document.querySelector('[data-trial-target=trial] .trial-connection-field select');e.value='binding/bailian/qwen-second';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=trial] .trial-connection-field select').value`),'binding/bailian/qwen-second');
    await click('[data-trial-target=trial] button','Review connection result'); await click('.trial-recovery button','Refresh result');
    await waitFor(client, `!document.querySelector('.trial-recovery button:disabled')`);
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=trial] .trial-connection-field select').value`),'binding/bailian/qwen-second','recovery must not replace a later same-model binding choice');
    assert.match(await evaluate(client, `document.querySelector('.trial-recovery').textContent`),/Use this saved model/);
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('[data-trial-target=trial] .trial-connection-field select')?.value==='binding/fixture/saved-model'`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('review-r2.same-model-selection-protects-draft', async () => {
    await open('additional-connection=1&same-names=1');
    await click('[data-trial-target=trial] button','Connect another source'); await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[type=password]','synthetic-draft-key');
    const chooseSecond = `(()=>{const e=document.querySelector('[data-trial-target=trial] .trial-connection-field select');e.value='binding/bailian/qwen-second';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`;
    await evaluate(client, chooseSecond); await waitFor(client, `!!document.querySelector('.discard-dialog[open]')`);
    await click('.discard-dialog button','Keep editing');
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=trial] .trial-connection-field select').value`),'binding/bailian/qwen-coder');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`),'synthetic-draft-key');
    await click('[data-trial-target=primary] button','Choose a connected model'); await click('.trial-picker .trial-choice','GPT-6-Astra');
    await waitFor(client, `!document.querySelector('.trial-picker')`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`),'synthetic-draft-key');
    await evaluate(client, chooseSecond); await waitFor(client, `!!document.querySelector('.discard-dialog[open]')`);
    await click('.discard-dialog button','Discard changes');
    await waitFor(client, `document.querySelector('[data-trial-target=trial] .trial-connection-field select')?.value==='binding/bailian/qwen-second'`);
    assert.equal(await evaluate(client, `!!document.querySelector('#mc-connection-form')`),false);
  });
  await run('review-r2.safe-generated-name', async () => {
    await open('unsafe-model-name=1'); await click('.trial-choice-list .trial-choice','GPT');
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'Model trial');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review')`);
  });
  await run('review-r2.manual-sensitive-name', async () => {
    for (const language of ['en','zh']) {
      await open('',language); await click('.trial-choice-list .trial-choice','GPT');
      await field('[data-trial-target=name] input','https://example.invalid');
      await click('.trial-footer .btn-primary',language==='zh'?'检查改动':'Review changes');
      assert.ok(await evaluate(client, `!!document.querySelector('[data-trial-target=name] .oc-inline-error') && !document.querySelector('.trial-review')`));
      assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'https://example.invalid');
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
      await field('[data-trial-target=name] input','API key rotation');
      await click('.trial-footer .btn-primary',language==='zh'?'检查改动':'Review changes');
      await waitFor(client, `!!document.querySelector('.trial-review')`);
    }
  });
  await run('review-r2.review-name-scalar-boundary', async () => {
    await open(''); await click('.trial-choice-list .trial-choice','GPT');
    await click('.trial-footer .btn-primary','Review changes'); await waitFor(client, `!!document.querySelector('.trial-review')`);
    const input='.trial-review .plan-identity-fields input';
    await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(input)});e.focus();e.select();return true;})()`);
    await client.send('Input.insertText',{text:'🚀'.repeat(128)});
    assert.equal(await evaluate(client, `Array.from(document.querySelector(${JSON.stringify(input)}).value).length`),128,'paste must retain all 128 scalar values');
    await client.send('Input.insertText',{text:'🚀'});
    await waitFor(client, `document.querySelector('.plan-identity-fields .oc-inline-error')?.textContent.includes('128')`);
    await waitFor(client, `document.querySelector('.trial-review > .actions .btn-primary')?.disabled`);
    await evaluate(client, `document.querySelector('.plan-identity-fields .oc-inline-error').scrollIntoView({block:'center'});true`);
    await shot('review-r2-name-limit-en-755');
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
    await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(input)});e.focus();e.select();return true;})()`);
    await client.send('Input.insertText',{text:'🚀'.repeat(128)});
    await click('.trial-review > .actions button','Back to setup');
    assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'🚀'.repeat(128));
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
    await click('.trial-review > .actions .btn-primary','Save and publish route');
    await waitFor(client, `trialTrace.some(x=>x.command==='preview_plan_editor' && x.payload.input.action==='save_draft')`);
    assert.equal(await evaluate(client, `trialTrace.find(x=>x.command==='preview_plan_editor' && x.payload.input.action==='save_draft').payload.input.editor.display_name`),'🚀'.repeat(128));
  });
  await run('capture.retry-at-most-once', async () => {
    await open('');
    const before=await evaluate(client, `trialTrace.filter(x=>x.command==='trial_setup_snapshot').length`);
    const catalog=await evaluate(client, `trialTrace.filter(x=>x.command==='model_availability').length`);
    await evaluate(client, `trialSetupControl.snapshotFailures=['READ_SNAPSHOT_CHANGED'];[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `trialTrace.filter(x=>x.command==='trial_setup_snapshot').length===${before+2}`);
    await waitFor(client, `!document.querySelector('.trial-footer button[disabled]')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='model_availability').length`),catalog);
    await evaluate(client, `trialSetupControl.snapshotFailures=['CHANGE_PREVIEW_STALE','CHANGE_PREVIEW_STALE','CHANGE_PREVIEW_STALE'];[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `document.querySelector('.trial-page').textContent.includes('could not be read')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='trial_setup_snapshot').length`),before+4);
    assert.equal(await evaluate(client, `trialSetupControl.snapshotFailures.length`),1);
  });
  await run('capture.descriptive-catalog-does-not-grant-binding', async () => {
    await open('');
    await waitFor(client, `!!document.querySelector('[data-trial-target="trial-reasoning"]')`);
    await evaluate(client, `trialSetupControl.catalogAdmission=false;[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.toLowerCase().includes('catalog match unconfirmed')`);
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `document.querySelector('.trial-validation-summary')?.textContent.includes('trial connection')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  });
  await run('capture.unadmitted-user-binding-stays-unlinked', async () => {
    await open('catalog-unadmitted-user=1');
    const selected = `:is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])`;
    assert.equal(await evaluate(client, `!!document.querySelector(${JSON.stringify(selected)}+' .model-fact-tags')`),false,
      'descriptive facts cannot be attached to a source-local candidate without catalog admission');
    await click('.trial-model-card','Kimi');
    await click('[data-trial-connected-model]');
    await click('.trial-picker .trial-choice','Qwen3-Coder-Plus');
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.includes('Qwen')`);
    assert.equal(await evaluate(client, `!!document.querySelector(${JSON.stringify(selected)}+' .model-fact-tags')`),false);
    assert.match(await evaluate(client, `document.querySelector('[data-trial-target=trial]').textContent`),/catalog version unconfirmed/i);
  });
  await run('readiness.all-blockers', async () => {
    await open('no-reasoning-default=1');
    await click('.trial-choice-list .trial-choice', 'GPT');
    const disabled = await evaluate(client, `document.querySelector('.trial-footer .btn-primary').disabled`);
    assert.equal(disabled, false, 'ordinary missing reasoning must leave review available for validation');
    await click('.trial-footer .btn-primary', 'Review changes');
    await waitFor(client, `document.querySelector('.trial-validation-summary')?.textContent.includes('Complete')`);
    assert.match(await evaluate(client, `document.querySelector('.trial-validation-summary').textContent`), /reasoning/i);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  });
  await run('save.user-declared-binding', async () => {
    await open('saved-user-declared=1');
    const catalogReads=await evaluate(client, `trialTrace.filter(x=>x.command==='model_availability').length`);
    const serviceReads=await evaluate(client, `trialTrace.filter(x=>x.command==='decision_services_v2').length`);
    await saveConnection();
    await waitFor(client, `!!document.querySelector('.trial-recovery')`);
    await waitFor(client, `!document.querySelector('.trial-recovery button:disabled')`);
    assert.match(await evaluate(client, `document.querySelector('.trial-recovery').textContent`),/Use this saved model/i,
      'saved unlinked declaration must offer its actual executable binding explicitly');
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('.trial-page').textContent.includes('Saved synthetic model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='check_registered_model_connection').some(x=>x.payload.request.inference_model_id)`),false);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='model_availability').length`),catalogReads,'unlinked save does not synchronously reread unrelated catalog history');
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='decision_services_v2').length`),serviceReads,'saved model selection does not wait for unrelated decision services');
  });
  await run('save.unlinked-id-never-becomes-catalog-proof', async () => {
    await open('saved-user-declared=1&saved-id-collision=1'); await saveConnection();
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.includes('Saved synthetic model')`);
    assert.equal(await evaluate(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model]) .model-fact-tags')`),null);
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-model-facts')`),false);
    assert.match(await evaluate(client, `document.querySelector('[data-trial-target=trial]').textContent`),/catalog version unconfirmed/i);
  });
  await run('picker.unlinked-collision-requires-explicit-choice', async () => {
    await open('unlinked-existing-collision=1');
    await click('.trial-model-card','Kimi K3');
    assert.match(await evaluate(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])').textContent`),/catalog match unconfirmed/i);
    assert.equal(await evaluate(client, `!!document.querySelector('[data-trial-target="trial-reasoning"]')`),false,'the catalog card cannot silently select an unlinked saved binding');
    await click('[data-trial-connected-model]');
    await click('.trial-picker .trial-choice','Saved synthetic model');
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.includes('Saved synthetic model')`);
    assert.equal(await evaluate(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model]) .model-fact-tags')`),null);
    assert.match(await evaluate(client, `document.querySelector('[data-trial-target=trial]').textContent`),/catalog version unconfirmed/i);
  });
  await run('save.custom-check-path', async () => {
    await open('saved-user-declared=1&saved-custom=1');
    await click('.trial-model-card','Kimi K3'); await click('.trial-connect-card button','Connect here');
    await connectedService();
    await click('.modal-foot button','Change service');
    await click('.connection-custom-entry');
    await waitFor(client, `!!document.querySelector('#mc-connection-form')`);
    const setLabel = async (label, value) => {
      const expression=`[...document.querySelectorAll('#mc-connection-form .field, #mc-manual-form .field')].find(e=>e.querySelector('.field-label')?.textContent.trim()===${JSON.stringify(label)})?.querySelector('input')`;
      await waitFor(client, `!!(${expression})`);
      await evaluate(client, `(()=>{const e=${expression};Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    };
    await setLabel('Connection name','Synthetic custom source');
    await setLabel('Base URL','https://synthetic.invalid/v1');
    await setLabel('API Key','synthetic-fixture-key');
    await click('#mc-connection-form button','Edit models manually');
    await setLabel('Model ID','fixture-unlinked-model');
    await click('.modal-foot button','Check and save');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='check_model_connection').length`),1);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('readiness.compound-live-focus', async () => {
    await open('no-trial-reasoning-default=1&disabled-trial=1');
    await click('.trial-choice-list .trial-choice','GPT');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `document.querySelector('.trial-validation-summary')?.textContent.includes('trial reasoning')`);
    await waitFor(client, `!!document.activeElement.closest('[data-trial-target="trial"]')`);
    const summary=await evaluate(client, `document.querySelector('.trial-validation-summary').textContent`);
    assert.match(summary,/trial connection/i); assert.match(summary,/trial reasoning/i);
    const geometry=await evaluate(client, `(()=>{const a=document.activeElement.getBoundingClientRect(), f=document.querySelector('.trial-footer').getBoundingClientRect();return {a:{top:a.top,bottom:a.bottom},footer:f.top}})()`);
    assert.ok(geometry.a.top>=0 && geometry.a.bottom<=geometry.footer+1,JSON.stringify(geometry));
    await shot('readiness-compound-en-755');
    assert.equal(await evaluate(client, `document.querySelectorAll('.trial-validation-summary[role=alert]').length`),1);
    await click('.trial-validation-summary button','trial reasoning');
    await waitFor(client, `!!document.activeElement.closest('[data-trial-target="trial-reasoning"]')`);
    const second=await evaluate(client, `(()=>{const a=document.activeElement.getBoundingClientRect(),f=document.querySelector('.trial-footer').getBoundingClientRect();return {top:a.top,bottom:a.bottom,footer:f.top};})()`);
    assert.ok(second.top>=0 && second.bottom<=second.footer+1,JSON.stringify(second));
    await evaluate(client, `trialSetupControl.setTrialAvailability(true);true`);
    await click('.trial-footer button','Refresh availability');
    await waitFor(client, `!!document.querySelector('.trial-validation-summary') && !document.querySelector('.trial-validation-summary').textContent.includes('trial connection')`);
    assert.match(await evaluate(client, `document.querySelector('.trial-validation-summary').textContent`),/trial reasoning/i);
    await click('[data-trial-target="trial-reasoning"] button','Choose reasoning');
    await click('[role=dialog] .segment','On'); await click('[role=dialog] button','Apply');
    await waitFor(client, `!document.querySelector('.trial-validation-summary')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  });
  await run('readiness.binding-and-primary-reasoning', async () => {
    await open('disabled-trial=1&no-reasoning-default=1');
    await click('.trial-choice-list .trial-choice','GPT');
    assert.equal(await evaluate(client, `document.querySelector('.trial-footer .btn-primary').disabled`),false,'field errors keep review available for validation');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-validation-summary')`);
    const summary=await evaluate(client, `document.querySelector('.trial-validation-summary').textContent`);
    assert.match(summary,/trial connection/i); assert.match(summary,/Primary reasoning/i);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  });
  await run('readiness.stale-reasoning-repair', async () => {
    await open('no-trial-reasoning-default=1');
    await click('[data-trial-target="trial-reasoning"] button','Choose reasoning');
    await click('[role=dialog] .segment','On'); await click('[role=dialog] button','Apply');
    await evaluate(client, `trialSetupControl.trialReasoningMode='default';true`);
    await click('.trial-footer button','Refresh availability');
    await waitFor(client, `document.querySelector('[data-trial-target="trial-reasoning"]')?.textContent.includes('current reasoning setting or capability is invalid')`);
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-validation-summary')`),false,'a known stale value is explained before an attempted review');
    await click('[data-trial-target="trial-reasoning"] button','Use the model’s fixed setting');
    await waitFor(client, `!document.querySelector('[data-trial-target="trial-reasoning"] .oc-inline-error')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  });
  await run('picker.exact-source-no-discovery', async () => {
    await open('same-names=1');
    const before=await evaluate(client, `trialTrace.length`);
    await click('[data-trial-connected-model]');
    await waitFor(client, `!!document.querySelector('.trial-picker')`);
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('A second provider'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector('[data-trial-target=trial]').textContent.includes('A second provider')`);
    await click('[data-trial-target=primary] button','Choose a connected model');
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('Codex'));[...source.querySelectorAll('.trial-choice')].find(e=>e.textContent.includes('GPT-5.6-Sol')).click();return true;})()`);
    await waitFor(client, `!document.querySelector('.trial-picker') && !!document.querySelector('[data-trial-target=primary] .trial-choice.selected')`);
    const commands=await evaluate(client, `trialTrace.slice(${before}).map(x=>x.command)`);
    assert.equal(commands.some(command=>['compute_scan','compute_subscriptions','check_model_connection','check_registered_model_connection','apply_compute_save'].includes(command)),false,JSON.stringify(commands));
    await shot('picker-selected-same-name-en-755');
    await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
    await click('.trial-review > .actions .btn-primary','Save and publish route');
    await waitFor(client, `!!document.querySelector('.trial-mapping')`);
    const saved=await evaluate(client, `trialTrace.find(x=>x.command==='preview_plan_editor' && x.payload.input.action==='save_draft').payload.input.editor`);
    assert.equal(saved.smart.economy[0].binding_id,'binding/second/same-name');
    assert.equal(saved.smart.primary[0].binding_id,'binding/codex/gpt-5.6-sol');
  });
  await run('picker.keyboard-dialog-geometry', async () => {
    for (const [language,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]) {
      await open(`same-names=1&theme=${theme}`,language,width,height);
      await evaluate(client, `document.querySelector('[data-trial-target=trial] [data-trial-focus]').focus();true`);
      await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13,text:'\r',unmodifiedText:'\r'});
      await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
      await waitFor(client, `!!document.querySelector('.trial-picker') && !!document.activeElement.closest('[role=dialog]')`);
      const geometry=await evaluate(client, `(()=>{const modal=document.querySelector('[role=dialog]'),r=modal.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,overflow:[...modal.querySelectorAll('.trial-picker-source,.trial-picker-row,.trial-choice')].some(e=>e.scrollWidth>e.clientWidth+1)};})()`);
      assert.ok(geometry.left>=-1 && geometry.right<=width+1 && geometry.top>=-1 && geometry.bottom<=height+1 && !geometry.overflow,JSON.stringify(geometry));
      await shot(`picker-dialog-${language}-${theme}-${width}`);
      await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
      await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
      await waitFor(client, `!document.querySelector('.trial-picker') && document.activeElement.matches('[data-trial-target=trial] [data-trial-focus]')`);
    }
  });
  await run('picker.changed-binding-and-cancelled-read', async () => {
    await open('');
    await click('[data-trial-connected-model]');
    await waitFor(client, `!!document.querySelector('.trial-picker')`);
    await evaluate(client, `trialSetupControl.setTrialAvailability(false);true`);
    await click('.trial-picker-source .trial-choice','Qwen3-Coder-Plus');
    await waitFor(client, `document.querySelector('.trial-picker [role=alert]')?.textContent.includes('changed or could not be read')`);
    assert.ok(await evaluate(client, `[...document.querySelectorAll('.trial-picker .trial-choice')].find(e=>e.textContent.includes('Qwen3-Coder-Plus')).disabled`));
    assert.match(await evaluate(client, `[...document.querySelectorAll('.trial-picker .trial-choice')].find(e=>e.textContent.includes('Qwen3-Coder-Plus')).textContent`),/disabled/i);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),0);
    await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await waitFor(client, `!document.querySelector('.trial-picker')`);
    await open('same-names=1');
    await click('[data-trial-connected-model]');
    await evaluate(client, `trialSetupControl.holdNextManagement=true;true`);
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('A second provider'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `trialSetupControl.pending.length===1`);
    await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await waitFor(client, `!document.querySelector('.trial-picker')`);
    await evaluate(client, `trialSetupControl.release();true`);
    await waitFor(client, `trialSetupControl.pending.length===0 && !document.querySelector('.trial-footer .btn-primary').disabled`);
    assert.ok(await evaluate(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.includes('Qwen3-Coder-Plus')`),'a late read cannot replace the role after cancellation');
    await click('[data-trial-connected-model]');
    assert.ok(await evaluate(client, `!!document.querySelector('.trial-picker .trial-choice:not(:disabled)')`));
  });
  await run('picker.append-to-exact-saved-api', async () => {
    await open('append-fixture=1');
    await click('[data-trial-target=trial] button','Connect another source');
    await waitFor(client, `!!document.querySelector('[data-trial-source-choice] option[value="source/bailian/coding"]')`);
    await shot('connect-here-source-choice-en-755');
    await evaluate(client, `(()=>{const e=document.querySelector('[data-trial-source-choice]');e.value='source/bailian/coding';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    await field('[data-trial-request-model]', 'fixture-added-model');
    await click('.trial-reuse-connection button','Check and add');
    await waitFor(client, `!document.querySelector('.trial-reuse-connection') && document.querySelector('[data-trial-target=trial]').textContent.includes('Added synthetic model')`);
    const change=await evaluate(client, `trialSetupControl.appendChange`);
    assert.equal(change.edit.action,'append_models');
    assert.equal(change.subject.candidate.candidate_ref,'candidate/recheck/source/bailian/coding');
    assert.deepEqual(change.selected_model_refs,['model-ref/append-new']);
    assert.deepEqual(change.key_edits,[]);
    const forbidden=await evaluate(client, `trialTrace.filter(x=>['compute_scan','compute_subscriptions','register_protected_model_input'].includes(x.command)).map(x=>x.command)`);
    assert.deepEqual(forbidden,[]);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('picker.append-to-exact-saved-subscription', async () => {
    await open('subscription-append=1');
    await click('[data-trial-target=trial] button','Connect another source');
    await waitFor(client, `!!document.querySelector('[data-trial-source-choice]')`);
    await evaluate(client, `(()=>{const e=document.querySelector('[data-trial-source-choice]');e.value=[...e.options].find(o=>o.textContent.includes('Codex')).value;e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    await field('[data-trial-request-model]', 'fixture-sub-added');
    await click('.trial-reuse-connection button','Check and add');
    await waitFor(client, `!document.querySelector('.trial-reuse-connection') && document.querySelector('[data-trial-target=trial]').textContent.includes('Added subscription model')`);
    const change=await evaluate(client, `trialSetupControl.appendChange`);
    assert.equal(change.edit.action,'append_models'); assert.deepEqual(change.selected_model_refs,['model-ref/sub-added']); assert.deepEqual(change.key_edits,[]);
    const forbidden=await evaluate(client, `trialTrace.filter(x=>['register_protected_model_input','manage_subscription_login'].includes(x.command)).map(x=>x.command)`);
    assert.deepEqual(forbidden,[]); assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.registered-observed', async () => {
    await open('saved-registered=1'); await saveConnection();
    await waitFor(client, `document.querySelector('.trial-saved')?.textContent.includes('Saved synthetic API')`);
    assert.ok(await evaluate(client, `document.querySelector('[data-trial-target=trial]').textContent.includes('Saved synthetic model')`));
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.late-check-cannot-replace-current-evidence', async () => {
    await open('saved-registered=1&check-race=1');
    await click('.trial-model-card','Kimi K3'); await click('.trial-connect-card button','Connect here');
    await connectedService();
    await field('#mc-connection-form input[type=password]','synthetic-fixture-key');
    await click('.modal-foot button','Check and continue');
    await waitFor(client, `trialSetupControl.pendingCheck.length===1`);
    await click('.modal-foot button','Cancel check');
    await click('.modal-foot button','Check and continue');
    await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
    await evaluate(client, `trialSetupControl.releaseCheck();true`);
    await waitFor(client, `trialSetupControl.pendingCheck.length===0`);
    await click('.model-selection input[type=checkbox]');
    await click('.modal-foot button','Save connection');
    await waitFor(client, `document.querySelector('.trial-saved')?.textContent.includes('Saved synthetic API')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.multiple-needs-choice', async () => {
    await open('saved-user-declared=1&saved-multiple=1'); await saveConnection(true);
    await waitFor(client, `document.querySelectorAll('.trial-saved-choice').length===2`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.trial-saved-choice button').length`),2);
    await evaluate(client, `[...document.querySelectorAll('.trial-saved-choice')].find(e=>e.textContent.includes('Second synthetic model')).querySelector('button').click();true`);
    await waitFor(client, `document.querySelector('[data-trial-target=trial]').textContent.includes('Second synthetic model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.excludes-unselected-later-binding', async () => {
    await open('saved-user-declared=1&saved-extra-binding=1'); await saveConnection();
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.trial-saved-choice').length`),1);
    assert.doesNotMatch(await evaluate(client, `document.querySelector('.trial-recovery').textContent`),/Second synthetic model/);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.failed-read-retry-once', async () => {
    await open('saved-user-declared=1&saved-read-failure=1'); await saveConnection();
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('could not be verified')`);
    await click('.trial-recovery button','Refresh result');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.lost-reply-recovers-original-operation', async () => {
    await open('saved-user-declared=1&lost-apply=1'); await saveConnection();
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Verifying connection result')`);
    await click('.trial-recovery button','Refresh result');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='observe_operation').length`),1);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='get_compute_save_result').length`),1);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('[data-trial-target=trial]').textContent.includes('Saved synthetic model')`);
  });
  await run('save.lost-reply-rejects-replaced-operation', async () => {
    await open('saved-user-declared=1&lost-apply=1'); await saveConnection();
    await waitFor(client, `!!document.querySelector('.trial-recovery')`);
    await evaluate(client, `trialSetupControl.replacePending();true`);
    await click('.trial-recovery button','Refresh result');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('still unconfirmed')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='get_compute_save_result').length`),0);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.stale-revision-remains-saved', async () => {
    for (const mismatch of ['saved-revision-mismatch','saved-candidate-mismatch']) {
      await open(`saved-user-declared=1&${mismatch}=1`); await saveConnection();
      await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('could not be confirmed')`);
      assert.doesNotMatch(await evaluate(client, `document.querySelector('.trial-recovery').textContent`),/Use this saved model/);
      await click('.trial-recovery button','Refresh result');
      await waitFor(client, `!document.querySelector('.trial-recovery button:disabled')`);
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    }
  });
  await run('save.rechecks-renamed-disabled-and-removed', async () => {
    for (const action of ['rename','disable','remove']) {
      await open('saved-user-declared=1'); await saveConnection();
      await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
      await evaluate(client, `trialSetupControl.changeSaved(${JSON.stringify(action)});true`);
      await click('.trial-recovery button','Use this saved model');
      if (action==='rename') {
        await waitFor(client, `document.querySelector('[data-trial-target=trial]')?.textContent.includes('Renamed synthetic API')`);
      } else {
        await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('changed or is unavailable')`);
        assert.equal(await evaluate(client, `document.querySelectorAll('.trial-saved-choice').length`),0);
      }
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    }
  });
  await run('save.old-read-coalesces-then-refreshes', async () => {
    await open('saved-user-declared=1&hold-apply=1'); await saveConnection();
    await waitFor(client, `trialSetupControl.pendingApply.length===1`);
    await evaluate(client, `trialSetupControl.holdNextManagement=true;[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `trialSetupControl.pending.length===1`);
    await evaluate(client, `trialSetupControl.releaseApply();true`);
    await waitFor(client, `trialTrace.some(x=>x.command==='get_compute_save_result')`);
    await evaluate(client, `trialSetupControl.release();true`);
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.ok(await evaluate(client, `trialTrace.filter(x=>x.command==='trial_setup_snapshot').length>=3`));
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('[data-trial-target=trial]').textContent.includes('Saved synthetic model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('save.does-not-wait-for-unrelated-services', async () => {
    await open('saved-user-declared=1&hold-apply=1'); await saveConnection();
    await waitFor(client, `trialSetupControl.pendingApply.length===1`);
    await evaluate(client, `trialSetupControl.holdNextDecision=true;[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `trialSetupControl.pendingDecision.length===1`);
    await evaluate(client, `trialSetupControl.releaseApply();true`);
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `trialSetupControl.pendingDecision.length`),1,'model resolution must not wait for decision-service refresh');
    await evaluate(client, `trialSetupControl.releaseDecision();true`);
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('[data-trial-target=trial]').textContent.includes('Saved synthetic model')`);
  });
  await run('save.closed-flow-ignores-late-read', async () => {
    await open('saved-user-declared=1&hold-apply=1'); await saveConnection();
    await waitFor(client, `trialSetupControl.pendingApply.length===1`);
    await evaluate(client, `trialSetupControl.holdNextManagement=true;[...document.querySelectorAll('.trial-footer button')].find(e=>e.textContent.includes('Refresh availability')).click();true`);
    await waitFor(client, `trialSetupControl.pending.length===1`);
    await evaluate(client, `trialSetupControl.releaseApply();true`);
    await waitFor(client, `!!document.querySelector('.trial-recovery')`);
    await click('.trial-page .toolbar-actions button','Back');
    await waitFor(client, `!!document.querySelector('.new-models') && !document.querySelector('.trial-page')`);
    await evaluate(client, `trialSetupControl.release();true`);
    await waitFor(client, `trialSetupControl.pending.length===0`);
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-page')`),false);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('trust.initial-presets-exact-review-back', async () => {
    for (const [language,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]) {
      await open(`theme=${theme}`,language,width,height);
      await click('.trial-choice-list .trial-choice','GPT');
      const selected=await evaluate(client, `document.querySelector('.trial-setup .trust-levels [aria-pressed=true]')?.textContent.trim()`);
      assert.equal(selected,language==='zh'?'试用':'Trial');
      const labels=await evaluate(client, `[...document.querySelectorAll('.trial-setup .trust-levels button')].map(e=>e.textContent.trim())`);
      assert.equal(labels.length,5);
      for (let i=0;i<labels.length;i++) {
        await click('.trial-setup .trust-levels button',labels[i]);
        assert.equal(await evaluate(client, `document.querySelector('.trial-setup .trust-levels [aria-pressed=true]').textContent.trim()`),labels[i]);
      }
      assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0$/);
      await field('.trial-setup .trust-slider','0');
      assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/1$/);
      await field('.trial-setup .trust-slider','1000');
      await evaluate(client, `document.querySelector('.trial-setup .trust-slider').focus();true`);
      await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'ArrowLeft',code:'ArrowLeft',windowsVirtualKeyCode:37});
      await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'ArrowLeft',code:'ArrowLeft',windowsVirtualKeyCode:37});
      assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0\.001$/);
      await field('.trial-setup .trust-slider','277');
      assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0\.723$/);
      assert.equal(await evaluate(client, `document.querySelectorAll('.trial-setup .trust-levels [aria-pressed=true]').length`),0);
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor' || (x.command==='observation_read' && x.payload?.request?.intent?.view==='plan_trust_replay')).length`),0);
      await evaluate(client, `document.querySelector('.trial-setup .trust-control').scrollIntoView({block:'center'});true`);
      await shot(`initial-trust-${language}-${theme}-${width}`);
      await click('.trial-footer .btn-primary',language==='zh'?'检查改动':'Review changes');
      await waitFor(client, `!!document.querySelector('.trial-review .trust-slider') && !!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
      assert.equal(await evaluate(client, `document.querySelector('.trial-review [data-decision-field$="-simple-threshold"]').value`),'0.723');
      await field('.trial-review .trust-slider','500');
      await field('.trial-review .plan-identity-fields input','Edited trial route');
      await click('[data-route-group=primary] button.effort-select','high');
      await click('[role=dialog] [data-reasoning-profile=low]'); await click('[role=dialog] button',language==='zh'?'应用':'Apply');
      await click('.trial-review summary',language==='zh'?'判断设置':'Judgment settings');
      await click('.trial-review summary',language==='zh'?'如何判断胜任':'How competence is assessed');
      await evaluate(client, `(()=>{const e=document.querySelector('.trial-review [data-decision-field$="-criterion-0"]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,'Edited trial criterion');e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
      if (language==='en' && width===755) await click('.trial-page .toolbar-actions button','Back');
      else await click('.trial-review > .actions button',language==='zh'?'返回设置':'Back to setup');
      await waitFor(client, `!!document.querySelector('.trial-setup .trust-slider')`);
      assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0\.5$/);
      assert.equal(await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`),'Edited trial route');
      assert.equal((await evaluate(client, `document.querySelector('[data-trial-target="primary-reasoning"] button').textContent`)).trim(),'low');
      await field('.trial-setup .trust-slider','277');
      await click('.trial-footer .btn-primary',language==='zh'?'检查改动':'Review changes');
      await waitFor(client, `!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
      await click('.trial-review > .actions .btn-primary',language==='zh'?'保存并发布路由':'Save and publish route');
      await waitFor(client, `!!document.querySelector('.trial-mapping')`);
      const writes=await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').map(x=>x.payload.input)`);
      assert.deepEqual(writes.map(x=>x.action),['save_draft','publish']);
      for(const write of writes) {assert.equal(write.editor.smart.judgment.degree.simple_threshold_millis,723);assert.equal(write.editor.display_name,'Edited trial route');assert.equal(write.editor.smart.judgment.competence.criteria[0],'Edited trial criterion');assert.equal(write.editor.smart.primary[0].reasoning.profile,'low');}
    }
  });
  await run('trust.publish-and-reopen-exact-values', async () => {
    for (const threshold of [0,723,1000]) {
      await open(''); await click('.trial-choice-list .trial-choice','GPT');
      await field('.trial-setup .trust-slider',String(1000-threshold));
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
      await click('.trial-footer .btn-primary','Review changes');
      await waitFor(client, `!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
      await click('.trial-review > .actions .btn-primary','Save and publish route');
      await waitFor(client, `!!document.querySelector('.trial-mapping')`);
      const writes=await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').map(x=>x.payload.input)`);
      assert.deepEqual(writes.map(value=>value.action),['save_draft','publish']);
      assert.ok(writes.every(value=>value.editor.smart.judgment.degree.simple_threshold_millis===threshold));
      await evaluate(client, `reopenPublishedTrial();true`);
      await waitFor(client, `!!document.querySelector('.plan-editor [data-trust-control="smart"] .trust-slider')`);
      assert.equal(await evaluate(client, `document.querySelector('.plan-editor [data-trust-control="smart"] .trust-slider').value`),String(1000-threshold));
    }
  });
  await run('review-repair.protected-existing-selection', async () => {
    await open('additional-connection=1&same-names=1');
    await click('[data-trial-target=trial] button','Connect another source');
    await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[maxlength="60"]','Edited synthetic connection');
    await field('#mc-connection-form input[type=password]','synthetic-draft-key');
    await click('[data-trial-connected-model]');
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('A second provider'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!!document.querySelector('.discard-dialog[open]')`);
    await shot('repair-guard-en-755');
    await click('.discard-dialog button','Keep editing');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`),'synthetic-draft-key');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[maxlength="60"]').value`),'Edited synthetic connection');
    assert.ok(await evaluate(client, `!!document.querySelector('.trial-picker') && document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent.includes('Qwen3')`));
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('A second provider'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!!document.querySelector('.discard-dialog[open]')`);
    await click('.discard-dialog button','Discard changes');
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector('[data-trial-target=trial]').textContent.includes('A second provider')`);
    assert.equal(await evaluate(client, `!!document.querySelector('#mc-connection-form')`),false);
  });
  await run('review-repair.primary-keeps-trial-editor', async () => {
    await open('additional-connection=1');
    await click('[data-trial-target=trial] button','Connect another source'); await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[type=password]','synthetic-draft-key');
    await click('[data-trial-target=primary] button','Choose a connected model');
    await click('.trial-picker .trial-choice','GPT-6-Astra');
    await waitFor(client, `!document.querySelector('.trial-picker')`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]')?.value`),'synthetic-draft-key');
    assert.equal(await evaluate(client, `!!document.querySelector('.discard-dialog[open]')`),false);
  });
  await run('review-repair.uncertain-save-blocks-review', async () => {
    await open('additional-connection=1&saved-user-declared=1&lost-apply=1');
    await click('.trial-choice-list .trial-choice','GPT');
    await click('[data-trial-target=trial] button','Connect another source'); await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[type=password]','synthetic-fixture-key');
    await click('.modal-foot button','Check and continue'); await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button','Save connection');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('unconfirmed')`);
    await evaluate(client, `document.querySelector('.trial-recovery').scrollIntoView({block:'center'});true`);
    await shot('repair-unconfirmed-en-755');
    await click('.trial-recovery button','Back to trial');
    assert.equal(await evaluate(client, `document.querySelector('.trial-footer .btn-primary').disabled`),true,'an unconfirmed original save cannot be abandoned by advancing');
    assert.match(await evaluate(client, `document.querySelector('.trial-footer-hint').textContent`),/confirm|verif/i);
    await click('[data-trial-target=trial] button','Review connection result');
    await click('.trial-recovery button','Refresh result');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('review-repair.uncertain-zh-geometry', async () => {
    for (const [theme,width,height] of [['dark',755,600],['light',1200,813]]) {
      await open(`additional-connection=1&saved-user-declared=1&lost-apply=1&theme=${theme}`,'zh',width,height);
      await click('.trial-choice-list .trial-choice','GPT');
      await click('[data-trial-target=trial] button','连接其他接入点'); await connectedService();
      await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
      await field('#mc-connection-form input[type=password]','synthetic-fixture-key');
      await click('.modal-foot button','检查并继续'); await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
      await click('.model-selection input[type=checkbox]'); await click('.modal-foot button','保存连接');
      await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('核实')`);
      await evaluate(client, `document.querySelector('.trial-recovery').scrollIntoView({block:'center'});true`);
      const geometry=await evaluate(client, `(()=>{const card=document.querySelector('.trial-recovery').getBoundingClientRect(),footer=document.querySelector('.trial-footer').getBoundingClientRect();return {left:card.left,right:card.right,top:card.top,bottom:card.bottom,footer:footer.top,width:innerWidth}})()`);
      assert.ok(geometry.left>=0 && geometry.right<=geometry.width && geometry.top>=0 && geometry.bottom<=geometry.footer+1,JSON.stringify(geometry));
      await shot(`repair-unconfirmed-zh-${theme}-${width}`);
    }
  });
  await run('review-repair.saved-result-survives-review', async () => {
    await open('additional-connection=1&saved-user-declared=1&same-names=1');
    await click('.trial-choice-list .trial-choice','GPT');
    await click('[data-trial-target=trial] button','Connect another source'); await connectedService();
    await waitFor(client, `document.querySelector('#mc-connection-form')?.hidden===false`);
    await field('#mc-connection-form input[type=password]','synthetic-fixture-key');
    await click('.modal-foot button','Check and continue'); await waitFor(client, `!!document.querySelector('.model-selection input[type=checkbox]')`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button','Save connection');
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('Use this saved model')`);
    await click('.trial-recovery button','Back to trial');
    await click('[data-trial-connected-model]');
    await evaluate(client, `(()=>{const source=[...document.querySelectorAll('.trial-picker-source')].find(e=>e.textContent.includes('A second provider'));source.querySelector('.trial-choice').click();return true;})()`);
    await waitFor(client, `!document.querySelector('.trial-picker') && document.querySelector('[data-trial-target=trial]').textContent.includes('A second provider')`);
    await click('.trial-footer .btn-primary','Review changes'); await waitFor(client, `!!document.querySelector('.trial-review')`);
    await click('.trial-review > .actions button','Back to setup');
    await click('[data-trial-target=trial] button','Review connection result');
    assert.match(await evaluate(client, `document.querySelector('.trial-recovery').textContent`),/Use this saved model/i);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('review-repair.reopen-does-not-adopt-late-result', async () => {
    await open('saved-registered=1&saved-read-failure=1');
    await saveConnection();
    await waitFor(client, `document.querySelector('.trial-recovery')?.textContent.includes('could not be verified')`);
    await click('.trial-recovery button','Back to trial');
    await waitFor(client, `!document.querySelector('.trial-setup')?.disabled`);
    await click('.trial-model-card','Qwen');
    await click('[data-trial-target=trial] button','Review connection result');
    await click('.trial-recovery button','Refresh result');
    await waitFor(client, `!document.querySelector('.trial-recovery') || !document.querySelector('.trial-recovery button:disabled')`);
    assert.match(await evaluate(client, `document.querySelector(':is(.trial-model-card[aria-pressed=true],[data-trial-connected-model])')?.textContent`),/Qwen/,
      'refreshing a retained result cannot replace the newer trial without an explicit choice');
    assert.match(await evaluate(client, `document.querySelector('.trial-recovery')?.textContent`),/Use this saved model/);
    await evaluate(client, `document.querySelector('.trial-recovery').scrollIntoView({block:'center'});true`);
    await shot('repair-saved-result-en-755');
    await click('.trial-recovery button','Use this saved model');
    await waitFor(client, `document.querySelector('[data-trial-target=trial]')?.textContent.includes('Saved synthetic model')`);
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('review-repair.parked-custom-first-judgment', async () => {
    await open('custom-service=1'); await click('.trial-choice-list .trial-choice','GPT');
    await click('.trial-setup [data-route-group=classifier] .decision-methods button','Custom extension');
    await click('.trial-footer .btn-primary','Review changes'); await waitFor(client, `!!document.querySelector('.trial-review')`);
    await click('.trial-review [data-route-group=classifier] .decision-methods button','Decision model');
    await waitFor(client, `!!document.querySelector('.trial-review .trust-slider')`);
    await field('.trial-review .trust-slider','277');
    await click('.trial-review summary','Judgment settings'); await click('.trial-review summary','How competence is assessed');
    await evaluate(client, `(()=>{const e=document.querySelector('.trial-review [data-decision-field$="-criterion-0"]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,'Parked custom criterion');e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    await click('.trial-review [data-route-group=classifier] .decision-methods button','Custom extension');
    await click('.trial-review > .actions button','Back to setup');
    await click('.trial-setup [data-route-group=classifier] .decision-methods button','Decision model');
    await evaluate(client, `(()=>{const e=document.querySelector('.trial-setup .decision-selector select');e.value='decision/trial';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    await click('.trial-footer .btn-primary','Review changes'); await waitFor(client, `!!document.querySelector('.trial-review .trust-slider')`);
    assert.match(await evaluate(client, `document.querySelector('.trial-review .trust-threshold').textContent`),/0\.723$/);
    await click('.trial-review summary','Judgment settings'); await click('.trial-review summary','How competence is assessed');
    assert.equal(await evaluate(client, `document.querySelector('.trial-review [data-decision-field$="-criterion-0"]').value`),'Parked custom criterion');
  });
  await run('review-repair.parked-judgment', async () => {
    await open('custom-service=1'); await click('.trial-choice-list .trial-choice','GPT');
    await evaluate(client, `(()=>{const e=document.querySelector('.trial-setup .decision-selector select');e.value='decision/trial';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    await click('.trial-footer .btn-primary','Review changes'); await waitFor(client, `!!document.querySelector('.trial-review .trust-slider')`);
    await field('.trial-review .trust-slider','277');
    await click('.trial-review [data-route-group=classifier] .decision-methods button','Custom extension');
    await click('.trial-review > .actions button','Back to setup');
    await click('.trial-setup [data-route-group=classifier] .decision-methods button','Decision model');
    await waitFor(client, `!!document.querySelector('.trial-setup .trust-threshold')`);
    assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0\.723$/);
    await evaluate(client, `setTrialLanguage('zh');true`);
    await click('.trial-footer .btn-primary','检查改动'); await waitFor(client, `!!document.querySelector('.trial-review .trust-slider')`);
    await click('.trial-review [data-route-group=classifier] .decision-methods button','自定义扩展');
    await click('.trial-review > .actions button','返回设置');
    await click('.trial-setup [data-route-group=classifier] .decision-methods button','决策模型');
    assert.match(await evaluate(client, `document.querySelector('.trial-setup .trust-threshold').textContent`),/0\.723$/);
  });
  await run('review-repair.generated-name-limit', async () => {
    await open('long-model-names=1');
    await evaluate(client, `document.querySelector('.trial-choice-list .trial-choice').click();true`);
    const value=await evaluate(client, `document.querySelector('[data-trial-target=name] input').value`);
    assert.ok([...value].length<=128 && value.includes('试') && value.includes('🚀'),`generated name has ${[...value].length} scalars`);
  });
  await run('review-repair.name-blank-and-unicode', async () => {
    await open(''); await click('.trial-choice-list .trial-choice','GPT');
    await field('[data-trial-target=name] input','   '); await click('.trial-footer .btn-primary','Review changes');
    assert.ok(await evaluate(client, `!!document.querySelector('[data-trial-target=name] .oc-inline-error') && !document.querySelector('.trial-review')`),'explicit blank must not fall back to default');
    await field('[data-trial-target=name] input','🚀'.repeat(129)); await click('.trial-footer .btn-primary','Review changes');
    assert.ok(await evaluate(client, `!!document.querySelector('[data-trial-target=name] .oc-inline-error') && !document.querySelector('.trial-review')`),'129 Unicode scalars must be invalid');
    const valid='🚀'.repeat(128); await field('[data-trial-target=name] input',valid); await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
    await click('.trial-review > .actions .btn-primary','Save and publish route'); await waitFor(client, `!!document.querySelector('.trial-mapping')`);
    assert.equal(await evaluate(client, `trialTrace.find(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft').payload.input.editor.display_name`),valid);
  });
  await run('review-repair.unpublished-trust-context', async () => {
    await open(''); await click('.trial-choice-list .trial-choice','GPT'); await click('.trial-footer .btn-primary','Review changes');
    await waitFor(client, `!!document.querySelector('.trial-review .trust-control')`);
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-review .trust-control-evidence')`),false,'an unpublished route has no active-version evidence');
    assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='observation_read'&&x.payload?.request?.intent?.view==='plan_trust_replay').length`),0);
  });
  await writeFile(path.join(output,'report.json'),JSON.stringify({proof:'production components and synthetic IPC; no model calls',results},null,2));
  console.log(JSON.stringify(results.map(({id,state,error})=>({id,state,error}))));
  if (results.some(result=>result.state==='failed')) process.exitCode=1;
} finally {clearInterval(keepAlive);client.close();}
