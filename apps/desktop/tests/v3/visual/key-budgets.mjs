import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const output = path.resolve(process.argv[2] ?? path.join(desktop, 'test-results/key-budgets'));
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
const httpPort = await port(), cdpPort = await port();
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-cost-presentation-'));
await mkdir(output, { recursive: true });
let vite, chrome, client;
try {
  vite = spawn(path.join(desktop, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktop, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { vite.once('spawn', resolve); vite.once('error', reject); });
  await ready(`http://127.0.0.1:${httpPort}/`);
  const executable = process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
  chrome = spawn(executable, ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, 'about:blank'], { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { chrome.once('spawn', resolve); chrome.once('error', reject); });
  await ready(`http://127.0.0.1:${cdpPort}/json/version`);
  client = await connectPage(cdpPort);
  const results = [];
  async function click(selector, text = '') {
    await waitFor(client, `(() => { const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length && !e.disabled && e.textContent.includes(${JSON.stringify(text)})); if(!e)return false; e.click();return true; })()`);
  }
  async function change(selector, value, select=false) {
    await evaluate(client, `(() => { const e=document.querySelector(${JSON.stringify(selector)}); Object.getOwnPropertyDescriptor(${select?'HTMLSelectElement':'HTMLInputElement'}.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(${select?"'change'":"'input'"},{bubbles:true}));return true;})()`);
  }
  for (const language of ['zh','en']) for (const theme of ['light','dark']) for (const [width,height] of [[1200,813],[755,600]]) {
    const name=`key-budgets-${language}-${theme}-${width}`;
    await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?lang=${language}&theme=${theme}`,{width,height});
    await waitFor(client, 'document.querySelector(".list-row")!==null');
    await click('.list-row');
    await click('.btn',language==='zh'?'管理凭据':'Manage credentials');
    await waitFor(client,'document.querySelector("[data-key-budget] input[type=checkbox]")?.checked===true');
    await click('[data-key-budget] summary',language==='zh'?'更多额度设置':'More allowance settings');
    const geometry=await evaluate(client,`(() => {const modal=document.querySelector('.modal'), body=modal.querySelector('.modal-body'),footer=modal.querySelector('.modal-foot');return {overflow:body.scrollWidth-body.clientWidth,footerBottom:footer.getBoundingClientRect().bottom,height:innerHeight,fields:[...body.querySelectorAll('.key-budget-editor input:not([type=checkbox]),.key-budget-editor select')].filter(e=>e.getClientRects().length).map(e=>{const r=e.getBoundingClientRect(),p=body.getBoundingClientRect();return {width:r.width,left:r.left-p.left,right:r.right-p.right}})};})()`);
    assert.ok(geometry.overflow<=1,JSON.stringify(geometry));assert.ok(geometry.footerBottom<=height+1,JSON.stringify(geometry));
    assert.ok(geometry.fields.every(f=>f.width>=70 && f.left>=-1 && f.right<=1),JSON.stringify(geometry));
    // The original monthly policy is visible; changing an amount retains its ID/CAS.
    await change('[data-key-budget] .key-budget-amount input','120');
    const shot=await client.send('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,`${name}.png`),Buffer.from(shot.data,'base64'));
    await click('.modal-foot .btn-primary');
    await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
    const saved=await evaluate(client,'window.budgetTrace[0]');
    assert.equal(saved.budgets[0].assignment.budget_id,'budget/monthly');assert.equal(saved.budgets[0].assignment.expected_revision,3);assert.equal(saved.budgets[0].assignment.policy.amount_micros,120000000);assert.equal(saved.budgets[0].assignment.initial_remaining_micros,null);
    results.push({name,...geometry,singleSave:true});
  }
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?unavailable=1`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理凭据');await waitFor(client,'document.querySelector(".modal")?.textContent.includes("额度暂不可用")');
  assert.equal(await evaluate(client,'document.querySelector("[data-key-budget]")!==null'),false);
  await writeFile(path.join(output,'results.json'),JSON.stringify({state:'green',scenarios:results,unavailablePreservesLimits:true},null,2));
  console.log(JSON.stringify({state:'green',scenarios:results.length+1,output}));
} finally {
  client?.close(); await stop(chrome); await stop(vite); await rm(profile, { recursive: true, force: true });
}
