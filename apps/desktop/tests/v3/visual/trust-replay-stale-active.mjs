import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';
const [port,base,output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
try {
  await client.send('Page.enable'); await client.send('Runtime.enable');
  await client.send('Page.navigate', {url:`${base}trust-replay-stale.html`});
  await waitFor(client, `!!window.replayFixture && !!document.querySelector('.trust-replay-bar')`);
  await evaluate(client, `replayFixture.setThreshold(900);true`);
  await waitFor(client, `replayFixture.trace.some(x=>x.threshold===900)`);
  await evaluate(client, `replayFixture.setThreshold(800);true`);
  await waitFor(client, `document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.8')`);
  await evaluate(client, `replayFixture.release();true`);
  await new Promise(resolve => setTimeout(resolve, 150));
  assert.ok(await evaluate(client, `document.querySelector('.trust-replay-bar').getAttribute('aria-label').includes('0.8')`));
  await evaluate(client, `replayFixture.setThreshold(900);true`);
  await waitFor(client, `replayFixture.trace.filter(x=>x.threshold===900).length===2`);
  await evaluate(client, `replayFixture.setPlan('plan-b');replayFixture.setThreshold(720);true`);
  await waitFor(client, `replayFixture.trace.some(x=>x.plan==='plan-b'&&x.threshold===720)`);
  await evaluate(client, `replayFixture.release();true`);
  await waitFor(client, `document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.72')`);
  assert.ok(await evaluate(client, `!document.querySelector('.trust-replay-bar').getAttribute('aria-label').includes('0.9')`));
  await evaluate(client, `replayFixture.setThreshold(900);true`);
  await waitFor(client, `replayFixture.trace.some(x=>x.plan==='plan-b'&&x.threshold===900)`);
  await evaluate(client, `replayFixture.setActive(false);true`);
  await waitFor(client, `replayFixture.active===false`);
  await evaluate(client, `replayFixture.release();true`);
  await new Promise(resolve => setTimeout(resolve, 150));
  await evaluate(client, `replayFixture.setActive(true);true`);
  await waitFor(client, `replayFixture.trace.filter(x=>x.plan==='plan-b'&&x.threshold===900).length===2`);
  await evaluate(client, `replayFixture.release();true`);
  await waitFor(client, `document.querySelector('.trust-replay-bar')?.getAttribute('aria-label').includes('0.9')`);
  results.push({id:'superseded-threshold-and-scope',status:'passed'});
} catch (error) { results.push({id:'superseded-threshold-and-scope',status:'failed',error:String(error.stack ?? error)}); }
finally { await writeFile(path.join(output,'trust-replay-stale-results.json'), JSON.stringify(results,null,2)); await client.close(); }
if (results.some(result=>result.status!=='passed')) process.exitCode = 1;
