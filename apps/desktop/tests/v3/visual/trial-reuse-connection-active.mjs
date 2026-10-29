import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000), results = [];
async function click(selector, text = '') {
  await waitFor(client, `(()=>{const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&e.textContent.includes(${JSON.stringify(text)}));if(!e)return false;e.click();return true;})()`);
}
async function open(query = '', language = 'en', width = 755, height = 600, recommended = true) {
  const url = `${base}new-model-trial.html?view=quickstart&native=1&reuse-target=1&lang=${language}&${query}`;
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  await client.send('Page.navigate',{url});
  await waitFor(client, `location.href===${JSON.stringify(url)} && !!document.querySelector('.new-models button')`);
  await click('.new-models button',language==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-footer') && !document.querySelector('.trial-setup')?.disabled`);
  if (recommended) await click('.trial-model-card','Kimi');
  await click('[data-trial-target=trial] button', recommended ? language==='zh'?'在此接入':'Connect here' : language==='zh'?'新增接入':'Add a connection');
  await waitFor(client,`!!document.querySelector('[data-trial-source-choice]')`);
}
async function source(id='source/bailian/coding') {
  await evaluate(client,`(()=>{const e=document.querySelector('[data-trial-source-choice]');e.value=${JSON.stringify(id)};e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
  await waitFor(client,`!!document.querySelector('[data-trial-request-model]')`);
}
async function field(value) {
  await evaluate(client,`(()=>{const e=document.querySelector('[data-trial-request-model]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
}
async function shot(name) {
  await evaluate(client, `Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true)))))`);
  const image = await client.send('Page.captureScreenshot',{format:'png'});
  await writeFile(path.join(output,`${name}.png`),Buffer.from(image.data,'base64'));
}
async function run(id, test) {
  if (process.env.TRIAL_REUSE_CASE && !process.env.TRIAL_REUSE_CASE.split(',').includes(id)) return;
  try {await test();results.push({id,state:'passed'});}
  catch(error){await shot(`failure-${id}`).catch(()=>{});results.push({id,state:'failed',error:String(error.stack??error),page:await evaluate(client,`({text:document.body.innerText,trace:window.trialTrace})`).catch(()=>null)});}
}
try {
  await run('existing-binding-no-write',async()=>{
    await open('', 'en',755,600,false); await source();
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'qwen3-coder-plus');
    await click('.trial-reuse-connection button','Use this connection');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>['apply_compute_save','check_saved_model_connection','compute_subscriptions'].includes(x.command)).length`),0);
  });
  await run('api-one-target-auto-return',async()=>{
    await open();await source();
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'fixture-added-model');
    assert.equal(await evaluate(client,`document.querySelectorAll('.trial-reuse-connection input[type=checkbox]').length`),0);
    await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection') && document.querySelector('[data-trial-target=trial]').textContent.includes('Actual saved trial model')`);
    const change=await evaluate(client,`trialSetupControl.appendChange`);
    assert.deepEqual(change.selected_model_refs,['model-ref/reuse-target']);assert.deepEqual(change.key_edits,[]);assert.equal(change.edit.action,'append_models');
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>['compute_scan','compute_subscriptions','register_protected_model_input'].includes(x.command)).length`),0);
  });
  await run('subscription-one-target-auto-return',async()=>{
    await open(); await source('source/codex/subscription');
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'fixture-sub-added');
    await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection') && document.querySelector('[data-trial-target=trial]').textContent.includes('Actual saved trial model')`);
    const change=await evaluate(client,`trialSetupControl.appendChange`);
    assert.deepEqual(change.selected_model_refs,['model-ref/reuse-target']);assert.ok(change.validation);assert.deepEqual(change.key_edits,[]);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>['register_protected_model_input','manage_subscription_login','check_saved_model_connection'].includes(x.command)).length`),0);
  });
  await run('edited-id-reuses-exact-binding',async()=>{
    await open();await source();await field('qwen3-coder-plus');
    await click('.trial-reuse-connection button','Use this connection');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),0);
  });
  await run('missing-default-and-unsupported-id',async()=>{
    await open('reuse-no-default=1');await source();
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'');
    await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`document.querySelector('#trial-reuse-error')?.textContent.includes('Enter the model ID')`);
    await field('unavailable-alias');await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`document.querySelector('#trial-reuse-error')?.textContent.includes('did not provide')`);
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'unavailable-alias');
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),0);
  });
  await run('failed-check-preserves-id',async()=>{
    await open('reuse-check-failure=1');await source();await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`!!document.querySelector('#trial-reuse-error')`);
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'fixture-added-model');
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),0);
  });
  await run('lost-reply-refreshes-original-save',async()=>{
    await open('reuse-lost=1');await source();await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`[...document.querySelectorAll('.trial-reuse-connection button')].some(e=>e.textContent.includes('Refresh result'))`);
    await click('.trial-reuse-connection button','Refresh result');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection') && document.querySelector('[data-trial-target=trial]').textContent.includes('Actual saved trial model')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('pending-save-never-resubmits',async()=>{
    await open('reuse-pending=1');await source();await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`document.querySelector('.trial-reuse-connection')?.textContent.includes('save is unconfirmed')`);
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').disabled`),true);
    await click('.trial-reuse-connection button','Refresh result');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')?.getAttribute('aria-busy') || document.querySelector('.trial-reuse-connection').getAttribute('aria-busy')==='false'`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
    await evaluate(client,`trialSetupControl.pendingSaveReleased=true;true`);
    await click('.trial-reuse-connection button','Refresh result');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')`);
  });
  await run('definite-save-failure-is-not-unknown',async()=>{
    await open('reuse-save-failed=1');await source();await click('.trial-reuse-connection button','Check and add');
    await waitFor(client,`!!document.querySelector('#trial-reuse-error') && !document.querySelector('[data-trial-request-model]').disabled`);
    assert.equal(await evaluate(client,`document.querySelector('.trial-reuse-connection').textContent.includes('save is unconfirmed')`),false);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),1);
  });
  await run('cancel-dirty-id-keeps-or-discards-explicitly',async()=>{
    await open();await source();await field('edited-alias');
    await click('.trial-reuse-connection button','Back to trial');
    await waitFor(client,`!!document.querySelector('dialog.discard-dialog')`);
    await click('dialog.discard-dialog button','Keep editing');
    assert.equal(await evaluate(client,`document.querySelector('[data-trial-request-model]').value`),'edited-alias');
    await click('.trial-reuse-connection button','Back to trial');await click('dialog.discard-dialog button','Discard changes');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='apply_compute_save').length`),0);
  });
  await run('cancel-late-read-does-not-change-trial',async()=>{
    await open();await evaluate(client,`trialSetupControl.holdNextManagement=true;true`);
    await evaluate(client,`(()=>{const e=document.querySelector('[data-trial-source-choice]');e.value='source/bailian/coding';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
    await waitFor(client,`trialSetupControl.pending.length===1`);
    await click('[role=dialog] .modal-head .icon-btn');
    await waitFor(client,`!document.querySelector('.trial-reuse-connection')`);
    await evaluate(client,`trialSetupControl.release();true`);
    await waitFor(client,`trialSetupControl.pending.length===0`);
    assert.equal(await evaluate(client,`!!document.querySelector('.trial-reuse-connection')`),false);
    assert.equal(await evaluate(client,`document.querySelector('.trial-model-card.selected').textContent.includes('Kimi')`),true);
  });
  await run('localized-layout',async()=>{
    for(const [language,theme,width,height] of [['zh','dark',755,600],['en','light',755,600],['zh','light',1200,813],['en','dark',1200,813]]) {
      await open(`theme=${theme}`,language,width,height);await source();
      const bounds=await evaluate(client,`(()=>{const form=document.querySelector('.trial-reuse-connection'),input=form.querySelector('input'),label=form.querySelector('.field-label'),r=input.getBoundingClientRect(),l=label.getBoundingClientRect();return {overflow:form.scrollWidth>form.clientWidth+1,inputWidth:r.width,labelWidth:l.width,left:r.left,right:r.right,width:innerWidth};})()`);
      assert.ok(!bounds.overflow && bounds.inputWidth>200 && bounds.labelWidth>100 && bounds.left>=0 && bounds.right<=bounds.width,JSON.stringify(bounds));
      await shot(`reuse-${language}-${theme}-${width}`);
    }
  });
} finally {
  await writeFile(path.join(output,'results.json'),JSON.stringify({results},null,2));
  clearInterval(keepAlive);client.close();
}
console.log(JSON.stringify(results.map(({id,state,error})=>({id,state,error}))));
if(results.some(result=>result.state==='failed'))process.exitCode=1;
