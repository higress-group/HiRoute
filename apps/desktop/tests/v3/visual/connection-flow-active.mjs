import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
await client.send('Page.addScriptToEvaluateOnNewDocument', { source: `
  window.fixtureBrowserErrors = [];
  addEventListener('error', event => window.fixtureBrowserErrors.push({ message: event.message, file: event.filename, line: event.lineno, target: event.target?.tagName, source: event.target?.src }), true);
  addEventListener('unhandledrejection', event => window.fixtureBrowserErrors.push({ rejection: String(event.reason) }));
` });
const settled = async () => {
  await evaluate(client, `Promise.all(document.getAnimations().filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => undefined))).then(() => true)`);
  await evaluate(client, 'new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))))');
};
const screenshot = async name => {
  await settled();
  const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(shot.data, 'base64'));
};
const open = async (query = '', viewport = { width: 1200, height: 813 }, page = 'connection-flow.html') => {
  const url = `${base}${page}?${query}&run=${crypto.randomUUID()}`;
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { ...viewport, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url });
  await waitFor(client, `location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`);
  await evaluate(client, 'document.fonts.ready.then(() => true)');
  await waitFor(client, `document.querySelector('.connection-flow') && !document.querySelector('.oc-spinner')`);
  await settled();
};
const click = async (selector, text = '') => {
  const expression = `Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(item => item.getClientRects().length && !item.disabled && (!${JSON.stringify(text)} || item.textContent.trim() === ${JSON.stringify(text)} || item.querySelector('strong')?.textContent === ${JSON.stringify(text)}))`;
  await waitFor(client, `Boolean(${expression})`);
  const point = await evaluate(client, `(() => { const item = ${expression}; item.scrollIntoView({block:'center',behavior:'instant'}); const rect=item.getBoundingClientRect();return {x:rect.x+rect.width/2,y:rect.y+rect.height/2}; })()`);
  await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
  await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
};
const field = async (label, value) => {
  const selector = `Array.from(document.querySelectorAll('.field')).find(item => item.getClientRects().length && item.querySelector('.field-label,.sr-only')?.textContent === ${JSON.stringify(label)})?.querySelector('input,select')`;
  await waitFor(client, `Boolean(${selector})`);
  await evaluate(client, `(() => {const input=${selector};const prototype=input instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event(input instanceof HTMLSelectElement?'change':'input',{bubbles:true}));})()`);
};
const provider = async name => {
  await click('.connection-provider-card', name);
  await waitFor(client, `document.querySelector('#mc-connection-form').getClientRects().length > 0`);
};
const backToCatalog = async () => {
  await click('.modal-foot button', '返回');
  await waitFor(client, `document.querySelectorAll('.connection-provider-card').length > 0`);
};
const run = async (name, task) => {
  if (process.env.HIROUTE_CONNECTION_CASE && process.env.HIROUTE_CONNECTION_CASE !== name) return;
  const started = performance.now();
  try { await task(); results.push({ name, state: 'green', elapsedMs: performance.now() - started }); }
  catch (error) {
    await screenshot(`failure-${results.length}`).catch(() => undefined);
    const browser = await evaluate(client, `({url:location.href,ready:document.readyState,root:document.querySelector('#root')?.innerHTML.slice(0,500),errors:window.fixtureBrowserErrors})`).catch(() => null);
    results.push({ name, state: 'red', error: error.stack || String(error), browser, elapsedMs: performance.now() - started });
  }
};
try {
  await mkdir(output, { recursive: true });
  await run('complete branded directory, search, plans and free scope', async () => {
    await open();
    await waitFor(client, `Array.from(document.querySelectorAll('.connection-provider-card img')).every(image=>image.complete&&image.naturalWidth>0)`);
    const facts = await evaluate(client, `({count:document.querySelectorAll('.connection-provider-card').length,intro:document.querySelector('.connection-directory-intro').textContent,logos:Array.from(document.querySelectorAll('.connection-provider-card img')).every(image=>image.complete&&image.naturalWidth>0)})`);
    assert.equal(facts.count, 13); assert.match(facts.intro, /13.*19/); assert.ok(facts.logos);
    await screenshot('catalog');
    await field('搜索供应商或产品', 'Coding Plan');
    await waitFor(client, `document.querySelectorAll('.connection-provider-card').length === 4`);
    await evaluate(client, `(() => {const input=document.querySelector('input[type=search]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'');input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await waitFor(client, `document.querySelectorAll('.connection-provider-card').length === 13`);
    await click('.connection-billing-filters button', '免费资源');
    await waitFor(client, `document.querySelectorAll('.connection-provider-card').length === 5`);
    await click('.connection-billing-filters button', '全部');
    await click('.connection-provider-card', '百炼3 种接入方式');
    await waitFor(client, `document.querySelectorAll('.connection-product-card').length === 3`);
    await screenshot('products');
    await click('.connection-product-card', '百炼 · Token Plan');
    await waitFor(client, `document.querySelector('#mc-connection-form').getClientRects().length > 0`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), '百炼 · Token Plan');
    await open('kind=free');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-provider-card').length`), 5);
    assert.equal(await evaluate(client, `document.querySelector('.connection-custom-entry')`), null);
  });
  await run('names and keys return to their own product; save uses the checked name', async () => {
    await open();
    await provider('Anthropic按量付费');
    await field('连接名称', '团队 · 生产'); await field('API Key', 'fixture-key-a');
    await screenshot('configuration');
    await backToCatalog(); await provider('DeepSeek按量付费');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '');
    await field('连接名称', '个人 · 备用'); await field('API Key', 'fixture-key-b');
    await backToCatalog(); await provider('Anthropic按量付费');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), '团队 · 生产');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    await click('.modal-foot button', '检查并继续');
    await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    assert.equal(await evaluate(client, `document.querySelector('.model-selection h3').textContent`), '团队 · 生产');
    assert.ok(await evaluate(client, `Array.from(document.querySelectorAll('.modal-foot button')).find(item=>item.textContent==='保存连接').disabled`));
    await click('.modal-foot button', '返回'); await backToCatalog(); await provider('DeepSeek按量付费');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-b'`), true);
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.model-selection input[type=checkbox]');
    await screenshot('models');
    await click('.modal-foot button', '保存连接');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', '放弃修改');
    await waitFor(client, `!document.querySelector('.modal')`);
    const trace = await evaluate(client, 'window.connectionFlowTrace');
    assert.deepEqual(trace.registered, ['a', 'b']);
    assert.deepEqual(trace.checks.map(item => item.display_name), ['团队 · 生产', '个人 · 备用']);
    assert.notEqual(trace.checks[0].connection_option_id, trace.checks[1].connection_option_id);
    assert.notEqual(trace.checks[0].lineage_ref, trace.checks[1].lineage_ref);
    assert.ok(trace.checks.every(item => !('secret' in item) && item.inference_model_id === null));
    assert.equal(trace.saves.length, 1); assert.equal(trace.saves[0].selected_model_refs.length, 1);
    assert.deepEqual(trace.released, ['input/1']);
    assert.ok(await evaluate(client, `Array.from(document.querySelectorAll('.row-meta')).some(item=>item.textContent.includes('个人 · 备用'))`));
  });
  await run('name is required and editing it invalidates the old checked selection', async () => {
    await open(); await provider('Anthropic按量付费');
    await field('连接名称', ' '); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('[data-error-code=MODEL_CONNECTION_NAME_REQUIRED]'))`);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.checks.length'), 0);
    await field('连接名称', '第一次'); await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', '返回');
    await field('连接名称', '修改后'); await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    const trace = await evaluate(client, 'window.connectionFlowTrace');
    assert.deepEqual(trace.checks.map(item=>item.display_name), ['第一次', '修改后']);
    assert.equal(trace.registered.length, 1);
    assert.ok(await evaluate(client, `Array.from(document.querySelectorAll('.modal-foot button')).find(item=>item.textContent==='保存连接').disabled`));
  });
  await run('custom no-auth and manual model declarations remain usable', async () => {
    await open('kind=custom');
    await field('连接名称', '团队 HTTP 模型'); await field('Base URL', 'http://192.0.2.10:51234/v1');
    await click('summary', '高级设置展开'); await field('认证方式', 'none');
    await waitFor(client, `!document.querySelector('#mc-connection-form input[type=password]')`);
    await click('#mc-connection-form button', '手工维护模型');
    await waitFor(client, `Boolean(document.querySelector('#mc-manual-form'))`);
    await field('模型 ID', 'local-test-model');
    await click('.modal-foot button', '检查并保存'); await waitFor(client, `!document.querySelector('.modal')`);
    const trace = await evaluate(client, 'window.connectionFlowTrace');
    assert.equal(trace.registered.length, 0); assert.equal(trace.saves.length, 1);
    assert.equal(trace.checks[0].draft.authentication.kind, 'none');
    assert.equal(trace.checks[0].draft.models[0].upstream_model_id, 'local-test-model');
    assert.equal(trace.checks[0].draft.display_name, '团队 HTTP 模型');
    assert.equal(trace.checks[0].draft.base_url, 'http://192.0.2.10:51234/v1');
    assert.equal(trace.checks[0].draft.inventory_path_override, null);
  });
  await run('explicit HTTP explains plaintext without blocking the configured endpoint', async () => {
    for (const lang of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await open(`kind=custom&lang=${lang}&theme=${theme}`, viewport);
      await field('Base URL', 'http://gateway.invalid:8123/v1');
      await waitFor(client, 'Boolean(document.querySelector("[data-plaintext-http-warning]"))');
      const warning = await evaluate(client, `(() => {
        const note = document.querySelector('[data-plaintext-http-warning]');
        note.scrollIntoView({ block: 'center', behavior: 'instant' });
        const rect = note.getBoundingClientRect(); const footer = document.querySelector('.modal-foot').getBoundingClientRect();
        const header = document.querySelector('.modal-head').getBoundingClientRect();
        return { text: note.textContent, left: rect.left, right: rect.right, height: rect.height,
          fontSize: parseFloat(getComputedStyle(note).fontSize),
          top: rect.top, bottom: rect.bottom, headerBottom: header.bottom, footerTop: footer.top,
          footerBottom: footer.bottom, overflow: document.documentElement.scrollWidth > innerWidth + 1 };
      })()`);
      assert.match(warning.text, /API Key|API keys/);
      assert.match(warning.text, /明文|plaintext/);
      assert.match(warning.text, /HTTPS/);
      assert.ok(warning.left >= 0 && warning.right <= viewport.width && warning.height >= warning.fontSize && !warning.overflow, JSON.stringify(warning));
      assert.ok(warning.top >= warning.headerBottom && warning.bottom <= warning.footerTop, JSON.stringify(warning));
      assert.ok(warning.footerBottom <= viewport.height + 1, JSON.stringify(warning));
      await screenshot(`http-warning-${lang}-${theme}-${viewport.width}`);
      assert.equal(await evaluate(client, 'window.connectionFlowTrace.checks.length'), 0);
      await field('Base URL', 'https://gateway.invalid/v1');
      await waitFor(client, '!document.querySelector("[data-plaintext-http-warning]")');
    }
  });
  await run('safe backend target reasons become actionable localized errors and retain input', async () => {
    for (const lang of ['zh', 'en']) for (const [reason, correction] of [
      ['invalid_url', /http:\/\/.*https:\/\//],
      ['unsafe_path', /路径|paths/],
      ['unsafe_authentication_header', /Bearer.*x-api-key/],
      ['unsafe_protocol_header', /协议.*Header|protocol headers/],
    ]) {
      const key = `model_connections.${reason}`;
      await open(`kind=custom&lang=${lang}&check_error=${key}`, { width: 755, height: 600 });
      await field(lang === 'zh' ? '连接名称' : 'Connection name', 'HTTP fixture');
      await field('Base URL', 'http://gateway.invalid:8123/v1');
      await click('summary', lang === 'zh' ? '高级设置展开' : 'Advanced settingsExpand');
      await field(lang === 'zh' ? '认证方式' : 'Authentication', 'none');
      await click('.modal-foot button', lang === 'zh' ? '检查并继续' : 'Check and continue');
      await waitFor(client, `Boolean(document.querySelector('[data-error-code="${key}"]'))`);
      const error = await evaluate(client, `document.querySelector('[data-error-code="${key}"]').textContent`);
      assert.match(error, correction);
      assert.ok(!/暂时无法完成操作|The operation could not complete/.test(error), error);
      assert.equal(await evaluate(client, 'window.connectionFlowTrace.saves.length'), 0);
      const retained = await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(item => item.querySelector('.field-label')?.textContent === 'Base URL')?.querySelector('input').value`);
      assert.equal(retained, 'http://gateway.invalid:8123/v1');
      await screenshot(`target-error-${lang}-${reason}`);
    }
  });
  await run('cancel preserves the credential and ignores late check results', async () => {
    await open('hold_check=1'); await provider('Anthropic按量付费');
    await field('连接名称', '取消后继续'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', '检查并继续');
    await waitFor(client, 'window.connectionFlowTrace.checks.length === 1');
    await click('.modal-foot button', '取消检查');
    await waitFor(client, `document.querySelector('#mc-connection-form').getClientRects().length > 0`);
    await evaluate(client, 'window.finishConnectionFlowCheck()');
    await field('连接名称', '继续检查'); await click('.modal-foot button', '检查并继续');
    await waitFor(client, 'window.connectionFlowTrace.checks.length === 2');
    await evaluate(client, 'window.finishConnectionFlowCheck()');
    await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    assert.equal(await evaluate(client, `document.querySelector('.model-selection h3').textContent`), '继续检查');
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.registered.length'), 1);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.saves.length'), 0);
  });
  await run('close guards changed drafts and releases every unsaved product credential', async () => {
    await open('kind=custom'); await click('.modal-head button');
    await waitFor(client, `!document.querySelector('.modal') && !document.querySelector('.discard-dialog')`);
    await open(); await provider('Anthropic按量付费'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.modal-foot button', '返回'); await backToCatalog(); await provider('DeepSeek按量付费'); await field('API Key', 'fixture-key-b');
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog'))`);
    await click('.discard-dialog button', '继续编辑');
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.released.length'), 0);
    await click('.modal-head button'); await click('.discard-dialog button', '放弃修改');
    await waitFor(client, `!document.querySelector('.modal')`);
    assert.deepEqual((await evaluate(client, 'window.connectionFlowTrace.released')).sort(), ['input/1', 'input/2']);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.saves.length'), 0);
  });
  await run('existing names are distinguished before any credential or check request', async () => {
    await open('occupied=1');
    await click('.connection-provider-card', '百炼3 种接入方式');
    await click('.connection-product-card', '百炼 · Token Plan');
    await waitFor(client, `document.querySelector('#mc-connection-form').getClientRects().length > 0`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), '百炼 · Token Plan · 3');
    await field('连接名称', '百炼 · Token Plan'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', '检查并继续');
    await waitFor(client, `Boolean(document.querySelector('[data-error-code=CONNECTION_NAME_CONFLICT]'))`);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.registered.length'), 0);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.checks.length'), 0);
    assert.ok(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`));
  });
  await run('a selected model remains selected after an explicit tool check', async () => {
    await open(); await provider('Anthropic按量付费'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.model-selection input[type=checkbox]');
    await click('.model-selection button');
    await waitFor(client, `window.connectionFlowTrace.checks.length === 2 && Boolean(document.querySelector('.model-selection')) && !document.querySelector('.oc-spinner')`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.model-selection input:checked').length`), 1);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.registered.length'), 1);
    assert.equal(await evaluate(client, 'window.connectionFlowTrace.saves.length'), 0);
    assert.ok(await evaluate(client, 'window.connectionFlowTrace.checks[1].inference_model_id'));
  });
  await run('append keeps saved configuration locked and preserves disabled state', async () => {
    await open('append=1');
    assert.equal(await evaluate(client, 'document.querySelectorAll(".connection-provider-card").length'), 0);
    assert.ok(await evaluate(client, `Array.from(document.querySelectorAll('#mc-connection-form input, #mc-connection-form select')).every(item=>item.matches(':disabled'))`));
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), '已有 · 接入');
    await click('.modal-foot button', '检查并继续'); await waitFor(client, `Boolean(document.querySelector('.model-selection'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', '保存连接');
    await waitFor(client, `!document.querySelector('.modal')`);
    const trace = await evaluate(client, 'window.connectionFlowTrace');
    assert.equal(trace.registered.length, 0);
    assert.equal(trace.saves.length, 1);
    assert.equal(trace.saves[0].edit.action, 'append_models');
    assert.equal(trace.saves[0].intent, 'save_disabled');
    assert.deepEqual(trace.saves[0].key_edits, []);
    assert.equal(trace.saves[0].selected_model_refs.length, 1);
  });
  await run('maximum text size fits the minimum window in both languages', async () => {
    for (const lang of ['zh', 'en']) {
      await open(`lang=${lang}&scale=2`, { width: 755, height: 600 });
      await click('.connection-custom-entry');
      await field(lang === 'zh' ? '连接名称' : 'Connection name', '团队模型服务 Team model service — production');
      await click('.connection-details .disclosure-label', lang === 'zh' ? '高级设置' : 'Advanced settings');
      await click('.modal-foot button', lang === 'zh' ? '检查并继续' : 'Check and continue');
      await waitFor(client, `Boolean(document.querySelector('[data-error-code=MODEL_CONNECTION_FIELDS_REQUIRED]'))`);
      const rect = await evaluate(client, `(() => {const dialog=document.querySelector('.modal').getBoundingClientRect(),footer=document.querySelector('.modal-foot').getBoundingClientRect();return {left:dialog.left,right:dialog.right,bottom:footer.bottom,viewport:innerWidth,height:innerHeight,overflow:document.querySelector('.modal-body').scrollWidth-document.querySelector('.modal-body').clientWidth};})()`);
      assert.ok(rect.left >= 0 && rect.right <= rect.viewport + 1 && rect.bottom <= rect.height + 1 && rect.overflow <= 1, JSON.stringify(rect));
      await screenshot(`maximum-text-${lang}`);
    }
  });
  await run('dialog, footer and branding fit narrow layouts in both themes', async () => {
    for (const width of [320, 736, 1280]) for (const theme of ['light', 'dark']) {
      await open(`lang=en&theme=${theme}`, { width, height: 813 });
      const rect = await evaluate(client, `(() => {const dialog=document.querySelector('.modal').getBoundingClientRect(),footer=document.querySelector('.modal-foot').getBoundingClientRect();return {left:dialog.left,right:dialog.right,bottom:footer.bottom,viewport:innerWidth,height:innerHeight,overflow:document.querySelector('.modal-body').scrollWidth-document.querySelector('.modal-body').clientWidth};})()`);
      assert.ok(rect.left >= 0 && rect.right <= rect.viewport + 1 && rect.bottom <= rect.height + 1 && rect.overflow <= 1, JSON.stringify(rect));
      await screenshot(`catalog-${width}-${theme}`);
    }
  });
  await run('the model page failure capture follows the real add-API entry and retains its draft', async () => {
    await open('page=models&scenario=api_failure&capture=api_failure', { width: 1200, height: 813 }, 'index.html');
    await waitFor(client, `Boolean(document.querySelector('[data-error-code=MODEL_CONNECTION_AUTHENTICATION_REJECTED]'))`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), '百炼 · 按量付费');
    assert.ok(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').placeholder.includes('••')`));
    await screenshot('api-failure');
  });
  if (!results.length) throw new Error('No connection scenario was selected');
  const report = { evidence: 'Production React components and mock IPC; native effects are a separate acceptance gate', tests: results.length, passed: results.filter(item=>item.state==='green').length, results };
  await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  if (report.passed !== report.tests) process.exitCode = 1;
} finally { clearInterval(keepAlive); client.close(); }
