import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, waitFor } from './cdp-client.mjs';
import { trialAgentReadChecks } from './trial-agent-read-checks.mjs';
const [port,base,output]=process.argv.slice(2);
const client=await connectPage(Number(port));
const keepAlive=setInterval(()=>{},1000);
const results=[];
await client.send('Page.addScriptToEvaluateOnNewDocument', { source: `
  window.fixtureBrowserErrors = [];
  addEventListener('error', event => window.fixtureBrowserErrors.push({ type: event.type, message: event.message, file: event.filename, line: event.lineno, stack: event.error?.stack, target: event.target?.tagName, source: event.target?.src, href: event.target?.href }), true);
  addEventListener('unhandledrejection', event => window.fixtureBrowserErrors.push({ rejection: String(event.reason) }));
` });
const click=async(selector,text)=>{ const deadline=Date.now()+5000; for(;;){ const result=await evaluate(client,`(()=>{const el=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length && !e.matches(':disabled') && e.textContent.includes(${JSON.stringify(text)}));if(!el)return JSON.stringify([...document.querySelectorAll(${JSON.stringify(selector)})].map(e=>[e.textContent.trim().slice(0,40),e.getClientRects().length,e.matches(':disabled')]));el.click();return true;})()`); if(result===true)return; if(Date.now()>deadline)throw Error(`Missing control ${selector} "${text}": ${result}`); await new Promise(resolve=>setTimeout(resolve,50)); } };
const shot=async name=>{await evaluate(client,`Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>true)`);const value=await client.send('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,`${name}.png`),Buffer.from(value.data,'base64'));};
const hostTransport=process.env.HIROUTE_BROWSER_TRANSPORT??'tauri';
if(!['tauri','console'].includes(hostTransport))throw new Error('Unknown browser host transport');
async function open(query,width=755,height=600){const params=new URLSearchParams(query);if(!params.has('custom')&&!params.has('native'))params.set('native','1');if(hostTransport==='console')params.set('transport','console');const url=`${base}new-model-trial.html?${params}`;await client.send('Page.enable');await client.send('Runtime.enable');await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await client.send('Page.navigate',{url});await waitFor(client,`location.href===${JSON.stringify(url)} && !!document.querySelector('.models-feature,.plan-editor,.sessions-page,.trust-attention,.new-models')`);await evaluate(client,'document.fonts.ready.then(()=>true)');if(hostTransport==='console')assert.equal(await evaluate(client,'window.fixtureTransport'),'console');}
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const timelineReadsExpr=`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline').length`;
const timelineReads=()=>evaluate(client,timelineReadsExpr);
const setVisibility=state=>evaluate(client,`(()=>{Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>${JSON.stringify(state)}});document.dispatchEvent(new Event('visibilitychange'));return true;})()`);
const typeInto=(selector,value)=>evaluate(client,`(()=>{const input=document.querySelector(${JSON.stringify(selector)});if(!input)throw Error('Missing input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
async function geometry(selector='.model-feed-panel,.trust-control,.plan-quality,.trust-suggestion,.modal,.trial-wizard'){return evaluate(client,`(()=>{const nodes=[...document.querySelectorAll(${JSON.stringify(selector)})].filter(e=>e.getClientRects().length);return nodes.map(e=>({class:e.className,width:e.clientWidth,scroll:e.scrollWidth,left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right}));})()`);}
async function timelineFocusChecks() {
 const enter=async()=>{for(const type of ['keyDown','keyUp'])await client.send('Input.dispatchKeyEvent',{type,key:'Enter',code:'Enter',windowsVirtualKeyCode:13});};
 for(const [lang,theme,width,height] of [['en','light',755,600],['zh','dark',1200,813]]) {
  for(const kind of ['overview-keyboard','lane-keyboard','lane-mouse','outside-control','retained-turn','body']) {
   await open(`lang=${lang}&theme=${theme}&view=timeline&absorbed-turn=1`,width,height);
   await waitFor(client,`document.querySelectorAll('.routing-turn').length===2 && [...document.querySelectorAll('.routing-turn')].every(node=>node.getClientRects().length>0)`);
   const overview=kind==='overview-keyboard';
   const selector=overview?'[data-overview-turn="request-2"]':`.routing-turn[data-turn-id="${kind==='retained-turn'?'request-1':'request-2'}"]`;
   if(kind==='lane-mouse') {
    const point=await evaluate(client,`(()=>{const node=document.querySelector(${JSON.stringify(selector)});node.scrollIntoView({block:'center',inline:'center'});const r=node.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
    for(const type of ['mousePressed','mouseReleased'])await client.send('Input.dispatchMouseEvent',{type,...point,button:'left',clickCount:1});
   } else if(kind==='body') await evaluate(client,`document.querySelector(${JSON.stringify(selector)}).click()`);
   else {await evaluate(client,`document.querySelector(${JSON.stringify(selector)}).focus()`);await enter();}
   if(kind==='outside-control')await evaluate(client,`document.querySelector('.search-box input').focus()`);
   const before=kind==='outside-control'?'.search-box input':selector;
   assert.equal(await evaluate(client,kind==='body'?`document.activeElement===document.body`:`document.activeElement.matches(${JSON.stringify(before)})`),true,`${kind}.${lang}: establishes the real focused control before completion`);
   await waitFor(client,`document.querySelectorAll('.routing-turn').length===1`);
   await shot(`timeline-focus-${kind}-${lang}-${width}`);
   const expected=kind==='outside-control'?'.search-box input':overview?'[data-overview-turn="request-1"]':'.routing-turn[data-turn-id="request-1"]';
   assert.equal(await evaluate(client,kind==='body'?`document.activeElement===document.body`:`document.activeElement.matches(${JSON.stringify(expected)})`),true,`${kind}: completion transfers only an absorbed control's DOM focus`);
   assert.equal(await evaluate(client,`document.querySelector('.routing-turn-detail').textContent.includes(${JSON.stringify(lang==='zh'?'请求（2）':'Requests (2)')})`),true,'the selected owner retains both requests');
   if(kind.endsWith('keyboard'))assert.equal(await evaluate(client,`document.activeElement.matches(':focus-visible')`),true,'keyboard focus remains visible');
   results.push({id:`timeline.completion-focus.${kind}.${lang}.${width}`,status:'passed'});
  }
 }
}
async function timelinePricingChecks() {
 await timelineFocusChecks();
 await open('lang=en&view=timeline&absorbed-turn=1');
 await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
 await click('.routing-turn','Turn 2');
 await waitFor(client,`document.querySelectorAll('.routing-turn').length===1`);
 assert.equal(await evaluate(client,`document.querySelector('.routing-turn-detail').textContent.includes('Requests (2)')`),true,'selection follows the completed opening turn');
 assert.equal(await evaluate(client,`document.querySelector('.routing-tile:last-child strong').textContent`),'1','the absorbed row is not counted as another Agent turn');
 assert.equal(await evaluate(client,`${timelineReadsExpr}>1 && trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline').every((x,i)=>i===0||x.payload.request.intent.query.after_watermark!==null)`),true,'normal completion needs no failed read or full reload');
 await shot('timeline-completion-removes-provisional-row-en-755');
 results.push({id:'timeline.completion-removes-provisional-row-and-preserves-selection',status:'passed'});
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&view=timeline&partial-cost=1`);
  await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
  const subtotal=await evaluate(client,`document.querySelector('.routing-tile').textContent`);
  assert.ok(subtotal.includes('0.0013'),'known request amounts survive another request with unknown amount');
  assert.ok(subtotal.includes(lang==='zh'?'另有未知部分':'Plus an unknown portion'),'partial totals retain the gap');
  assert.ok(subtotal.includes(lang==='zh'?'API 等价估算':'API equivalent estimate'),'reference value is labeled as an estimate');
  await shot(`timeline-partial-reference-subtotal-${lang}-755`);
  results.push({id:`timeline.known-reference-subtotal-with-unknown-requests.${lang}`,status:'passed'});
 }
}
async function additionalAgentMappings() {
 for (const lang of ['en','zh']) for (const [family,name] of [['dsh','DeepSeek Harness'],['pi','Pi'],['qoder','Qoder'],['opencode','OpenCode']]) {
  await open(`lang=${lang}&view=quickstart&agent-family=${family}`,755,600);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-agent-choice select') && !!document.querySelector('.trial-presets input:not(:disabled)')`);
  await evaluate(client,`(()=>{const el=[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name));el.click();return true;})()`);
  if(lang==='zh') await evaluate(client,`document.querySelector('.trial-presets input:checked').click()`);
  await evaluate(client,`(()=>{const el=document.querySelector('.trial-agent-choice select');el.value=${JSON.stringify(`agent_${family}_default`)};el.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
  await waitFor(client,`!document.querySelector('.trial-presets')`);
  await evaluate(client,`(()=>{const el=[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name));el.click();return true;})()`);
  await waitFor(client,`!document.querySelector('.trial-footer .btn-primary').disabled`);
  assert.equal(await evaluate(client,`document.querySelector('.trial-footer .btn-primary').disabled`),false,'additional routing does not require Claude presets');
  await click('.trial-footer button',lang==='zh'?'检查改动':'Review changes');
  await waitFor(client,`!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
  await click('.trial-review button',lang==='zh'?'保存并发布路由':'Save and publish route');
  await waitFor(client,`!!document.querySelector('.trial-mapping .trial-agent-choice')`);
  await click('.trial-mapping button',lang==='zh'?'检查并映射 Agent':'Review and map Agent');
  await waitFor(client,`document.querySelector('.trial-mapping .callout.good')?.textContent.includes(${JSON.stringify(lang==='zh'?'映射已确认':'Mapping confirmed')})`);
  const label=await evaluate(client,`document.querySelector('.trial-mapping .callout.good strong').textContent`);
  assert.ok(label.includes(name)); assert.doesNotMatch(label,/opus|sonnet|haiku/i,'non-Claude confirmation never claims Claude presets');
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='preview_agent_settings').payload.input.spec.model.settings.mode`),`${family}_additional`);
  await shot(`quickstart-${family}-mapping-${lang}-755`);
  results.push({id:`quickstart.${family}.${lang}.no-claude-presets`,status:'passed'});
 }
}
async function trialIdentityChecks() {
 let criterion, rubric;
 const variants=['en','zh'].flatMap(lang=>['light','dark'].flatMap(theme=>[755,1200].map(width=>[lang,theme,width,width===755?600:813])));
 const enter=async selector=>{
  await evaluate(client,`document.querySelector(${JSON.stringify(selector)}).focus();true`);
  await client.send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13,text:'\r',unmodifiedText:'\r'});
  await client.send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
 };
 for (const [lang,theme,width,height] of variants) {
  await open(`lang=${lang}&theme=${theme}&view=quickstart&agent-family=dsh&no-reasoning-default=1&hold-writes=1`,width,height);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-agent-choice select')`);
  await evaluate(client,`(()=>{const e=document.querySelector('.trial-agent-choice select');e.value='agent_dsh_default';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
  await waitFor(client,`!!document.querySelector('.trial-choice-list .trial-choice')`);
  await evaluate(client,`[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name)).click();true`);
  await waitFor(client,`!document.querySelector('.trial-footer .btn-primary').disabled`);
  await enter('.trial-footer .btn-primary');
  assert.match(await evaluate(client,`document.querySelector('.trial-validation-summary').textContent`),/reasoning|思考/i,'a binding without a supported default requires an explicit choice');
  await enter('.trial-column .trial-reasoning button');
  await waitFor(client,`!!document.querySelector('[role=dialog] .segment')`);
  await enter('[role=dialog] .segment');
  assert.equal(await evaluate(client,`document.querySelector('[role=dialog] .segment').getAttribute('aria-pressed')`),'true','reasoning can be selected with the keyboard');
  await enter('[role=dialog] .btn-primary');
  await waitFor(client,`!document.querySelector('.trial-footer .btn-primary').disabled`);
  assert.match(await evaluate(client,`document.querySelector('.trial-page').textContent`),/trial-qwen3-coder-plus-[0-9a-f]{8}/);
  await shot(`trial-selection-${lang}-${theme}-${width}`);
  for(const node of await geometry('.trial-card,.trial-reasoning')) assert.ok(node.scroll<=node.width+1,`horizontal overflow: ${JSON.stringify(node)}`);
  await enter('.trial-footer .btn-primary');
  await waitFor(client,`!!document.querySelector('[data-dsh-capability-state="available"]') && !!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
  assert.equal(await evaluate(client,`document.querySelector('[data-codex-capability-state]')`),null,'DSH never shows Codex capability results');
  assert.ok(await evaluate(client,`trialTrace.some(x=>x.command==='plan_editor_options'&&x.payload.input.preview_agent==='dsh'&&x.payload.input.editor)`));
  await shot(`trial-review-${lang}-${theme}-${width}`);
  await evaluate(client,`document.querySelector('.trial-review > .actions .btn-primary').scrollIntoView({block:'center'});true`);
  await shot(`trial-review-action-${lang}-${theme}-${width}`);
  const before=await evaluate(client,`(()=>{const r=document.querySelector('.trial-review > .actions .btn-primary').getBoundingClientRect();return {left:r.left,top:r.top};})()`);
  await enter('.trial-review > .actions .btn-primary');
  await waitFor(client,`trialWrites.pending.length===1`);
  const during=await evaluate(client,`(()=>{const r=document.querySelector('.trial-review > .actions .btn-primary').getBoundingClientRect();return {left:r.left,top:r.top};})()`);
  assert.ok(Math.abs(before.left-during.left)<=2 && Math.abs(before.top-during.top)<=2,`saving moved the action: ${JSON.stringify({before,during})}`);
  await evaluate(client,`trialWrites.pending[0].release();true`);
  await waitFor(client,`trialWrites.pending.length===2`);
  assert.equal(await evaluate(client,`trialWrites.pending[1].action`),'publish');
  await evaluate(client,`trialWrites.pending[1].release();true`);
  await waitFor(client,`!!document.querySelector('.trial-mapping')`);
  const writes=await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor').map(x=>x.payload.input)`);
  assert.deepEqual(writes.map(write=>write.action),['save_draft','publish']);
  assert.equal(writes[0].editor.smart.primary[0].reasoning.profile,'low');
  assert.equal(writes[1].editor.smart.primary[0].reasoning.profile,'low');
  assert.equal(writes[0].editor.smart.judgment.competence.standard,'custom');
  const current=writes[0].editor.smart.judgment.competence.criteria[0];
  assert.match(current,/clear user dissatisfaction/);
  if (criterion === undefined) criterion=current; else assert.equal(current,criterion,'the stored model-facing criterion is identical in both UI languages');
  if (rubric === undefined) rubric=writes[0].editor.smart.judgment; else assert.deepEqual(writes[0].editor.smart.judgment,rubric,'all model-facing rubric instructions and criteria are identical across UI languages');
  results.push({id:`quickstart.identity-preview-publication.${lang}.${theme}.${width}`,status:'passed'});
 }
 for (const [lang,theme,width,height] of variants) {
  await open(`lang=${lang}&theme=${theme}&view=quickstart&agent-family=dsh&same-names=1`,width,height);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-agent-choice select') && !!document.querySelector('.trial-connection-field select')`);
  await evaluate(client,`(()=>{const e=document.querySelector('.trial-agent-choice select');e.value='agent_dsh_default';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
  const options=await evaluate(client,`[...document.querySelector('.trial-connection-field select').options].slice(1).map(e=>({label:e.textContent,value:e.value}))`);
  assert.equal(options.length,2); assert.notEqual(options[0].label,options[1].label,'same-source connections have distinguishable labels');
  await evaluate(client,`(()=>{const e=document.querySelector('.trial-connection-field select');e.value='binding/bailian/qwen-second';e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
  const showAll=await evaluate(client,`!!document.querySelector('.trial-choice-list .trial-inline-action')`);
  if(showAll) await evaluate(client,`document.querySelector('.trial-choice-list .trial-inline-action').click();true`);
  await waitFor(client,`[...document.querySelectorAll('.trial-choice-list .trial-choice')].some(e=>e.textContent.includes('A second provider'))`);
  const names=await evaluate(client,`[...document.querySelectorAll('.trial-choice-list .trial-choice')].filter(e=>e.querySelector('strong')?.textContent===primaryModel.display_name).map(e=>e.textContent)`);
  assert.equal(names.length,2); assert.notEqual(names[0],names[1],'same model names show their distinct connection source');
  await evaluate(client,`document.querySelector('.trial-choice-list').scrollIntoView({block:'center'});true`);
  await shot(`trial-same-name-options-${lang}-${theme}-${width}`);
  await evaluate(client,`[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes('A second provider')).click();true`);
  await waitFor(client,`!document.querySelector('.trial-footer .btn-primary').disabled`);
  for(const node of await geometry('.trial-choice,.trial-connection-field')) assert.ok(node.scroll<=node.width+1,`horizontal overflow: ${JSON.stringify(node)}`);
  await shot(`trial-same-names-${lang}-${theme}-${width}`);
  await click('.trial-footer button',lang==='zh'?'检查改动':'Review changes');
  await waitFor(client,`!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
  assert.ok(await evaluate(client,`document.querySelector('.trial-review').textContent.includes('A second provider')`));
  await click('.trial-review button',lang==='zh'?'保存并发布路由':'Save and publish route');
  await waitFor(client,`!!document.querySelector('.trial-mapping')`);
  const writes=await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor').map(x=>x.payload.input.editor)`);
  assert.equal(writes.length,2);
  for(const editor of writes) { assert.equal(editor.smart.economy[0].binding_id,'binding/bailian/qwen-second'); assert.equal(editor.smart.primary[0].binding_id,'binding/second/same-name'); }
  results.push({id:`quickstart.same-name-distinct-bindings.${lang}.${width}`,status:'passed'});
 }
}
try {
 if(process.argv.includes('--trial-identity')) { await trialIdentityChecks(); }
 else if(process.argv.includes('--trial-agent-reads')) { await trialAgentReadChecks({client,open,click,shot,results}); }
 else if(process.argv.includes('--timeline-pricing')) { await timelinePricingChecks(); }
 else {
 await trialAgentReadChecks({client,open,click,shot,results});
 await additionalAgentMappings();
 for(const [lang,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]) {
  await open(`lang=${lang}&theme=${theme}&view=feed-home`,width,height);
  await waitFor(client,`!!document.querySelector('.new-model-card')`);
  const homeCopy=await evaluate(client,`document.querySelector('.new-models').textContent`);
  assert.ok(homeCopy.includes(lang==='zh'?'内容已校验':'Content validated'));
  assert.doesNotMatch(homeCopy,/signature|签名/i);
  await shot(`feed-origin-home-${lang}-${theme}-${width}`);
  await open(`lang=${lang}&theme=${theme}&view=quickstart&feed-trial=1`,width,height);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-page .trial-agent-choice input:not(:disabled)')`);
  const wizardCopy=await evaluate(client,`document.querySelector('.trial-page').textContent`);
  assert.ok(wizardCopy.includes(lang==='zh'?'内容已校验':'content validated'));
  assert.doesNotMatch(wizardCopy,/signature|签名/i);
  await evaluate(client,`[...document.querySelectorAll('.trial-page .field-help')].find(e=>e.textContent.includes('hiroute.ai')&&e.textContent.includes(${JSON.stringify(lang==='zh'?'内容已校验':'content validated')})).scrollIntoView({block:'center'});true`);
  await shot(`feed-origin-trial-${lang}-${theme}-${width}`);
  assert.ok(await evaluate(client,`[...document.querySelectorAll('.trial-page .field-help')].filter(e=>e.textContent.includes('hiroute.ai')).every(e=>e.scrollWidth<=e.clientWidth+1)`));
  results.push({id:`feed.origin-content-validation.${lang}.${theme}.${width}`,status:'passed'});
 }
 for (const lang of ['en','zh']) for (const native of [true,false]) for (const width of [755,1200]) for(const theme of ['light','dark']) {
  const query=`lang=${lang}&theme=${theme}&${native?'native':'custom'}=1`, height=width===755?600:813;
  const expectations=native ? (lang==='zh'?['问题仍相关','无法判断','问题已修正或无关']:['earlier problem still matters','cannot be checked','fixed or unrelated']) : (lang==='zh'?['扩展','选择','可选评分','观察','不应用阈值或低分保护']:['extension chooses','Optional scores','observation only','no threshold or low-score protection']);
  const noCustomPromises=text=>assert.doesNotMatch(text,/P\(simple\)|safety net|threshold \d|low-competence protection|Primary protection|catches what it misses|next (?:agent )?turn goes to|安全网|阈值 [\d.]|低胜任度保护|主力保护|下一轮交给/i);
  const check=async selector=>{
   const text=await evaluate(client,`document.querySelector(${JSON.stringify(selector)})?.textContent`);
   for(const expected of expectations) assert.ok(text?.includes(expected),`${selector}: ${expected}: ${text}`);
   if(!native) noCustomPromises(text);
  };
  await open(`${query}&view=editor&division=1&review-created-draft=1`,width,height);
  await waitFor(client,`!!document.querySelector('.decision-selector')`);
  if(native) {await check('.trust-safety-net');await evaluate(client,`document.querySelector('.trust-safety-net').scrollIntoView({block:'center'});true`);}
  else assert.equal(await evaluate(client,`document.querySelectorAll('.trust-control,.trust-safety-net,.judgment-fields').length`),0,'custom editor has no built-in judgment controls');
  await shot(`protection-${native?'native':'custom'}-${lang}-${width}-${theme}`);
  await click('[role=tab]',lang==='zh'?'模型表现':'Model performance');
  if(native) {await waitFor(client,`!!document.querySelector('.trust-suggestion.cyan')`);await click('.trust-suggestion button',lang==='zh'?'查看分工草稿':'Review division draft');}
  else {
   await waitFor(client,`!!document.querySelector('.quality-footer-links button')`);
   assert.equal(await evaluate(client,`document.querySelectorAll('.trust-suggestion').length`),0,'manual custom division needs no invented threshold suggestion');
   await click('.quality-footer-links button',lang==='zh'?'创建分工草稿':'Create a division draft');
  }
  await waitFor(client,`!!document.querySelector('.division-draft-form input[type=checkbox]')`);
  await evaluate(client,`document.querySelector('.division-draft-form input[type=checkbox]').click();true`);
  await check('.division-protection');
  if(!native) noCustomPromises(await evaluate(client,`document.querySelector('.division-draft-form').textContent`));
  await evaluate(client,`document.querySelector('.division-protection').scrollIntoView({block:'center'});true`);
  await shot(`division-protection-${native?'native':'custom'}-${lang}-${width}-${theme}`);
  if(!native) {
   await click('[role=dialog] .btn-primary',lang==='zh'?'在编辑器中查看草稿':'Review draft in editor');
   await waitFor(client,`!!document.querySelector('.division-protection-summary')`);
   await check('.division-protection-summary');
   const summary=await evaluate(client,`document.querySelector('.division-protection-summary').textContent`);
   assert.ok(summary.includes(lang==='zh'?'仅有一个组':'one group'),'single-group category does not promise a Primary choice');
   assert.equal(await evaluate(client,`document.querySelectorAll('.trust-control,.judgment-fields').length`),0);
   assert.equal(await evaluate(client,`trialTrace.filter(x=>['preview_plan_editor','preview_agent_settings'].includes(x.command)).length`),0,'opening a division draft neither saves/publishes nor changes Agent mappings');
   await evaluate(client,`document.querySelector('.division-protection-summary').scrollIntoView({block:'center'});true`);
   await shot(`division-custom-summary-${lang}-${width}-${theme}`);
  }
  await open(`${query}&view=quickstart`,width,height);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-page .trial-agent-choice input:not(:disabled)')`);
  await check('.trial-rule');
  await evaluate(client,`document.querySelector('.trial-rule').scrollIntoView({block:'center'});true`);
  await shot(`trial-protection-${native?'native':'custom'}-${lang}-${width}-${theme}`);
  if(!native) {
   noCustomPromises(await evaluate(client,`document.querySelector('.trial-page').textContent`));
   assert.equal(await evaluate(client,`document.querySelectorAll('.trial-level,.trial-criteria,.trust-control').length`),0,'custom quick start has no built-in level or assessment rubric');
   await evaluate(client,`(()=>{const el=[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name));el.click();return true;})()`);
   await click('.trial-footer button',lang==='zh'?'检查改动':'Review changes');
   await waitFor(client,`!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
   noCustomPromises(await evaluate(client,`document.querySelector('.trial-page').textContent`));
   assert.equal(await evaluate(client,`document.querySelectorAll('.trust-control,.judgment-fields').length`),0);
   await click('.trial-review button',lang==='zh'?'保存并发布路由':'Save and publish route');
   await waitFor(client,`trialTrace.some(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft')`);
   const saved=await evaluate(client,`trialTrace.find(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft').payload.input.editor`);
   assert.equal(saved.smart.classifier.service.connection.kind,'custom');
   assert.deepEqual(saved.smart.judgment,await evaluate(client,`quickStartAuthoringJudgment`),'custom quick start preserves parked judgment instead of installing a native trial threshold or rubric');
   noCustomPromises(saved.purpose);
  }
  for(const node of await geometry('.division-protection-summary,.division-protection,.trial-rule,.trial-card')) assert.ok(node.scroll<=node.width+1,`horizontal overflow: ${JSON.stringify(node)}`);
  results.push({id:`protection.${native?'native-three-outcomes':'custom-observation-only'}.${lang}.${width}.${theme}`,status:'passed'});
 }
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&custom=1&single-group=1&view=editor`,755,600);
  await click('[role=tab]',lang==='zh'?'模型表现':'Model performance');
  await waitFor(client,`trialTrace.some(x=>x.command==='observation_read'&&x.payload.request.intent.view==='plan_trust') && !!document.querySelector('.quality-tables')`);
  assert.equal(await evaluate(client,`document.querySelectorAll('.quality-footer-links button,.trust-suggestion').length`),0,'a custom route without two groups has no invalid division entry');
  results.push({id:`division.custom-no-eligible-group.${lang}`,status:'passed'});
 }
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&view=handoff`,755,600);
  await waitFor(client,`trialTrace.some(x=>x.command==='observation_read' && x.payload.request.intent.view==='sessions')`);
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='observation_read').payload.request.intent.query.from_ms===readCostState.confirmed`),true);
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='observation_read').payload.request.intent.query.plan_id`),'plan/trial');
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='observation_read').payload.request.intent.query.agent_id`),null,'a Plan match does not require Agent attribution');
  assert.equal(await evaluate(client,`!!document.querySelector('.trial-session-handoff .btn-primary')`),false,'activity before confirmation is excluded');
  await shot(`handoff-waiting-${lang}-755`);
  await evaluate(client,`hideReadCost(true);tickReadCost(5000);true`);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read').length`),1);
  await evaluate(client,`readCostState.fail=true;hideReadCost(false);true`);
  await waitFor(client,`!!document.querySelector('.trial-session-handoff [role=alert]')`);
  await shot(`handoff-error-${lang}-755`);
  await evaluate(client,`readCostState.fail=false;readCostState.found=true;true`);
  await click('.trial-session-handoff button',lang==='zh'?'重试':'Retry');
  await waitFor(client,`!!document.querySelector('.trial-session-handoff .btn-primary')`);
  await shot(`handoff-found-${lang}-755`);
  const foundReads=await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read').length`);
  await evaluate(client,`tickReadCost(5000);true`);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read').length`),foundReads);
  await click('.trial-session-handoff button',lang==='zh'?'观察此会话':'Watch this session');
  await waitFor(client,`trialTrace.some(x=>x.command==='observation_read' && x.payload.request.intent.view==='routing_timeline' && x.payload.request.intent.query.session_id==='trial-found')`);
  results.push({id:`handoff.wait-hide-error-found-navigate.${lang}`,status:'passed'});
 }
 await open('lang=en&view=handoff');await waitFor(client,`trialTrace.some(x=>x.command==='observation_read')`);await click('button','Close waiting');
 const closedReads=await evaluate(client,`trialTrace.length`);await evaluate(client,`tickReadCost(5000);true`);assert.equal(await evaluate(client,`trialTrace.length`),closedReads);
 results.push({id:'handoff.close-stops-reads',status:'passed'});
 await open('lang=en&view=handoff&hold-session-read=1');
 await waitFor(client,`readCostState.pending.length===1`);
 await evaluate(client,`tickReadCost(5000);tickReadCost(5000);true`);
 assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read').length`),1,'a pending session read is never overlapped by the five-second timer');
 await evaluate(client,`readCostState.hold=false;readCostState.pending[0]();true`);
 await waitFor(client,`readCostState.completed===1`);
 await evaluate(client,`tickReadCost(5000);true`);
 await waitFor(client,`trialTrace.filter(x=>x.command==='observation_read').length===2`);
 results.push({id:'handoff.single-in-flight-five-second-cadence',status:'passed'});
 await open('lang=en&view=suggestion-cadence');await waitFor(client,`!!document.querySelector('.trust-attention-item')`);
 const suggestionReads=()=>evaluate(client,`trialTrace.filter(x=>x.command==='observation_read' && x.payload.request.intent.view==='plan_trust_suggestions').length`);
 assert.equal(await suggestionReads(),1);await evaluate(client,`tickReadCost(30000);true`);assert.equal(await suggestionReads(),1);
 await evaluate(client,`tickReadCost(300000);true`);await waitFor(client,`trialTrace.length===2`);assert.equal(await suggestionReads(),2);
 await click('button','Publish fixture');await waitFor(client,`trialTrace.length===3`);assert.equal(await suggestionReads(),3);
 await click('button','Toggle Home');assert.equal(await suggestionReads(),3);await click('button','Toggle Home');await waitFor(client,`trialTrace.length===4`);
 await evaluate(client,`hideReadCost(true);tickReadCost(300000);true`);assert.equal(await suggestionReads(),4);await evaluate(client,`hideReadCost(false);true`);await waitFor(client,`trialTrace.length===5`);
 assert.ok(await evaluate(client,`document.querySelector('.trust-attention [role=alert]').textContent.includes('Other suggestions remain available')`));
 assert.equal(await evaluate(client,`document.querySelector('.trust-attention-item button').disabled`),false);await shot('suggestion-partial-failure-en-755');
 results.push({id:'suggestions.five-minute-cadence-publish-home-visible-partial-failure',status:'passed'});
 if(!process.env.TRIAL_QUICK_ONLY) {
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&view=feed-home`); await waitFor(client,`!!document.querySelector('.model-feed-disclosure')`);
  assert.ok(await evaluate(client,`document.querySelector('.model-feed-disclosure').textContent.includes('hiroute.ai')`));
  await shot(`feed-first-home-${lang}-755`);
  await click('.model-feed-disclosure button',lang==='zh'?'关闭自动获取':'Turn off automatic retrieval');
  await waitFor(client,`!document.querySelector('.model-feed-disclosure')`);
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='model_feed_settings_apply').payload.input.retrieval_enabled`),false);
  results.push({id:`feed.first-home-disclosure.${lang}`,status:'passed'});
 }
 await open('lang=en&recovery=1'); await waitFor(client,`document.body.textContent.includes('Saved feed state is damaged') && !!document.querySelector('.model-feed-details')`);
 await click('button','Reset damaged cache…'); await waitFor(client,`!!document.querySelector('[role=dialog]')`); await shot('feed-reset-damaged-cache-en-755');
 assert.ok(await evaluate(client,`document.querySelector('[role=dialog]').textContent.includes('damaged state file will be preserved')`));
 await click('[role=dialog] button','Confirm reset'); await waitFor(client,`!document.querySelector('[role=dialog]')`);
 assert.equal(await evaluate(client,`[...document.querySelectorAll('.model-feed-actions button')].find(b=>b.textContent.includes('Refresh')).disabled`),false);
 results.push({id:'feed.confirmed-corrupt-state-recovery',status:'passed'});
 await open('lang=en&view=timeline&scope-error=1'); await waitFor(client,`!!document.querySelector('.routing-timeline [role=alert]')`);
 const scopeReads=await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline').length`);
 await evaluate(client,`refreshTrialTimeline();true`); await evaluate(client,`hideTrialTimeline(true);true`); await evaluate(client,`hideTrialTimeline(false);true`);
 await new Promise(resolve=>setTimeout(resolve,2200));
 assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline').length`),scopeReads);
 await click('.routing-timeline button','Retry'); await waitFor(client,`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline').length===${scopeReads+1}`);
 results.push({id:'timeline.scope-error-stops-until-explicit-retry',status:'passed'});
 await open('lang=en&view=timeline&busy-read=1'); await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
 assert.equal(await evaluate(client,`document.querySelector('.routing-timeline [role=alert]')`),null);
 assert.ok(await evaluate(client,`(()=>{const calls=trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline');return calls[1].at-calls[0].at<1000;})()`));
 results.push({id:'timeline.busy-retries-quietly',status:'passed'});
 for (const lang of ['en','zh']) for (const width of [755,1200]) {
  await open(`lang=${lang}&view=editor&cost=1`,width,width===755?600:813);
  await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
  await click('[role=tab]',lang==='zh'?'模型表现':'Model performance');
  await waitFor(client,`!!document.querySelector('.trust-cost [data-cost-metric="difference"]')`);
  await evaluate(client,`(()=>{document.querySelector('.occami-root')?.setAttribute('data-theme',${JSON.stringify(width===755?'light':'dark')});document.querySelector('.trust-cost').scrollIntoView({block:'center'});return true;})()`);
  const state=await evaluate(client,`(()=>{const card=document.querySelector('.trust-cost'),row=key=>card.querySelector('[data-cost-metric="'+key+'"] strong');return {routed:row('routed').innerText,reference:row('reference').innerText,difference:row('difference').innerText,tone:row('difference').className,open:card.querySelector('.trust-cost-details').open,text:card.innerText};})()`);
  assert.equal(state.routed,'$0.0180'); assert.equal(state.reference,'$0.0120');
  assert.equal(state.difference,'+$0.0060'); assert.equal(state.tone,'more');
  assert.equal(state.open,false,'calculation details start collapsed');
  assert.ok(!state.text.includes(lang==='zh'?'可精确比较':'Exactly comparable'));
  for(const g of await geometry('.trust-cost,.trust-cost-row,.trust-cost-details')){assert.ok(g.scroll<=g.width+2,JSON.stringify(g));assert.ok(g.left>=-1&&g.right<=width+1,JSON.stringify(g));}
  await shot(`trust-cost-positive-${lang}-${width}`);
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`document.querySelector('.trust-cost-details').open`);
  const details=await evaluate(client,`document.querySelector('.trust-cost-details .disclosure-body').innerText`);
  assert.ok(details.includes('3/3'));
  assert.ok(details.includes(lang==='zh'?'仅已知配对请求的小计':'Known paired-request subtotals only'));
  assert.ok(details.includes('$0.0180 / $0.0120'));
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`!document.querySelector('.trust-cost-details').open`);
  results.push({id:`trust.cost-positive-difference-disclosure.${lang}.${width}`,status:'passed'});
 }
 for (const lang of ['en','zh']) for (const width of [755,1200]) for (const kind of ['partial','unknown']) {
  await open(`lang=${lang}&view=editor&cost-${kind}=1`,width,width===755?600:813);
  await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
  await click('[role=tab]',lang==='zh'?'模型表现':'Model performance');
  await waitFor(client,`!!document.querySelector('.trust-cost [data-cost-metric="difference"]')`);
  await evaluate(client,`(()=>{document.querySelector('.occami-root')?.setAttribute('data-theme',${JSON.stringify(width===755?'light':'dark')});document.querySelector('.trust-cost').scrollIntoView({block:'center'});return true;})()`);
  const state=await evaluate(client,`(()=>{const card=document.querySelector('.trust-cost'),row=key=>card.querySelector('[data-cost-metric="'+key+'"] strong').innerText;return {routed:row('routed'),reference:row('reference'),difference:row('difference'),open:card.querySelector('.trust-cost-details').open,text:card.innerText};})()`);
  assert.equal(state.routed,lang==='zh'?'未知':'Unknown'); assert.equal(state.reference,lang==='zh'?'未知':'Unknown');
  assert.equal(state.difference,lang==='zh'?'差额未知':'Difference unknown');
  assert.equal(state.open,false,'calculation details start collapsed');
  assert.ok(!state.text.includes(lang==='zh'?'可精确比较':'Exactly comparable'));
  for(const g of await geometry('.trust-cost,.trust-cost-row,.trust-cost-details')){assert.ok(g.scroll<=g.width+2,JSON.stringify(g));assert.ok(g.left>=-1&&g.right<=width+1,JSON.stringify(g));}
  await shot(`trust-cost-gaps-${kind}-${lang}-${width}`);
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`document.querySelector('.trust-cost-details').open`);
  const details=await evaluate(client,`document.querySelector('.trust-cost-details .disclosure-body').innerText`);
  const missing=kind==='partial'?2:3;
  assert.ok(details.includes(`${kind==='partial'?1:0}/3`));
  assert.ok(details.includes(lang==='zh'?`缺口 ${missing}`:`${missing} missing`));
  assert.ok(details.includes(lang==='zh'?`排除 ${missing} 个请求，已包含在缺口数量中`:`${missing} excluded requests are included in the missing count`));
  assert.equal(details.includes(lang==='zh'?'仅已知配对请求的小计':'Known paired-request subtotals only'),kind==='partial');
  assert.equal(details.includes('$0.0060 / $0.0040'),kind==='partial');
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`!document.querySelector('.trust-cost-details').open`);
  results.push({id:`trust.cost-gaps.${kind}.${lang}.${width}`,status:'passed'});
 }
 for (const lang of ['en','zh']) for (const width of [755,1200]) {
  await open(`lang=${lang}&view=editor&cost-routed-only=1`,width,width===755?600:813);
  await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);
  await click('[role=tab]',lang==='zh'?'模型表现':'Model performance');
  await waitFor(client,`!!document.querySelector('.trust-cost [data-cost-metric="difference"]')`);
  await evaluate(client,`(()=>{document.querySelector('.occami-root')?.setAttribute('data-theme',${JSON.stringify(width===755?'light':'dark')});document.querySelector('.trust-cost').scrollIntoView({block:'center'});return true;})()`);
  const state=await evaluate(client,`(()=>{const card=document.querySelector('.trust-cost'),row=key=>card.querySelector('[data-cost-metric="'+key+'"] strong');return {routed:row('routed').innerText,reference:row('reference').innerText,difference:row('difference').innerText,tone:row('difference').className,open:card.querySelector('.trust-cost-details').open,text:card.innerText};})()`);
  assert.equal(state.routed,'$0.0180');
  assert.ok(state.reference.includes(lang==='zh'?'$0.0928 至 $0.1158':'$0.0928 to $0.1158'));
  assert.ok(state.difference.includes(lang==='zh'?'−$0.0978 至 −$0.0748':'−$0.0978 to −$0.0748'));
  assert.ok(state.reference.includes(lang==='zh'?'范围':'Range') && state.difference.includes(lang==='zh'?'范围':'Range'));
  assert.equal(state.tone,'less'); assert.equal(state.open,false,'calculation details start collapsed');
  assert.ok(!state.text.includes(lang==='zh'?'差额未知':'Difference unknown'));
  assert.ok(state.text.includes(lang==='zh'?'不是账单，也不是实际节省':'not a bill or an actual cash saving'));
  assert.doesNotMatch(state.text,/you (?:saved|save)|cash saved|已节省|节省了/i);
  for(const g of await geometry('.trust-cost,.trust-cost-row,.trust-cost-details')){assert.ok(g.scroll<=g.width+2,JSON.stringify(g));assert.ok(g.left>=-1&&g.right<=width+1,JSON.stringify(g));}
  await shot(`trust-cost-routed-only-${lang}-${width}`);
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`document.querySelector('.trust-cost-details').open`);
  const details=await evaluate(client,`document.querySelector('.trust-cost-details .disclosure-body').innerText`);
  assert.ok(details.includes('0/1'));
  assert.ok(details.includes(lang==='zh'?'缺口 1':'1 missing'));
  assert.ok(details.includes('92880 / 115738 · -97738 / -74880'));
  assert.ok(details.includes(lang==='zh'?'未上报缓存写入量的上下限':'possible unreported cache-write amount'));
  assert.ok(!details.includes(lang==='zh'?'仅已知配对请求的小计':'Known paired-request subtotals only'));
  await click('.trust-cost-details summary',lang==='zh'?'计算方式':'Calculation details');
  await waitFor(client,`!document.querySelector('.trust-cost-details').open`);
  results.push({id:`trust.cost-routed-only.${lang}.${width}`,status:'passed'});
 }

 for(const [lang,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]){
  await open(`lang=${lang}&theme=${theme}`,width,height);
  await waitFor(client,`!!document.querySelector('.model-feed-actions')`);
  await click('.model-feed-actions button',lang==='zh'?'详情':'Details');
  await waitFor(client,`!!document.querySelector('.model-feed-details')`);
  const feedText = await evaluate(client,`document.querySelector('.model-feed-panel').textContent`);
  assert.ok(feedText.includes(lang==='zh'?'内容已校验':'Content validated'));
  assert.ok(!/signature|签名/i.test(feedText));
  await shot(`feed-${lang}-${theme}-${width}`);
  for(const g of await geometry()){assert.ok(g.scroll<=g.width+1,JSON.stringify(g));assert.ok(g.left>=-1 && g.right<=width+1,JSON.stringify(g));}
  await click('button',lang==='zh'?'移除本机缓存…':'Remove local copy…');
  await waitFor(client,`!!document.querySelector('[role="dialog"]')`);
  assert.equal(await evaluate(client,`document.querySelector('.model-feed-checkbox input').checked`),true);
  await shot(`feed-removal-${lang}-${theme}-${width}`);
  await click('[role="dialog"] button',lang==='zh'?'取消':'Cancel');
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='model_feed_remove_apply').length`),0);
  results.push({id:`feed.review.${lang}.${theme}.${width}`,status:'passed'});
 }
 await open('lang=en&stale=1');await waitFor(client,`!!document.querySelector('.model-feed-actions')`);await click('.model-feed-actions button','Details');await click('button','Remove local copy…');await waitFor(client,`!!document.querySelector('[role="dialog"]')`);await click('[role="dialog"] .btn-primary','Remove');await waitFor(client,`document.body.textContent.includes('Review the impact again')`);assert.equal(await evaluate(client,`document.querySelector('[role="dialog"]')===null`),true);results.push({id:'feed.stale-removal.repreview',status:'passed'});
 await open('lang=en&view=editor&empty=1&native=1');await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);assert.equal(await evaluate(client,`[...document.querySelectorAll('.trust-levels button')].some(e=>e.getAttribute('aria-pressed')==='true')`),false);assert.ok(await evaluate(client,`document.querySelector('.trust-control').textContent.includes('0.72') && !document.querySelector('.trust-control').textContent.includes('Custom')`));assert.ok(await evaluate(client,`!!document.querySelector('.trust-control').textContent.includes('No recorded evidence')`));await evaluate(client,`document.querySelector('.trust-control').scrollIntoView({block:'center'});true`);await shot('trust-custom-empty-en-755');await click('.trust-levels button','Trusted');assert.ok(await evaluate(client,`!!document.querySelector('.trust-control').textContent.includes('0.65')`));assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);results.push({id:'trust.custom-threshold.explicit-change',status:'passed'});
 await open('lang=en&view=editor&replay-error=1&native=1');await waitFor(client,`document.querySelector('.trust-replay')?.textContent.includes('Replay could not be read')`);await click('.trust-replay button','Retry');await waitFor(client,`document.querySelector('.trust-replay')?.textContent.includes('At 0.72')`);await click('.trust-levels button','Trusted');await waitFor(client,`document.querySelector('.trust-replay')?.textContent.includes('At 0.65')`);assert.ok(await evaluate(client,`trialTrace.some(x=>x.command==='observation_read'&&x.payload.request.intent.view==='plan_trust_replay'&&x.payload.request.intent.query.thresholds_millis.includes(650)&&x.payload.request.intent.query.branch_id==='smart_saving')`));assert.ok(await evaluate(client,`document.querySelector('.trust-replay').textContent.includes('Built-in rules 2')`));await evaluate(client,`document.querySelector('.trust-replay').scrollIntoView({block:'center'});true`);await shot('replay-threshold-650-en');results.push({id:'replay.custom-threshold.retry-and-exclusions',status:'passed'});
 await open('lang=zh&view=editor&theme=dark',1200,813);await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);await click('[role="tab"]','模型表现');await waitFor(client,`!!document.querySelector('.quality-model-row') && !!document.querySelector('.trust-suggestion')`);await shot('trust-evidence-zh-dark-1200');for(const g of await geometry())assert.ok(g.scroll<=g.width+1,JSON.stringify(g));assert.ok(await evaluate(client,`document.querySelector('.trust-suggestion').textContent.includes('完整评分阶段 41（要求 ≥30）')`));assert.ok(await evaluate(client,`document.querySelector('.trust-scope-notes').textContent.includes('2026-10-10')`));assert.equal(await evaluate(client,`/序号|发布修订版|client_version/.test(document.querySelector('.plan-quality').textContent)`),false);results.push({id:'trust.coverage-and-version-disclosure',status:'passed'});
 for (const [lang,theme,width,height] of [['zh','dark',1200,813],['en','light',755,600]]) {
  await open(`lang=${lang}&view=editor&theme=${theme}`,width,height);await waitFor(client,`!!document.querySelector('[data-trust-control="smart"]')`);await click('[role="tab"]',lang==='zh'?'模型表现':'Model performance');
  const regular=`document.querySelector('.quality-model-row[data-model-configuration="qwen3-coder-plus"] .quality-model-counts')`;
  await waitFor(client,`!!${regular}?.querySelector('.quality-stage-states')`);
  for(const part of lang==='zh'?['评分覆盖 71%','41 完整评分','4 部分评分','1 等待评分','12 未评分']:['71% scoring coverage','41 complete','4 partial','1 awaiting assessment','12 unscored']) assert.ok(await evaluate(client,`${regular}.textContent.includes(${JSON.stringify(part)})`),`model view: ${part}`);
  await evaluate(client,`${regular}.scrollIntoView({block:'center'});true`);await shot(`trust-scoring-states-model-${lang}-${width}`);
  await click('.quality-tables-head button',lang==='zh'?'按执行 Agent':'By execution agent');
  const executor=`document.querySelector('[data-execution-group="executor"] .quality-model-row')`;
  await waitFor(client,`!!${executor}?.querySelector('.quality-stage-states')`);
  for(const part of lang==='zh'?['71%','58 次执行','41 完整评分','4 部分评分','1 等待评分','12 未评分']:['71%','58 executed','41 complete','4 partial','1 awaiting assessment','12 unscored']) assert.ok(await evaluate(client,`${executor}.textContent.includes(${JSON.stringify(part)})`),`executor view: ${part}`);
  for(const g of await evaluate(client,`[...document.querySelectorAll('.quality-model-row,.quality-table')].map(e=>({width:e.clientWidth,scroll:e.scrollWidth}))`)) assert.ok(g.scroll<=g.width+1,JSON.stringify(g));
  await evaluate(client,`${executor}.scrollIntoView({block:'center'});true`);await shot(`trust-scoring-states-executor-${lang}-${width}`);
  results.push({id:`trust.scoring-states.both-groupings.${lang}.${width}`,status:'passed'});
 }
 for (const [lang,theme,width,height,target] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813],['en','dark',1200,813,'request-1']]) {
  await open(`lang=${lang}&theme=${theme}&view=timeline${target ? '&initial-request='+target : ''}`,width,height);
  if (target) { await waitFor(client,`!!document.querySelector('.session-view-tabs')`); await click('.session-view-tabs button','Routing timeline'); }
  await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
  await waitFor(client,`document.querySelector('.routing-turn').textContent.includes('0.40')`);
  assert.equal(await evaluate(client,`document.querySelectorAll('.routing-turn').length`),2);
  assert.equal(await evaluate(client,`document.querySelectorAll('.routing-ticks .failed').length`),0,'delivered accepted requests are not failed ticks');
  assert.equal(await evaluate(client,`document.querySelectorAll('.routing-turn-head .badge').length`),1,'unknown boundaries have no heading badge');
  assert.equal(await evaluate(client,`/After compaction|压缩后/.test(document.querySelector('.routing-timeline').textContent)`),false);
  await evaluate(client,`document.querySelectorAll('.routing-turn')[0].click();true`);
  assert.ok(await evaluate(client,`document.querySelector('.routing-boundary-basis').textContent.includes(${JSON.stringify(lang==='zh'?'本轮重新判断':'A fresh decision for this turn')})`));
  await click('.routing-turn-detail .disclosure summary',lang==='zh'?'查看边界依据':'View boundary evidence');
  assert.ok(await evaluate(client,`document.querySelector('.routing-turn-detail .disclosure-body').textContent.includes(${JSON.stringify(lang==='zh'?'后台消息':'background messages')})`));

  assert.ok(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline')[1].payload.request.intent.query.after_watermark==='watermark-1'`));
  await click('button',lang==='zh'?'紧凑视图':'Compact view');
  assert.equal(await evaluate(client,`getComputedStyle(document.querySelector('.session-master')).display`),'none');
  await evaluate(client,`document.querySelectorAll('.routing-turn')[1].click();true`);
  await waitFor(client,`document.querySelector('.routing-request.selected')?.textContent.includes('trial-model')`);
  assert.equal(await evaluate(client,`document.querySelectorAll('.routing-request.failed').length`),0,'delivered accepted requests are not failed detail rows');
  assert.ok(await evaluate(client,`document.querySelector('.routing-detail-head').textContent.includes(${JSON.stringify(lang==='zh'?'边界未确认':'Boundary unknown')})`));
  assert.ok(await evaluate(client,`document.querySelector('.routing-request.selected')?.textContent.includes('trial-model')`));
  for (const g of await evaluate(client,`[...document.querySelectorAll('.routing-timeline,.routing-conversation,.routing-turn-detail,.session-routing-and-conversation')].map(e=>({width:e.clientWidth,scroll:e.scrollWidth,left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right}))`)) {assert.ok(g.scroll<=g.width+1,JSON.stringify(g));assert.ok(g.left>=-1&&g.right<=width+1,JSON.stringify(g));}
  await shot(`timeline-${lang}-${theme}-${width}`);
  results.push({id:`timeline.late-score-and-request-selection.${lang}.${theme}.${width}${target ? '-from-request' : ''}`,status:'passed'});
 }
 for (const [lang,width,height,theme] of [['zh',755,600,'light'],['en',755,600,'light'],['zh',1200,813,'dark'],['en',1200,813,'dark']]) {
  await open(`lang=${lang}&theme=${theme}&view=timeline&boundary=rebuilt`,width,height);
  await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
  await evaluate(client,`document.querySelectorAll('.routing-turn')[1].click();true`);
  assert.ok(await evaluate(client,`document.querySelector('.routing-detail-head').textContent.includes(${JSON.stringify(lang==='zh'?'上下文重建':'Context rebuilt')})`));
  await click('.routing-turn-detail .disclosure summary',lang==='zh'?'查看边界依据':'View boundary evidence');
  assert.ok(await evaluate(client,`document.querySelector('.routing-turn-detail .disclosure-body').textContent.includes(${JSON.stringify(lang==='zh'?'消息步骤比上一份可比较记录少':'fewer message steps')})`));
  assert.ok(await evaluate(client,`document.querySelector('.routing-legend').textContent.includes(${JSON.stringify(lang==='zh'?'新消息或上下文重建':'new message or rebuilt context')})`));
  await evaluate(client,`document.querySelector('.routing-turn-detail .disclosure-body').scrollIntoView({block:'center'});true`);
  const basis=await evaluate(client,`(()=>{const e=document.querySelector('.routing-turn-detail .disclosure-body'),r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:innerHeight,scroll:e.scrollWidth,width:e.clientWidth};})()`);
  assert.ok(basis.top>=0&&basis.bottom<=basis.height&&basis.scroll<=basis.width+1,JSON.stringify(basis));
  for (const g of await geometry()) assert.ok(g.scroll<=g.width+1,JSON.stringify(g));
  await shot(`timeline-rebuilt-${lang}-${width}-${theme}`);
  results.push({id:`timeline.recorded-history-rewrite.${lang}.${width}.${theme}`,status:'passed'});
 }
 await open('lang=en&view=timeline&request-outcome=failed');
 await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
 assert.equal(await evaluate(client,`document.querySelectorAll('.routing-ticks .failed').length`),2,'actual request failures retain failed ticks');
 assert.ok(await evaluate(client,`document.querySelectorAll('.routing-request.failed').length>0`),'actual request failures retain failed detail rows');
 results.push({id:'timeline.actual-request-outcomes',status:'passed'});
 await timelinePricingChecks();
 await open('lang=en&view=timeline&stale-timeline=1');
 await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
 await waitFor(client,`document.querySelectorAll('.routing-turn').length===3`);
 assert.equal(await evaluate(client,`document.querySelector('.routing-timeline [role=alert]')`),null);
 assert.ok(await evaluate(client,`document.querySelector('.routing-turn').textContent.includes('0.40')`));
 assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline')[2].payload.request.intent.query.after_watermark`),null);
 await shot('timeline-stale-cursor-recovered-en-755');
 results.push({id:'timeline.stale-cursor.rebuilds-and-appends-without-manual-refresh',status:'passed'});
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&view=timeline&timeline-error=1`);
  await waitFor(client,`document.querySelector('.routing-timeline [role=alert]')?.getClientRects().length>0`);
  assert.ok(await evaluate(client,`document.querySelector('.session-view-tabs')?.getClientRects().length>0`),'a failed first read keeps the timeline tab');
  assert.ok(await evaluate(client,`document.querySelector('.routing-timeline [role=alert]').textContent.includes(${JSON.stringify(lang==='zh'?'暂时无法读取':'temporarily unavailable')})`));
  assert.ok(await evaluate(client,`!document.querySelector('.routing-timeline [role=alert]').textContent.includes(${JSON.stringify(lang==='zh'?'上次读取的内容':'last successful read')})`),'a failed first read does not imply retained content');
  await shot(`timeline-initial-failure-${lang}-755`);
  await evaluate(client,`window.timelineFail=false;true`);
  const retriedAt=await evaluate(client,'performance.now()');
  await click('.routing-timeline [role=alert] button',lang==='zh'?'重试':'Retry');
  await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
  assert.equal(await evaluate(client,`document.querySelector('.routing-timeline [role=alert]')`),null);
  assert.ok(await evaluate(client,`trialTrace.some(x=>x.command==='observation_read'&&x.payload.request.intent.view==='routing_timeline'&&x.at>=${retriedAt}&&x.at-${retriedAt}<1000)`),`Retry reads at once: ${retriedAt} ${await evaluate(client,`JSON.stringify(trialTrace.filter(x=>x.payload?.request?.intent?.view==='routing_timeline').map(x=>Math.round(x.at)))`)}`);
  results.push({id:`timeline.initial-failure-retry-recovers.${lang}`,status:'passed'});
 }
 await open('lang=en&view=timeline&timeline-empty=1');
 await waitFor(client,`document.querySelector('.routing-timeline')?.textContent.includes('No recorded turns yet')`);
 assert.equal(await evaluate(client,`document.querySelector('.session-view-tabs')`),null);
 assert.equal(await evaluate(client,`document.querySelector('.routing-conversation').hidden`),false);
 const emptyReads=await timelineReads(); await sleep(2500); assert.equal(await timelineReads(),emptyReads);
 results.push({id:'timeline.confirmed-empty-hides-tab-without-polling',status:'passed'});
 await open('lang=en&view=timeline'); await waitFor(client,`document.querySelectorAll('.routing-turn').length===2`);
 await click('.session-view-tabs button','Conversation'); await sleep(300);
 const conversationReads=await timelineReads(); await sleep(4500);
 assert.equal(await timelineReads(),conversationReads,'Conversation pauses timeline polling');
 await click('.session-view-tabs button','Routing timeline'); await waitFor(client,`${timelineReadsExpr}>${conversationReads}`);
 await setVisibility('hidden'); await sleep(300);
 const hiddenReads=await timelineReads(); await sleep(4500);
 assert.equal(await timelineReads(),hiddenReads,'a hidden window pauses timeline polling');
 await setVisibility('visible'); await waitFor(client,`${timelineReadsExpr}>${hiddenReads}`);
 results.push({id:'timeline.polls-only-while-displayed-and-visible',status:'passed'});
 await open('lang=en&view=editor&division=1',755,600);
 await click('[role="tab"]','Model performance');await waitFor(client,`!!document.querySelector('.trust-suggestion.cyan')`);await click('.trust-suggestion button','Review division draft');
 await waitFor(client,`!!document.querySelector('.division-draft-form') && document.querySelector('.division-draft-form').textContent.includes('DeepSeek trial')`);
 assert.equal(await evaluate(client,`document.querySelector('.division-draft-form input[type=checkbox]').checked`),false);
 await click('[role="dialog"] button','Review draft in editor');
 await waitFor(client,`window.divisionDraft?.branch_routing?.branches?.[1]?.primary_candidates?.length===0 && window.divisionDraft?.branch_routing?.branches?.[0]?.candidates?.[0]?.binding_id==='binding/codex/gpt-5.6-sol'`);
 assert.equal(await evaluate(client,`divisionDraft.branch_routing.branches[1].primary_candidates.length`),0);
 assert.equal(await evaluate(client,`divisionDraft.branch_routing.branches[0].candidates[0].binding_id`),'binding/codex/gpt-5.6-sol');
 await evaluate(client,`document.querySelector('.division-draft-form input[type=checkbox]').click();true`);await click('[role="dialog"] button','Review draft in editor');
 await waitFor(client,`window.divisionDraft?.branch_routing?.branches?.[1]?.primary_candidates?.[0]?.binding_id==='binding/codex/gpt-5.6-sol'`);
 await click('[role="dialog"] button','Delegate to an agent');await waitFor(client,`document.querySelector('.division-executors').textContent.includes('Dependencies ready')`);
 assert.equal(await evaluate(client,`document.querySelectorAll('.division-executors input:checked').length`),0);
 assert.ok(await evaluate(client,`document.querySelector('.division-executors label').textContent.includes('DeepSeek Harness')`));
 assert.equal(await evaluate(client,`[...document.querySelectorAll('[role="dialog"] button')].find(e=>e.textContent.includes('Review draft in editor')).disabled`),true);
 await evaluate(client,`document.querySelector('.division-executors input').click();true`);await click('[role="dialog"] button','Review draft in editor');
 await waitFor(client,`window.divisionDraft?.delegation_enabled&&divisionDraft.work?.harness==='deepseek_harness'&&divisionDraft.branch_routing.branches.every(b=>b.primary_candidates.length===0&&b.candidates[0].binding_id==='binding/bailian/qwen-coder')`);
 assert.equal(await evaluate(client,`divisionDraft.delegation_enabled&&divisionDraft.work.harness==='deepseek_harness'&&divisionDraft.branch_routing.branches.every(b=>b.primary_candidates.length===0&&b.candidates[0].binding_id==='binding/bailian/qwen-coder')`),true);
 assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
 await shot('division-worker-en-755');results.push({id:'division.optional-primary-and-explicit-executor.no-publication',status:'passed'});
 for(const [lang,theme,width,height] of [['en','light',755,600],['zh','dark',755,600],['en','dark',1200,813],['zh','light',1200,813]]) {
  await open(`lang=${lang}&theme=${theme}&view=editor&division=1`,width,height);
  await click('[role="tab"]',lang==='zh'?'模型表现':'Model performance');await waitFor(client,`!!document.querySelector('.trust-suggestion.cyan')`);await click('.trust-suggestion button',lang==='zh'?'查看分工草稿':'Review division draft');
  await click('[role="dialog"] button',lang==='zh'?'委派给执行 Agent':'Delegate to an agent');await waitFor(client,`!!document.querySelector('.division-executors')`);
  await evaluate(client,`[...document.querySelectorAll('.division-executors label')].find(e=>e.textContent.includes('OpenCode')).querySelector('input').click();true`);await click('[role="dialog"] button',lang==='zh'?'在编辑器中查看草稿':'Review draft in editor');
  await waitFor(client,`window.divisionDraft?.delegation_enabled&&divisionDraft.work?.harness==='opencode'&&divisionDraft.work.protocol==='responses'&&divisionDraft.branch_routing.branches.every(b=>b.primary_candidates.length===0&&b.candidates[0].binding_id==='binding/bailian/qwen-coder')`);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor').length`),0);
  await evaluate(client,`[...document.querySelectorAll('.division-executors label')].find(e=>e.textContent.includes('OpenCode')).scrollIntoView({block:'center'});true`);
  assert.ok(await evaluate(client,`(()=>{const label=[...document.querySelectorAll('.division-executors label')].find(e=>e.textContent.includes('OpenCode')).getBoundingClientRect(),buttons=[...document.querySelectorAll('[role="dialog"] button')].filter(e=>e.textContent.includes(${JSON.stringify(lang==='zh'?'在编辑器中查看草稿':'Review draft in editor')}));return document.documentElement.scrollWidth<=innerWidth+1&&label.top>=0&&buttons.every(e=>{const b=e.getBoundingClientRect();return label.bottom<=b.top&&b.bottom<=innerHeight;})&&[...document.querySelectorAll('.division-executors label')].every(e=>e.scrollWidth<=e.clientWidth+1);})()`));
  await shot(`division-opencode-${lang}-${theme}-${width}`);results.push({id:`division.opencode-explicit-executor.no-publication.${lang}.${theme}.${width}`,status:'passed'});
 }
 await open('lang=zh&view=attention&theme=dark');await shot('attention-zh-dark-755');await click('.trust-attention .btn-primary','打开调整草稿');assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='review-trust-action').length`),1);await click('.trust-attention button','查看依据');assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='review-trust').length`),1);results.push({id:'attention.optional-evidence-review',status:'passed'});
 await open('lang=en&view=attention&empty=1');assert.ok(await evaluate(client,`document.querySelector('.trust-attention').textContent.includes('No routing adjustments are suggested')`));assert.equal(await evaluate(client,`document.querySelectorAll('.trust-attention button').length`),0);results.push({id:'attention.empty-is-normal',status:'passed'});
 await open('lang=en&view=attention&error=1');assert.ok(await evaluate(client,`document.querySelector('.trust-attention [role=alert]').textContent.includes('retry before deciding')`));assert.equal(await evaluate(client,`[...document.querySelectorAll('.trust-attention button')].find(e=>e.textContent.includes('View evidence')).disabled`),true);await click('button','Retry');assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='retry-suggestions').length`),1);results.push({id:'attention.failed-read-is-not-empty',status:'passed'});
 }
 for(const [lang,theme,lost] of [['en','light',false],['zh','dark',true]]) {
  await open(`lang=${lang}&theme=${theme}&view=quickstart${lost?'&lost=1&saved-only=1':''}`,755,600);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-page .trial-agent-choice input:not(:disabled)')`);
  assert.equal(await evaluate(client,`document.querySelector('.trial-steps [aria-current=step]').textContent.includes(${JSON.stringify(lang==='zh'?'选择模型':'Choose models')})`),true);
  assert.equal(await evaluate(client,`[...document.querySelectorAll('.trial-presets input:checked')].length`),1);
  assert.equal(await evaluate(client,`document.querySelector('.trial-choice-list .trial-choice[aria-pressed=true]')`),null);
  await waitFor(client,`!document.querySelector('.trial-footer .btn-primary').disabled`);
  await click('.trial-footer button',lang==='zh'?'检查改动':'Review changes');
  assert.match(await evaluate(client,`document.querySelector('.trial-validation-summary').textContent`),/Primary|主力/);
  await evaluate(client,`(()=>{const el=[...document.querySelectorAll('.trial-choice-list .trial-choice')].find(e=>e.textContent.includes(primaryModel.display_name));if(!el)throw Error('Missing primary choice');el.click();return true;})()`);
  await waitFor(client,`!!document.querySelector('.trial-choice.selected[aria-expanded=false]')`);
  await shot(`quickstart-setup-${lang}-${theme}`);
  await evaluate(client,`document.querySelector('.trial-footer').scrollIntoView({block:'end'});true`);
  assert.ok(await evaluate(client,`(()=>{const r=document.querySelector('.trial-footer .btn-primary').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight+1;})()`));
  await shot(`quickstart-actions-${lang}-${theme}`);
  await click('.trial-footer button',lang==='zh'?'检查改动':'Review changes');
  await waitFor(client,`!!document.querySelector('.trial-review .trust-control')`);
  assert.equal(await evaluate(client,`document.querySelector('.trial-review .trust-levels [aria-pressed=true]').textContent.trim()`),lang==='zh'?'试用':'Trial');
  assert.equal(await evaluate(client,`document.querySelector('.trial-review [data-decision-field$="-simple-threshold"]').value`),'0.9');
  await waitFor(client,`!!document.querySelector('.trial-review > .actions .btn-primary:not(:disabled)')`);
  await click('.trial-review button',lang==='zh'?'保存并发布路由':'Save and publish route');
  if(lost) {
   await waitFor(client,`trialTrace.some(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='publish')`);
   await waitFor(client,`document.querySelector('.trial-publication-status')?.textContent.includes('正在确认发布结果')`);
   await click('.trial-review button',lang==='zh'?'刷新结果':'Refresh result');
  }
  await waitFor(client,`!!document.querySelector('.trial-mapping .trial-agent-choice')`);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='save_draft').length`),1);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_plan_editor'&&x.payload.input.action==='publish').length`),1);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_agent_settings').length`),0);
  await click('.trial-mapping button',lang==='zh'?'检查并映射 Agent':'Review and map Agent');
  await waitFor(client,`document.querySelector('.trial-mapping').textContent.includes(${JSON.stringify(lang==='zh'?'映射已确认':'Mapping confirmed')})`);
  assert.equal(await evaluate(client,`trialTrace.filter(x=>x.command==='preview_agent_settings').length`),1);
  assert.ok(await evaluate(client,`document.querySelector('.trial-steps [aria-current=step]').textContent.includes(${JSON.stringify(lang==='zh'?'开始使用':'Start using it')})`));
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='preview_agent_settings').payload.input.spec.model.settings.preset_mappings.sonnet.kind`),'preserve_native');
  assert.equal(await evaluate(client,`trialTrace.find(x=>x.command==='preview_agent_settings').payload.input.spec.collaboration.intent`),'keep');
  for(const g of await evaluate(client,`[...document.querySelectorAll('.trial-wizard,.trial-mapping')].map(e=>({width:e.clientWidth,scroll:e.scrollWidth,left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right}))`)) {assert.ok(g.scroll<=g.width+1,JSON.stringify(g));assert.ok(g.left>=-1&&g.right<=756,JSON.stringify(g));}
  await shot(`quickstart-complete-${lang}-${theme}`);
  results.push({id:`quickstart.${lang}.${lost?'lost-publication-response':'first-mapping'}.no-duplicate`,status:'passed'});
 }
 for(const lang of ['en','zh']) {
  await open(`lang=${lang}&view=quickstart`,755,600);
  await waitFor(client,`[...document.querySelectorAll('.new-models button')].some(e=>e.textContent.includes(${JSON.stringify(lang==='zh'?'试用':'Try')}))`);
  await click('.new-models button',lang==='zh'?'试用':'Try');
  await waitFor(client,`!!document.querySelector('.trial-page .trial-agent-choice input:not(:disabled)')`);
  await click('.trial-model-card','Kimi K3'); await click('.trial-connect-card button',lang==='zh'?'在此接入':'Connect here');
  await waitFor(client,`!!document.querySelector('.trial-inline-connection .connection-flow')`);
  await waitFor(client,`document.querySelector('#mc-connection-form')?.hidden===false || !!document.querySelector('.connection-provider-card')`);
  await evaluate(client,`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))`);
  if (!await evaluate(client,`document.querySelector('#mc-connection-form')?.hidden===false`)) await click('.trial-inline-connection button','Kimi');
  await waitFor(client,`document.querySelector('#mc-connection-form')?.hidden===false`);
  await typeInto('#mc-connection-form input[maxlength="60"]','Kimi trial edit');
  const keep=lang==='zh'?'继续编辑':'Keep editing', discard=lang==='zh'?'放弃修改':'Discard changes';
  const pressed=`document.querySelector('.trial-model-card[aria-pressed=true]')?.textContent`;
  // The connection dialog makes the cards inert to the pointer; a scripted change must still meet the guard.
  await evaluate(client,`(()=>{[...document.querySelectorAll('.trial-model-card')].find(e=>e.textContent.includes(trialModel.display_name)).click();return true;})()`);
  await waitFor(client,`!!document.querySelector('.discard-dialog[open]')`); await shot(`quickstart-connection-guard-${lang}`);
  await click('.discard-dialog button',keep);
  await waitFor(client,`!document.querySelector('.discard-dialog')`);
  assert.ok(await evaluate(client,`${pressed}.includes('Kimi K3')`),'cancel keeps the selected model');
  assert.equal(await evaluate(client,`document.querySelector('#mc-connection-form input[maxlength="60"]').value`),'Kimi trial edit');
  await evaluate(client,`(()=>{[...document.querySelectorAll('.trial-model-card')].find(e=>e.textContent.includes(trialModel.display_name)).click();return true;})()`);
  await waitFor(client,`!!document.querySelector('.discard-dialog[open]')`); await click('.discard-dialog button',discard);
  await waitFor(client,`!document.querySelector('.trial-inline-connection') && ${pressed}?.includes(trialModel.display_name)`);
  await click('.trial-model-card','Kimi K3'); await click('.trial-connect-card button',lang==='zh'?'在此接入':'Connect here');
  await waitFor(client,`!!document.querySelector('.trial-inline-connection .connection-flow')`);
  await waitFor(client,`document.querySelector('#mc-connection-form')?.hidden===false || !!document.querySelector('.connection-provider-card')`);
  await evaluate(client,`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))`);
  if (!await evaluate(client,`document.querySelector('#mc-connection-form')?.hidden===false`)) await click('.trial-inline-connection button','Kimi');
  await waitFor(client,`document.querySelector('#mc-connection-form')?.hidden===false`); await typeInto('#mc-connection-form input[maxlength="60"]','Kimi back edit');
  await evaluate(client,`(()=>{[...document.querySelectorAll('.trial-page .toolbar-actions button')].find(e=>e.textContent.includes(${JSON.stringify(lang==='zh'?'返回':'Back')})).click();return true;})()`);
  await waitFor(client,`!!document.querySelector('.discard-dialog[open]')`); await click('.discard-dialog button',keep);
  await waitFor(client,`!document.querySelector('.discard-dialog') && !!document.querySelector('.trial-page')`);
  assert.equal(await evaluate(client,`document.querySelector('#mc-connection-form input[maxlength="60"]').value`),'Kimi back edit');
  await evaluate(client,`(()=>{[...document.querySelectorAll('.trial-page .toolbar-actions button')].find(e=>e.textContent.includes(${JSON.stringify(lang==='zh'?'返回':'Back')})).click();return true;})()`);
  await waitFor(client,`!!document.querySelector('.discard-dialog[open]')`); await click('.discard-dialog button',discard);
  await waitFor(client,`!document.querySelector('.trial-page') && !!document.querySelector('.new-models')`);
  results.push({id:`quickstart.${lang}.inline-connection-guard.model-change-and-back`,status:'passed'});
 }
 }
 await writeFile(path.join(output,'report.json'),JSON.stringify({proof:'production components with synthetic IPC; no native or provider claim',results},null,2));console.log(JSON.stringify({passed:results.length,output}));
} catch(error) {
 const failure={error:String(error.stack??error),results};
 try { failure.page=await evaluate(client,`({url:location.href,text:document.body?.innerText??'',html:document.documentElement.outerHTML,ready:document.readyState,trace:window.trialTrace??null,errors:window.fixtureBrowserErrors??[],resources:performance.getEntriesByType('resource').map(e=>({name:e.name,status:e.responseStatus,duration:e.duration}))})`); }
 catch(captureError) {failure.pageCaptureError=String(captureError);}
 try {await shot('failure');} catch(captureError) {failure.screenshotError=String(captureError);}
 await writeFile(path.join(output,'failure.json'),JSON.stringify(failure,null,2));
 throw error;
} finally {clearInterval(keepAlive);client.close();}
