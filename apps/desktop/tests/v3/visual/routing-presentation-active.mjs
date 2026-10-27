import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';
const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const shot = async name => { const value = await client.send('Page.captureScreenshot', {format:'png'}); await writeFile(path.join(output, `${name}.png`), Buffer.from(value.data, 'base64')); };
async function open(lang, theme, width, height, scenario = 'pending', extra = '') {
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', {width,height,deviceScaleFactor:1,mobile:false});
  await client.send('Page.navigate', {url:`${base}routing-timeline-presentation.html?lang=${lang}&theme=${theme}&scenario=${scenario}${extra}`});
  await waitFor(client, `document.querySelectorAll('.routing-turn').length > 0 && !!window.routingPresentation`);
  await evaluate(client, `document.fonts.ready.then(()=>true)`);
}
try {
  for (const lang of ['en','zh']) for (const theme of ['light','dark']) for (const [width,height] of [[1200,813],[755,600]]) {
    const id = `${lang}-${theme}-${width}`;
    try {
      await open(lang, theme, width, height);
      await evaluate(client, `document.querySelector('.routing-lanes-card').scrollIntoView({block:'center'});document.querySelector('.routing-turn').focus();true`);
      assert.equal(await evaluate(client, `document.querySelectorAll('.routing-lane-label').length`), 2);
      assert.equal(await evaluate(client, `document.querySelectorAll('.routing-pending-cell .routing-turn').length`), 1);
      const pending = await evaluate(client, `document.querySelector('.routing-pending-cell').textContent`);
      assert.ok(pending.includes(lang === 'zh' ? '判断中' : 'Choosing a model'));
      assert.doesNotMatch(pending, /Group unconfirmed|P\(simple\) unknown|选择原因未记录/);
      await shot(`pending-${id}`);
      await evaluate(client, `routingPresentation.resolve();true`);
      await waitFor(client, `!document.querySelector('.routing-pending-cell') && !!document.querySelector('[data-turn-id="turn-1"]')`);
      assert.equal(await evaluate(client, `document.activeElement?.getAttribute('data-turn-id')`), 'turn-1');
      assert.equal(await evaluate(client, `document.querySelectorAll('[data-turn-id="turn-1"]').length`), 1);
      assert.ok(await evaluate(client, `document.documentElement.scrollWidth<=innerWidth+1 && [...document.querySelectorAll('.routing-turn-detail,.routing-turn')].every(e=>e.scrollWidth<=e.clientWidth+1)`));
      await shot(`resolved-${id}`);
      results.push({id,status:'passed'});
    } catch (error) { await shot(`failure-${id}`); results.push({id,status:'failed',error:String(error.stack ?? error)}); }
  }
  try {
    await open('en','light',1200,813,'many');
    await waitFor(client, `routingPresentation.trace.filter(x=>x.command==='compute_management_snapshot').length===1`);
    const input = await evaluate(client, `routingPresentation.trace.find(x=>x.command==='plan_editor_options').input`);
    assert.deepEqual(input, { requirements:{}, native_selections:{}, suggest_free:false });
    for (let n=1;n<=10;n++) await evaluate(client, `document.querySelector('[data-turn-id="turn-${n}"]').click();true`);
    for (let n=0;n<20;n++) {
      const before = await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='routing_timeline').length`);
      await evaluate(client, `document.dispatchEvent(new Event('visibilitychange'));true`);
      await waitFor(client, `routingPresentation.trace.filter(x=>x.command==='routing_timeline').length>${before}`);
    }
    assert.equal(await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='plan_editor_options').length`), 1);
    assert.equal(await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='compute_management_snapshot').length`), 1);
    assert.ok(await evaluate(client, `document.querySelector('.routing-lane-label.cyan').textContent.includes('Latest: Astra')`));
    assert.equal(await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='routing_timeline').every(x=>x.query.include_presentation===true)`), true);
    results.push({id:'bounded-label-reads',status:'passed'});
  } catch (error) { await shot('failure-bounded-label-reads'); results.push({id:'bounded-label-reads',status:'failed',error:String(error.stack ?? error)}); }
  try {
    await open('en','light',1200,813,'many','&one-name=1');
    await waitFor(client, `routingPresentation.trace.some(x=>x.command==='plan_editor_options')`);
    assert.equal(await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='compute_management_snapshot').length`), 0);
    await open('en','light',1200,813,'many','&delay-source=1');
    await waitFor(client, `routingPresentation.trace.some(x=>x.command==='compute_management_snapshot')`);
    await waitFor(client, `document.querySelector('.routing-lane-label.cyan')?.textContent.includes('Source unconfirmed')`);
    await evaluate(client, `routingPresentation.invalidate();routingPresentation.releaseSources();true`);
    await waitFor(client, `!document.querySelector('.routing-lane-label.cyan')?.textContent.includes('Source unconfirmed')`);
    assert.ok(!(await evaluate(client, `document.querySelector('.routing-lane-label.cyan').textContent`)).includes('First'));
    await open('en','light',1200,813,'many','&labels-error=1');
    await waitFor(client, `routingPresentation.trace.some(x=>x.command==='plan_editor_options')`);
    assert.ok(await evaluate(client, `document.querySelectorAll('.routing-turn').length===12`));
    assert.equal(await evaluate(client, `routingPresentation.trace.filter(x=>x.command==='plan_editor_options').length`), 1);
    results.push({id:'label-fallback-and-lifecycle',status:'passed'});
  } catch (error) { await shot('failure-label-lifecycle'); results.push({id:'label-fallback-and-lifecycle',status:'failed',error:String(error.stack ?? error)}); }
  for (const [extra, state, text] of [['&body-loading=1','loading','Loading message'],['&body-error=1','error','Message could not be read'],['&body-cleared=1','cleared','Content cleared'],['','pending','Message record pending']]) {
    try {
      await open('en','light',1200,813,'pending',extra);
      await evaluate(client, `document.querySelector('.routing-quote').scrollIntoView({block:'center'});true`);
      await waitFor(client, `document.querySelector('.routing-quote [data-content-state]')?.dataset.contentState===${JSON.stringify(state)}`);
      assert.ok(await evaluate(client, `document.querySelector('.routing-quote').textContent.includes(${JSON.stringify(text)})`));
      await shot(`content-${state}`);
      results.push({id:`content-${state}`,status:'passed'});
    } catch (error) { await shot(`failure-content-${state}`); results.push({id:`content-${state}`,status:'failed',error:String(error.stack ?? error)}); }
  }
  try {
    await open('en','light',1200,813,'no-target');
    await evaluate(client, `document.querySelector('.routing-quote').scrollIntoView({block:'center'});true`);
    await waitFor(client, `document.querySelector('.routing-quote [data-content-state]')?.dataset.contentState==='pending'`);
    assert.ok(await evaluate(client, `document.querySelector('.routing-quote').textContent.includes('Message record is incomplete')`));
    results.push({id:'content-terminal-incomplete',status:'passed'});
  } catch (error) { await shot('failure-content-incomplete'); results.push({id:'content-terminal-incomplete',status:'failed',error:String(error.stack ?? error)}); }
  try {
    await open('en','light',1200,813,'mixed');
    await waitFor(client, `document.querySelector('.routing-detail-head')?.textContent.includes('Multiple execution models')`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.routing-lane-label').length`), 2);
    assert.ok(await evaluate(client, `document.querySelector('.routing-turn-requests').textContent.includes('Astra') && document.querySelectorAll('.routing-request').length===2`));
    await open('en','light',1200,813,'unknown');
    assert.equal(await evaluate(client, `document.querySelectorAll('.routing-lane-label').length`), 3);
    assert.ok(await evaluate(client, `document.querySelector('.routing-lane-label:not(.cyan):not(.purple)').textContent.includes('Group unconfirmed')`));
    assert.equal(await evaluate(client, `document.querySelectorAll('.routing-pending-cell').length`), 0);
    await open('en','light',1200,813,'no-target');
    assert.ok(await evaluate(client, `document.querySelector('.routing-turn-detail').textContent.includes('No prior stage is available for this decision')`));
    assert.ok(await evaluate(client, `document.querySelector('.routing-turn-detail').textContent.includes('Model default reasoning')`));
    results.push({id:'mixed-unknown-and-no-target',status:'passed'});
  } catch (error) { await shot('failure-mixed-unknown'); results.push({id:'mixed-unknown-and-no-target',status:'failed',error:String(error.stack ?? error)}); }
} finally { await writeFile(path.join(output,'routing-presentation-results.json'), JSON.stringify(results,null,2)); await client.close(); }
if (results.some(result => result.status !== 'passed')) process.exitCode = 1;
