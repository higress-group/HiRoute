import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000);
const results = [];
const settle = () => evaluate(client, 'new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))))');
async function open(page, query, width = 755, height = 600) {
  const url = `${base}${page}?${query}&run=${crypto.randomUUID()}`;
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url });
  await waitFor(client, `location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`);
  await evaluate(client, 'document.fonts.ready.then(() => true)');
  await waitFor(client, page.startsWith('connection') ? `Boolean(document.querySelector('.connection-flow'))` : page.startsWith('trial-explanation') ? `Boolean(document.querySelector('.trial-rule'))` : `Boolean(document.querySelector('.new-models'))`);
  await settle();
}
async function click(selector, text = '') {
  const expr = `Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(el => el.getClientRects().length && !el.disabled && (!${JSON.stringify(text)} || el.textContent.trim().includes(${JSON.stringify(text)})))`;
  await waitFor(client, `Boolean(${expr})`);
  await evaluate(client, `${expr}.click(); true`); await settle();
}
async function field(label, value) {
  const expr = `Array.from(document.querySelectorAll('.field')).find(el => el.getClientRects().length && el.querySelector('.field-label,.sr-only')?.textContent === ${JSON.stringify(label)})?.querySelector('input,select')`;
  await waitFor(client, `Boolean(${expr})`);
  await evaluate(client, `(() => {const el=${expr}; const proto=el instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(proto,'value').set.call(el,${JSON.stringify(value)});el.dispatchEvent(new Event(el instanceof HTMLSelectElement?'change':'input',{bubbles:true}));return true;})()`);
  await settle();
}
async function fieldAt(label, value, index) {
  const expr = `Array.from(document.querySelectorAll('.field')).filter(el => el.getClientRects().length && el.querySelector('.field-label,.sr-only')?.textContent === ${JSON.stringify(label)})[${index}]?.querySelector('input,select')`;
  await waitFor(client, `Boolean(${expr})`);
  await evaluate(client, `(() => {const el=${expr}; const proto=el instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(proto,'value').set.call(el,${JSON.stringify(value)});el.dispatchEvent(new Event(el instanceof HTMLSelectElement?'change':'input',{bubbles:true}));return true;})()`);
  await settle();
}
async function screenshot(name) {
  await settle(); const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(shot.data, 'base64'));
}
async function run(name, fn) {
  if (process.env.HIROUTE_TRIAL_SETUP_CASE && !process.env.HIROUTE_TRIAL_SETUP_CASE.split(',').includes(name)) return;
  try { await fn(); results.push({ name, state: 'green' }); }
  catch (error) { await screenshot(`failure-${name}`).catch(() => {}); results.push({ name, state: 'red', error: error.stack || String(error) }); }
}
try {
  await run('review-r6-protocol-auth-roundtrip', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('API Key', 'fixture-key-a'); await click('summary', 'Advanced settings');
    await click('button', 'Add protocol endpoint'); await fieldAt('Authentication', 'none', 1);
    await fieldAt('Protocol', 'responses', 0);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form input[type=password]'))`), false);
    await fieldAt('Protocol', 'chat_completions', 0);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true, 'switching primary endpoints retains the same-context Key');
  });
  await run('review-r6-protocol-immediate-close-and-switch', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-custom-entry');
    await field('API Key', 'fixture-key-a'); await click('summary', 'Advanced settings');
    await click('button', 'Add protocol endpoint'); await fieldAt('Authentication', 'none', 1);
    const switchAnd = selector => evaluate(client, `(() => {const p=Array.from(document.querySelectorAll('.field')).find(e=>e.getClientRects().length && e.querySelector('.field-label')?.textContent==='Protocol').querySelector('select');Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(p,'responses');p.dispatchEvent(new Event('change',{bubbles:true}));document.querySelector(${JSON.stringify(selector)}).click();return true;})()`);
    await switchAnd('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await fieldAt('Protocol', 'chat_completions', 0);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    await switchAnd('.modal-foot button'); await waitFor(client, `Boolean(document.querySelector('.connection-provider-card'))`);
    await click('.connection-provider-card', 'Anthropic');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '');
    await click('.modal-foot button', 'Back'); await click('.connection-custom-entry');
    await fieldAt('Protocol', 'chat_completions', 0);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
  });
  await run('review-r6-none-check-save-and-product-detour', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-custom-entry');
    await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
    await click('summary', 'Advanced settings'); await click('button', 'Add protocol endpoint');
    await fieldAt('Base URL', 'https://example.test/v1', 1); await fieldAt('Authentication', 'none', 1); await fieldAt('Protocol', 'responses', 0);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'Anthropic');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '', 'the hidden Key does not cross products');
    await click('.modal-foot button', 'Back'); await click('.connection-custom-entry');
    await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.deepEqual(await evaluate(client, `({registered:connectionFlowTrace.registered.length,checks:connectionFlowTrace.checks.length,usesKey:connectionFlowTrace.checks.some(x=>'input_candidate' in x || JSON.stringify(x).includes('fixture-key-a')),saves:connectionFlowTrace.saves.length})`), { registered: 0, checks: 1, usesKey: false, saves: 0 });
    await click('.discard-dialog button', 'Keep editing'); await click('.modal-foot button', 'Back');
    await fieldAt('Protocol', 'chat_completions', 0);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
  });
  await run('review-r6-protocol-releases-protected-only-once', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('Base URL', 'https://example.test/v1'); await click('summary', 'Advanced settings');
    await click('button', 'Add protocol endpoint'); await fieldAt('Authentication', 'none', 1);
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('#mc-manual-form:not([hidden])'))`);
    await click('.modal-foot button', 'Back'); await fieldAt('Protocol', 'responses', 0);
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1']);
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('Re-enter the previous key')`));
    await fieldAt('Protocol', 'chat_completions', 0);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Discard changes');
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1']);
  });
  await run('review-r6-none-save-confirms-without-key', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
    await click('summary', 'Advanced settings'); await click('button', 'Add protocol endpoint');
    await fieldAt('Base URL', 'https://example.test/v1', 1); await fieldAt('Authentication', 'none', 1); await fieldAt('Protocol', 'responses', 0);
    await click('.connection-manual-entry'); await field('Model ID', 'example-model'); await click('.modal-foot button', 'Check and save');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.saves.length`), 0, 'the unsaved Key is protected before save');
    await click('.discard-dialog button', 'Discard changes'); await waitFor(client, `!document.querySelector('.connection-flow')`);
    assert.deepEqual(await evaluate(client, `({registered:connectionFlowTrace.registered.length,checks:connectionFlowTrace.checks.length,saves:connectionFlowTrace.saves.length,applies:connectionFlowTrace.applies.length,hasInput:'input_candidate' in connectionFlowTrace.checks[0],usesKey:JSON.stringify([connectionFlowTrace.checks,connectionFlowTrace.saves,connectionFlowTrace.applies]).includes('fixture-key-a')})`), { registered: 0, checks: 1, saves: 1, applies: 1, hasInput: false, usesKey: false });
  });
  await run('review-r6-discard-layout-captures', async () => {
    for (const [lang, theme, width, height] of [['zh', 'dark', 755, 600], ['en', 'light', 755, 600], ['en', 'dark', 1200, 813], ['zh', 'light', 1200, 813]]) {
      await open('connection-flow.html', `lang=${lang}&theme=${theme}&kind=custom`, width, height);
      await field('API Key', 'fixture-key-a'); await click('summary', lang === 'zh' ? '高级设置' : 'Advanced settings');
      await click('button', lang === 'zh' ? '添加协议端点' : 'Add protocol endpoint');
      await fieldAt(lang === 'zh' ? '认证方式' : 'Authentication', 'none', 1);
      await fieldAt(lang === 'zh' ? '协议' : 'Protocol', 'responses', 0); await click('.modal-head button');
      await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
      const bounds = await evaluate(client, `(() => {const d=document.querySelector('.discard-dialog[open]');const r=d.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,overflow:d.scrollWidth-d.clientWidth};})()`);
      assert.ok(bounds.left >= 0 && bounds.top >= 0 && bounds.right <= width + 1 && bounds.bottom <= height + 1 && bounds.overflow <= 1, JSON.stringify(bounds));
      await screenshot(`review-r6-discard-${lang}-${theme}-${width}`);
    }
  });
  await run('review-r7-current-prefill-default-is-clean', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom&prefill_model=1');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'an untouched current prefill is its own baseline');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-r7-retained-prefill-default-is-clean', async () => {
    await open('connection-flow.html', 'lang=en&prefill_model=1'); await click('.connection-custom-entry');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.modal-foot button', 'Back'); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'an untouched retained product prefill is its own baseline');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-r7-save-another-product-with-clean-prefill', async () => {
    await open('connection-flow.html', 'lang=en&prefill_model=1'); await click('.connection-custom-entry');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'Anthropic');
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
    await settle();
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'saving another product does not discard a fictitious prefill edit');
    await waitFor(client, `!document.querySelector('.connection-flow')`);
  });
  await run('review-r7-prefill-detours-preserve-edits-and-reversion', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom'); await click('summary', 'Use connection prefill');
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    const initial = await evaluate(client, `({name:Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Connection name').querySelector('input').value,base:Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Base URL').querySelector('input').value})`);
    await field('Connection name', 'Edited account'); await field('Base URL', 'https://edited.example.test/v1'); await field('API Key', 'fixture-key-a');
    await field('Connection prefill (optional)', 'template:deepseek.official.global.v1');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '', 'a different prefill receives no Key');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    assert.equal(await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Connection name').querySelector('input').value`), 'Edited account');
    await field('Connection name', initial.name); await field('Base URL', initial.base); await field('API Key', '');
    await field('Connection prefill (optional)', 'template:deepseek.official.global.v1');
    assert.equal(await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Connection name').querySelector('input').value`), 'Edited account');
    await field('Connection name', initial.name); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await evaluate(client, `setConnectionLanguage('zh'); true`); await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner')`);
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'reverting to the first prefill baseline remains clean after a refresh');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-r7-prefill-does-not-absorb-carried-name', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    const initial = await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`);
    await field('Connection name', 'Edited before template'); await click('summary', 'Use connection prefill');
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Edited before template');
    await field('Connection prefill (optional)', ''); await field('Connection name', initial);
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Connection name', initial); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'reverting the carried edit in both contexts restores clean state');
  });
  await run('review-r7-deleted-prefill-model-is-dirty', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom&prefill_model=1'); await click('summary', 'Use connection prefill');
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1'); await click('.connection-manual-entry');
    assert.ok(await evaluate(client, `document.querySelector('#mc-manual-form').innerText.includes('fixture-prefill-model') || Array.from(document.querySelectorAll('#mc-manual-form input')).some(e=>e.value==='fixture-prefill-model')`));
    await click('#mc-manual-form button', 'Remove model'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-r7-prefill-capability-reverts-to-clean', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom&prefill_model=1'); await click('summary', 'Use connection prefill');
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1'); await click('.connection-manual-entry');
    await click('summary', 'Capabilities and limits');
    const initial = await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Image input').querySelector('select').value`);
    await field('Image input', initial === 'supported' ? 'unsupported' : 'supported'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Image input', initial); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-r7-prefill-model-selection-stays-dirty', async () => {
    await open('connection-flow.html', 'lang=en&options=one-noauth'); await click('.connection-custom-entry');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Back');
    await field('Connection prefill (optional)', ''); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.registered.length`), 0, 'the pending model selection is protected without a Key');
    await click('.discard-dialog button', 'Keep editing'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-r7-check-does-not-absorb-manual-model', async () => {
    await open('connection-flow.html', 'lang=en&options=one-noauth'); await click('.connection-custom-entry');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.connection-manual-entry'); await field('Model ID', 'qwen3-coder-plus'); await field('Model name (optional)', 'qwen3-coder-plus');
    await click('.modal-foot button', 'Back'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.model-selection button', 'Test tool calling');
    await waitFor(client, `connectionFlowTrace.checks.length === 2 && Boolean(document.querySelector('.model-selection input[type=checkbox]:checked'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-r7-check-retains-unknown-capability-edit', async () => {
    await open('connection-flow.html', 'lang=en&options=one-noauth&prefill_model=1'); await click('.connection-custom-entry');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await click('.connection-manual-entry'); await click('summary', 'Capabilities and limits'); await field('Image input', 'unknown');
    await click('.modal-foot button', 'Back'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.checks[0].draft.models[0].capabilities.vision.value`), false, 'the check receives the established fallback');
    await click('.modal-foot button', 'Back'); await click('.connection-manual-entry'); await click('summary', 'Capabilities and limits');
    assert.equal(await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Image input').querySelector('select').value`), 'unknown', 'checking does not erase the user edit');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-r7-edited-prefill-blocks-other-save', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-custom-entry'); await click('summary', 'Use connection prefill');
    await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1'); await field('Base URL', 'https://edited.example.test/v1');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'Anthropic');
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.saves.length + connectionFlowTrace.applies.length`), 0);
  });
  await run('review-r7-customize-retains-earlier-user-edit', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    const initial = await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`);
    await field('Connection name', 'Edited before customize'); await click('summary', 'Connection settings');
    await click('button', 'Customize connection settings'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Connection name', initial); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'the original product name remains the baseline after customizing');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-key-auth-toggle', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('API Key', 'fixture-key-a'); await click('summary', 'Advanced settings');
    await field('Authentication', 'none'); await field('Authentication', 'bearer');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]')?.value === 'fixture-key-a'`), true, 'authentication toggle retains the current product Key');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-current-option-qualification', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&no-custom=1');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('Connection name', 'Retained account'); await field('API Key', 'fixture-key-a');
    await evaluate(client, `setConnectionOptionMode('unavailable'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner') && Boolean(document.querySelector('.connection-config-heading'))`);
    assert.equal(await evaluate(client, `document.querySelector('.modal-foot button[type=submit]').disabled`), true, 'the same product is no longer executable');
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('当前选择的产品暂不可用')`));
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Retained account');
  });
  await run('review-current-option-submit', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a');
    await evaluate(client, `setConnectionOptionMode('unavailable'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner') && Boolean(document.querySelector('.connection-config-heading'))`);
    await evaluate(client, `document.querySelector('#mc-connection-form').requestSubmit(); true`); await settle();
    assert.equal(await evaluate(client, `connectionFlowTrace.registered.length`), 0, 'the handler rejects unavailable eligibility before credential registration');
    assert.equal(await evaluate(client, `connectionFlowTrace.checks.length`), 0);
  });
  await run('review-provider-detail-refresh', async () => {
    await open('connection-flow.html', 'lang=en&options=same-brand');
    await click('.connection-provider-card', 'Alibaba');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 2);
    await evaluate(client, `setConnectionOptionMode('one-bailian'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner') && Boolean(document.querySelector('.connection-directory'))`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 1, 'provider detail derives products from the current directory');
    assert.ok(await evaluate(client, `document.querySelector('.connection-product-heading h3').innerText.includes('百炼')`));
  });
  await run('review-stale-catalog-recovery', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await evaluate(client, `setConnectionOptionMode('hold-one'); true`);
    await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `pendingConnectionOptions() === 1`);
    await click('.modal-foot button', 'Cancel check'); await click('.modal-foot button', 'Change service');
    await evaluate(client, `setConnectionOptionMode('all'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `document.querySelectorAll('.connection-provider-card').length === 13`);
    await evaluate(client, `finishConnectionOptions(); true`); await settle();
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-provider-card').length`), 13, 'an older recovery read cannot replace the later catalog');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('[data-error-code="compute.registered_catalog_changed"]'))`), false, 'an old check cannot restore an obsolete error');
  });
  await run('review-pristine-automatic-model', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await click('.connection-manual-entry'); await click('#mc-manual-form button', 'Remove model');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'removing an untouched automatic placeholder is not an edit');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-placeholder-keeps-other-product-key-dirty', async () => {
    await open('connection-flow.html', 'lang=en');
    await click('.connection-provider-card', 'Anthropic'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', 'Back'); await click('.connection-custom-entry');
    await click('.connection-manual-entry'); await click('#mc-manual-form button', 'Remove model');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await click('.modal-foot button', 'Back');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'Anthropic');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true, 'another product remains dirty and retains its own Key');
  });
  await run('review-credential-none-and-product-isolation', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-custom-entry');
    await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
    await click('summary', 'Advanced settings'); await field('Authentication', 'none');
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `connectionFlowTrace.checks.length === 1`);
    const none = await evaluate(client, `({registered:connectionFlowTrace.registered.length, auth:connectionFlowTrace.checks[0].draft.authentication.kind, hasInput:'input_candidate' in connectionFlowTrace.checks[0], containsSecret:JSON.stringify(connectionFlowTrace.checks[0]).includes('fixture-key-a')})`);
    assert.deepEqual(none, { registered: 0, auth: 'none', hasInput: false, containsSecret: false });
    await waitFor(client, `Boolean(document.querySelector('#mc-manual-form') || document.querySelector('.model-selection'))`);
    await click('.modal-foot button', 'Back'); await click('.modal-foot button', 'Back');
    await click('.connection-provider-card', 'Anthropic');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '', 'a different product has no Key from the custom draft');
    await click('.modal-foot button', 'Back'); await click('.connection-custom-entry');
    await click('summary', 'Advanced settings'); await field('Authentication', 'api_key_header');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
  });
  await run('review-prefill-does-not-transfer-hidden-key', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('API Key', 'fixture-key-a'); await click('summary', 'Advanced settings'); await field('Authentication', 'none');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]')?.value ?? ''`), '', 'a Key retained for the prior custom setup is not copied into another product template');
    await field('Connection prefill (optional)', '');
    await field('Authentication', 'bearer');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]')?.value === 'fixture-key-a'`), true, 'returning to the original custom setup restores its own Key');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('review-none-save-protects-retained-key', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
    await click('summary', 'Advanced settings'); await field('Authentication', 'none');
    await click('.connection-manual-entry'); await field('Model ID', 'example-model');
    await click('.modal-foot button', 'Check and save'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.deepEqual(await evaluate(client, `({registered:connectionFlowTrace.registered.length,saves:connectionFlowTrace.saves.length,applies:connectionFlowTrace.applies.length,hasInput:'input_candidate' in connectionFlowTrace.checks[0]})`), { registered: 0, saves: 0, applies: 0, hasInput: false });
    await click('.discard-dialog button', 'Keep editing'); await click('.modal-foot button', 'Back');
    await click('summary', 'Advanced settings'); await field('Authentication', 'bearer');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
  });
  await run('review-prefill-browsing-is-not-edit', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await click('summary', 'Use connection prefill'); await field('Connection prefill (optional)', 'template:anthropic.platform.global.v1');
    await field('Connection prefill (optional)', ''); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'viewing and returning from an unedited template does not create a dirty draft');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-protected-key-reentry-and-release', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('#mc-manual-form') || document.querySelector('.model-selection'))`);
    await click('.modal-foot button', 'Back'); await click('summary', 'Advanced settings');
    await field('Authentication', 'none');
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1']);
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('Re-enter the previous key')`));
    await field('Authentication', 'api_key_header');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing');
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('Re-enter the previous key')`));
    await click('.modal-head button'); await click('.discard-dialog button', 'Discard changes');
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1'], 'the invalidated protected input is released once');
  });
  await run('review-product-eligibility-recovery', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&no-custom=1');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('Connection name', 'Retained account'); await field('API Key', 'fixture-key-a');
    for (const [mode, lang] of [['unavailable', 'zh'], ['none', 'en'], ['fail', 'zh'], ['one', 'en']]) {
      const before = await evaluate(client, `connectionFlowTrace.optionReads`);
      await evaluate(client, `setConnectionOptionMode(${JSON.stringify(mode)}); setConnectionLanguage(${JSON.stringify(lang)}); true`);
      await waitFor(client, `connectionFlowTrace.optionReads > ${before} && !document.querySelector('.connection-flow .oc-spinner')`);
      assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Retained account');
      assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
      assert.equal(await evaluate(client, `document.querySelector('.modal-foot button[type=submit]').disabled`), mode !== 'one', mode);
      if (mode === 'none') assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form .callout.warn').innerText.includes('advanced custom')`), false, 'do not offer a disabled custom entry');
      assert.equal(await evaluate(client, `connectionFlowTrace.registered.length + connectionFlowTrace.checks.length`), 0);
    }
  });
  await run('review-provider-current-products-and-restoration', async () => {
    await open('connection-flow.html', 'lang=en&options=same-brand'); await click('.connection-provider-card', 'Alibaba');
    const refresh = async (mode, lang) => {
      const before = await evaluate(client, `connectionFlowTrace.optionReads`);
      await evaluate(client, `setConnectionOptionMode(${JSON.stringify(mode)}); setConnectionLanguage(${JSON.stringify(lang)}); true`);
      await waitFor(client, `connectionFlowTrace.optionReads > ${before} && !document.querySelector('.connection-flow .oc-spinner')`);
    };
    await refresh('bailian-disabled', 'zh');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 2);
    assert.equal(await evaluate(client, `Array.from(document.querySelectorAll('.connection-product-card')).find(x=>x.textContent.includes('按量付费')).disabled`), true);
    await refresh('all', 'en');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 3, 'new product appears in the current provider');
    assert.ok(await evaluate(client, `document.querySelector('.connection-product-heading h3').innerText.includes('Alibaba')`));
    await refresh('none', 'zh');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 0);
    assert.ok(await evaluate(client, `document.querySelector('.connection-directory').innerText.includes('当前不可用')`));
    await refresh('same-brand', 'en');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 2, 'same provider detail recovers when it returns');
    await click('.connection-product-card', 'Pay as you go');
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `connectionFlowTrace.checks.length === 1`);
    assert.equal(await evaluate(client, `connectionFlowTrace.checks[0].connection_option_id`), 'bailian.payg.cn.v1');
  });
  await run('review-stale-recovery-failure-and-edit', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await evaluate(client, `setConnectionOptionMode('hold-fail'); true`);
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `pendingConnectionOptions() === 1`);
    await click('.modal-foot button', 'Cancel check');
    await field('Connection name', 'Edited after cancel');
    await evaluate(client, `setConnectionOptionMode('all'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner') && connectionFlowTrace.optionReads >= 3`);
    await evaluate(client, `finishConnectionOptions(); true`); await settle();
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Edited after cancel');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('[data-error-code="compute.registered_catalog_changed"]'))`), false);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('[data-error-code="MODEL_CONNECTION_OPTIONS_UNAVAILABLE"]'))`), false, 'late failure cannot replace the successful read');
    assert.equal(await evaluate(client, `connectionFlowTrace.released.length`), 0, 'the current editor still owns its protected input');
  });
  await run('review-newer-read-supersedes-pending-check', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&hold_check=1&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `connectionFlowTrace.registered.length === 1`);
    await evaluate(client, `setConnectionOptionMode('all'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `connectionFlowTrace.optionReads >= 2 && !document.querySelector('.connection-flow .oc-spinner')`);
    await evaluate(client, `finishConnectionFlowCheck(); true`); await settle();
    assert.equal(await evaluate(client, `pendingConnectionOptions()`), 0, 'a superseded check cannot begin another recovery read');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('[data-error-code="compute.registered_catalog_changed"]'))`), false);
    assert.equal(await evaluate(client, `connectionFlowTrace.released.length`), 0);
    assert.equal(await evaluate(client, `document.querySelector('.connection-config-heading h3').textContent.includes('Anthropic')`), true);
  });
  await run('review-new-read-supersedes-deferred-recovery', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('Connection name', 'Keep this name'); await field('API Key', 'fixture-key-a');
    await evaluate(client, `setConnectionOptionMode('hold-one'); true`); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `pendingConnectionOptions() === 1`);
    await evaluate(client, `setConnectionOptionMode('all'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `connectionFlowTrace.optionReads >= 3 && !document.querySelector('.connection-flow .oc-spinner')`);
    await evaluate(client, `finishConnectionOptions(); true`); await settle();
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Keep this name');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('[data-error-code="compute.registered_catalog_changed"]'))`), false);
    assert.equal(await evaluate(client, `connectionFlowTrace.released.length`), 0);
    await click('.modal-foot button', '更换服务');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-provider-card').length`), 13);
  });
  await run('review-current-catalog-recovery-and-retry', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await evaluate(client, `setConnectionOptionMode('hold-fail'); true`);
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `pendingConnectionOptions() === 1`);
    await evaluate(client, `finishConnectionOptions(); true`);
    await waitFor(client, `Boolean(document.querySelector('[data-error-code="compute.registered_catalog_changed"]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.checks.length`), 1);
    await evaluate(client, `setConnectionOptionMode('one'); setConnectionCheckError(null); true`);
    await click('.connection-flow button', 'Reload');
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner') && !document.querySelector('.modal-foot button[type=submit]').disabled`);
    assert.equal(await evaluate(client, `connectionFlowTrace.checks.length`), 1, 'reload itself does not check');
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `connectionFlowTrace.checks.length === 2`);
    assert.equal(await evaluate(client, `connectionFlowTrace.registered.length`), 1, 'retry reuses the same retained protected input');
  });
  await run('review-cancelled-check-does-not-release-owned-input', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&hold_check=1&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `connectionFlowTrace.registered.length === 1`); await click('.modal-foot button', 'Cancel check');
    await evaluate(client, `finishConnectionFlowCheck(); true`); await settle();
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), []);
    await click('.modal-head button'); await click('.discard-dialog button', 'Discard changes');
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1']);
  });
  await run('review-closed-recovery-releases-once', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=compute.registered_catalog_changed');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await evaluate(client, `setConnectionOptionMode('hold-one'); true`);
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `pendingConnectionOptions() === 1`);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Discard changes'); await waitFor(client, `!document.querySelector('.connection-flow')`);
    await evaluate(client, `finishConnectionOptions(); true`); await settle();
    assert.deepEqual(await evaluate(client, `connectionFlowTrace.released`), ['input/1']);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('review-repair-layout-captures', async () => {
    for (const [lang, theme, width, height] of [['en', 'light', 755, 600], ['zh', 'dark', 755, 600], ['en', 'dark', 1200, 813], ['zh', 'light', 1200, 813]]) {
      const other = lang === 'zh' ? 'en' : 'zh';
      await open('connection-flow.html', `lang=${other}&theme=${theme}&shortcut=1&options=one`, width, height);
      await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
      await field(other === 'zh' ? '连接名称' : 'Connection name', 'Review fixture'); await field('API Key', 'fixture-key-a');
      const before = await evaluate(client, `connectionFlowTrace.optionReads`);
      await evaluate(client, `setConnectionOptionMode('unavailable'); setConnectionLanguage(${JSON.stringify(lang)}); true`);
      await waitFor(client, `connectionFlowTrace.optionReads > ${before} && !document.querySelector('.connection-flow .oc-spinner')`);
      await evaluate(client, `document.querySelector('#mc-connection-form .callout.warn').scrollIntoView({block:'center'}); true`);
      const bounds = await evaluate(client, `(() => {const m=document.querySelector('.modal').getBoundingClientRect(), f=document.querySelector('.modal-foot').getBoundingClientRect(), w=document.querySelector('#mc-connection-form .callout.warn').getBoundingClientRect(), b=document.querySelector('.modal-body');return {left:m.left,right:m.right,footerBottom:f.bottom,warningTop:w.top,warningBottom:w.bottom,footerTop:f.top,overflow:b.scrollWidth-b.clientWidth};})()`);
      assert.ok(bounds.left >= 0 && bounds.right <= width + 1 && bounds.footerBottom <= height + 1 && bounds.warningTop >= 0 && bounds.warningBottom <= bounds.footerTop + 1 && bounds.overflow <= 1, JSON.stringify(bounds));
      await screenshot(`review-unavailable-${lang}-${theme}-${width}`);
      await open('connection-flow.html', `lang=${lang}&theme=${theme}&kind=custom`, width, height);
      await field('Base URL', 'https://example.test/v1'); await field('API Key', 'fixture-key-a');
      await click('.modal-foot button', lang === 'zh' ? '检查并继续' : 'Check and continue');
      await waitFor(client, `Boolean(document.querySelector('#mc-manual-form') || document.querySelector('.model-selection'))`);
      await click('.modal-foot button', lang === 'zh' ? '返回' : 'Back'); await click('summary', lang === 'zh' ? '高级设置' : 'Advanced settings');
      await field(lang === 'zh' ? '认证方式' : 'Authentication', 'none');
      await field(lang === 'zh' ? '认证方式' : 'Authentication', 'bearer');
      const note = `Array.from(document.querySelectorAll('.connection-flow [role=status]')).find(e=>e.textContent.includes(${JSON.stringify(lang === 'zh' ? '重新输入' : 'Re-enter')}))`;
      await waitFor(client, `Boolean(${note})`);
      const noteBounds = await evaluate(client, `(() => {const e=${note};e.scrollIntoView({block:'center'});const r=e.getBoundingClientRect(),f=document.querySelector('.modal-foot').getBoundingClientRect();return {top:r.top,bottom:r.bottom,footer:f.top};})()`);
      assert.ok(noteBounds.top >= 0 && noteBounds.bottom <= noteBounds.footer + 1, JSON.stringify(noteBounds));
      await screenshot(`review-reentry-${lang}-${theme}-${width}`);
    }
  });
  await run('review-model-placeholder-boundaries', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom'); await click('.connection-manual-entry');
    await click('#mc-manual-form button', 'Remove model'); await click('.modal-foot button', 'Back');
    await click('.connection-manual-entry'); await click('#mc-manual-form button', 'Remove model'); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false);
    await open('connection-flow.html', 'lang=en&kind=custom'); await click('.connection-manual-entry');
    await field('Model name (optional)', 'Partial name'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await open('connection-flow.html', 'lang=en&kind=custom'); await click('.connection-manual-entry');
    await click('summary', 'Capabilities and limits'); await field('Image input', 'supported'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await open('connection-flow.html', 'lang=en&kind=custom&initial_model=1'); await click('.connection-manual-entry');
    await click('#mc-manual-form button', 'Remove model'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await open('connection-flow.html', 'lang=en&kind=custom'); await field('Base URL', 'https://example.test/v1');
    await click('summary', 'Advanced settings'); await field('Authentication', 'none'); await click('.connection-manual-entry');
    await click('#mc-manual-form button', 'Remove model'); await click('.modal-foot button', 'Check and save');
    await waitFor(client, `Boolean(document.querySelector('[data-error-code="MODEL_ID_REQUIRED"]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.checks.length + connectionFlowTrace.applies.length`), 0);
  });
  await run('single-product-entry', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`), true, 'one scoped product opens its configuration');
  });
  await run('default-selection-is-not-an-edit', async () => {
    await open('connection-flow.html', 'lang=zh');
    await click('.connection-provider-card', 'Anthropic');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog[open]'))`), false, 'generated product defaults do not prompt');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('trial-explanation-is-progressive', async () => {
    await open('new-model-trial.html', 'lang=en&native=1&view=quickstart');
    await click('.new-models button', 'Try');
    await waitFor(client, `Boolean(document.querySelector('.trial-rule'))`);
    assert.ok(await evaluate(client, `Boolean(document.querySelector('.trial-rule details:not([open]) summary'))`), 'the complete rules have an initially closed disclosure');
  });
  await run('shortcut-scope-and-return', async () => {
    for (const allowCustom of [true, false]) {
      await open('connection-flow.html', `lang=en&shortcut=1&options=one${allowCustom ? '' : '&no-custom=1'}`);
      await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
      assert.match(await evaluate(client, `document.querySelector('.connection-config-heading h3').textContent`), /Anthropic/);
      assert.deepEqual(await evaluate(client, `({checks:connectionFlowTrace.checks.length, registered:connectionFlowTrace.registered.length, saves:connectionFlowTrace.saves.length, applies:connectionFlowTrace.applies.length})`), { checks: 0, registered: 0, saves: 0, applies: 0 });
      await click('.modal-foot button', 'Change service');
      assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-custom-entry'))`), allowCustom);
      await evaluate(client, `setConnectionLanguage('zh'); true`);
      await waitFor(client, `Boolean(document.querySelector('.connection-provider-card'))`);
      assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`), false, 'return is not overridden by a language refresh');
    }
    await open('connection-flow.html', 'lang=en&options=one');
    await waitFor(client, `Boolean(document.querySelector('.connection-provider-card'))`);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`), false, 'ordinary entry keeps the directory');
    await open('connection-flow.html', 'lang=en&shortcut=1&options=same-brand');
    await field('Search providers or products', 'Alibaba');
    await evaluate(client, `setConnectionOptionMode('one'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `Boolean(document.querySelector('.connection-directory input[type=search]')) && !document.querySelector('.connection-flow .oc-spinner')`);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`), false, 'a search is not overridden by a later one-product refresh');
    assert.equal(await evaluate(client, `document.querySelector('input[type=search]').value`), 'Alibaba');
    await open('connection-flow.html', 'lang=en&shortcut=1&options=same-brand');
    await click('.connection-provider-card', 'Alibaba');
    assert.equal(await evaluate(client, `document.querySelectorAll('.connection-product-card').length`), 2, 'same brand still has two products');
    for (const option of ['none', 'fail', 'unavailable']) {
      await open('connection-flow.html', `lang=en&shortcut=1&options=${option}`);
      await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner')`);
      assert.equal(await evaluate(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`), false, option);
    }
  });
  await run('async-options-retain-edited-product', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('Connection name', 'My edited connection'); await field('API Key', 'fixture-key-a');
    await evaluate(client, `setConnectionOptionMode('none'); setConnectionLanguage('zh'); true`);
    await waitFor(client, `Boolean(document.querySelector('.connection-config-heading')) && !document.querySelector('.connection-flow .oc-spinner')`);
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'My edited connection');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('当前选择的产品暂不可用')`));
    assert.equal(await evaluate(client, `connectionFlowTrace.checks.length`), 0);
  });
  await run('browse-and-revert-without-discard', async () => {
    await open('connection-flow.html', 'lang=zh');
    await click('.connection-provider-card', 'Anthropic');
    await click('.modal-foot button', '返回'); await click('.connection-provider-card', 'DeepSeek');
    await click('.modal-foot button', '返回');
    await evaluate(client, `document.querySelector('.connection-directory input[type=search]').focus(); true`);
    for (const type of ['keyDown', 'keyUp']) await client.send('Input.dispatchKeyEvent', { type, key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    await waitFor(client, `!document.querySelector('.connection-flow')`);
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false);
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    const initial = await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`);
    await field('Connection name', 'A real edit'); await field('Connection name', initial);
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'reverting to the per-product baseline clears the prompt');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.connection-flow'))`), false);
  });
  await run('key-and-stashed-edits-are-protected', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-provider-card', 'Anthropic');
    await field('Connection name', 'Account A'); await field('API Key', 'fixture-key-a');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'DeepSeek');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value`), '', 'a Key does not cross products');
    await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').focus(); true`);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `document.querySelectorAll('.discard-dialog[open]').length`), 1);
    await click('.discard-dialog button', 'Keep editing');
    assert.equal(await evaluate(client, `document.activeElement === document.querySelector('#mc-connection-form input:not([type=password])')`), true, 'continuing restores focus');
    await click('.modal-foot button', 'Back'); await click('.connection-provider-card', 'Anthropic');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input:not([type=password])').value`), 'Account A');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    await click('.modal-head button'); await click('.discard-dialog button', 'Discard changes');
    await waitFor(client, `!document.querySelector('.connection-flow')`);
  });
  await run('current-password-and-escape', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a');
    await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').focus(); true`);
    for (const type of ['keyDown', 'keyUp']) await client.send('Input.dispatchKeyEvent', { type, key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing');
    assert.equal(await evaluate(client, `document.querySelector('#mc-connection-form input[type=password]').value === 'fixture-key-a'`), true);
    await field('API Key', ''); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'clearing the only temporary key returns to baseline');
  });
  await run('custom-fields-and-model-declaration', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await field('Base URL', 'https://example.test/v1');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Base URL', '');
    await click('.connection-manual-entry');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'opening an empty manual row is a generated default');
    await open('connection-flow.html', 'lang=en&kind=custom');
    await click('.connection-manual-entry'); await field('Model ID', 'example-model');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Model ID', '');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'a reverted model declaration does not remain touched');
    await open('connection-flow.html', 'lang=en&kind=custom');
    await click('summary', 'Advanced settings'); await field('Authentication', 'none');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
  });
  await run('model-selection-and-failed-check', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one-noauth');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-head button');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await click('.model-selection input[type=checkbox]');
    await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'deselection returns to the initial selection');
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&check_error=model_connections.invalid_url');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('[data-error-code]'))`);
    const failure = await evaluate(client, `document.querySelector('[data-error-code]').getAttribute('data-error-code')`);
    await evaluate(client, `setConnectionLanguage('zh'); true`);
    await waitFor(client, `!document.querySelector('.connection-flow .oc-spinner')`);
    assert.equal(await evaluate(client, `document.querySelector('[data-error-code]')?.getAttribute('data-error-code')`), failure, 'language refresh preserves the original check error');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.released.length`), 0, 'failed check retains the protected input until discard');
  });
  await run('selection-survives-product-detour', async () => {
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one-noauth');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]');
    const selectedName = await evaluate(client, `document.querySelector('.model-result-row:has(input:checked) strong').textContent`);
    await click('.modal-foot button', 'Back'); await click('.modal-foot button', 'Change service');
    await click('.connection-provider-card', 'Anthropic'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:checked'))`);
    assert.equal(await evaluate(client, `document.querySelector('.model-result-row:has(input:checked) strong').textContent`), selectedName);
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    assert.equal(await evaluate(client, `connectionFlowTrace.applies.length`), 0);
  });
  await run('model-capability-revert', async () => {
    await open('connection-flow.html', 'lang=en&kind=custom');
    await click('.connection-manual-entry'); await click('summary', 'Capabilities and limits');
    const initial = await evaluate(client, `Array.from(document.querySelectorAll('.field')).find(e=>e.querySelector('.field-label')?.textContent==='Image input').querySelector('select').value`);
    await field('Image input', initial === 'supported' ? 'unsupported' : 'supported');
    await click('.modal-head button'); await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing'); await field('Image input', initial); await click('.modal-head button');
    assert.equal(await evaluate(client, `Boolean(document.querySelector('.discard-dialog'))`), false, 'returning capability to its baseline does not retain a touched flag');
  });
  await run('saving-protects-other-product-drafts', async () => {
    await open('connection-flow.html', 'lang=en'); await click('.connection-provider-card', 'Anthropic');
    await field('Connection name', 'Unfinished A'); await click('.modal-foot button', 'Back');
    await click('.connection-provider-card', 'DeepSeek'); await field('API Key', 'fixture-key-b');
    await click('.modal-foot button', 'Check and continue'); await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
    await waitFor(client, `Boolean(document.querySelector('.discard-dialog[open]'))`);
    await click('.discard-dialog button', 'Keep editing');
    assert.equal(await evaluate(client, `connectionFlowTrace.applies.length`), 0);
    await click('.modal-foot button', 'Save connection'); await click('.discard-dialog button', 'Discard changes');
    await waitFor(client, `!document.querySelector('.connection-flow')`);
    assert.equal(await evaluate(client, `connectionFlowTrace.applies.length`), 1);
  });

  await run('save-operation-ownership', async () => {
    for (const mode of ['pending', 'uncertain', 'saved']) {
      await open('connection-flow.html', `lang=en&shortcut=1&options=one&save_mode=${mode}`);
      await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
      await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
      await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
      await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
      await waitFor(client, `!document.querySelector('.connection-flow')`);
      const outcome = await evaluate(client, `({applies:connectionFlowTrace.applies.length,accepted:connectionFlowTrace.accepted.length,uncertain:connectionFlowTrace.uncertain.length,released:connectionFlowTrace.released.length})`);
      assert.equal(outcome.applies, 1); assert.equal(outcome.released, 0, 'Operation-owned input is not released by the editor');
      assert.equal(outcome.uncertain, mode === 'saved' ? 0 : 1);
      if (mode === 'pending') assert.equal(outcome.accepted, 1, 'pending keeps the admitted Operation');
    }
    await open('connection-flow.html', 'lang=en&shortcut=1&options=one&hold_apply=1');
    await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
    await field('API Key', 'fixture-key-a'); await click('.modal-foot button', 'Check and continue');
    await waitFor(client, `Boolean(document.querySelector('.model-selection input[type=checkbox]:not(:disabled)'))`);
    await click('.model-selection input[type=checkbox]'); await click('.modal-foot button', 'Save connection');
    await waitFor(client, `connectionFlowTrace.applies.length === 1`);
    assert.equal(await evaluate(client, `requestConnectionReplacement()`), false, 'an in-flight save cannot be represented as a cancelled close');
    assert.equal(await evaluate(client, `document.querySelector('.modal-head button').disabled`), true);
    await evaluate(client, `finishConnectionFlowApply(); true`); await waitFor(client, `!document.querySelector('.connection-flow')`);
    assert.equal(await evaluate(client, `connectionFlowTrace.applies.length`), 1);
  });
  await run('classifier-copy-and-keyboard', async () => {
    const expected = {
      en: { native: ['trial model handles suitable tasks', 'earlier problem still matters', 'cannot be checked', 'fixed or unrelated', 'previous Primary stage'], local: ['Current task signals', 'do not assess the previous stage'], custom: ['custom extension chooses', 'Optional scores are observation only'] },
      zh: { native: ['新模型先处理适合它的任务', '问题仍相关', '无法判断', '问题已修正或无关', '上一阶段已使用主力'], local: ['当前任务信号', '不评估上一阶段'], custom: ['自定义扩展选择', '可选评分仅用于观察'] },
    };
    const summaries = {
      en: { native: 'The trial model handles suitable tasks; Primary handles complex tasks. When low-score protection applies, this turn uses Primary.', local: 'Current task signals choose between the trial model and Primary. This mode does not apply low-score protection.', custom: 'The custom extension chooses the trial model or Primary. HiRoute adds no trust threshold or low-score protection.' },
      zh: { native: '新模型先处理适合它的任务，主力承担复杂任务。需要低分保护时，本轮使用主力。', local: '按当前任务信号在新模型和主力之间选择；此模式不启用低分保护。', custom: '由自定义扩展选择新模型或主力；HiRoute 不额外应用信任阈值或低分保护。' },
    };
    for (const lang of ['en', 'zh']) for (const mode of ['native', 'local', 'custom']) {
      await open('trial-explanation.html', `lang=${lang}`);
      await evaluate(client, `setTrialExplanationMode(${JSON.stringify(mode)}); true`); await settle();
      const collapsed = await evaluate(client, `document.querySelector('.trial-rule').innerText`);
      assert.ok(collapsed.includes(expected[lang][mode][0]), collapsed);
      assert.equal(await evaluate(client, `document.querySelector('.trial-rule-content > p').textContent`), summaries[lang][mode]);
      assert.equal(await evaluate(client, `document.querySelector('.trial-rule details').open`), false);
      assert.equal(await evaluate(client, `document.querySelectorAll('.trial-rule .disclosure-body a,.trial-rule .disclosure-body button,.trial-rule .disclosure-body input,.trial-rule .disclosure-body [tabindex]').length`), 0, 'the closed explanation has no duplicate focusable controls');
      assert.equal(collapsed.includes(expected[lang][mode][1]), false, 'the full explanation is not presented while closed');
      await evaluate(client, `document.querySelector('.trial-rule summary').focus(); true`);
      await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' });
      await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
      await waitFor(client, `document.querySelector('.trial-rule details').open`);
      const expanded = await evaluate(client, `document.querySelector('.trial-rule').innerText`);
      for (const phrase of expected[lang][mode]) assert.ok(expanded.includes(phrase), `${mode}/${lang}: ${phrase}: ${expanded}`);
      await evaluate(client, `setTrialExplanationLanguage(${JSON.stringify(lang === 'zh' ? 'en' : 'zh')}); true`); await settle();
      assert.equal(await evaluate(client, `document.querySelector('.trial-card input').value`), 'retain me');
      assert.equal(await evaluate(client, `document.querySelector('.trial-rule details').open`), true);
      await evaluate(client, `setTrialExplanationMode('local'); true`); await settle();
      assert.equal(await evaluate(client, `document.querySelector('.trial-rule-content > p').textContent`), summaries[lang === 'zh' ? 'en' : 'zh'].local);
      assert.equal(await evaluate(client, `document.querySelector('.trial-card input').value`), 'retain me');
    }
  });
  await run('production-layout-captures', async () => {
    for (const lang of ['en', 'zh']) for (const theme of ['light', 'dark']) for (const [width, height] of [[755, 600], [1200, 813]]) {
      await open('new-model-trial.html', `lang=${lang}&native=1&view=quickstart&theme=${theme}`, width, height);
      await click('.new-models button', lang === 'zh' ? '试用' : 'Try');
      await waitFor(client, `Boolean(document.querySelector('.trial-rule details'))`);
      const geometry = await evaluate(client, `(() => {const nodes=[...document.querySelectorAll('.trial-card,.trial-rule')].filter(e=>e.getClientRects().length); return nodes.map(e=>({width:e.clientWidth,scroll:e.scrollWidth,left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right}));})()`);
      assert.ok(geometry.length && geometry.every(g => g.scroll <= g.width + 1 && g.left >= 0 && g.right <= width + 1), JSON.stringify(geometry));
      const cards = await evaluate(client, `[...document.querySelectorAll('.trial-model-card')].map(e=>{const r=e.getBoundingClientRect();return {left:r.left,top:r.top,width:r.width,height:r.height};})`);
      assert.ok(cards.length >= 3 && cards.slice(0, 3).every(r => r.width > 150 && r.height > 150 && Math.abs(r.top - cards[0].top) < 1), JSON.stringify(cards));
      await screenshot(`trial-${lang}-${theme}-${width}`);
      await evaluate(client, `document.querySelector('.trial-rule').scrollIntoView({block:'center'}); true`);
      await screenshot(`trial-rule-collapsed-${lang}-${theme}-${width}`);
      await click('.trial-rule summary');
      await evaluate(client, `document.querySelector('.trial-rule .disclosure-body p').scrollIntoView({block:'center'}); true`);
      const detailBounds = await evaluate(client, `(() => {const p=document.querySelector('.trial-rule .disclosure-body p').getBoundingClientRect(), f=document.querySelector('.trial-footer').getBoundingClientRect();return {top:p.top,bottom:p.bottom,footer:f.top};})()`);
      assert.ok(detailBounds.top >= 0 && detailBounds.bottom <= detailBounds.footer + 1, JSON.stringify(detailBounds));
      await screenshot(`trial-rule-expanded-${lang}-${theme}-${width}`);
      await open('connection-flow.html', `lang=${lang}&theme=${theme}&shortcut=1&options=one`, width, height);
      await waitFor(client, `Boolean(document.querySelector('#mc-connection-form:not([hidden])'))`);
      const bounds = await evaluate(client, `(() => {const body=document.querySelector('.modal-body'), r=document.querySelector('.modal').getBoundingClientRect(), f=document.querySelector('.modal-foot').getBoundingClientRect();return {overflow:body.scrollWidth-body.clientWidth,left:r.left,right:r.right,bottom:f.bottom};})()`);
      assert.ok(bounds.overflow <= 1 && bounds.left >= 0 && bounds.right <= width + 1 && bounds.bottom <= height + 1, JSON.stringify(bounds));
      await screenshot(`single-product-${lang}-${theme}-${width}`);
    }
  });
  if (!results.length) throw new Error('No trial setup scenario was selected');
  const report = { evidence: 'Production React components and styles with controlled IPC; native effects are separate', tests: results.length, passed: results.filter(r => r.state === 'green').length, results };
  await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  if (report.passed !== report.tests) process.exitCode = 1;
} finally { clearInterval(keepAlive); client.close(); }
