import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';
const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
const fixture = 'sessionRefreshFixture';
const flush = () => new Promise(resolve => setTimeout(resolve, 100));
const readCount = filter => evaluate(client, `${fixture}.reads.filter(r=>${filter}).length`);
async function advance(ms) { await evaluate(client, `${fixture}.advance(${ms});true`); await flush(); }
async function open(query, width = 1200, height = 813) {
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url: `${base}session-live-refresh.html?${query}` });
  await waitFor(client, '!!document.querySelector("#toggle-active")');
  await evaluate(client, 'document.fonts.ready.then(()=>true)');
}
async function shot(name) { const capture = await client.send('Page.captureScreenshot', { format: 'png' }); await writeFile(path.join(output, `${name}.png`), Buffer.from(capture.data, 'base64')); }
async function run(id, action) { if (process.env.SESSION_REFRESH_CASE && !id.includes(process.env.SESSION_REFRESH_CASE)) return; try { await action(); results.push({ id, status: 'passed' }); } catch (error) { results.push({ id, status: 'failed', error: String(error) }); throw error; } }
try {
  await run('session.selected-summary-refresh-preserves-pages-selection-and-scroll', async () => {
    await open('body=available');
    await waitFor(client, 'document.querySelectorAll(".session-item").length===12 && !!document.querySelector(".transcript-head p")');
    await evaluate(client, 'document.querySelector(".session-more").click();true');
    await waitFor(client, 'document.querySelectorAll(".session-item").length===13');
    const before = await evaluate(client, `(()=>{const list=document.querySelector('.session-list');list.scrollTop=80;return {time:document.querySelector('.session-item-time').textContent,header:document.querySelector('.transcript-head p').textContent,scroll:list.scrollTop,listReads:${fixture}.reads.filter(r=>r.view==='sessions'&&!r.query.session_id).length};})()`);
    await evaluate(client, `${fixture}.phase=1;true`);
    await advance(70000);
    await waitFor(client, `document.querySelector('.session-item-time').textContent!==${JSON.stringify(before.time)}`);
    const after = await evaluate(client, `({count:document.querySelectorAll('.session-item').length,selected:[...document.querySelectorAll('.session-item')].findIndex(n=>n.getAttribute('aria-current')==='page'),scroll:document.querySelector('.session-list').scrollTop,header:document.querySelector('.transcript-head p').textContent,badge:document.querySelector('.session-item:first-child .badge')?.textContent,listReads:${fixture}.reads.filter(r=>r.view==='sessions'&&!r.query.session_id).length,query:${fixture}.reads.filter(r=>r.view==='sessions'&&r.query.session_id).at(-1)?.query})`);
    assert.equal(after.count, 13); assert.equal(after.selected, 0); assert.ok(after.scroll > 0 && Math.abs(after.scroll - before.scroll) < 32, 'the list keeps its visible location as the completeness badge disappears'); assert.equal(after.listReads, before.listReads);
    assert.notEqual(after.header, before.header); assert.equal(after.badge, undefined); assert.ok(after.header.includes('Plan: Daily coding trial'));
    assert.equal(after.query.limit, 1); assert.equal(after.query.cursor, null); assert.ok(after.query.to_ms > after.query.from_ms);
    await shot('session-summary-live-en-light-1200');
  });
  await run('session.summary-hidden-inactive-and-late-selection-response', async () => {
    await open('body=available'); await waitFor(client, '!!document.querySelector(".transcript-head p")');
    await evaluate(client, `${fixture}.visibility(true);true`);
    const hiddenCount = await readCount("r.view==='sessions'"); await advance(60000); assert.equal(await readCount("r.view==='sessions'"), hiddenCount);
    await evaluate(client, `${fixture}.visibility(false);true`); await flush();
    assert.ok(await readCount("r.view==='sessions'") > hiddenCount);
    await evaluate(client, `document.querySelector('#toggle-active').click();true`); await flush();
    const inactive = await readCount("r.view==='sessions'"); await advance(60000); assert.equal(await readCount("r.view==='sessions'"), inactive);
    await evaluate(client, `document.querySelector('#toggle-active').click();true`); await flush();
    await evaluate(client, `${fixture}.deferSummary=true;${fixture}.phase=1;true`); await advance(2000);
    await waitFor(client, `${fixture}.releases.length>0`);
    await evaluate(client, `document.querySelectorAll('.session-item')[1].click();${fixture}.deferSummary=false;${fixture}.phase=0;${fixture}.release();true`); await flush();
    assert.equal(await evaluate(client, `document.querySelectorAll('.session-item')[1].getAttribute('aria-current')`), 'page');
    assert.ok(await evaluate(client, `document.querySelector('.transcript-head h2').textContent.includes('session-2')`));
  });
  for (const [language, theme, width, height] of [['en', 'light', 755, 600], ['zh', 'dark', 1200, 813]]) {
    for (const initial of ['pending', 'error']) await run(`session.excerpt-${initial}-recovers-with-unchanged-digest.${language}`, async () => {
      await open(`mode=excerpt&body=${initial}&lang=${language}&theme=${theme}`, width, height);
      await waitFor(client, `${fixture}.reads.some(r=>r.view==='catalog')`); await flush();
      const initialText = await evaluate(client, `document.querySelector('.selected-excerpt').textContent`);
      await shot(`session-excerpt-${initial}-${language}-${theme}-${width}`);
      await evaluate(client, `${fixture}.body='available';true`); await advance(2000);
      await waitFor(client, `document.querySelector('.selected-excerpt').textContent.includes('Ready user content')`);
      assert.ok(!initialText.includes('No user message recorded'));
      const reads = await readCount("r.view==='catalog'"); await advance(60000); assert.equal(await readCount("r.view==='catalog'"), reads);
      await shot(`session-excerpt-recovered-${initial}-${language}-${theme}-${width}`);
    });
  }
  await run('session.excerpt-complete-items-do-not-hide-request-or-body-gaps', async () => {
    await open('mode=excerpt'); await waitFor(client, `${fixture}.reads.length===1`);
    await evaluate(client, `${fixture}.body='available';${fixture}.onlyResponse=true;${fixture}.rootsPartial=true;true`); await advance(2000);
    assert.equal(await evaluate(client, `document.querySelector('.selected-excerpt span').dataset.contentState`), 'pending');
    await evaluate(client, `${fixture}.onlyResponse=false;${fixture}.rootsPartial=false;${fixture}.partPartial=true;true`); await advance(4000);
    await waitFor(client, `document.querySelector('.selected-excerpt').textContent.includes('Ready user content')`);
    const partialReads = await readCount("r.view==='content'");
    await evaluate(client, `${fixture}.partPartial=false;true`); await advance(8000);
    assert.equal(await readCount("r.view==='content'"), partialReads + 1);
    const completeReads = await readCount('true'); await advance(60000); assert.equal(await readCount('true'), completeReads);
    await evaluate(client, `${fixture}.onlyResponse=true;${fixture}.invalidate();true`); await flush();
    assert.equal(await evaluate(client, `document.querySelector('.selected-excerpt').textContent`), 'No user message recorded');
    const absentReads = await readCount('true'); await advance(60000); assert.equal(await readCount('true'), absentReads);
  });
  await run('session.excerpt-bounded-backoff-hidden-offscreen-inactive', async () => {
    await open('mode=excerpt'); await waitFor(client, `${fixture}.reads.length===1`);
    for (const delay of [2000, 4000, 8000, 16000, 30000, 30000]) { const previous = await readCount("r.view==='catalog'"); await advance(delay - 1); assert.equal(await readCount("r.view==='catalog'"), previous); await advance(1); assert.equal(await readCount("r.view==='catalog'"), previous + 1); }
    await evaluate(client, `${fixture}.visibility(true);true`); const hidden = await readCount('true'); await advance(60000); assert.equal(await readCount('true'), hidden);
    await evaluate(client, `${fixture}.visibility(false);true`); await flush(); assert.equal(await readCount('true'), hidden + 1);
    await evaluate(client, `document.querySelector('#toggle-active').click();true`); await flush(); const inactive = await readCount('true'); await advance(60000); assert.equal(await readCount('true'), inactive);
    await evaluate(client, `document.querySelector('#toggle-active').click();true`); await flush();
    await evaluate(client, `document.querySelector('main').scrollTop=document.querySelector('main').scrollHeight;true`); await flush(); const offscreen = await readCount('true'); await advance(60000); assert.equal(await readCount('true'), offscreen);
    await evaluate(client, `${fixture}.body='available';document.querySelector('main').scrollTop=0;true`); await waitFor(client, `document.querySelector('.selected-excerpt').textContent.includes('Ready user content')`);
  });
  await run('session.cleanup-already-cleared-session-opens-without-body-reads', async () => {
    await open('body=available&routed&cleared');
    await waitFor(client, "document.querySelector('.transcript-head h2')?.textContent==='Content cleared'");
    await waitFor(client, "document.querySelector('.routing-quote span')?.dataset.contentState==='cleared'");
    await advance(60000);
    assert.equal(await readCount("['catalog','content'].includes(r.view)&&r.query.request_id==='request/session-1'"), 0);
  });
  await run('session.excerpt-empty-partial-catalog-is-not-proof-of-cleanup', async () => {
    await open('mode=excerpt'); await waitFor(client, `${fixture}.reads.length===1`);
    await evaluate(client, `${fixture}.rootsPartial=true;true`); await advance(2000);
    assert.equal(await evaluate(client, "document.querySelector('.selected-excerpt span').dataset.contentState"), 'pending');
    await evaluate(client, `${fixture}.rootsPartial=false;${fixture}.body='available';true`); await advance(4000);
    await waitFor(client, "document.querySelector('.selected-excerpt').textContent.includes('Ready user content')");
  });
  await run('session.cleanup-empty-catalog-invalidates-in-flight-and-stops-retries', async () => {
    await open('body=available&routed');
    await waitFor(client, "!!document.querySelector('.session-pane:not([hidden]) .routing-quote')");
    await evaluate(client, "document.querySelector('.routing-quote').scrollIntoView({block:'center'});true");
    await waitFor(client, "document.querySelector('.routing-quote').textContent.includes('Ready user content')");
    await evaluate(client, `${fixture}.deferContent=true;${fixture}.invalidate();true`);
    await waitFor(client, `${fixture}.releases.length>0`);
    await evaluate(client, `document.querySelector('[aria-label="Clear this session"]').click();true`);
    await waitFor(client, "!!document.querySelector('[role=dialog] .btn-danger')");
    await evaluate(client, "document.querySelector('[role=dialog] .btn-danger').click();true");
    await waitFor(client, `${fixture}.clearedSessions.has('session-1')`);
    await waitFor(client, "document.querySelector('.transcript-head h2').textContent==='Content cleared'");
    await evaluate(client, `${fixture}.deferContent=false;${fixture}.release();true`); await flush();
    await waitFor(client, "!document.querySelector('[role=dialog]')");
    await waitFor(client, "document.querySelector('.routing-quote span')?.dataset.contentState==='cleared'");
    const filter = "['catalog','content'].includes(r.view)&&r.query.request_id==='request/session-1'";
    const reads = await readCount(filter); await advance(60000); assert.equal(await readCount(filter), reads);
    assert.equal(await evaluate(client, "document.querySelector('.routing-quote').textContent"), 'Content cleared');
    assert.equal(await evaluate(client, "document.querySelector('.transcript-head h2').textContent"), 'Content cleared');
    assert.equal(await evaluate(client, "document.querySelector('.session-item[aria-current=page] .session-item-title').textContent"), 'Content cleared');
    await shot('session-cleanup-empty-catalog-en-light');
    // Timeline and summary reads are independent. Do not permanently classify
    // a newly arrived request as cleared using an older session summary.
    await evaluate(client, `${fixture}.deferSummary=true;true`); await advance(2000);
    await waitFor(client, `${fixture}.releases.length>0`);
    await evaluate(client, `${fixture}.revivedSessions.add('session-1');true`); await advance(2000);
    await waitFor(client, "document.querySelectorAll('.routing-turn').length===2");
    await evaluate(client, `${fixture}.deferSummary=false;${fixture}.release();true`); await flush();
    await evaluate(client, `${fixture}.revive('session-1');true`); await advance(2000);
    await waitFor(client, "document.querySelector('.transcript-head h2').textContent.includes('session-1/new')");
    await waitFor(client, "document.querySelectorAll('.routing-turn').length===2");
    await evaluate(client, `document.querySelector('[data-turn-id="request/session-1"]').click();true`);
    await waitFor(client, "document.querySelector('.routing-quote').textContent==='Content cleared'");
    await evaluate(client, `document.querySelector('[data-turn-id="request/session-1/new"]').click();true`);
    await evaluate(client, "document.querySelector('.routing-quote').scrollIntoView({block:'center'});true");
    await waitFor(client, "document.querySelector('.routing-quote').textContent.includes('session-1/new')");
    assert.ok(!(await evaluate(client, "document.querySelector('.session-item[aria-current=page]').textContent")).includes('Content cleared'));
  });
  for (const state of ['deleted', 'expired']) await run(`session.cleanup-${state}-summary-invalidates-cached-excerpts-and-reopens`, async () => {
    await open('body=available&routed&lang=zh&theme=dark', 755, 600);
    await waitFor(client, "document.querySelector('.transcript-head h2')?.textContent.includes('Ready user content')");
    await evaluate(client, "document.querySelector('.routing-quote').scrollIntoView({block:'center'});true");
    await waitFor(client, "document.querySelector('.routing-quote').textContent.includes('Ready user content')");
    const summaryBefore = await readCount("r.view==='sessions'&&r.query.session_id");
    await evaluate(client, `${fixture}.clear('session-1','${state}');true`); await advance(2000);
    await waitFor(client, "document.querySelector('.transcript-head h2').textContent==='正文已清理'");
    assert.ok(await readCount("r.view==='sessions'&&r.query.session_id") > summaryBefore);
    assert.equal(await evaluate(client, "document.querySelector('.session-item[aria-current=page] .session-item-title').textContent"), '正文已清理');
    assert.equal(await evaluate(client, "document.querySelector('.routing-quote').textContent"), '正文已清理');
    const filter = "['catalog','content'].includes(r.view)&&r.query.request_id==='request/session-1'";
    const reads = await readCount(filter); await advance(60000); assert.equal(await readCount(filter), reads);
    await shot(`session-cleanup-${state}-zh-dark-755`);
    await evaluate(client, "document.querySelectorAll('.session-item')[1].click();true");
    await waitFor(client, "document.querySelector('.transcript-head h2').textContent.includes('session-2')");
    await evaluate(client, "document.querySelectorAll('.session-item')[0].click();true");
    await waitFor(client, "document.querySelector('.routing-quote span')?.dataset.contentState==='cleared'");
    assert.equal(await readCount(filter), reads, 'reopening a known cleared session does not fetch body');
  });
} finally {
  await writeFile(path.join(output, 'session-live-refresh-results.json'), JSON.stringify(results, null, 2));
  clearInterval(keepAlive); client.close();
}
