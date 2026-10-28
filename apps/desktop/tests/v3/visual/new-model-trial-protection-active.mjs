import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const transport = process.env.HIROUTE_BROWSER_TRANSPORT ?? 'tauri';
const shot = async (name, settle = true) => {
  if (settle) await evaluate(client, `Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>true)`);
  const value = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(value.data, 'base64'));
};
async function open(query, width, height, theme, lang) {
  const url = `${base}new-model-trial.html?polish=1&native=1&lang=${lang}&theme=${theme}&${query}${transport === 'console' ? '&transport=console' : ''}`;
  await client.send('Page.enable');
  await client.send('Runtime.enable');
  await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await client.send('Page.navigate', { url });
  await waitFor(client, `location.href===${JSON.stringify(url)} && !!document.querySelector('.plan-editor,.new-models,.sessions-page')`);
  await evaluate(client, 'document.fonts.ready.then(()=>true)');
  if (transport === 'console') assert.equal(await evaluate(client, 'window.fixtureTransport'), 'console');
  if (width === 755 && query.startsWith('view=timeline')) {
    await click('.toolbar-actions button', lang === 'zh' ? '紧凑视图' : 'Compact view');
    await waitFor(client, `document.querySelector('.sessions-page').classList.contains('is-compact')`);
  }
}
async function click(selector, text) {
  const expression = `(()=>{const el=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&e.textContent.includes(${JSON.stringify(text)}));if(!el)return false;el.click();return true;})()`;
  await waitFor(client, expression);
}
async function check(id, run) {
  if (process.env.PROTECTION_CASE && !id.startsWith(process.env.PROTECTION_CASE)) return;
  try { await run(); results.push({ id, status: 'passed' }); }
  catch (error) {
    await shot(`failure-${id}`);
    const geometry = await evaluate(client, `({width:innerWidth,document:document.documentElement.scrollWidth,lanes:[...document.querySelectorAll('.routing-lanes-scroll')].map(e=>({width:e.clientWidth,scroll:e.scrollWidth,visible:e.getClientRects().length}))})`).catch(() => null);
    await writeFile(path.join(output, `failure-${id}.json`), JSON.stringify({ error: String(error.stack ?? error), geometry }, null, 2));
    results.push({ id, status: 'failed', error: String(error.stack ?? error) });
    await client.send('Emulation.setEmulatedMedia', { features: [] });
  }
}
const matrix = process.env.PROTECTION_BASELINE ? [['en', 'light', process.env.PROTECTION_BASELINE === '755' ? 755 : 1200, process.env.PROTECTION_BASELINE === '755' ? 600 : 813]]
  : ['en', 'zh'].flatMap(lang => ['light', 'dark'].flatMap(theme => [[lang, theme, 1200, 813], [lang, theme, 755, 600]]));
try {
  for (const [lang, theme, width, height] of matrix) {
    const name = `${lang}-${theme}-${width}`;
    const load = query => open(query,width,height,theme,lang);
    const visible = i => `(()=>{const p=document.querySelector('.routing-lanes-scroll'),v=p.getBoundingClientRect(),l=p.querySelector('.routing-lane-label').getBoundingClientRect().width,r=p.querySelector('[data-turn-id="request-${i}"]').getBoundingClientRect();return r.left>=v.left+l-1&&r.right<=v.right+1;})()`;
    const choose = async i => {
      await evaluate(client,`document.querySelector('[data-overview-turn="request-${i}"]').dispatchEvent(new MouseEvent('click',{bubbles:true}));true`);
      await waitFor(client,`document.querySelector('.routing-detail-head > strong')?.textContent==='T${i}'`);
      await waitFor(client,visible(i));
    };
    const fits = `document.documentElement.scrollWidth<=innerWidth+1 && [...document.querySelectorAll('.routing-overview,.routing-turn-detail,.trust-protection')].filter(e=>e.getClientRects().length).every(e=>e.scrollWidth<=e.clientWidth+1)`;
    const range = `(()=>{const p=document.querySelector('.routing-lanes-scroll'),v=p.getBoundingClientRect(),l=p.querySelector('.routing-lane-label').getBoundingClientRect().width,es=[...p.querySelectorAll('[data-turn-id]')],is=es.map((e,i)=>({i,r:e.getBoundingClientRect()})).filter(({r})=>r.right>v.left+l&&r.left<v.right).map(({i})=>i),w=document.querySelector('.routing-overview-window');return is.length&&Math.abs(parseFloat(w.getAttribute('x'))*es.length/100-is[0])<.01&&Math.abs(parseFloat(w.getAttribute('width'))*es.length/100-(is.at(-1)-is[0]+1))<.01;})()`;
    await check(`release-evidence-${name}`,async()=>{
      for(const state of ['valid','zero','partial','empty','missing','plan','revision','time','branch']) {
        await load(`view=editor&release-evidence=${state}`);
        await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
        await click('[role="tab"]',lang==='zh'?'模型表现':'Model performance');
        const numeric=state==='valid'||state==='zero';
        await waitFor(client,`document.querySelector('.trust-release')?.dataset.releaseState===${JSON.stringify(numeric?'available':'unavailable')}`);
        const value=await evaluate(client,`document.querySelector('.trust-release').textContent`);
        if(numeric) {
          assert.ok(value.includes(lang==='zh'?`解除 ${state==='zero'?0:5} 次`:`${state==='zero'?0:5} protections released`),value);
          assert.ok(value.includes(lang==='zh'?'本轮重新按难度选择模型':'this turn selects a model based on difficulty'),value);
          assert.doesNotMatch(value,/allowing Regular to continue|允许新模型继续/);
        }
        else {assert.ok(value.includes('—'),value);assert.ok(!/5 protections|解除 5/.test(value),value);}
        if(!['valid','zero','missing'].includes(state)) await waitFor(client,`!!document.querySelector('.routing-observations')`);
        const reasons={partial:lang==='zh'?'覆盖不完整':'incomplete coverage',plan:lang==='zh'?'路由范围不同':'different route scope',revision:lang==='zh'?'路由版本不同':'different route version',time:lang==='zh'?'时间范围不同':'different time range',branch:lang==='zh'?'分支范围不同':'different branches'};
        if(reasons[state]) await waitFor(client,`document.querySelector('.trust-release').textContent.includes(${JSON.stringify(reasons[state])})`);
        assert.ok(await evaluate(client,fits));
        if(['valid','partial','empty'].includes(state)) {await evaluate(client,`document.querySelector('.trust-protection').scrollIntoView({block:'center'});true`);await shot(`released-${state}-${name}`);}
      }
    });
    await check(`retained-window-${name}`,async()=>{
      await load('view=editor&release-evidence=retention');
      await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
      await click('[role="tab"]',lang==='zh'?'模型表现':'Model performance');
      for(const period of ['seven_days','all','one_day']) {
        await evaluate(client,`(()=>{const s=document.querySelector('.quality-filters select');s.value=${JSON.stringify(period)};s.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
        await waitFor(client,`document.querySelector('.trust-release')?.dataset.releaseState==='available'`);
        assert.equal(await evaluate(client,`document.querySelector('.trust-release').dataset.releaseState`),'available',`${period} must use one retained interval`);
        assert.ok(await evaluate(client,`document.querySelector('.trust-release').textContent.includes(${JSON.stringify(lang==='zh'?'解除 5 次':'5 protections released')})`));
        const scope=await evaluate(client,`(()=>{const xs=window.trialTrace.filter(x=>x.command==='observation_read').map(x=>x.payload.request.intent),t=xs.filter(x=>x.view==='plan_trust').at(-1).query,q=xs.filter(x=>x.view==='plan_quality'&&x.query.include_routing_observations).at(-1).query;return {trust:[t.from_ms,t.to_ms],quality:[q.from_ms,q.to_ms]};})()`);
        assert.deepEqual(scope.trust,scope.quality);
        if(period==='one_day') assert.equal(scope.trust[1]-scope.trust[0],86_400_001);
        else {assert.ok(scope.trust[0]>scope.trust[1]-7*86_400_000+20);assert.ok(scope.trust[1]-scope.trust[0]>6.99*86_400_000);}
        await evaluate(client,`document.querySelector('.trust-protection').scrollIntoView({block:'center'});true`);
        await shot(`retained-window-${period}-${name}`);
      }
      await click('.segmented button',lang==='zh'?'按执行 Agent':'By execution agent');
      await waitFor(client,`!document.querySelector('.trust-release')`);
      await click('.segmented button',lang==='zh'?'按模型':'By model');
      await waitFor(client,`document.querySelector('.trust-release')?.dataset.releaseState==='available'`);
      assert.equal(await evaluate(client,`document.querySelector('.quality-filters select').value`),'one_day');
      assert.ok(await evaluate(client,fits));
    });
    await check(`aged-retained-window-${name}`,async()=>{
      for (const period of ['seven_days','all']) {
        await load('view=editor&release-evidence=retention');
        await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
        await click('[role="tab"]',lang==='zh'?'模型表现':'Model performance');
        await evaluate(client,`(()=>{const s=document.querySelector('.quality-filters select');s.value=${JSON.stringify(period)};s.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
        await waitFor(client,`document.querySelector('.trust-release')?.dataset.releaseState==='available'`);
        const trustCount=await evaluate(client,`trialTrace.filter(x=>x.payload?.request?.intent?.view==='plan_trust').length`);
        await evaluate(client,`window.advanceRetentionClock(120_000);true`);
        await click('.trust-protection button',lang==='zh'?'查看全部低分阶段':'View all low-score stages');
        await waitFor(client,`!!document.querySelector('.quality-more:not(:disabled)')`);
        assert.equal(await evaluate(client,`document.querySelector('.trust-release').dataset.releaseState`),'available','filter preserves paired base summary after retention moves');
        await click('.quality-more',lang==='zh'?'加载更多阶段':'Load more stages');
        await waitFor(client,`!document.querySelector('.quality-more')`);
        assert.ok(await evaluate(client,`document.querySelector('.trust-release').textContent.includes(${JSON.stringify(lang==='zh'?'解除 5 次':'5 protections released')})`),'pagination preserves the paired count');
        assert.equal(await evaluate(client,`trialTrace.filter(x=>x.payload?.request?.intent?.view==='plan_trust').length`),trustCount,'filters do not refresh the aggregate scope');
        await click('.segmented button',lang==='zh'?'按执行 Agent':'By execution agent');
        await waitFor(client,`!!document.querySelector('[data-execution-group="executor"]')`);
        await evaluate(client,`window.advanceRetentionClock(120_000);true`);
        await click('.segmented button',lang==='zh'?'按模型':'By model');
        await waitFor(client,`document.querySelector('.trust-release')?.dataset.releaseState==='available'`);
        const scope=await evaluate(client,`(()=>{const xs=trialTrace.filter(x=>x.command==='observation_read').map(x=>x.payload.request.intent),t=xs.filter(x=>x.view==='plan_trust').at(-1).query,q=xs.filter(x=>x.view==='plan_quality'&&x.query.include_routing_observations).at(-1).query;return {trust:[t.from_ms,t.to_ms],quality:[q.from_ms,q.to_ms],now:Date.now()};})()`);
        assert.deepEqual(scope.trust,scope.quality);
        assert.equal(scope.trust[1],scope.now+1,'group changes capture a fresh interval');
      }
    });
    await check(`history-outcomes-${name}`,async()=>{
      await load('view=timeline&turns=40&protection=1');
      await waitFor(client,`document.querySelectorAll('.routing-turn').length===40`);
      for(const [i,phrase] of [[1,lang==='zh'?'之前的问题仍相关':'earlier problem still matters'],[2,lang==='zh'?'之前的问题已修正或不再相关':'earlier problem was fixed or no longer matters'],[3,lang==='zh'?'无法判断之前的问题，保留保护':'problem judgment unavailable; protection retained'],[4,lang==='zh'?'无需历史低分保护':'Historical protection not needed'],[5,lang==='zh'?'未进入历史低分保护判断':'Historical protection was not considered'],[6,lang==='zh'?'历史保护结果未记录':'Historical protection result was not recorded']]) {
        await choose(i);await waitFor(client,`document.querySelector('.routing-detail-block')?.textContent.includes(${JSON.stringify(phrase)})`);
        if(i<=3) assert.ok(await evaluate(client,`document.querySelector('.routing-judgments').textContent.includes('0.30')&&document.querySelector('.routing-judgments').textContent.includes('0.50')`));
        if(i===1) assert.ok(await evaluate(client,`document.querySelector('.routing-judgments').textContent.includes('0.99')&&!document.querySelector('.routing-judgments').textContent.includes('0.91')`));
        if(i===2) assert.ok(await evaluate(client,`document.querySelector('[data-turn-id="request-2"] .routing-protection-badge').textContent.includes(${JSON.stringify(lang==='zh'?'已解除保护':'Protection released')})&&document.querySelector('.routing-judgments').textContent.includes('0.05')`));
        if(i===3) assert.equal(await evaluate(client,`document.querySelector('.routing-judgments').textContent.includes('0.00')`),false);
        assert.ok(await evaluate(client,fits));
        if(i===1||i===2){await evaluate(client,`document.querySelector('.routing-turn-detail').scrollIntoView({block:'center'});true`);await shot(`outcome-${i===1?'protected':'released'}-${name}`);}
      }
      assert.equal(await evaluate(client,`/q[012]|digest|retained_unavailable|suppressed|system_one/.test(document.querySelector('.routing-timeline').textContent)`),false);
    });
    await check(`overview-navigation-${name}`,async()=>{
      await load('view=timeline&turns=40&protection=1');await waitFor(client,`document.querySelectorAll('.routing-overview-turn').length===40`);
      assert.equal(await evaluate(client,`document.querySelectorAll('.routing-overview-turn[tabindex="0"]').length`),1,'The entire overview has one tab stop');
      assert.equal(await evaluate(client,`document.querySelectorAll('.routing-overview-turn[tabindex="-1"]').length`),39);
      assert.ok(await evaluate(client,`[...document.querySelectorAll('.routing-overview-turn')].every(e=>e.getAttribute('aria-label')?.startsWith('T'))`));
      await evaluate(client,`document.querySelector('.routing-overview-turn[tabindex="0"]').focus();true`);
      const key=async(key,code=key)=>{for(const type of ['keyDown','keyUp'])await client.send('Input.dispatchKeyEvent',{type,key,code});};
      await key('Home');assert.equal(await evaluate(client,`document.activeElement.dataset.overviewTurn`),'request-1');
      await key('ArrowLeft');assert.equal(await evaluate(client,`document.activeElement.dataset.overviewTurn`),'request-40');
      await key('ArrowRight');assert.equal(await evaluate(client,`document.activeElement.dataset.overviewTurn`),'request-1');
      await key('ArrowRight');assert.equal(await evaluate(client,`document.activeElement.dataset.overviewTurn`),'request-2');
      await key(' ','Space');await waitFor(client,`document.querySelector('.routing-detail-head > strong')?.textContent==='T2'`);
      await key('End');assert.equal(await evaluate(client,`document.activeElement.dataset.overviewTurn`),'request-40');
      await key('Tab');assert.ok(await evaluate(client,`!document.activeElement.closest('.routing-overview')`),'Tab leaves the overview');
      assert.equal(await evaluate(client,`document.querySelectorAll('.routing-overview-turn[data-protection=suppressed]').length`),7);
      assert.ok(await evaluate(client,`document.querySelectorAll('.routing-overview-turn[data-problem=true]').length>0`));
      assert.ok(await evaluate(client,`getComputedStyle(document.querySelector('.routing-overview-turn.cyan')).color===getComputedStyle(document.querySelector('.routing-turn.cyan .routing-turn-reason')).color`));
      await choose(1);await waitFor(client,range);const before=await evaluate(client,`document.querySelector('.routing-overview-window').getAttribute('x')`);
      await choose(40);await waitFor(client,range);assert.notEqual(await evaluate(client,`document.querySelector('.routing-overview-window').getAttribute('x')`),before);
      await client.send('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-motion',value:'reduce'}]});
      await evaluate(client,`document.querySelector('[data-overview-turn="request-5"]').focus();true`);
      for(const type of ['keyDown','keyUp'])await client.send('Input.dispatchKeyEvent',{type,key:'Enter',code:'Enter'});
      await waitFor(client,`document.querySelector('.routing-detail-head > strong')?.textContent==='T5'`);await waitFor(client,visible(5));await waitFor(client,range);
      await client.send('Emulation.setDeviceMetricsOverride',{width:width===755?1200:755,height:height===600?813:600,deviceScaleFactor:1,mobile:false});await waitFor(client,range);assert.ok(await evaluate(client,fits));
      await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await waitFor(client,range);
      await evaluate(client,`document.querySelector('[data-turn-id="request-1"]').focus();true`);
      for(const type of ['keyDown','keyUp'])await client.send('Input.dispatchKeyEvent',{type,key:'Tab',code:'Tab'});
      assert.equal(await evaluate(client,`document.activeElement.dataset.turnId`),'request-2');
      await choose(5);await evaluate(client,'appendTrialTurns(1);true');await waitFor(client,`document.querySelectorAll('.routing-overview-turn').length===41`);
      assert.equal(await evaluate(client,`document.querySelector('.routing-overview-turn[aria-pressed=true]').dataset.overviewTurn`),'request-5');await waitFor(client,range);
      await evaluate(client,`document.querySelector('.routing-overview').scrollIntoView({block:'center'});true`);
      await waitFor(client,`(()=>{const e=document.querySelector('.routing-overview'),r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&e.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2));})()`);
      await shot(`overview-${name}`);assert.ok(await evaluate(client,fits));
      await client.send('Emulation.setEmulatedMedia',{features:[]});
    });
  }
} finally {
  client.close();
  await writeFile(path.join(output, 'report.json'), JSON.stringify({ proof: 'Production components with synthetic IPC', results }, null, 2));
}
console.log(JSON.stringify({ passed: results.filter(r => r.status === 'passed').length, failed: results.filter(r => r.status !== 'passed').length, output }));
if (!results.length || results.some(result => result.status !== 'passed')) process.exitCode = 1;
