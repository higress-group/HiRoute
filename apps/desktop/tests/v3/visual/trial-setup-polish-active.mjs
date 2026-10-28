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
    assert.ok(await evaluate(client, `document.querySelector('.connection-flow').innerText.includes('当前没有可用的内置接入')`));
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
