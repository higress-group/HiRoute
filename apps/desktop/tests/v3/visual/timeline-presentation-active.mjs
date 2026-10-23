import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const shot = async name => {
  const image = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
};
async function open(scenario, lang = 'en', theme = 'light', width = 1200, height = 813) {
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url: `${base}timeline-presentation.html?scenario=${scenario}&lang=${lang}&theme=${theme}` });
  await waitFor(client, `!!document.querySelector('.routing-timeline') && !!window.presentationFixture`);
  await evaluate(client, 'document.fonts.ready.then(()=>true)');
}
async function select(n) {
  await evaluate(client, `document.querySelector('[data-overview-turn="turn-${n}"]').dispatchEvent(new MouseEvent('click', { bubbles: true }));true`);
  await waitFor(client, `document.querySelector('.routing-detail-head > strong')?.textContent === 'T${n}'`);
  await evaluate(client, `document.querySelector('.routing-turn-detail').scrollIntoView({block:'center'});true`);
}
async function check(id, run) {
  if (process.env.PRESENTATION_CASE && !id.startsWith(process.env.PRESENTATION_CASE)) return;
  try { await run(); results.push({ id, status: 'passed' }); }
  catch (error) { await shot(`failure-${id}`); results.push({ id, status: 'failed', error: String(error.stack ?? error) }); }
}
try {
  for (const lang of ['en', 'zh']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200, 813], [755, 600]]) {
    const name = `${lang}-${theme}-${width}`;
    await check(`identity-${name}`, async () => {
      await open('coordinates', lang, theme, width, height);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length === 18`);
      await select(14);
      const details = await evaluate(client, `document.querySelector('.routing-turn-detail').textContent`);
      assert.ok(details.includes(lang === 'zh' ? '覆盖轮次 T14' : 'covers turn T14'), details);
      assert.ok(!details.includes('2–2'), details);
      assert.ok(details.includes(lang === 'zh' ? '请求间切换证据尚不完整' : 'Request transition evidence incomplete'), details);
      assert.ok(details.includes(lang === 'zh' ? '跨轮模型切换' : 'Between-turn model change'), details);
      assert.ok(details.includes(lang === 'zh' ? '请求内模型回退' : 'Within-request fallback'), details);
      const model = await evaluate(client, `document.querySelector('.routing-detail-head').textContent`);
      assert.ok(model.includes(lang === 'zh' ? '模型：managed-provider/exact-model-id' : 'Model: managed-provider/exact-model-id'), model);
      assert.ok(await evaluate(client, `document.documentElement.scrollWidth <= innerWidth + 1 && [...document.querySelectorAll('.routing-turn-detail,.routing-overview')].every(e=>e.scrollWidth<=e.clientWidth+1)`));
      await shot(`identity-${name}`);
      await select(15);
      const missing = await evaluate(client, `document.querySelector('.routing-turn-detail').textContent`);
      assert.ok(missing.includes(lang === 'zh' ? '当前已加载范围外' : 'outside the loaded range'), missing);
      assert.ok(missing.includes(lang === 'zh' ? '切换证据尚不完整' : 'Transition evidence incomplete'), missing);
      assert.ok(!missing.includes(lang === 'zh' ? '请求间切换证据尚不完整' : 'Request transition evidence incomplete'), missing);
    });
    await check(`occlusion-focus-${name}`, async () => {
      await open('coordinates', lang, theme, width, height);
      await waitFor(client, `document.querySelectorAll('.routing-turn').length === 18`);
      await evaluate(client, `document.querySelector('.routing-lanes-scroll').scrollIntoView({block:'center'});true`);
      const mask = await evaluate(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll'),label=s.querySelector('.routing-lane-label');let leak=null;for(let x=100;x<Math.min(s.scrollWidth-s.clientWidth,1800);x+=8){s.scrollLeft=x;const r=s.getBoundingClientRect(),l=label.getBoundingClientRect(),hit=document.elementFromPoint(r.left+1,l.top+l.height/2);if(hit?.closest('.routing-turn')){leak={x,hit:hit.className};break;}}const r=s.getBoundingClientRect(),l=label.getBoundingClientRect();return {gap:l.left-r.left,leak};})()`);
      assert.equal(mask.gap, 0, 'sticky layer must cover the scroller left edge');
      assert.equal(mask.leak, null, 'hidden cards must not intercept pointer input beneath labels');
      await evaluate(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll');s.scrollLeft=0;const b=s.querySelector('.routing-turn');b.focus();return true;})()`);
      await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
      await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
      const focus = await evaluate(client, `(()=>{const b=document.activeElement,s=document.querySelector('.routing-lanes-scroll'),r=b.getBoundingClientRect(),p=s.getBoundingClientRect(),style=getComputedStyle(b);return {button:b.matches('.routing-turn'),visible:b.matches(':focus-visible'),width:parseFloat(style.outlineWidth),offset:parseFloat(style.outlineOffset),left:r.left-p.left,right:p.right-r.right};})()`);
      assert.ok(focus.button && focus.visible && focus.width >= 2, JSON.stringify(focus));
      assert.ok(focus.left >= 0 && focus.right >= 0 && focus.offset <= -focus.width, JSON.stringify(focus));
      await shot(`occlusion-focus-${name}`);
      const click = await evaluate(client, `(()=>{const s=document.querySelector('.routing-lanes-scroll'),b=s.querySelector('[data-turn-id="turn-2"]'),r=b.getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2};})()`);
      await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', clickCount: 1, ...click });
      await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', clickCount: 1, ...click });
      await waitFor(client, `document.querySelector('.routing-detail-head > strong')?.textContent==='T2'`);
    });
  }
  await check('timeout-retains-selection-and-backs-off', async () => {
    await open('timeout'); await waitFor(client, `document.querySelectorAll('.routing-turn').length===18`);
    await select(14);
    await waitFor(client, `!!document.querySelector('[data-error-code]')`);
    const stale = await evaluate(client, `({title:document.querySelector('.routing-detail-head > strong').textContent,status:document.querySelector('.routing-live').textContent,count:document.querySelectorAll('.routing-turn').length})`);
    assert.equal(stale.title, 'T14'); assert.equal(stale.count, 18);
    assert.ok(stale.status.includes('Update delayed'), stale.status);
    await evaluate(client, `document.querySelector('.routing-live').scrollIntoView({block:'start'});true`);
    await shot('timeout-retained');
    await sleep(6500);
    await waitFor(client, `presentationFixture.reads.length>=4 && !document.querySelector('[data-error-code]')`);
    const readTimes = await evaluate(client, 'presentationFixture.reads');
    assert.ok(readTimes[3].at-readTimes[2].at>=3500, JSON.stringify(readTimes));
    assert.equal(await evaluate(client, `document.querySelector('.routing-detail-head > strong').textContent`), 'T14');
    assert.ok(await evaluate(client, `document.querySelector('.routing-live').textContent.includes('Live')`));
  });
  await check('capacity-requires-explicit-retry', async () => {
    await open('scope'); await waitFor(client, `!!document.querySelector('[data-error-code]')`);
    await sleep(4200);
    assert.equal(await evaluate(client, 'presentationFixture.reads.length'), 1);
    await evaluate(client, `document.querySelector('[data-error-code] button').click();true`);
    await waitFor(client, `document.querySelectorAll('.routing-turn').length===18`);
  });
} finally {
  await writeFile(path.join(output, 'timeline-presentation-results.json'), JSON.stringify({ proof: 'production components/styles with controlled IPC; no native/provider calls', results }, null, 2));
  client.close();
}
console.log(JSON.stringify({ suite: 'timeline-presentation', total: results.length, failed: results.filter(r=>r.status!=='passed').length }));
if (!results.length || results.some(result=>result.status!=='passed')) process.exitCode=1;
