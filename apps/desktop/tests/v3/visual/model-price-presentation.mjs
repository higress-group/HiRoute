import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const output = path.resolve(process.argv[2] ?? path.join(desktop, 'test-results/model-price-presentation'));
const port = () => new Promise((resolve, reject) => {
  const server = net.createServer(); server.once('error', reject);
  server.listen(0, '127.0.0.1', () => { const address = server.address(); server.close(() => resolve(address.port)); });
});
const ready = async url => {
  for (let i = 0; i < 200; i++) { try { if ((await fetch(url)).ok) return; } catch {} await new Promise(r => setTimeout(r, 50)); }
  throw new Error(`Timed out waiting for ${url}`);
};
const stop = async child => {
  if (!child?.pid) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 1000))]);
  child.stdout?.destroy(); child.stderr?.destroy();
};
const httpPort = await port(); let cdpPort = await port(); while (cdpPort === httpPort) cdpPort = await port();
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-model-price-'));
await mkdir(output, { recursive: true });
let vite, chrome, client;
const results = [];
try {
  vite = spawn(path.join(desktop, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktop, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { vite.once('spawn', resolve); vite.once('error', reject); });
  await ready(`http://127.0.0.1:${httpPort}/`);
  const executable = process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
  chrome = spawn(executable, ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, 'about:blank'], { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { chrome.once('spawn', resolve); chrome.once('error', reject); });
  await ready(`http://127.0.0.1:${cdpPort}/json/version`);
  client = await connectPage(cdpPort);
  const open = async (query, width, height) => {
    await navigate(client, `http://127.0.0.1:${httpPort}/model-price-presentation.html?${query}`, { width, height });
    await waitFor(client, `!!document.querySelector('.models-feature .master-list .list-row')`);
    await evaluate(client, `document.querySelector('.models-feature .master-list .list-row').click();true`);
    await waitFor(client, `document.querySelector('.v3-price')?.getClientRects().length > 0 && !document.querySelector('.v3-price').innerText.includes('Reading pricing') && !document.querySelector('.v3-price').innerText.includes('正在读取价格')`);
  };
  const click = async (selector, label) => {
    const found = await evaluate(client, `(()=>{const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(n=>n.getClientRects().length && n.innerText.includes(${JSON.stringify(label)}));if(!e)return false;e.click();return true;})()`);
    assert.equal(found, true, `Missing ${label}`);
  };
  const snapshot = async name => {
    const geometry = await evaluate(client, `(()=>{const e=document.querySelector('.v3-price');e.scrollIntoView({block:'center'});const r=e.getBoundingClientRect();return {left:r.left,right:r.right,width:e.clientWidth,scroll:e.scrollWidth,body:document.documentElement.scrollWidth,viewport:innerWidth};})()`);
    assert.ok(geometry.scroll <= geometry.width + 1 && geometry.left >= -1 && geometry.right <= geometry.viewport + 1 && geometry.body <= geometry.viewport + 1, JSON.stringify(geometry));
    const shot = await client.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(output, `${name}.png`), Buffer.from(shot.data, 'base64'));
  };
  const detailReads = `window.modelPriceFixture.requests.filter(r=>r.command==='effective_price_query' && r.payload.input.targets[0].query_id==='model-detail')`;
  const price = `document.querySelector('.v3-price').innerText`;
  const edit = async lang => {
    await click('.detail-section button', lang === 'zh' ? '调整价格' : 'Edit pricing');
    await waitFor(client, `!!document.querySelector('[role="dialog"] .price-mode-list')`);
  };
  const manual = async (lang, input, outputRate) => {
    await click('[role="dialog"] label', lang === 'zh' ? '手工设置费率' : 'Set rates manually');
    for (const [name, value] of [['input_rate', input], ['output_rate', outputRate]]) {
      await evaluate(client, `(()=>{const e=document.querySelector('[name="${name}"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    }
  };
  const save = async lang => click('[role="dialog"] button', lang === 'zh' ? '保存价格' : 'Save pricing');
  const sequentialFailures = [];
  for (const scope of ['same', 'other']) {
    await open(`lang=en&stateful=1&rates=unknown&two=${scope}`, 1200, 813);
    const selected = await evaluate(client, `document.querySelector('.detail-identity h2').innerText`);
    await edit('en'); await manual('en', '0.30', '1.20'); await save('en');
    await waitFor(client, `!document.querySelector('[role="dialog"]') && ${price}.includes('$0.3 / $1.2')`);
    await evaluate(client, `document.querySelectorAll('.models-feature .master-list .list-row')[1].click();true`);
    await waitFor(client, `document.querySelector('.detail-identity h2')?.innerText==='Other fixture model' && ${price}.includes('$9 / $10')`);
    await edit('en'); await manual('en', '2.50', '5');
    await evaluate(client, `window.modelPriceFixture.outcome='hold'`); await save('en');
    await waitFor(client, `typeof window.modelPriceFixture.releaseSave==='function'`);
    await evaluate(client, `document.querySelectorAll('.models-feature .master-list .list-row')[0].click();true`);
    await waitFor(client, `document.querySelector('.detail-identity h2')?.innerText===${JSON.stringify(selected)} && ${price}.includes('$0.3 / $1.2')`);
    const primaryReads = `${detailReads}.filter(r=>r.payload.input.targets[0].target_locator.binding_id===window.modelPriceFixture.primaryBinding).length`;
    const before = await evaluate(client, primaryReads);
    await evaluate(client, `window.modelPriceFixture.releaseSave();true`);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && window.modelPriceFixture.quotes['binding/fixture/other'].revision===7`);
    await evaluate(client, `new Promise(resolve=>setTimeout(()=>resolve(true),150))`);
    const after = await evaluate(client, primaryReads);
    const state = await evaluate(client, `({selected:document.querySelector('.detail-identity h2').innerText,price:${price},primary:window.modelPriceFixture.quotes[window.modelPriceFixture.primaryBinding],other:window.modelPriceFixture.quotes['binding/fixture/other']})`);
    assert.equal(state.selected, selected);
    assert.ok(state.price.includes('$0.3 / $1.2'));
    assert.deepEqual(state.primary.rates, ['0.30', '1.20', null, null]);
    assert.deepEqual(state.other.rates, ['2.50', '5', null, null]);
    if (after !== before) {
      sequentialFailures.push({ scope, before, after, state });
      const shot = await client.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(path.join(output, `cross-binding-failure-${scope}.png`), Buffer.from(shot.data, 'base64'));
    }
    await evaluate(client, `document.querySelectorAll('.models-feature .master-list .list-row')[1].click();true`);
    await waitFor(client, `document.querySelector('.detail-identity h2')?.innerText==='Other fixture model' && ${price}.includes('$2.5 / $5')`);
    results.push({ scenario: `sequential-save-preserves-other-summary-${scope}-source`, status: after === before ? 'passed' : 'failed' });
  }
  if (sequentialFailures.length) {
    await writeFile(path.join(output, 'cross-binding-failures.json'), JSON.stringify(sequentialFailures, null, 2));
    assert.equal(sequentialFailures.length, 0, 'Saving another binding must not re-read a previously refreshed summary');
  }
  for (const lang of ['en', 'zh']) for (const width of [755, 1200]) {
    await open(`lang=${lang}&theme=${width === 755 ? 'dark' : 'light'}&stateful=1&rates=unknown`, width, width === 755 ? 600 : 813);
    const selected = await evaluate(client, `document.querySelector('.detail-identity h2').innerText`);
    const initialReads = await evaluate(client, `${detailReads}.length`);
    await edit(lang); await manual(lang, '0.30', '1.20'); await save(lang);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && !!document.querySelector('.toast')`);
    try {
      await waitFor(client, `${detailReads}.length === ${initialReads + 1} && ${price}.includes('$0.3 / $1.2')`, { timeout: 3000 });
    } catch (error) {
      const state = await evaluate(client, `({price:${price},reads:${detailReads}.length,rates:window.modelPriceFixture.rates,revision:window.modelPriceFixture.revision})`);
      await writeFile(path.join(output, 'save-refresh-failure.json'), JSON.stringify({ error: String(error), state }, null, 2));
      const shot = await client.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(path.join(output, 'save-refresh-failure.png'), Buffer.from(shot.data, 'base64'));
      throw error;
    }
    assert.equal(await evaluate(client, `document.querySelector('.detail-identity h2').innerText`), selected);
    assert.ok((await evaluate(client, price)).includes(lang === 'zh' ? '人工费率' : 'Manual rates'));
    const request = await evaluate(client, `window.modelPriceFixture.requests.find(r=>r.command==='preview_price_change').payload.input`);
    assert.deepEqual(request.action, { kind: 'set', input_uncached: '0.30', output: '1.20', cache_read: null, cache_write: null });
    assert.equal(await evaluate(client, `${detailReads}.every(r=>r.payload.input.targets[0].target_locator.binding_id===${JSON.stringify(request.target_locator.binding_id)} && r.payload.input.targets[0].currency==='USD' && r.payload.input.targets[0].valuation_kind==='api_equivalent')`), true);
    await snapshot(`saved-manual-${lang}-${width}`);

    await edit(lang); await click('[role="dialog"] label', lang === 'zh' ? '跟随目录价格' : 'Follow catalog pricing'); await save(lang);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && ${detailReads}.length === ${initialReads + 2} && ${price}.includes(${JSON.stringify(lang === 'zh' ? '未计价' : 'Unpriced')})`);
    assert.equal(await evaluate(client, `document.querySelector('.detail-identity h2').innerText`), selected);
    await snapshot(`restored-unknown-${lang}-${width}`);

    await edit(lang); await manual(lang, '0', '1.20'); await save(lang);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && ${detailReads}.length === ${initialReads + 3} && ${price}.includes('$0 / $1.2')`);
    await edit(lang); await manual(lang, '0.30', '0'); await save(lang);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && ${detailReads}.length === ${initialReads + 4} && ${price}.includes('$0.3 / $0')`);
    results.push({ scenario: 'successful-save-refresh-and-restore', lang, width, status: 'passed' });
  }
  for (const outcome of ['cancel', 'failure', 'pending', 'failed_operation']) {
    await open('lang=en&stateful=1&rates=unknown', 1200, 813);
    const before = await evaluate(client, `${detailReads}.length`);
    await edit('en'); await manual('en', '0.30', '1.20');
    await evaluate(client, `window.modelPriceFixture.outcome=${JSON.stringify(outcome)}`);
    await save('en');
    const message = outcome === 'cancel' ? 'Cancelled. Pricing was not changed.' : outcome === 'failure' ? 'Pricing changed elsewhere.' : 'Confirming the save result.';
    await waitFor(client, `document.querySelector('[role="dialog"]')?.innerText.includes(${JSON.stringify(message)})`);
    await evaluate(client, `new Promise(resolve=>setTimeout(()=>resolve(true),150))`);
    assert.equal(await evaluate(client, `${detailReads}.length`), before, `${outcome} cannot refresh the summary as a successful save`);
    assert.ok((await evaluate(client, price)).includes('Unpriced'));
    assert.equal(await evaluate(client, `window.modelPriceFixture.revision`), 6);
    assert.equal(await evaluate(client, `!!document.querySelector('.toast')`), false);
    await click('[role="dialog"] button', 'Cancel');
    results.push({ scenario: `unsuccessful-save-${outcome}`, status: 'passed' });
  }
  for (const scope of ['same', 'other']) {
    await open(`lang=en&stateful=1&rates=unknown&two=${scope}`, 1200, 813);
    await edit('en'); await manual('en', '0.30', '1.20');
    await evaluate(client, `window.modelPriceFixture.outcome='hold'`); await save('en');
    await waitFor(client, `typeof window.modelPriceFixture.releaseSave==='function'`);
    await evaluate(client, `document.querySelectorAll('.models-feature .master-list .list-row')[1].click();true`);
    await waitFor(client, `document.querySelector('.detail-identity h2')?.innerText==='Other fixture model' && ${price}.includes('$9 / $10')`);
    const otherReads = await evaluate(client, `${detailReads}.filter(r=>r.payload.input.targets[0].target_locator.binding_id==='binding/fixture/other').length`);
    await evaluate(client, `window.modelPriceFixture.releaseSave();true`);
    await waitFor(client, `!document.querySelector('[role="dialog"]') && !!document.querySelector('.toast')`);
    await evaluate(client, `new Promise(resolve=>setTimeout(()=>resolve(true),150))`);
    assert.equal(await evaluate(client, `document.querySelector('.detail-identity h2').innerText`), 'Other fixture model');
    assert.ok((await evaluate(client, price)).includes('$9 / $10'));
    assert.equal(await evaluate(client, `${detailReads}.filter(r=>r.payload.input.targets[0].target_locator.binding_id==='binding/fixture/other').length`), otherReads, 'saving another binding must not re-read this model');
    await evaluate(client, `document.querySelectorAll('.models-feature .master-list .list-row')[0].click();true`);
    await waitFor(client, `${price}.includes('$0.3 / $1.2')`);
    results.push({ scenario: `save-retains-selection-and-target-${scope}-source`, status: 'passed' });
  }

  for (const lang of ['en', 'zh']) for (const theme of ['light', 'dark']) for (const width of [755, 1200]) {
    for (const scenario of ['unknown', 'input', 'output', 'zero_input', 'zero_output']) {
      await open(`lang=${lang}&theme=${theme}&rates=${scenario}`, width, width === 755 ? 600 : 813);
      const state = await evaluate(client, `(()=>{const price=document.querySelector('.v3-price'),section=price.closest('.detail-section');return {price:price.innerText,edit:[...section.querySelectorAll('button')].some(e=>e.getClientRects().length && e.innerText.includes(${JSON.stringify(lang === 'zh' ? '调整价格' : 'Edit pricing')}) && !e.disabled)};})()`);
      assert.equal(state.edit, true, `${scenario}: explicit context remains editable`);
      if (['unknown', 'input', 'output'].includes(scenario)) {
        assert.ok(state.price.includes(lang === 'zh' ? '未计价' : 'Unpriced'), `${scenario}: ${state.price}`);
        assert.ok(!state.price.includes('$—'), `${scenario}: no partially invented summary`);
      } else {
        assert.ok(state.price.includes(scenario === 'zero_input' ? '$0 / $2' : '$0.8 / $0'), `${scenario}: ${state.price}`);
        assert.ok(!state.price.includes(lang === 'zh' ? '未计价' : 'Unpriced'));
      }
      await snapshot(`${scenario}-${lang}-${theme}-${width}`);
      results.push({ scenario, lang, theme, width, status: 'passed' });
    }
  }
  for (const [scenario, cacheRead, cacheWrite] of [['unknown', null, null], ['known_cache', '0.05', '0.01']]) {
    await open(`lang=en&rates=${scenario}`, 755, 600);
    await click('.detail-section button', 'Edit pricing');
    await waitFor(client, `!!document.querySelector('[role="dialog"] .price-mode-list')`);
    await click('[role="dialog"] label', 'Set rates manually');
    for (const [name, value] of [['input_rate', '0.8'], ['output_rate', '2']]) {
      await evaluate(client, `(()=>{const e=document.querySelector('[name="${name}"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    }
    const geometry = await evaluate(client, `(()=>{const d=document.querySelector('[role="dialog"]'),field=d.querySelector('[name="output_rate"]');field.scrollIntoView({block:'nearest'});const b=[...d.querySelectorAll('button')].find(e=>e.innerText.includes('Save pricing')),r=b.getBoundingClientRect(),f=field.getBoundingClientRect(),body=d.querySelector('.modal-body').getBoundingClientRect(),foot=d.querySelector('.modal-foot').getBoundingClientRect();return {scroll:d.scrollWidth,width:d.clientWidth,left:r.left,right:r.right,bottom:r.bottom,viewport:innerWidth,height:innerHeight,fieldTop:f.top,fieldBottom:f.bottom,bodyTop:body.top,footerTop:foot.top};})()`);
    assert.ok(geometry.scroll <= geometry.width + 1 && geometry.left >= 0 && geometry.right <= geometry.viewport && geometry.bottom <= geometry.height && geometry.fieldTop >= geometry.bodyTop - 1 && geometry.fieldBottom <= geometry.footerTop + 1, JSON.stringify(geometry));
    const shot = await client.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(output, `manual-editor-${scenario}-en-755.png`), Buffer.from(shot.data, 'base64'));
    await click('[role="dialog"] button', 'Save pricing');
    await waitFor(client, `window.modelPriceFixture.requests.some(r=>r.command==='preview_price_change')`);
    const request = await evaluate(client, `window.modelPriceFixture.requests.find(r=>r.command==='preview_price_change').payload.input`);
    assert.equal(request.currency, 'USD'); assert.equal(request.valuation_kind, 'api_equivalent');
    assert.equal(request.expected_override_revision, 6);
    assert.deepEqual(request.action, { kind: 'set', input_uncached: '0.8', output: '2', cache_read: cacheRead, cache_write: cacheWrite });
    results.push({ scenario: `manual-editor-preserves-${scenario}-cache`, status: 'passed' });
  }
  await open('lang=en&rates=unknown&fail=1', 1200, 813);
  assert.ok((await evaluate(client, `document.querySelector('.v3-price').innerText`)).includes('temporarily unavailable'));
  await click('.v3-price button', 'Retry');
  await waitFor(client, `document.querySelector('.v3-price').innerText.includes('No verified rates')`);
  results.push({ scenario: 'failed-read-retry', status: 'passed' });
  for (const [kind, label] of [['subscription', 'Subscription'], ['free', 'Free']]) {
    await open(`lang=en&kind=${kind}`, 1200, 813);
    assert.ok((await evaluate(client, `document.querySelector('.v3-price').innerText`)).includes(label));
    assert.equal(await evaluate(client, `[...document.querySelectorAll('.detail-section button')].some(e=>e.innerText.includes('Edit pricing'))`), false);
    results.push({ scenario: `${kind}-billing`, status: 'passed' });
  }
  await writeFile(path.join(output, 'report.json'), JSON.stringify({ proof: 'production model management and price editor with synthetic IPC; no native or provider claim', results }, null, 2));
  console.log(JSON.stringify({ passed: results.length, output }));
} finally {
  client?.close(); await stop(chrome); await stop(vite); await rm(profile, { recursive: true, force: true });
}
