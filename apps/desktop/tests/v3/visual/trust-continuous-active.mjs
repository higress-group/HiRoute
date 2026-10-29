import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const shot = async name => {
  const image = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
};
async function open(lang, theme, width, height) {
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url: `${base}new-model-trial.html?polish=1&native=1&view=editor&lang=${lang}&theme=${theme}` });
  await waitFor(client, `!!document.querySelector('.trust-slider')`, { timeout: 15000 });
  await evaluate(client, `document.querySelector('.trust-control').scrollIntoView({block:'start'}); window.submissions=0;document.addEventListener('submit',()=>window.submissions++,true);document.fonts.ready.then(()=>true)`);
}
const setRange = async value => evaluate(client, `(()=>{const e=document.querySelector('.trust-slider');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(String(value))});e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
const click = async label => evaluate(client, `(()=>{const e=[...document.querySelectorAll('.trust-levels button')].find(x=>x.textContent.trim()===${JSON.stringify(label)});if(!e)throw Error('missing preset');e.click();return true;})()`);
const key = async name => { await client.send('Input.dispatchKeyEvent', {type:'keyDown',key:name,code:name,...(name === 'Enter' ? {text:'\r'} : {}),windowsVirtualKeyCode:({Home:36,End:35,ArrowLeft:37,Enter:13})[name]}); await client.send('Input.dispatchKeyEvent', {type:'keyUp',key:name,code:name,windowsVirtualKeyCode:({Home:36,End:35,ArrowLeft:37,Enter:13})[name]}); };
try {
  const matrix = process.env.TRUST_BASELINE ? [['en', 'light', 755, 600]] : ['en','zh'].flatMap(lang => ['light','dark'].flatMap(theme => [[lang,theme,1200,813],[lang,theme,755,600]]));
  for (const [lang, theme, width, height] of matrix) {
    const id = `${lang}-${theme}-${width}`;
    try {
      await open(lang, theme, width, height);
      const range = await evaluate(client, `(()=>{const e=document.querySelector('.trust-slider');return {min:e.min,max:e.max,step:e.step,value:e.value};})()`);
      assert.deepEqual(range, { min:'0',max:'1000',step:'1',value:'280' }, 'saved 0.720 must stay exact on a continuous track');
      await setRange(277);
      assert.ok(await evaluate(client, `document.querySelector('.trust-control').textContent.includes('0.723')`));
      assert.equal(await evaluate(client, `document.querySelectorAll('.trust-levels button[aria-pressed=true]').length`), 0);
      assert.ok((await evaluate(client, `document.querySelector('.trust-slider').getAttribute('aria-valuetext')`)).includes('0.723'));
      await shot(`intermediate-${id}`);
      const labels = lang === 'zh' ? ['试用','熟悉','信任','放手','托付'] : ['Trial','Familiar','Trusted','Hands-off','Entrusted'];
      for (const [i, label] of labels.entries()) {
        await click(label);
        assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), String([100,200,350,500,1000][i]));
        assert.equal(await evaluate(client, `document.querySelectorAll('.trust-levels button[aria-pressed=true]').length`), 1);
        await setRange(277);
        await evaluate(client, `([...document.querySelectorAll('.trust-levels button')].find(e=>e.textContent.trim()===${JSON.stringify(label)})).focus();true`);
        await key('Enter');
        assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), String([100,200,350,500,1000][i]));
      }
      assert.equal(await evaluate(client, 'window.submissions'), 0);
      assert.ok(await evaluate(client, `document.documentElement.scrollWidth <= innerWidth + 1 && [...document.querySelectorAll('.trust-control,.trust-levels button')].every(e=>e.scrollWidth<=e.clientWidth+1)`));
      await shot(`zero-${id}`);
      await evaluate(client, `document.querySelector('.trust-slider').focus();true`);
      await key('Home'); assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), '0');
      await key('End'); assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), '1000');
      await key('ArrowLeft'); assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), '999');
      await evaluate(client, `document.querySelector('.trust-exact summary').click();true`);
      await evaluate(client, `(()=>{const e=document.querySelector('[data-decision-field="smart-simple-threshold"]');e.focus();Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,'1.5');e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
      assert.equal(await evaluate(client, `document.querySelector('[data-decision-field="smart-simple-threshold"]').getAttribute('aria-invalid')`), 'true');
      assert.equal(await evaluate(client, `document.querySelector('.trust-slider').value`), '999');
      await evaluate(client, `document.querySelector('[data-decision-field="smart-simple-threshold"]').blur();true`);
      assert.equal(await evaluate(client, `document.querySelector('[data-decision-field="smart-simple-threshold"]').value`), '0.001');
      results.push({id,status:'passed'});
    } catch (error) {
      await shot(`failure-${id}`);
      results.push({id,status:'failed',error:String(error.stack ?? error)});
    }
  }
  if (!process.env.TRUST_BASELINE) {
    try {
      await open('en','light',1200,813);
      await setRange(277);
      assert.equal(await evaluate(client, `trialTrace.filter(x=>x.command==='preview_plan_editor').length`), 0);
      await evaluate(client, `([...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='Save draft')).click();true`);
      await waitFor(client, `trialTrace.some(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft')`);
      assert.equal(await evaluate(client, `trialTrace.find(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft').payload.input.editor.smart.judgment.degree.simple_threshold_millis`), 723);
      results.push({id:'explicit-save-carries-exact-thousandths',status:'passed'});
    } catch (error) { await shot('failure-exact-save'); results.push({id:'explicit-save-carries-exact-thousandths',status:'failed',error:String(error.stack ?? error)}); }
    try {
      await open('en','light',1200,813);
      const reads = `trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='plan_trust_replay')`;
      await waitFor(client, `${reads}.length===1 && !!document.querySelector('.trust-replay-bar')`);
      assert.deepEqual(await evaluate(client, `${reads}[0].payload.request.intent.query.thresholds_millis`), [720]);
      await setRange(100); await setRange(101); await setRange(102);
      await waitFor(client, `${reads}.length===2 && document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.898')`);
      assert.deepEqual(await evaluate(client, `${reads}[1].payload.request.intent.query.thresholds_millis`), [898]);
      const windowBounds = await evaluate(client, `${reads}[0].payload.request.intent.query`);
      for (let x=103;x<=134;x++) {
        const before = await evaluate(client, `${reads}.length`);
        await setRange(x);
        await waitFor(client, `${reads}.length>${before} && document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes(${JSON.stringify(String((1000-x)/1000))})`);
      }
      assert.ok(await evaluate(client, `${reads}.every(x=>x.payload.request.intent.query.from_ms===${windowBounds.from_ms} && x.payload.request.intent.query.to_ms===${windowBounds.to_ms})`));
      let before = await evaluate(client, `${reads}.length`);
      await setRange(280);
      await waitFor(client, `document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.72')`);
      assert.equal(await evaluate(client, `${reads}.length`), before, 'saved threshold stays cached');
      await setRange(102);
      await waitFor(client, `${reads}.length>${before}`);
      assert.deepEqual(await evaluate(client, `${reads}.at(-1).payload.request.intent.query.thresholds_millis`), [898]);
      results.push({id:'replay-debounce-and-bounded-cache',status:'passed'});
    } catch (error) { await shot('failure-replay-debounce'); results.push({id:'replay-debounce-and-bounded-cache',status:'failed',error:String(error.stack ?? error)}); }
  }
} finally {
  await writeFile(path.join(output, 'trust-continuous-results.json'), JSON.stringify(results, null, 2));
  await client.close();
}
if (results.some(result => result.status !== 'passed')) process.exitCode = 1;
