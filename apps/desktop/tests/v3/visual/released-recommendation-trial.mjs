// Actual installed console and public backend; no IPC handlers or production-source overlays.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const [output, option] = process.argv.slice(2);
// Single-use console authority stays out of command lines, reports and screenshots.
let inputBytes = '';
for await (const chunk of process.stdin) inputBytes += chunk;
const { url, expectedDisabled = false } = JSON.parse(inputBytes);
const port = await new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once('error', reject);
  server.listen(0, '127.0.0.1', () => {
    const value = server.address().port;
    server.close(error => error ? reject(error) : resolve(value));
  });
});
await mkdir(output, { recursive: true });
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-released-recommendation-'));
const chrome = spawn(process.env.CHROME_BIN || 'google-chrome', [
  '--headless=new', '--disable-gpu', `--remote-debugging-port=${port}`,
  '--remote-allow-origins=*', `--user-data-dir=${profile}`, 'about:blank',
], { detached: true, stdio: 'ignore' });
let client;
const visible = selector => `[...document.querySelectorAll(${JSON.stringify(selector)})].filter(e=>e.getClientRects().length)`;
async function click(selector, text = '') {
  await waitFor(client, `(()=>{const e=${visible(selector)}.find(e=>!e.disabled&&e.textContent.includes(${JSON.stringify(text)}));if(!e)return false;e.click();return true;})()`, { timeout: 20_000 });
}
async function input(label, value) {
  await waitFor(client, `(()=>{const label=${visible('label')}.find(e=>e.textContent.includes(${JSON.stringify(label)}));const e=label?.querySelector('input');if(!e)return false;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
}
async function screenshot(name) {
  await evaluate(client, 'Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{})))');
  const result = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, name + '.png'), Buffer.from(result.data, 'base64'));
}
try {
  const deadline = Date.now() + 15_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error('Browser did not start');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  client = await connectPage(port);
  await client.send('Page.addScriptToEvaluateOnNewDocument', { source: "localStorage.setItem('hiroute.language','en');" });
  await navigate(client, url, { width: 1200, height: 900 });
  await waitFor(client, "!!document.querySelector('.new-model-card')", { timeout: 30_000 });
  assert.equal(await evaluate(client, "document.querySelector('.new-model-card h3').textContent"), 'Qwen4-Max');
  assert.equal(await evaluate(client, "document.querySelector('.new-model-card button').disabled"), expectedDisabled);
  if (expectedDisabled) {
    await screenshot('recommendation-disabled');
  } else {
    await screenshot('recommendation-enabled');
    await click('.new-model-card button');
    // Current clients show the choice introduced in #501; released clients enter the trial.
    if (await evaluate(client, "!!document.querySelector('.recommendation-dialog')")) {
      await click('.recommendation-dialog button', 'Create a trial route');
      await click('.recommendation-dialog button', 'Create a connection');
    } else {
      await waitFor(client, "!!document.querySelector('[data-trial-target=trial]')", { timeout: 20_000 });
      await click('[data-trial-target=trial] button', 'Connect here');
    }
    await waitFor(client, "!!document.querySelector('.trial-inline-connection')", { timeout: 20_000 });
    await waitFor(client, `${visible('.connection-provider-card, .connection-dialog input[type=password]')}.length > 0`, { timeout: 20_000 });
    if (await evaluate(client, `${visible('.connection-provider-card')}.length > 0`)) {
      await click('.connection-provider-card');
      await click('.connection-product-card', option.includes('token-plan') ? 'Token Plan' : 'Pay as you go');
    }
    await input('Connection name', option);
    await input('API Key', 'isolated-recommendation-key');
    await screenshot('connection-form');
    await click('.connection-dialog button', 'Check and continue');
    await waitFor(client, "!!document.querySelector('.model-selection')", { timeout: 30_000 });
    await evaluate(client, `(()=>{for(const row of document.querySelectorAll('.model-result-row')){const e=row.querySelector('input[type=checkbox]');const desired=row.textContent.includes('Qwen4-Max');if(e&&!e.disabled&&e.checked!==desired)e.click();}})()`);
    if (await evaluate(client, "!document.querySelector('.model-selection')?.textContent.includes('qwen4-max passed a function tool call')")) {
      await click('.model-selection button', 'Test tool calling');
      await waitFor(client, "document.querySelector('.model-selection')?.textContent.includes('qwen4-max passed a function tool call')", { timeout: 30_000 });
    }
    await screenshot('connection-review');
    await click('.connection-dialog button', 'Save connection');
    await waitFor(client, "!document.querySelector('.trial-inline-connection') && document.querySelector('[data-trial-target=trial]')?.textContent.includes('Qwen4-Max')", { timeout: 30_000 });
    await screenshot('trial-selected');
    await writeFile(path.join(output, 'browser.json'), JSON.stringify({ scenario: 'released-recommendation-card-connect-trial', state: 'green', option }) + '\n');
  }
} catch (error) {
  if (client) {
    await screenshot('failure').catch(() => {});
    await writeFile(path.join(output, 'failure.txt'), await evaluate(client, 'document.body.innerText')).catch(() => {});
  }
  throw error;
} finally {
  client?.close();
  try { process.kill(-chrome.pid, 'SIGTERM'); } catch {}
  await Promise.race([new Promise(resolve => chrome.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 1500))]);
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try { await rm(profile, { recursive: true, force: true }); break; } catch (error) {
      if (attempt === 4) throw error;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
}
