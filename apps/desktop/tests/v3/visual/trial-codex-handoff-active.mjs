import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const keepAlive = setInterval(() => {}, 1000), results = [];
const baseline = process.env.HIROUTE_CODEX_HANDOFF_BASELINE === '1';
const text = (lang, zh, en) => lang === 'zh' ? zh : en;
const targetContext = mode => mode === 'desktop' ? 'agent-context/codex-desktop/trial' : `agent-context/codex/${mode}`;
const count = command => evaluate(client, `trialTrace.filter(x=>x.command===${JSON.stringify(command)}).length`);
const settle = () => evaluate(client, 'new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))');
async function click(selector, label) {
 const find = `(()=>[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&e.textContent.includes(${JSON.stringify(label)})))()`;
 await waitFor(client, `!!${find}`);
 await evaluate(client, `${find}.click();true`);
}
async function shot(name, selector = '.trial-mapping') {
 await evaluate(client, `document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'start'});true`);
 await settle();
 const image = await client.send('Page.captureScreenshot', { format: 'png' });
 await writeFile(path.join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
}
async function start(mode = 'profile', lang = 'en', theme = 'light', width = 755) {
 await client.send('Page.enable'); await client.send('Runtime.enable');
 await client.send('Emulation.setDeviceMetricsOverride', { width, height: width === 755 ? 600 : 813, deviceScaleFactor: 1, mobile: false });
 const url = `${base}new-model-trial.html?native=1&view=quickstart&agent-family=${mode === 'desktop' ? 'codex-desktop' : 'codex'}&codex-mode=${mode}&lang=${lang}&theme=${theme}`;
 await client.send('Page.navigate', { url });
 await waitFor(client, `location.href===${JSON.stringify(url)} && !!document.querySelector('.new-models')`);
 await evaluate(client, 'document.fonts.ready.then(()=>true)');
 await click('.new-models button', text(lang, '试用', 'Try'));
 await waitFor(client, `!!document.querySelector('.trial-presets input:not(:disabled)')`);
 await evaluate(client, `(()=>{const e=document.querySelector('.trial-agent-choice select');e.value=${JSON.stringify(mode === 'desktop' ? 'agent_codex_desktop' : 'agent_codex_default')};e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
 await waitFor(client, `!document.querySelector('.trial-presets') && !document.querySelector('.trial-agent-choice [role=alert]')`);
 await evaluate(client, `[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name)).click();true`);
 await waitFor(client, `!!document.querySelector('.trial-footer .btn-primary:not(:disabled)')`);
 return { mode, lang, theme, width, ordinaryCodex: await evaluate(client, 'JSON.stringify(trialAgentControl.codexState())') };
}
async function publishAndMap({ mode, lang, ordinaryCodex }) {
 await click('.trial-footer button', text(lang, '检查改动', 'Review changes'));
 await click('.trial-review > .actions button', text(lang, '保存并发布路由', 'Save and publish route'));
 await waitFor(client, `!!document.querySelector('.trial-mapping .trial-agent-choice')`);
 const discoveries = await count('trial_agent_choices');
 await click('.trial-mapping > .actions button', text(lang, '检查并映射 Agent', 'Review and map Agent'));
 await waitFor(client, `document.querySelector('.trial-mapping .callout.good')?.textContent.includes(${JSON.stringify(text(lang, '映射已确认', 'Mapping confirmed'))})`);
 assert.equal(await count('preview_agent_settings'), 1, 'one mapping submission');
 assert.equal(await evaluate(client, `trialAgentControl.mappedContexts[0]`), targetContext(mode), 'preserve the selected connection');
 if (mode === 'desktop') assert.equal(await evaluate(client, 'JSON.stringify(trialAgentControl.codexState())'), ordinaryCodex, 'Desktop mapping must preserve the ordinary CLI connection');
 assert.equal(await count('trial_agent_choices'), discoveries, 'mapping does not rediscover unrelated Agents');
 return discoveries;
}
async function commandReady() { await waitFor(client, `!!document.querySelector('.trial-codex-handoff .agent-launch-command')`); }
async function unconfirmed() { await waitFor(client, `document.querySelector('.trial-codex-handoff')?.textContent.includes('unconfirmed')`); }
async function assertNoMutation() { assert.equal(await count('preview_agent_settings'), 1, 'read recovery must not remap'); }

try {
 for (const mode of ['profile', 'root', 'desktop']) for (const lang of ['en', 'zh']) for (const theme of ['light', 'dark']) for (const width of [755, 1200]) {
  const state = await start(mode, lang, theme, width);
  if (!baseline) assert.match(await evaluate(client, `document.querySelector('.trial-agent-choice').textContent`), mode === 'desktop' ? /独立.*Desktop|independent.*Desktop/i : mode === 'profile' ? /HiRoute profile/ : /默认接入|default connection/);
  await shot(`codex-${mode}-choice-${lang}-${theme}-${width}`, '.trial-agent-choice');
  await publishAndMap(state);
  if (!baseline) {
   await waitFor(client, `!!document.querySelector('.trial-codex-handoff ${mode === 'desktop' ? '[data-trial-codex-desktop-start]' : '[data-codex-history-guidance]'}')`);
   assert.equal(await count('agent_snapshot'), 1);
   assert.deepEqual(await evaluate(client, `trialTrace.find(x=>x.command==='agent_snapshot').payload`), { contextId: targetContext(mode), nativeTargets: [] });
   if (mode === 'desktop') {
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff [data-codex-launch-instructions], .trial-codex-handoff [data-codex-history-guidance]')`), false, 'Desktop must not show CLI profile or root takeover guidance');
    await click('.trial-mapping > .actions button', text(lang, '查看 Agent 配置', 'Inspect Agent settings'));
    assert.deepEqual(await evaluate(client, 'trialAgentControl.openedAgents'), ['agent_codex_desktop'], 'handoff must open the selected independent instance');
    assert.equal(await count('launch_codex_desktop'), 0, 'mapping does not launch another app');
    assert.equal(await count('shell_launch_codex_desktop'), 0);
   } else if (mode === 'profile') {
    const expected = await evaluate(client, `document.querySelector('.trial-codex-handoff code').textContent`);
    assert.match(expected, /--profile trial-fixture/, 'display the snapshot command rather than a guessed profile');
    await evaluate(client, `Object.defineProperty(window,'isTauri',{configurable:true,value:true});true`);
    await click('.trial-codex-handoff button', text(lang, '复制启动命令', 'Copy launch command'));
    await waitFor(client, `document.querySelector('.trial-codex-handoff [role=status]')?.textContent.includes(${JSON.stringify(text(lang, '已复制', 'Copied'))})`);
    assert.equal(await evaluate(client, `trialTrace.find(x=>x.command==='plugin:clipboard-manager|write_text').payload.text`), expected, 'copy exactly the displayed command');
   } else {
    assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff [data-codex-launch-instructions]')`), false);
    assert.match(await evaluate(client, `document.querySelector('.trial-codex-handoff').textContent`), /重新启动|Restart/);
   }
   const geometry = await evaluate(client, `(()=>{const e=document.querySelector('.trial-codex-handoff');return {width:e.clientWidth,scroll:e.scrollWidth,overflow:document.documentElement.scrollWidth>innerWidth+1};})()`);
   assert.ok(!geometry.overflow && geometry.scroll <= geometry.width + 1, 'launch guidance fits the supported window');
  }
  await shot(`codex-${mode}-mapped-${lang}-${theme}-${width}`);
  results.push({ id: `trial.codex.${mode}.${lang}.${theme}.${width}`, status: 'passed', baseline });
 }
 if (!baseline) {
  let state = await start();
  await evaluate(client, 'trialAgentControl.failNextLaunchRead=true;true');
  const discoveries = await publishAndMap(state);
  await unconfirmed();
  assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff .agent-launch-command')`), false);
  await click('.trial-codex-handoff button', 'Reload launch instructions');
  await commandReady();
  assert.equal(await count('trial_agent_choices'), discoveries);
  await assertNoMutation();
  results.push({ id: 'trial.codex.failed-launch-read-retry-is-read-only', status: 'passed' });

  // A stale native target must disappear immediately on explicit refresh.
  await evaluate(client, `trialAgentControl.setCodexMode('root');trialAgentControl.holdNextLaunchRead=true;true`);
  await click('.trial-mapping > .actions button', 'Refresh result');
  await waitFor(client, 'trialAgentControl.pendingLaunchReads.length===1');
  assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff .agent-launch-command')`), false);
  await evaluate(client, 'trialAgentControl.pendingLaunchReads.shift()();true');
  await unconfirmed();
  await assertNoMutation();
  results.push({ id: 'trial.codex.refresh-retires-command-after-external-mode-change', status: 'passed' });

  state = await start();
  await evaluate(client, 'trialAgentControl.holdNextLaunchRead=true;true');
  await publishAndMap(state);
  await waitFor(client, 'trialAgentControl.pendingLaunchReads.length===1');
  await evaluate(client, 'setTrialMutable(false);true'); await settle();
  await evaluate(client, 'trialAgentControl.pendingLaunchReads.shift()();true'); await settle();
  assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff .agent-launch-command')`), false, 'a late reply cannot restore instructions after authority loss');
  await evaluate(client, 'setTrialMutable(true);true'); await commandReady();
  await evaluate(client, 'trialAgentControl.setCodexRevoked(true);true');
  await click('.trial-mapping > .actions button', 'Refresh result');
  await unconfirmed();
  assert.equal(await evaluate(client, `!!document.querySelector('.trial-codex-handoff .agent-launch-command')`), false, 'revoked access has no launch command');
  await assertNoMutation();
  results.push({ id: 'trial.codex.late-read-and-revocation-do-not-show-launch-command', status: 'passed' });

  state = await start('desktop');
  await evaluate(client, 'trialAgentControl.holdNextLaunchRead=true;true');
  await publishAndMap(state);
  await waitFor(client, 'trialAgentControl.pendingLaunchReads.length===1');
  await evaluate(client, 'setTrialMutable(false);true'); await settle();
  await evaluate(client, 'trialAgentControl.pendingLaunchReads.shift()();true'); await settle();
  assert.equal(await evaluate(client, `!!document.querySelector('[data-trial-codex-desktop-start]')`), false, 'late Desktop read cannot restore guidance after authority loss');
  await evaluate(client, 'setTrialMutable(true);true');
  await waitFor(client, `!!document.querySelector('[data-trial-codex-desktop-start]')`);
  await evaluate(client, 'trialAgentControl.invalidateDesktop();true');
  await click('.trial-mapping > .actions button', 'Refresh result');
  await unconfirmed();
  assert.equal(await evaluate(client, `!!document.querySelector('[data-trial-codex-desktop-start]')`), false, 'mismatched Desktop model context must retire confirmed guidance');
  await assertNoMutation();
  results.push({ id: 'trial.codex.desktop.late-read-and-context-change-retire-guidance', status: 'passed' });

  state = await start('desktop');
  await evaluate(client, 'trialAgentControl.untrustedNextLaunchRead=true;true');
  await publishAndMap(state); await unconfirmed();
  assert.equal(await evaluate(client, `!!document.querySelector('[data-trial-codex-desktop-start]')`), false, 'untrusted Desktop snapshot cannot certify the handoff');
  await click('.trial-codex-handoff button', 'Reload launch instructions');
  await waitFor(client, `!!document.querySelector('[data-trial-codex-desktop-start]')`);
  await assertNoMutation();
  results.push({ id: 'trial.codex.desktop.untrusted-handoff-retry-is-read-only', status: 'passed' });
 }
} catch (error) {
 results.push({ id: 'trial.codex.unexpected', status: 'failed', error: String(error), page: await evaluate(client, 'document.body.innerText') });
 await shot('failure', '.trial-page'); process.exitCode = 1;
} finally {
 await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2));
 console.log(JSON.stringify({ cases: results.length, failed: results.filter(item => item.status !== 'passed').length, baseline }));
 clearInterval(keepAlive); client.close();
}
