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
    assert.equal(await evaluate(client,`document.querySelector('#model-default-vision').value`),'custom');
    // Retained saved/exact-locator choices map to exactly two visible modes.
    assert.deepEqual(await evaluate(client,`[...document.querySelector('#model-default-native_reasoning').options].map(o=>o.value)`),['follow_metadata','custom']);
    for(const fallback of [false,true]) {
      await evaluate(client,`metadataFixture.automaticReasoning=true;metadataFixture.fallbackReasoning=${fallback};dispatchEvent(new Event('model-feed-changed'));true`);
      await waitFor(client,`document.querySelector('#model-default-native_reasoning')?.value==='follow_metadata'`);
      assert.equal(await evaluate(client,`document.querySelector('#model-default-native_reasoning').value`),'follow_metadata');
      assert.equal(await evaluate(client,`document.querySelector('#model-default-native_reasoning').closest('dd').querySelector('strong').textContent`),'low / medium / high / xhigh / max');
      await evaluate(client,`document.querySelector('#model-default-native_reasoning').scrollIntoView({block:'center'});true`);
      assert.equal(await evaluate(client,`(()=>{const r=document.querySelector('#model-default-native_reasoning').closest('.model-default-field').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight;})()`),true,'the affected reasoning row must be visible');
      await geometry(width);await shot(`reasoning-automatic-${fallback?'fallback':'missing'}-${lang}-${theme}-${width}`);
    }
    results.push({id:`metadata.reasoning-automatic.${lang}.${theme}.${width}`,status:'passed',scope:'production UI; mocked IPC; missing and derived fallback facts'});
    await evaluate(client,`metadataFixture.automaticReasoning=false;dispatchEvent(new Event('model-feed-changed'));true`);
    await waitFor(client,`document.querySelector('#model-default-native_reasoning')?.value==='follow_metadata'`);

    await select('#model-default-context_tokens','custom');
    await evaluate(client,`(()=>{const e=document.querySelector('[data-default-field=context_tokens] input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,'64000');e.dispatchEvent(new Event('input',{bubbles:true}));metadataFixture.failSave=true;return true;})()`);
    await click('.model-defaults-editor button',lang==='zh'?'保存修改':'Save changes');
    await waitFor(client,`!!document.querySelector('.model-defaults-editor [role=alert]')`);
    assert.deepEqual(await evaluate(client,`trialTrace.find(x=>x.command==='preview_compute_save_v3').payload.defaultPreferences['binding/bailian/qwen-coder'].context_tokens`),{intent:'custom',value:64000});
    assert.equal(await evaluate(client,`document.querySelector('#model-default-context_tokens').value`),'custom');
    assert.equal(await evaluate(client,`document.querySelector('[data-default-field=context_tokens] input').value`),'64000');
    await evaluate(client,`metadataFixture.fail=true;dispatchEvent(new Event('model-feed-changed'));true`);
    await waitFor(client,`!!document.querySelector('.model-defaults-editor [role=alert]')`);
    assert.equal(await evaluate(client,`document.querySelector('[data-default-field=context_tokens] input').value`),'64000');
    await shot(`defaults-error-retains-draft-${lang}-${theme}-${width}`);
    await evaluate(client,`metadataFixture.fail=false;true`);
    await click('.model-defaults-editor [role=alert] button',lang==='zh'?'重新读取':'Reload');
    await waitFor(client,`!document.querySelector('.model-defaults-editor [role=alert]')`);
    await geometry(width);await shot(`defaults-current-and-preserved-${lang}-${theme}-${width}`);
    // Native custom values are staged, invalid input submits nothing, and one Save carries the complete edits.
    const fill=async(selector,value)=>evaluate(client,`(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    await select('#model-default-tool','custom');await select('[data-default-field=tool] select:not([id])','false');
    await select('[data-default-field=vision] select:not([id])','unknown');
    await select('#model-default-max_output_tokens','custom');await fill('[data-default-field=max_output_tokens] input','4096');
    await select('#model-default-native_reasoning','custom');await select('.model-default-reasoning > select','discrete');
    await waitFor(client,`!!document.querySelector('[data-capability-profiles]')`);
    await fill('[data-capability-profiles]','low, low');
    const beforeSave=await evaluate(client,`trialTrace.filter(x=>x.command==='preview_compute_save_v3').length`);
    await click('.model-defaults-editor .btn-primary',lang==='zh'?'保存修改':'Save changes');
    await waitFor(client,`!!document.querySelector('.model-defaults-editor [role=alert]')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_compute_save_v3').length`),beforeSave);
    await fill('[data-capability-profiles]','low, high, ');
    assert.equal(await evaluate(client,`document.querySelector('[data-capability-profiles]').value`),'low, high, ');
    await select('.model-default-reasoning label select','low');
    await select('#model-default-context_tokens','follow_metadata');await select('#model-default-context_tokens','custom');
    assert.equal(await evaluate(client,`document.querySelector('[data-default-field=context_tokens] input').value`),'64000');
    await evaluate(client,`metadataFixture.failSave=false;document.querySelector('.model-defaults-editor .actions').scrollIntoView({block:'center'});true`);
    await geometry(width);await shot(`defaults-custom-native-${lang}-${theme}-${width}`);
    await evaluate(client,`(()=>{const b=document.querySelector('.model-defaults-editor .btn-primary');b.click();b.click();return true;})()`);
    await waitFor(client,`!document.querySelector('.model-defaults-editor .actions')`);
    const payloads=await evaluate(client,`trialTrace.filter(x=>x.command==='preview_compute_save_v3').map(x=>x.payload)`);
    assert.equal(payloads.length,beforeSave+1);
    assert.deepEqual(payloads.at(-1).defaultPreferences['binding/bailian/qwen-coder'],{context_tokens:{intent:'custom',value:64000},tool:{intent:'custom',value:false},vision:{intent:'custom',value:null},max_output_tokens:{intent:'custom',value:4096},native_reasoning:{intent:'custom',value:{kind:'discrete',parameter:'reasoning_effort',profiles:['low','high'],default_profile:'low'}}});
    assert.deepEqual(payloads.at(-1).change.key_edits,[]);
    results.push({id:`metadata.custom-save.${lang}.${theme}.${width}`,status:'passed',scope:'production UI; mocked save callback; invalid/retained/unknown/typed values and one Save'});
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
    await evaluate(client,`metadataFixture.linkFail=true;document.querySelector('.model-metadata-evidence a').dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}));true`);
    await waitFor(client,`!!document.querySelector('.model-metadata-link-error')`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-inferred').length`),4);
    await evaluate(client,`metadataFixture.linkFail=false;document.querySelector('.model-metadata-evidence a').dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}));true`);
    await waitFor(client,`!document.querySelector('.model-metadata-link-error')`);
    assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='open_external_url').length`),2);
    await geometry(width);await shot(`metadata-inferred-sources-${lang}-${theme}-${width}`);
    await client.send('Page.bringToFront');
    await evaluate(client,`document.querySelector('.model-metadata-evidence summary').focus();true`);
    assert.equal(await evaluate(client,`document.activeElement.tagName`),'SUMMARY');
    await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',text:'\r',windowsVirtualKeyCode:13});
    await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
    await waitFor(client,`!document.querySelector('.model-metadata-evidence').open`);
    await evaluate(client,`metadataFixture.stale=true;true`);
    await click('[role=dialog] button',lang==='zh'?'加载更多':'Load more');
    await waitFor(client,`trialTrace.filter(x=>x.command==='list_model_metadata').length>=3`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records li').length`),1);
    await click('[role=dialog] button',lang==='zh'?'加载更多':'Load more');
    await waitFor(client,`document.querySelectorAll('.model-metadata-records li').length===2`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records > li:nth-child(2) .model-metadata-inferred').length`),0);
    await evaluate(client,`metadataFixture.evidenceStale=true;true`);
    await click('.model-metadata-records > li:nth-child(2) summary',lang==='zh'?'来源与推断依据':'Sources and inference');
    await waitFor(client,`!!document.querySelector('.model-metadata-records > li:nth-child(2) [role=alert]')`);
    await click('.model-metadata-records > li:nth-child(2) button',lang==='zh'?'重新读取目录':'Reload directory');
    await waitFor(client,`document.querySelectorAll('.model-metadata-records li').length===1`);
    await click('[role=dialog] button',lang==='zh'?'加载更多':'Load more');
    await waitFor(client,`document.querySelectorAll('.model-metadata-records li').length===2`);
    await click('.model-metadata-records > li:nth-child(2) summary',lang==='zh'?'来源与推断依据':'Sources and inference');
    await waitFor(client,`!!document.querySelector('.model-metadata-records > li:nth-child(2) .model-metadata-source')`);
    assert.equal(await evaluate(client,`document.querySelectorAll('.model-metadata-records > li:nth-child(2) .model-metadata-rule').length`),0);
    assert.ok(await evaluate(client,`document.querySelector('.model-metadata-records > li:nth-child(2)').textContent.includes('source-079')`));
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
  await client.send('Page.navigate',{url:`${base}new-model-trial.html?metadata=1&transport=console&lang=en`});
  await click('.model-feed-actions button','Details');
  await click('.model-feed-details button','Browse all model information');
  await click('.model-metadata-records summary','Sources and inference');
  await waitFor(client,`!!document.querySelector('.model-metadata-rule')`);
  const prevented=await evaluate(client,`(()=>{let before=null;document.addEventListener('click',event=>{before=event.defaultPrevented;event.preventDefault();},{once:true});document.querySelector('.model-metadata-evidence a').dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}));return before;})()`);
  assert.equal(prevented,false,'web anchors retain their normal browser action; the harness alone cancels navigation');
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='open_external_url').length`),0);
  results.push({id:'metadata.sources.console.en',status:'passed',scope:'production UI; mocked IPC; no external navigation'});
} catch(error) {results.push({status:'failed',error:String(error)});process.exitCode=1;}
finally {await writeFile(path.join(output,'results.json'),JSON.stringify(results,null,2));clearInterval(keepAlive);client.close();}
