// node tests/v3/visual/list-scroll-check.mjs CDP_PORT VITE_URL [EVIDENCE_DIRECTORY]
// Uses real React pages with synthetic IPC; not native Desktop acceptance.
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { connectPage, navigate, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, evidence] = process.argv.slice(2);
assert(port && base, 'Provide an isolated Chrome CDP port and Vite URL');
if (evidence) mkdirSync(evidence, { recursive: true });
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
const measure = `(() => {
  const list = document.querySelector('.master-list');
  const rect = list.getBoundingClientRect();
  const first = list.firstElementChild.getBoundingClientRect();
  const last = list.lastElementChild.getBoundingClientRect();
  return {height:list.clientHeight, content:list.scrollHeight, top:list.scrollTop,
    x:rect.x, y:rect.y, bottom:rect.bottom, first:first.top, lastTop:last.top, lastBottom:last.bottom,
    viewport:innerHeight, toolbar:document.querySelector('.page-toolbar').getBoundingClientRect().bottom};
})()`;
try {
  for (const width of [1280, 780]) for (const scale of [1, 1.5, 2]) {
    for (const page of ['models', 'routing']) for (const warning of [false, true]) {
      const name = `${page}-${width}-${scale}-${warning ? 'warning' : 'ready'}`;
      const started = performance.now();
      try {
        const query = new URLSearchParams({ page, scale: String(scale), warning: String(warning) });
        await navigate(client, `${base}/tests/v3/browser/list-scroll.html?${query}`, { width, height: 800 });
        await waitFor(client, `document.querySelectorAll('.master-list > .list-row').length === 40 && document.querySelector('.hr-ui').dataset.textScale === '${scale}'`);
        const before = await evaluate(client, measure);
        assert(before.height > 62 && before.content > before.height, 'Long list needs a bounded scroll viewport');
        assert(before.bottom <= before.viewport + 1, 'List viewport must fit below notices');
        const wheel = deltaY => client.send('Input.dispatchMouseEvent', {
          type: 'mouseWheel', x: before.x + 100, y: before.y + Math.min(40, before.height / 2), deltaX: 0, deltaY,
        });
        await wheel(100000);
        await waitFor(client, `document.querySelector('.master-list').scrollTop > 0`);
        await waitFor(client, `Math.abs(document.querySelector('.master-list').scrollHeight - document.querySelector('.master-list').clientHeight - document.querySelector('.master-list').scrollTop) < 2`);
        const bottom = await evaluate(client, measure);
        assert(bottom.lastTop >= bottom.y - 1 && bottom.lastBottom <= Math.min(bottom.bottom, bottom.viewport) + 1, 'Entire last row must be visible after wheel');
        assert.equal(bottom.toolbar, before.toolbar, 'Page toolbar must stay fixed');
        if (evidence) {
          const shot = await client.send('Page.captureScreenshot', { format: 'png' });
          writeFileSync(join(evidence, `${name}.png`), Buffer.from(shot.data, 'base64'));
        }
        await wheel(-100000);
        await waitFor(client, `document.querySelector('.master-list').scrollTop === 0`);
        await wheel(100000);
        await waitFor(client, `Math.abs(document.querySelector('.master-list').scrollHeight - document.querySelector('.master-list').clientHeight - document.querySelector('.master-list').scrollTop) < 2`);
        const last = await evaluate(client, `(() => {const r=document.querySelector('.master-list > .list-row:last-child').getBoundingClientRect();return {x:r.x+100,y:r.y+r.height/2};})()`);
        await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...last, button: 'left', clickCount: 1 });
        await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...last, button: 'left', clickCount: 1 });
        await waitFor(client, `document.querySelector('.master-list > .list-row:last-child').getAttribute('aria-current') === 'page'`);
        await waitFor(client, page === 'models'
          ? `document.querySelector('.detail-pane h2')?.textContent === 'Scroll model 40'`
          : `[...document.querySelectorAll('.detail-pane input')].some(input => input.value === 'Scroll route 40')`);
        results.push({ name, state: 'green', milliseconds: Math.round(performance.now() - started), before, bottom });
      } catch (error) {
        results.push({ name, state: 'red', error: String(error) });
      }
    }
  }
  const report = { evidence: 'Chromium, real React pages with synthetic IPC; not native Desktop', results };
  if (evidence) writeFileSync(join(evidence, 'results.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  assert.equal(results.length, 24);
  assert.equal(results.filter(result => result.state === 'red').length, 0, 'Every selected scenario must pass');
} finally {
  client.close();
  clearInterval(keepAlive);
}
