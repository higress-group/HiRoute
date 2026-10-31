import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';
import { reasoningControlScenarios } from './route-reasoning-controls.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const results = [];
const visible = `e => e.getClientRects().length && !e.matches(':disabled')`;
async function click(selector, label) {
  await waitFor(client, `(()=>{const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>(${visible})(e)&&e.textContent.includes(${JSON.stringify(label)}));if(!e)return false;e.click();return true;})()`);
}
async function shot(name) {
  await evaluate(client, 'Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))).then(()=>true)');
  const capture = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(capture.data, 'base64'));
}
async function setup({ language = 'zh', theme = 'light', width = 1200, height = 813, others = true, existing = false, stale = false, delayed = false, mode = 'fixed_model', reasoning, savedReasoning } = {}) {
  await navigate(client, `${base}product-shell.html`, { width, height });
  await client.send('Page.bringToFront');
  await waitFor(client, 'Boolean(window.productShell) && !!document.querySelector(".sidebar")');
  await evaluate(client, `(()=>{
    const c=productShell;
    c.routeTemplate=structuredClone(c.desktop.catalog.plans[0]);
    const source=structuredClone(c.management.sources.find(s=>s.models.some(m=>m.binding_id==='binding/codex/gpt-6-astra')));
    const model=source.models.find(m=>m.binding_id==='binding/codex/gpt-6-astra');
    Object.assign(model,{upstream_model_id:'gpt-6.1-sol',display_name:'GPT-6.1 Sol',catalog_configuration_id:undefined,native_reasoning:${JSON.stringify(reasoning ?? {kind:'discrete',parameter:'reasoning_effort',profiles:['low','high']})}});
    source.models=[model]; source.state='ready'; source.display_name='Codex subscription';
    const api=structuredClone(source);api.source_id='source/api/sol';api.display_name='OpenAI API';api.display_template_id='openai.platform.global.v1';
    Object.assign(api.models[0],{binding_id:'binding/api/sol',model_ref:'model-ref/api/sol'});
    c.management.sources=${others ? '[source,api]' : '[source]'};
    c.desktop.catalog={plans:[],drafts:[]};
    if(${existing}){
      const p=structuredClone(c.routeTemplate);Object.assign(p,{agent_plan_id:'plan/existing-sol',model_alias:'gpt-6.1-sol'});p.head={...p.head,head_revision:7,status:'enabled'};
      p.desired={...p.desired,mode:${JSON.stringify(mode)},display_name:'gpt-6.1-sol',purpose:'Existing purpose',delegation_enabled:false,limits:{...p.desired.limits,maximum_attempts:2},strategy:{mode:'custom',candidates:[{binding_id:model.binding_id,reasoning:{kind:'profile',profile:'low'}}]}};
      if(${JSON.stringify(savedReasoning)} && p.desired.strategy.candidates) p.desired.strategy.candidates[0].reasoning=${JSON.stringify(savedReasoning)};
      if(${JSON.stringify(mode)}!=='fixed_model'){p.desired.strategy=structuredClone(c.routeTemplate.desired.strategy);}
      c.desktop.catalog.plans=[p];
    }
    c.handlers.plan_editor_options=payload=>{
      const v=c.fixtureResponse('plan_editor_options',payload);
      v.candidates=c.management.sources.flatMap(s=>s.models.map(m=>({binding_id:m.binding_id,model_configuration_id:m.upstream_model_id,display_name:m.display_name,reasoning:m.native_reasoning,billing_class:'paid',routable:true,ingress_protocols:['responses','messages'],native_ingress_protocols:['responses','messages']})));
      v.context_window={maximum_tokens:128000,default_tokens:128000};
      if(v.authoring){
        v.authoring.editor.candidates=v.authoring.editor.candidates.map(s=>({...s,reasoning:s.reasoning??{kind:'profile',profile:'high'}}));
        if(${stale}&&payload.input.authoring.plan_id)v.authoring.expected_head_revision=8;
      }
      if(v.authoring && !payload.input.editor && ${!!reasoning} && !${existing}) {
        const native=${JSON.stringify(reasoning ?? null)};
        const selected=${JSON.stringify(savedReasoning ?? null)} ?? (native?.kind==='discrete'?{kind:'profile',profile:native.profiles.at(-1)}:native?.kind==='toggle'?{kind:'toggle',enabled:true}:native?.kind==='budget'?{kind:'budget',tokens:native.minimum_tokens}:undefined);
        v.authoring.editor.candidates=v.authoring.editor.candidates.map(s=>({...s,reasoning:selected}));
      }
      return ${delayed}&&payload.input.editor?new Promise(resolve=>{c.releaseModelRouteOptions=()=>resolve(v);}):v;
    };
    c.handlers.preview_plan_editor=payload=>{
      const i=payload.input;
      const p=structuredClone(c.routeTemplate);Object.assign(p,{agent_plan_id:i.plan_id??'plan/new-sol',model_alias:i.editor.custom_alias,agent_plan_revision:1});
      p.head={...p.head,head_revision:(i.expected_head_revision??0)+1,status:'enabled'};
      p.desired={display_name:i.editor.display_name,purpose:i.editor.purpose,follow_request_reasoning:i.editor.follow_request_reasoning,mode:i.editor.mode,strategy:{mode:'custom',candidates:structuredClone(i.editor.candidates)},delegation_enabled:i.editor.delegation_enabled,requirements:i.editor.requirements,limits:i.editor.limits};
      c.desktop.catalog={plans:[p],drafts:[]};
      return {state:'succeeded',operation:{operation_id:'operation/create-sol',state:'succeeded',sequence:1,cancellable:false}};
    };
    localStorage.setItem('hiroute.language',${JSON.stringify(language)});localStorage.setItem('hiroute.theme',${JSON.stringify(theme)});c.commands=[];c.remount();return true;
  })()`);
  await click('.sidebar button', language === 'zh' ? '模型' : 'Models');
  await waitFor(client, '!!document.querySelector(".models-feature .master-list .list-row")');
  await click('.models-feature .master-list .list-row', 'GPT-6.1 Sol');
  await waitFor(client, '!!document.querySelector(".model-route-action button")');
}
async function editorReady() {
  await waitFor(client, '!!document.querySelector(".plan-alias-field") && !!document.querySelector(".route-candidate")');
  await waitFor(client, '!document.querySelector(".plan-editor [aria-busy=true]")');
}
async function choose(selector, value) {
  await waitFor(client,`!!document.querySelector(${JSON.stringify(selector)})`);
  await evaluate(client,`(()=>{const e=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('change',{bubbles:true}));return true;})()`);
}
async function current() {
  return evaluate(client, `({name:document.querySelector('.plan-identity-fields input')?.value,alias:document.querySelector('.plan-alias-field')?.value,purpose:document.querySelectorAll('.plan-identity-fields input')[1]?.value,required:document.querySelectorAll('.plan-identity-fields input')[1]?.required,bindings:[...document.querySelectorAll('.route-candidate')].map(e=>e.dataset.bindingId),writes:productShell.commands.filter(c=>c.command==='preview_plan_editor').length})`);
}
async function run(name, fn) {
  if (process.argv.includes('--reasoning-controls') && !name.startsWith('model-route.reasoning-control.')) return;
  try { await fn(); results.push({ name, state: 'green' }); console.log(`green: ${name}`); }
  catch (error) { results.push({ name, state: 'red', error: error.message }); console.log(`red: ${name}: ${error.message}`); await shot(`failure-${results.length}`).catch(() => {}); }
}
try {
  await reasoningControlScenarios({ client, run, setup, click, shot, choose, current, editorReady });
  await run('model-route.reasoning-control.discoverability',async()=>{
    await setup({others:false,width:755,height:600});await click('.model-route-action button','创建智能路由');await editorReady();
    await waitFor(client,'![...document.querySelectorAll(".toolbar-actions .btn-primary")].find(e=>e.getClientRects().length).disabled');
    const field=await evaluate(client,`(()=>{const row=document.querySelector('.route-candidate');const label=row.querySelector('.route-reasoning-label');const select=row.querySelector('select.effort-select');return {label:label?.textContent,select:select?.tagName,font:select?Number.parseFloat(getComputedStyle(select).fontSize):0,height:select?.getBoundingClientRect().height,associated:!!label&&!!select&&label.htmlFor===select.id};})()`);
    assert.equal(field.label,'思考档位');assert.equal(field.select,'SELECT');assert.ok(field.associated);assert.ok(field.font>=12&&field.height>=32,JSON.stringify(field));
    await evaluate(client,'document.querySelector(".route-reasoning-field").scrollIntoView({block:"center"});true');await shot('reasoning-control-visible-label');
  });
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200,813],[755,600]]) {
    const id=`${language}-${theme}-${width}`;
    await run(`model-route.layout-and-one-save.${id}`, async () => {
      await setup({ language, theme, width, height });
      const hero=await evaluate(client, `(()=>{const e=document.querySelector('.model-route-action button');const r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,right:r.right,trial:[...document.querySelectorAll('button')].some(b=>/试用这个模型|Try this model/.test(b.textContent))};})()`);
      assert.equal(hero.trial,false);assert.ok(hero.top>=0&&hero.bottom<=height&&hero.right<=width,JSON.stringify(hero));
      await shot(`model-route-detail-${id}`);
      await click('.model-route-action button',language==='zh'?'创建智能路由':'Create routing');
      await waitFor(client,'!!document.querySelector("[role=dialog]")');
      assert.equal(await evaluate(client,'productShell.commands.filter(c=>c.command==="preview_plan_editor").length'),0);
      const bounds=await evaluate(client,`(()=>{const e=document.querySelector('.modal-foot .btn-primary');const r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,right:r.right,overflow:document.querySelector('.modal').scrollWidth-document.querySelector('.modal').clientWidth};})()`);
      assert.ok(bounds.bottom<=height&&bounds.top>=0&&bounds.right<=width&&bounds.overflow<=1,JSON.stringify(bounds));
      await shot(`model-route-choice-${id}`);
      await click('.modal-foot .btn-primary',language==='zh'?'一起加入候选':'Add together as candidates');
      await editorReady();
      assert.deepEqual(await current(),{name:'gpt-6.1-sol',alias:'gpt-6.1-sol',purpose:'',required:false,bindings:['binding/codex/gpt-6-astra','binding/api/sol'],writes:0});
      await waitFor(client, '![...document.querySelectorAll(".routing-page .toolbar-actions .btn-primary")].find(e=>e.getClientRects().length&&/保存并创建|Save and create/.test(e.textContent)).disabled');
      const policyBounds=await evaluate(client,`(()=>{const e=document.querySelector('.route-reasoning-policy');const label=e.querySelector('label');return {overflow:e.scrollWidth-e.clientWidth,labelHeight:label.getBoundingClientRect().height};})()`);
      assert.ok(policyBounds.overflow<=1&&policyBounds.labelHeight<90,JSON.stringify(policyBounds));
      await shot(`model-route-editor-${id}`);
      await evaluate(client,'document.querySelector(".route-reasoning-policy").scrollIntoView({block:"start"});true');
      await shot(`model-route-reasoning-${id}`);
      await click('.toolbar-actions button',language==='zh'?'保存并创建':'Save and create');
      await waitFor(client,'productShell.commands.some(c=>c.command==="preview_plan_editor")');
      const saved=await evaluate(client,'productShell.commands.filter(c=>c.command==="preview_plan_editor").map(c=>c.payload.input)');
      assert.equal(saved.length,1);assert.equal(saved[0].editor.follow_request_reasoning,true);assert.equal(saved[0].action,'publish');assert.equal(saved[0].editor.mode,'fixed_model');assert.equal(saved[0].editor.purpose,'');assert.equal(saved[0].editor.delegation_enabled,false);assert.equal(saved[0].editor.custom_alias,'gpt-6.1-sol');
    });
  }
  await run('model-route.reasoning-mode-defaults-and-explicit-choice',async()=>{
    await setup({others:false});await click('.model-route-action button','创建智能路由');await editorReady();
    const checked=()=>evaluate(client,'document.querySelector(".route-reasoning-policy input").checked');
    assert.equal(await checked(),true);
    await waitFor(client,'document.querySelector(".effort-select")?.getAttribute("aria-label").includes("默认")');
    await click('.mode-card','按难度分流');assert.equal(await checked(),false);
    assert.match(await evaluate(client,'document.querySelector(".route-reasoning-policy").textContent'),/能力与成本/);
    await click('.mode-card','固定模型');assert.equal(await checked(),true);
    await evaluate(client,'document.querySelector(".route-reasoning-policy input[value=fixed]").click();true');
    assert.equal(await checked(),false);
    await click('.mode-card','按难度分流');await click('.mode-card','固定模型');assert.equal(await checked(),false);
    assert.match(await evaluate(client,'document.querySelector(".effort-select").getAttribute("aria-label")'),/固定/);
    await shot('model-route-reasoning-fixed-choice');
  });
  await run('model-route.published-policy-survives-remount-and-mode-switch',async()=>{
    await setup({others:false});await click('.model-route-action button','创建智能路由');await editorReady();
    await click('.toolbar-actions button','保存并创建');
    await waitFor(client,'productShell.desktop.catalog.plans.length===1 && !!document.querySelector(".editor-actions")');
    assert.equal(await evaluate(client,'document.querySelector(".route-reasoning-policy input").checked'),true);
    await click('.mode-card','按难度分流');
    assert.equal(await evaluate(client,'document.querySelector(".route-reasoning-policy input").checked'),true);
  });
  await run('model-route.current-only-and-cancel',async()=>{
    await setup();await click('.model-route-action button','创建智能路由');await click('.modal-foot button','取消');
    assert.equal(await evaluate(client,'productShell.commands.filter(c=>c.command==="preview_plan_editor").length'),0);
    await click('.model-route-action button','创建智能路由');await click('.modal-foot button','仅用当前模型');await editorReady();
    assert.deepEqual((await current()).bindings,['binding/codex/gpt-6-astra']);
  });
  await run('model-route.single-source-goes-directly-to-prefilled-editor',async()=>{
    await setup({others:false});await click('.model-route-action button','创建智能路由');await editorReady();
    assert.equal(await evaluate(client,'document.querySelectorAll("[role=dialog]").length'),0);assert.equal((await current()).alias,'gpt-6.1-sol');
  });
  await run('model-route.save-waits-for-current-model-options',async()=>{
    await setup({others:false,delayed:true});await click('.model-route-action button','创建智能路由');await editorReady();
    await waitFor(client,'typeof productShell.releaseModelRouteOptions==="function"');
    assert.match(await evaluate(client,'document.querySelector(".route-reasoning-readonly").textContent'),/正在读取/);
    assert.equal(await evaluate(client,'document.querySelector(".route-reasoning-field").querySelectorAll("button,select,input").length'),0);
    assert.equal(await evaluate(client,'[...document.querySelectorAll(".routing-page .toolbar-actions .btn-primary")].find(e=>e.getClientRects().length&&/保存并创建|Save and create/.test(e.textContent)).disabled'),true);
    assert.equal((await current()).writes,0);
    await evaluate(client,'productShell.releaseModelRouteOptions();true');
    await click('.toolbar-actions button','保存并创建');
    await waitFor(client,'productShell.commands.some(c=>c.command==="preview_plan_editor")');
  });
  await run('model-route.existing-fixed-route-retains-settings-and-order',async()=>{
    await setup({existing:true});await click('.model-route-action button','创建智能路由');await click('.modal-foot .btn-primary','一起加入候选');await editorReady();
    assert.deepEqual((await current()).bindings,['binding/codex/gpt-6-astra','binding/api/sol']);
    const request=await evaluate(client,'productShell.commands.find(c=>c.command==="plan_editor_options"&&c.payload.input.authoring.plan_id)?.payload.input.authoring');
    assert.equal(request.plan_id,'plan/existing-sol');assert.deepEqual(request.candidates[0].reasoning,{kind:'profile',profile:'low'});assert.equal((await current()).purpose,'Existing purpose');
    assert.equal(await evaluate(client,'document.querySelector(".plan-alias-field").readOnly'),true);
    await click('.editor-actions button','发布更改');
    await waitFor(client,'productShell.commands.some(c=>c.command==="preview_plan_editor")');
    const saved=await evaluate(client,'productShell.commands.filter(c=>c.command==="preview_plan_editor").map(c=>c.payload.input)');
    assert.equal(saved.length,1);assert.equal(saved[0].expected_head_revision,7);assert.equal(saved[0].editor.limits.maximum_attempts,2);
    assert.equal(saved[0].editor.purpose,'Existing purpose');assert.equal(!!saved[0].editor.follow_request_reasoning,false);assert.deepEqual(saved[0].editor.candidates[0].reasoning,{kind:'profile',profile:'low'});
    assert.equal(await evaluate(client,'productShell.desktop.catalog.plans[0].head.head_revision'),8);
  });
  await run('model-route.existing-candidate-is-not-an-unsaved-change',async()=>{
    await setup({others:false,existing:true});await click('.model-route-action button','创建智能路由');await click('.modal-foot .btn-primary','编辑已有路由');await editorReady();
    assert.equal(await evaluate(client,'[...document.querySelectorAll(".editor-actions .btn-primary")].find(e=>e.getClientRects().length).disabled'),true);
    assert.equal((await current()).writes,0);
  });
  await run('model-route.concurrent-head-change-refuses-stale-additions',async()=>{
    await setup({existing:true,stale:true});await click('.model-route-action button','创建智能路由');await click('.modal-foot .btn-primary','一起加入候选');
    await waitFor(client,'document.querySelector(".plan-editor")?.textContent.includes("未能读取计划设置")');
    assert.equal(await evaluate(client,'productShell.commands.filter(c=>c.command==="preview_plan_editor").length'),0);
  });
  await run('model-route.other-mode-opens-without-replacing-strategy',async()=>{
    await setup({existing:true,mode:'smart_saving'});await click('.model-route-action button','创建智能路由');await click('.modal-foot .btn-primary','查看路由');
    await waitFor(client,'!!document.querySelector(".plan-alias-field")');
    assert.equal(await evaluate(client,'productShell.commands.some(c=>c.command==="plan_editor_options"&&c.payload.input.authoring?.candidates)'),false);
    assert.equal(await evaluate(client,'productShell.desktop.catalog.plans[0].desired.mode'),'smart_saving');
  });
  await run('model-route.purpose-required-only-while-delegation-enabled',async()=>{
    await setup({others:false});await click('.model-route-action button','创建智能路由');await editorReady();
    await evaluate(client,`(()=>{const d=document.querySelector('.plan-editor details');d.open=true;return true;})()`);
    await click('[data-route-group=executor] button','允许委派任务');
    assert.equal((await current()).required,true);
    await click('.toolbar-actions button','保存并创建');
    await waitFor(client,'!!document.querySelector("#route-purpose-error")');
    assert.equal((await current()).writes,0);
    await shot('model-route-purpose-required');
    await click('[data-route-group=executor] button','允许委派任务');
    assert.equal((await current()).required,false);
    await click('.toolbar-actions button','保存并创建');
    await waitFor(client,'productShell.commands.some(c=>c.command==="preview_plan_editor")');
  });
  await writeFile(path.join(output,'model-route-report.json'),JSON.stringify({evidence:'Production DesktopApp/components/styles with synthetic IPC; native persistence is separate',tests:results.length,failed:results.filter(r=>r.state==='red').length,results},null,2));
  if(results.some(r=>r.state==='red'))process.exitCode=1;
} finally {await client.close();}
