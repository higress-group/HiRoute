// Capture production UI in a disposable browser profile.
// Run from any directory after installing apps/desktop dependencies.
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { connectPage, evaluate, navigate, waitFor } from '../../desktop/tests/v3/visual/cdp-client.mjs';
import { agentGuideCaptureStates, openAgentGuideCapture } from './agent-guide-captures.mjs';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const desktop = path.resolve(website, '../desktop');
const output = path.join(website, 'public/guide-assets');
const generalStates = ['models-add', 'trial-entry', 'trial-setup', 'agent-editor', 'agent-management', 'routing-worker', 'agent-task', 'tasks'];
const captureStates = process.env.GUIDE_CAPTURE_STATE
  ? [...generalStates, ...agentGuideCaptureStates].filter(state => state === process.env.GUIDE_CAPTURE_STATE)
  : process.env.GUIDE_CAPTURE_GROUP === 'agents' ? agentGuideCaptureStates : generalStates;
if (!captureStates.length) throw new Error('Unknown GUIDE_CAPTURE_STATE');
const availablePort = () => new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once('error', reject);
  server.listen(0, '127.0.0.1', () => {
    const port = server.address().port;
    server.close(error => error ? reject(error) : resolve(port));
  });
});
async function ready(url) {
  for (let attempt = 0; attempt < 200; attempt++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Could not start ${url}`);
}
async function stop(child) {
  if (!child?.pid || child.exitCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve));
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 1500))]);
  if (child.exitCode === null) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
}
const httpPort = await availablePort();
let cdpPort = await availablePort();
while (cdpPort === httpPort) cdpPort = await availablePort();
const profile = await mkdtemp(path.join(os.homedir(), '.hiroute-guide-capture-'));
const base = `http://127.0.0.1:${httpPort}/`;
let vite, chrome, client;
try {
  await mkdir(output, { recursive: true });
  vite = spawn(path.join(desktop, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktop, detached: true, stdio: 'ignore' });
  await ready(base);
  chrome = spawn(process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome'), ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, '--window-size=1200,900', 'about:blank'], { detached: true, stdio: 'ignore' });
  await ready(`http://127.0.0.1:${cdpPort}/json/version`);
  client = await connectPage(cdpPort);
  const click = copy => evaluate(client, `(() => {
    const button = [...document.querySelectorAll('button,summary')].find(n => n.getClientRects().length &&
      (n.getAttribute('aria-label') === ${JSON.stringify(copy)} || n.textContent.trim() === ${JSON.stringify(copy)} || n.querySelector('.disclosure-label')?.textContent.trim() === ${JSON.stringify(copy)} || n.querySelector('strong')?.textContent.trim() === ${JSON.stringify(copy)}));
    if (!button || button.disabled) throw Error('Missing action: ' + ${JSON.stringify(copy)});
    button.click();
  })()`);
  for (const language of ['zh', 'en']) {
    const t = (zh, en) => language === 'zh' ? zh : en;
    for (const state of captureStates) {
      if (agentGuideCaptureStates.includes(state)) {
        await openAgentGuideCapture(client, base, language, state, click);
      } else if (state.startsWith('trial-')) {
        await navigate(client, `${base}new-model-trial.html?native=1&view=${state === 'trial-entry' ? 'home' : 'quickstart'}&lang=${language}&theme=dark`, { width: 1200, height: 900 });
        await waitFor(client, 'Boolean(document.querySelector(".new-models"))');
        await waitFor(client, `[...document.querySelectorAll('.new-models button')].some(n => !n.disabled && /^(试用|接入并试用|Try it|Try|Connect and try)$/.test(n.textContent.trim()))`);
        if (state === 'trial-setup') {
          await click(t('接入并试用', 'Connect and try'));
          await waitFor(client, 'Boolean(document.querySelector(".trial-setup"))');
        }
      } else {
        await navigate(client, `${base}product-shell.html`, { width: 1200, height: 900 });
        await waitFor(client, 'Boolean(window.productShell)');
        await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m => m.seedInteractionAudit(window.productShell, ${JSON.stringify({ language, theme: 'dark', state })}))`);
        if (state === 'agent-management') await evaluate(client, `(() => {
          for (const agent of window.productShell.agents.agents) if (agent.settings) {
            agent.settings.management_skill = { schema: 'hiroute.agent-management-skill-status/v1',
              state: 'not_configured', restore_point_ref: null, operation_id: null, operation_state: null };
          }
          window.productShell.handlers.cli_entry_status = () => ({ state: 'missing',
            command_resolves_to_app_cli: false, path_contains_entry_directory: false });
          window.productShell.remount();
        })()`);
        if (language === 'en') await evaluate(client, `(() => {
          const names = { '日常编码': 'Daily coding', '免费资料整理': 'Free research', '发布文案': 'Release copy', '代码实现': 'Code implementation' };
          for (const plan of window.productShell.desktop.catalog.plans) {
            if (names[plan.desired.display_name]) plan.desired.display_name = names[plan.desired.display_name];
            plan.desired.purpose = 'Complete well-scoped tasks and report the results';
          }
          window.productShell.remount();
        })()`);
        await waitFor(client, `Boolean(document.querySelector('.sidebar-health .good') && document.querySelector('.hr-ui')?.lang === ${JSON.stringify(language === 'zh' ? 'zh-CN' : 'en')})`);
        await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m => m.openInteractionAudit(${JSON.stringify(['agent-task', 'agent-management'].includes(state) ? 'agent' : state)}, ${JSON.stringify(language)}))`);
        if (state === 'agent-task') {
          await evaluate(client, 'document.querySelector("[data-agent-facet=collaboration]").click()');
          await waitFor(client, 'Boolean(document.querySelector("[role=dialog]"))');
        }
        if (state === 'agent-management') {
          await evaluate(client, 'document.querySelector("[data-agent-facet=management_skill]").click()');
          await waitFor(client, 'Boolean(document.querySelector("[role=dialog] [data-agent-skill-details] button") && document.querySelector("[role=dialog] button[type=submit]:not(:disabled)"))');
        }
        if (state === 'routing-worker') {
          await evaluate(client, `(() => { const disclosure = document.querySelector('.plan-more-settings'); if (!disclosure.open) disclosure.querySelector('summary').click(); })()`);
          await waitFor(client, 'Boolean(document.querySelector(".plan-more-settings[open]"))');
          await evaluate(client, `(() => { const toggle = document.querySelector('[data-route-group=executor] .option-row'); if (toggle.getAttribute('aria-pressed') !== 'true') toggle.click(); })()`);
          await waitFor(client, 'Boolean(document.querySelector("[data-route-group=executor] .option-row[aria-pressed=true]"))');
          await evaluate(client, `[...document.querySelectorAll('.v3-executor')].find(n => n.querySelector('strong')?.textContent === 'Codex CLI').click()`);
          await waitFor(client, `[...document.querySelectorAll('.v3-executor[aria-pressed=true]')].some(n => n.textContent.includes('Codex CLI'))`);
          await evaluate(client, 'document.querySelector("[data-route-group=executor]").scrollIntoView({block:"start"})');
        }
      }
      await evaluate(client, 'document.fonts.ready.then(() => Promise.all(document.getAnimations().filter(a => a.effect?.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))))');
      const shot = await client.send('Page.captureScreenshot', { format: 'webp', quality: 88, captureBeyondViewport: false });
      await writeFile(path.join(output, `${state}-${language}.webp`), Buffer.from(shot.data, 'base64'));
      process.stdout.write(`Captured ${state}-${language}\n`);
    }
  }
} finally {
  client?.close();
  await stop(chrome);
  await stop(vite);
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 150 });
}
