import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import path from 'node:path';
import {connectPage,evaluate,waitFor} from './cdp-client.mjs';
const [port,base,output]=process.argv.slice(2);
const client=await connectPage(Number(port));
const keepAlive=setInterval(()=>{},1000);const results=[];
async function click(selector,text) {
  await waitFor(client,`[...document.querySelectorAll(${JSON.stringify(selector)})].some(e=>e.getClientRects().length&&!e.disabled&&e.textContent.includes(${JSON.stringify(text)}))`);
  await evaluate(client,`(()=>{[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.disabled&&e.textContent.includes(${JSON.stringify(text)})).click();return true;})()`);
}
async function select(selector,value) {
  await evaluate(client,`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
}
async function shot(name) {
  await evaluate(client,`Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>true)`);
  const screenshot=await client.send('Page.captureScreenshot',{format:'png'});
  await writeFile(path.join(output,`${name}.png`),Buffer.from(screenshot.data,'base64'));
}
async function geometry(width) {
  const values=await evaluate(client,`[...document.querySelectorAll('.model-default-field,.model-default-field select,.model-metadata-filters,.model-metadata-records li,[role=dialog]')].filter(e=>e.getClientRects().length).map(e=>({name:e.className,width:e.clientWidth,scroll:e.scrollWidth,left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right}))`);
  for(const v of values) {assert.ok(v.width>80,JSON.stringify(v));assert.ok(v.scroll<=v.width+1,JSON.stringify(v));assert.ok(v.left>=-1&&v.right<=width+1,JSON.stringify(v));}
}
try {
  for(const [lang,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]) {
    await client.send('Page.enable');await client.send('Runtime.enable');
    await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
    await client.send('Page.navigate',{url:`${base}new-model-trial.html?metadata=1&lang=${lang}&theme=${theme}`});
    await waitFor(client,`!!document.querySelector('.models-feature .list-row')`);
    await click('.list-row','Qwen3-Coder-Plus');
    await waitFor(client,`document.querySelectorAll('.model-default-field').length===6`);
    await evaluate(client,`document.querySelector('.model-defaults-editor').scrollIntoView({block:'center'});true`);
    assert.equal(await evaluate(client,`document.querySelector('#model-default-vision').value`),'preserve_saved');
    await select('#model-default-context_tokens','preserve_saved');
    await click('.model-defaults-editor button',lang==='zh'?'预览修改':'Preview changes');
    await waitFor(client,`[...document.querySelectorAll('.model-defaults-editor button')].some(e=>e.textContent.includes(${JSON.stringify(lang==='zh'?'保存默认值选择':'Save default choices')}))`);
    assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='preview_compute_save_v3').payload.defaultPreferences['binding/bailian/qwen-coder'].context_tokens.intent`),'preserve_saved');
    await evaluate(client,`metadataFixture.fail=true;dispatchEvent(new Event('model-feed-changed'));true`);
    await waitFor(client,`!!document.querySelector('.model-defaults-editor [role=alert]')`);
    assert.equal(await evaluate(client,`document.querySelector('#model-default-context_tokens').value`),'preserve_saved');
    await shot(`defaults-error-retains-draft-${lang}-${theme}-${width}`);
    await evaluate(client,`metadataFixture.fail=false;true`);
    await click('.model-defaults-editor [role=alert] button',lang==='zh'?'重新读取':'Reload');
    await waitFor(client,`!document.querySelector('.model-defaults-editor [role=alert]')`);
    await geometry(width);await shot(`defaults-current-and-preserved-${lang}-${theme}-${width}`);
    await click('.model-feed-actions button',lang==='zh'?'详情':'Details');
    await click('.model-feed-details button',lang==='zh'?'浏览全部模型资料':'Browse all model information');
    await waitFor(client,`document.querySelectorAll('.model-metadata-records li').length===1`);
    // These are actual generated records: Gemini fields are inferred; Spark's are observed.
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records > li:first-child .model-metadata-inferred').length`),4);
    assert.ok(await evaluate(client,`document.querySelector('.model-metadata-records').textContent.includes(${JSON.stringify(lang==='zh'?'推断':'Inferred')})`));
    await evaluate(client,`metadataFixture.evidenceFail=true;true`);
    await click('.model-metadata-records summary',lang==='zh'?'来源与推断依据':'Sources and inference');
    await waitFor(client,`!!document.querySelector('.model-metadata-evidence [role=alert]')`);
    await evaluate(client,`metadataFixture.evidenceFail=false;true`);
    await click('.model-metadata-evidence button',lang==='zh'?'重试':'Retry');
    await waitFor(client,`!!document.querySelector('.model-metadata-evidence .model-metadata-rule')`);
    assert.ok(await evaluate(client,`document.querySelector('.model-metadata-evidence').textContent.includes('rule.google.gemini/v1')`));
    assert.ok(await evaluate(client,`[...document.querySelectorAll('.model-metadata-evidence a')].some(a=>a.href.includes('github.com/NousResearch/hermes-agent'))`));
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='get_model_metadata').every(x=>x.payload.input.snapshot?.generation===1)`),true);
    await geometry(width);await shot(`metadata-inferred-sources-${lang}-${theme}-${width}`);
    await evaluate(client,`metadataFixture.stale=true;true`);
    await click('[role=dialog] button',lang==='zh'?'加载更多':'Load more');
    await waitFor(client,`trialTrace.filter(x=>x.command==='list_model_metadata').length>=3`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records li').length`),1);
    await click('[role=dialog] button',lang==='zh'?'加载更多':'Load more');
    await waitFor(client,`document.querySelectorAll('.model-metadata-records li').length===2`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records > li:nth-child(2) .model-metadata-inferred').length`),0);
    await geometry(width);await shot(`metadata-paged-${lang}-${theme}-${width}`);
    await select('.model-metadata-filters select','price');
    await waitFor(client,`document.querySelector('.model-metadata-records')?.textContent.includes('0.125')`);
    assert.ok(await evaluate(client,`document.querySelector('.model-metadata-records').textContent.includes(${JSON.stringify(lang==='zh'?'缓存读取':'Cache read')})`));
    assert.equal(await evaluate(client,`document.querySelector('.model-metadata-records').textContent.includes('"kind"')`),false);
    await geometry(width);await shot(`metadata-prices-${lang}-${theme}-${width}`);
    await click('[role=dialog] footer button',lang==='zh'?'关闭':'Close');
    await click('.model-feed-details button',lang==='zh'?'移除本机缓存…':'Remove local copy…');
    await waitFor(client,`!!document.querySelector('.model-feed-checkbox')`);
    assert.equal(await evaluate(client,`document.querySelector('[role=dialog] .btn-primary').disabled`),true);
    await click('[role=dialog] button',lang==='zh'?'继续查看影响':'Review more impacts');
    await waitFor(client,`!document.querySelector('[role=dialog] .btn-primary').disabled`);
    await geometry(width);await shot(`removal-reviewed-all-pages-${lang}-${theme}-${width}`);
    await click('[role=dialog] footer button',lang==='zh'?'取消':'Cancel');
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='model_feed_remove_apply').length`),0);
    results.push({id:`metadata.defaults-pages-impact.${lang}.${theme}.${width}`,status:'passed',scope:'production UI; mocked IPC'});
  }
} catch(error) {results.push({status:'failed',error:String(error)});process.exitCode=1;}
finally {await writeFile(path.join(output,'results.json'),JSON.stringify(results,null,2));clearInterval(keepAlive);client.close();}
