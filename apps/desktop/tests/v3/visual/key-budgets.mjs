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
  async function assertLayout(height) {
    const geometry=await evaluate(client,`(() => {const modal=document.querySelector('.modal'), body=modal.querySelector('.modal-body'),footer=modal.querySelector('.modal-foot');return {overflow:body.scrollWidth-body.clientWidth,footerBottom:footer.getBoundingClientRect().bottom,height:innerHeight,fields:[...body.querySelectorAll('.key-budget-editor input:not([type=checkbox]),.key-budget-editor select')].filter(e=>e.getClientRects().length).map(e=>{const r=e.getBoundingClientRect(),p=body.getBoundingClientRect();return {width:r.width,left:r.left-p.left,right:r.right-p.right}})};})()`);
    assert.ok(geometry.overflow<=1,JSON.stringify(geometry));assert.ok(geometry.footerBottom<=height+1,JSON.stringify(geometry));
    assert.ok(geometry.fields.every(f=>f.width>=70 && f.left>=-1 && f.right<=1),JSON.stringify(geometry));
    const alignment=await evaluate(client,`[...document.querySelectorAll('.key-budget-grid,.key-budget-month-day')].flatMap(grid=>{const fields=[...grid.children].filter(e=>e.matches('.field')&&e.getClientRects().length);return fields.flatMap((a,i)=>fields.slice(i+1).flatMap(b=>{const x=a.getBoundingClientRect(),y=b.getBoundingClientRect();return x.right<=y.left && Math.min(x.bottom,y.bottom)>Math.max(x.top,y.top)?[Math.abs(x.top-y.top)]:[]}))})`);
    assert.ok(alignment.every(delta=>delta<=1),`Fields in the same row must align: ${alignment}`);
    return geometry;
  }
  async function selectRule(period) {
    await change('[data-budget-period]',period,true);
    await waitFor(client,`document.querySelector('[data-budget-period]')?.value === ${JSON.stringify(period)}`);
    const fields=await evaluate(client,`({weekday:!!document.querySelector('[data-budget-weekday]'),month:!!document.querySelector('[data-budget-month]'),day:!!document.querySelector('[data-budget-day]'),time:document.querySelector('[data-budget-time]')?.type})`);
    assert.deepEqual(fields,{weekday:period==='weekly',month:period==='yearly',day:period==='monthly'||period==='yearly',time:'time'});
    if(period==='weekly') await change('[data-budget-weekday]','3',true);
    if(period==='yearly') {
      await change('[data-budget-month]','2',true);
      await waitFor(client,'document.querySelector("[data-budget-day]").options.length === 29');
      await change('[data-budget-day]','29',true);
    }
    if(period==='monthly') await change('[data-budget-day]','31',true);
    await change('[data-budget-time]','08:45');
    await waitFor(client,'document.querySelector("[data-budget-time]").value === "08:45"');
  }
  for (const language of ['zh','en']) for (const theme of ['light','dark']) for (const [width,height] of [[1200,813],[755,600]]) {
    const name=`key-budgets-${language}-${theme}-${width}`;
    await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?lang=${language}&theme=${theme}`,{width,height});
    await waitFor(client, 'document.querySelector(".list-row")!==null');
    await click('.list-row');
    await click('.btn',language==='zh'?'管理 Key 与额度':'Manage keys and allowances');
    await waitFor(client,'document.querySelector("[data-key-budget] input[type=checkbox]")?.checked===true');
    assert.equal(await evaluate(client,'document.querySelector("[data-key-budget] input[type=datetime-local]") !== null'),false,'recurring allowances must select the period rule without requiring a year');
    await click('[data-key-budget] summary',language==='zh'?'更多额度设置':'More allowance settings');
    for(const period of ['daily','weekly','yearly','monthly']) {
      await selectRule(period);
      const geometry=await assertLayout(height);
      assert.equal(await evaluate(client,'document.querySelector("[data-key-budget] [role=status]")?.textContent.includes("2026")'),false,'editing the schedule must not show its stale saved next-renewal date');
      await evaluate(client,'document.querySelector("[data-budget-period]").closest(".key-budget-grid").scrollIntoView({block:"center"});true');
      const shot=await client.send('Page.captureScreenshot',{format:'png'});
      await writeFile(path.join(output,`${name}-${period}.png`),Buffer.from(shot.data,'base64'));
      results.push({name:`${name}-${period}`,...geometry});
    }
    const geometry=await assertLayout(height);
    // Changing a rule or amount retains its ID/CAS and uses the same primary Save.
    await change('[data-key-budget] .key-budget-amount input','120');
    const shot=await client.send('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,`${name}.png`),Buffer.from(shot.data,'base64'));
    await evaluate(client, '(()=>{const body=document.querySelector(".modal-body");body.scrollTop=body.scrollHeight;return true;})()');
    await new Promise(resolve=>setTimeout(resolve,100));
    const bottom=await client.send('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,`${name}-advanced.png`),Buffer.from(bottom.data,'base64'));
    await click('.modal-foot .btn-primary');
    await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
    const saved=await evaluate(client,'window.budgetTrace[0]');
    assert.equal(saved.budgets[0].assignment.budget_id,'budget/monthly');assert.equal(saved.budgets[0].assignment.expected_revision,3);assert.equal(saved.budgets[0].assignment.policy.amount_micros,120000000);assert.equal(saved.budgets[0].assignment.initial_remaining_micros,null);
    assert.equal(saved.budgets[0].assignment.policy.renewal.period,'monthly');assert.match(saved.budgets[0].assignment.policy.renewal.anchor_local,/-31T08:45:00$/);
    results.push({name,...geometry,singleSave:true});
  }
  for(const period of ['daily','weekly','yearly','monthly']) {
    await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html`,{width:755,height:600});
    await click('.list-row');await click('.btn','管理 Key 与额度');
    await waitFor(client,'document.querySelector("[data-budget-period]")!==null');
    await selectRule(period);await click('.modal-foot .btn-primary');
    await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
    const renewal=await evaluate(client,'window.budgetTrace[0].budgets[0].assignment.policy.renewal');
    assert.equal(renewal.period,period);assert.equal(renewal.timezone,'America/Los_Angeles');assert.match(renewal.anchor_local,/T08:45:00$/);
    if(period==='weekly') assert.equal(new Date(`${renewal.anchor_local}Z`).getUTCDay(),3);
    if(period==='monthly') assert.match(renewal.anchor_local,/-31T/);
    if(period==='yearly') assert.match(renewal.anchor_local,/-02-29T/);
    results.push({name:`save-${period}`,renewal,singleSave:true});
  }
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理 Key 与额度');
  await waitFor(client,'document.querySelector("[data-budget-period]")!==null');
  await change('[data-key-budget] .key-budget-amount input','120');await click('.modal-foot .btn-primary');
  await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
  assert.equal(await evaluate(client,'window.budgetTrace[0].budgets[0].assignment.policy.renewal.anchor_local'),'2026-11-30T09:00:00');
  results.push({name:'amount-only-retains-saved-anchor'});
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?seconds=1`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理 Key 与额度');
  await waitFor(client,'document.querySelector("[data-budget-time]")!==null');
  assert.equal(await evaluate(client,'document.querySelector("[data-budget-time]").value'),'09:00');
  await change('[data-key-budget] .key-budget-amount input','120');await click('.modal-foot .btn-primary');
  await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
  assert.equal(await evaluate(client,'window.budgetTrace[0].budgets[0].assignment.policy.renewal.anchor_local'),'2026-11-30T09:00:30.123456789');
  results.push({name:'amount-only-retains-subminute-anchor'});
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理 Key 与额度');
  await waitFor(client,'document.querySelector("[data-budget-time]")!==null');
  await change('[data-budget-time]','');await click('.modal-foot .btn-primary');
  await waitFor(client,'!!document.querySelector(".modal [role=alert]")');
  assert.equal(await evaluate(client,'window.budgetTrace.length'),0,'an incomplete time must not save a guessed schedule');
  await change('[data-budget-time]','10:15');await click('.modal-foot .btn-primary');
  await waitFor(client,'window.budgetTrace.length===1 && !document.querySelector(".modal")');
  results.push({name:'incomplete-time-requires-repair'});
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?pending=1`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理 Key 与额度');
  await waitFor(client,'document.querySelector("[data-key-budget] input[type=checkbox]")?.checked===true');
  await change('[data-key-budget] .key-budget-amount input','120');await click('.modal-foot .btn-primary');
  await waitFor(client,'window.budgetTrace.length===1 && !!window.finishBudgetSave');
  assert.equal(await evaluate(client,'[...document.querySelectorAll(".oc-credential-fields input,.oc-credential-fields select,.oc-credential-fields button")].every(e=>e.matches(":disabled"))'),true,'pending save freezes all Key and allowance edits');
  await evaluate(client,'window.finishBudgetSave();true');await waitFor(client,'!document.querySelector(".modal")');
  await navigate(client,`http://127.0.0.1:${httpPort}/key-budgets.html?unavailable=1`,{width:755,height:600});
  await click('.list-row');await click('.btn','管理 Key 与额度');await waitFor(client,'document.querySelector(".modal")?.textContent.includes("额度暂不可用")');
  assert.equal(await evaluate(client,'document.querySelector("[data-key-budget]")!==null'),false);
  await writeFile(path.join(output,'results.json'),JSON.stringify({state:'green',scenarios:results,unavailablePreservesLimits:true},null,2));
  console.log(JSON.stringify({state:'green',scenarios:results.length+2,output}));
} finally {
  client?.close(); await stop(chrome); await stop(vite); await rm(profile, { recursive: true, force: true });
}
