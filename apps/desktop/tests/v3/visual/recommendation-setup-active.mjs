import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const [port, base, output] = process.argv.slice(2);
const client = await connectPage(Number(port));
const featured = JSON.parse(await readFile(new URL('../../../../../assets/model-feed/featured.json', import.meta.url)));
const official = JSON.parse(await readFile(new URL('../../../../../assets/release-facts/current/bundle/official-model-data.json', import.meta.url))).data;
const identity = featured[0].model_configuration_id;
const key = 'qwen-4-max';
const metadata = official.metadata_catalog;
const endpoint_bindings = metadata.endpoint_bindings.filter(binding => binding.model_key === key);
const catalog = {
  canonical_models: metadata.canonical_models.filter(model => model.model_key === key),
  endpoint_bindings,
  access_products: metadata.access_products.filter(product => endpoint_bindings.some(binding => binding.product_key === product.product_key)),
  provider_records: [],
  model_records: [],
};
const optionsFixture = { identity, featured, catalog };
const results = [];
const readyBinding = 'binding/recommendation/qwen4';
const savedSource = 'source/recommendation/bailian';

async function click(selector, label = '') {
  await waitFor(client, `(()=>{const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length&&!e.disabled&&e.textContent.includes(${JSON.stringify(label)}));if(!e)return false;e.click();return true;})()`);
}
async function change(selector, value, type = 'select') {
  await waitFor(client, `[...document.querySelectorAll(${JSON.stringify(selector)})].some(e=>e.getClientRects().length)`);
  await evaluate(client, `(()=>{
    const e=[...document.querySelectorAll(${JSON.stringify(selector)})].find(e=>e.getClientRects().length);
    Object.getOwnPropertyDescriptor(${type === 'input' ? 'HTMLInputElement' : 'HTMLSelectElement'}.prototype,'value').set.call(e,${JSON.stringify(value)});
    e.dispatchEvent(new Event(${JSON.stringify(type === 'input' ? 'input' : 'change')},{bubbles:true}));
    return true;
  })()`);
}
async function shot(name) {
  await evaluate(client, `Promise.all(document.getAnimations().filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity)
    .map(animation => animation.finished.catch(() => {}))).then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true)))))`);
  const image = await client.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, `${name}.png`), Buffer.from(image.data, 'base64'));
}
async function setup({ language = 'zh', theme = 'light', width = 1200, height = 813, ready = true, reuse = true, priorDraft = true, route = true, mutable = true } = {}) {
  await navigate(client, `${base}product-shell.html`, { width, height });
  await waitFor(client, 'Boolean(window.productShell) && !!document.querySelector(".sidebar")');
  await evaluate(client, `(() => {
    const c=productShell;
    const {identity,featured,catalog}=${JSON.stringify(optionsFixture)};
    const plan = c.desktop.catalog.plans[0];
    Object.assign(plan.desired, { display_name: 'Daily coding', delegation_enabled: false });
    c.desktop.service.mutation_available = ${mutable};
    const planId = plan.agent_plan_id;
    const baseline = c.fixtureResponse('plan_editor_options', { input: { authoring: { plan_id: planId } } }).authoring.editor;
    if (${priorDraft}) {
      const draftEditor = structuredClone(baseline);
      draftEditor.purpose = 'Keep the existing purpose';
      draftEditor.limits.maximum_attempts = 4;
      c.desktop.catalog.drafts = [{ draft_id: 'draft/existing', plan_id: planId, base_head_revision: plan.head.head_revision, revision: 3, editor: draftEditor }];
    }
    if (!${route}) c.desktop.catalog = { plans: [], drafts: [] };
    c.baseline = structuredClone(c.desktop.catalog);
    const member = { ...c.management.sources[0].models[0], binding_id: '${readyBinding}', model_ref: 'model-ref/qwen4',
      revision: 1, upstream_model_id: 'qwen4-max', display_name: 'Qwen4-Max', catalog_configuration_id: undefined,
      native_reasoning: { kind: 'discrete', parameter: 'reasoning_effort', profiles: ['low','high','xhigh'], default_profile: 'high' },
      membership: 'user_declared', presentation: { availability: 'available', billing_class: 'subscription' } };
    const token = {
      ...c.management.sources[0], source_id: '${savedSource}', display_name: 'Bailian Token Plan', revision: 1,
      state: 'ready', actions: ['append_models'], provenance: 'registered',
      connection_identity: { access_kind: 'api', connection_option_id: 'bailian.token-plan.cn.v1', product_label: 'Bailian Token Plan' },
      models: ${ready} ? [member] : [],
    };
    const other = { ...token, source_id: 'source/wrong-product', display_name: 'Same ID on a different product',
      connection_identity: { ...token.connection_identity, connection_option_id: 'bailian.coding-plan.cn.v1' },
      models: [{ ...member, binding_id: 'binding/wrong', model_ref: 'model-ref/wrong' }] };
    c.management.sources.push(other);
    if (${reuse}) c.management.sources.push(token);
    c.handlers.model_availability = () => ({ featured, models: [
      { model_configuration_id: identity, display_name: 'Qwen4-Max', origin: { kind:'embedded', client_version:'0.6.4' },
        availability: { kind: 'needs_connection' }, binding_ids: [], facts: { publisher_id:'publisher.alibaba-qwen', context_tokens:1000000, tool:true, vision:true, reasoning:null } },
      { model_configuration_id: featured[1].model_configuration_id, display_name:'DeepSeek V4.1 Flash', origin:{ kind:'feed', sequence:16, as_of:featured[1].included_on },
        availability: { kind: 'needs_connection' }, binding_ids: [], facts:null },
    ] });
    c.handlers.compute_connection_options = payload => ({ ...c.fixtureResponse('compute_connection_options', payload), metadata_catalog: catalog });
    const options = payload => {
      const value = c.fixtureResponse('plan_editor_options', payload);
      value.candidates = c.management.sources.flatMap(source => source.models.map(m => ({
        binding_id: m.binding_id, model_configuration_id: m.catalog_configuration_id ?? m.upstream_model_id,
        display_name: m.display_name, reasoning: m.native_reasoning ?? { kind: 'fixed', profile: 'provider-default' },
        billing_class: m.presentation?.billing_class ?? 'unknown',
        routable: source.state === 'ready' && m.presentation?.availability === 'available', ingress_protocols: ['responses','messages'],
      })));
      if (value.authoring) value.authoring.candidate_defaults.push({ binding_id: member.binding_id, economy: { kind:'profile',profile:'low' }, primary: { kind:'profile',profile:'high' } });
      return value;
    };
    c.handlers.plan_editor_options = options;
    c.handlers.trial_setup_snapshot = () => ({
      schema: 'hiroute.trial-setup-snapshot/v1',
      options: options({ input: { authoring:{} } }),
      management: { revisions:c.management.revisions, retained_api_connections:[], sources:c.management.sources.map(source=>({
        ...source, access_kind: source.connection_identity?.access_kind ?? 'api',
        models:source.models.map(model=>({ ...model, availability: model.presentation?.availability ?? 'unknown', catalog_admitted:!!model.catalog_configuration_id })),
      })) },
    });
    c.handlers.preview_plan_editor = ({input}) => {
      assertInput(input.action === 'save_draft');
      const currentPlan = c.desktop.catalog.plans.find(p => p.agent_plan_id === input.plan_id);
      const currentDraft = c.desktop.catalog.drafts.find(d => d.draft_id === input.draft_id);
      if (input.expected_head_revision !== currentPlan?.head.head_revision || input.expected_draft_revision !== (currentDraft?.revision ?? null)) throw {code:'DRAFT_REVISION_STALE'};
      const draft = {draft_id:input.draft_id,plan_id:input.plan_id,base_head_revision:input.expected_head_revision,revision:(input.expected_draft_revision??0)+1,editor:input.editor};
      c.desktop.catalog.drafts = [draft];
      return {state:'applied',operation:{operation_id:'operation/recommendation-draft',state:'succeeded',sequence:1,cancellable:false,safe_error_code:null}};
    };
    const assertInput = value => { if (!value) throw Error('unexpected recommendation fixture mutation'); };
    const connectionResult = {disposition:'saved',source_id:token.source_id,saved_revision:1,management_state:'ready',
      bindings:[{model_ref:member.model_ref,binding_id:member.binding_id,revision:member.revision}]};
    c.handlers.check_registered_model_connection_v2 = payload => {
      const result = c.fixtureResponse('check_registered_model_connection_v2', payload);
      const request = payload.request.request;
      assertInput(['bailian.token-plan.cn.v1','bailian.payg.cn.v1'].includes(request.connection_option_id) && request.inference_model_id === 'qwen4-max');
      const check = result.check;
      check.candidate.models.push({model_ref:member.model_ref,upstream_model_id:member.upstream_model_id,display_name:member.display_name,membership:'user_declared',selectable:true});
      check.inference = c.failRecommendationCheck ? 'failed' : 'verified';
      check.inference_model_id = 'qwen4-max';
      token.connection_identity.connection_option_id = request.connection_option_id;
      connectionResult.candidate = check.candidate.candidate;
      return result;
    };
    c.handlers.check_saved_model_connection_v2 = payload => {
      assertInput(payload.request.request.source_id === token.source_id);
      const result = c.fixtureResponse('check_registered_model_connection_v2',payload);
      result.check.candidate.existing_source_id = token.source_id;
      result.check.candidate.models = [{model_ref:member.model_ref,upstream_model_id:member.upstream_model_id,display_name:member.display_name,membership:'observed',selectable:true}];
      connectionResult.candidate = result.check.candidate.candidate;
      return result;
    };
    c.handlers.preview_compute_save = c.handlers.preview_compute_save_v4 = c.handlers.preview_compute_save_v3 = payload => {
      assertInput(JSON.stringify(payload.change.selected_model_refs) === JSON.stringify([member.model_ref]));
      return c.fixtureResponse('preview_compute_save', payload);
    };
    c.handlers.apply_compute_save = payload => {
      token.models = [member];
      if (!c.management.sources.includes(token)) c.management.sources.push(token);
      return c.fixtureResponse('apply_compute_save', payload);
    };
    c.handlers.get_compute_save_result = () => connectionResult;
    localStorage.setItem('hiroute.language',${JSON.stringify(language)});
    localStorage.setItem('hiroute.theme',${JSON.stringify(theme)});
    c.commands=[];c.remount();return true;
  })()`);
  await waitFor(client, 'document.querySelector(".new-model-card h3")?.textContent==="Qwen4-Max"');
  assert.equal(await evaluate(client, `document.querySelectorAll('.new-model-card h3')[1].textContent`), 'DeepSeek V4.1 Flash 1017');
}
const dialog = '.recommendation-dialog';
const button = `${dialog} button`;
const writes = 'productShell.commands.filter(c=>c.command==="preview_plan_editor")';

async function begin(language = 'zh', replace = true) {
  const en = language === 'en';
  await click('.new-model-card:first-child button', en ? 'Start configuration' : '开始配置');
  await click(button, replace ? (en ? 'Replace a model' : '替换已有路由里的模型') : (en ? 'Create a trial route' : '新建试用路由'));
}
async function position(language = 'zh') {
  await waitFor(client, `!!document.querySelector('${dialog} select option[value^="plan/"]')`);
  const planId = await evaluate(client, 'productShell.desktop.catalog.plans[0].agent_plan_id');
  await change(`${dialog} select`, planId);
  await change(`${dialog} select:has(option[value="economy/0"])`, 'economy/0');
  await click(button, language === 'en' ? 'Preview replacement' : '预览替换');
  assert.equal(await evaluate(client, `${writes}.length`), 0);
}
async function run(id, body) {
  try { await body(); results.push({id,status:'passed'}); console.log(`green: ${id}`); }
  catch (error) {
    await shot(`failure-${id}`).catch(() => {});
    results.push({id,status:'failed',error:String(error.stack ?? error),page:await evaluate(client,'document.body.innerText').catch(()=>null)});
    console.log(`red: ${id}: ${error.message}`);
  }
}

try {
  for (const [language, theme, width, height] of [['zh','light',1200,813],['en','dark',755,600]]) {
    await run(`exact-draft-${language}`, async () => {
      await setup({language,theme,width,height});
      await begin(language);
      const source = await evaluate(client, `[...document.querySelectorAll('${dialog} select option')].map(e=>e.textContent)`);
      assert.ok(!source.includes('Same ID on a different product'));
      assert.equal(await evaluate(client, `!!document.querySelector('${dialog} option[value^="plan/"]')`), false);
      await click(button, language === 'zh' ? '使用所选接入' : 'Use selected connection');
      await position(language);
      const geometry = await evaluate(client, `(()=>{const dialog=document.querySelector('${dialog}');const box=dialog.getBoundingClientRect();return dialog.scrollWidth<=dialog.clientWidth+1 && box.right<=innerWidth && box.bottom<=innerHeight;})()`);
      assert.equal(geometry, true);
      await shot(`recommendation-review-${language}-${theme}-${width}`);
      await click(button, language === 'zh' ? '保存草稿' : 'Save draft');
      await waitFor(client, `${writes}.length === 1 && !!document.querySelector('.routing-page .route-candidate[data-binding-id="${readyBinding}"]')`);
      const {before, after, write} = await evaluate(client, `({before:productShell.baseline,after:productShell.desktop.catalog,write:${writes}[0].payload.input})`);
      assert.equal(write.action,'save_draft');
      assert.equal(write.draft_id,'draft/existing');
      assert.equal(write.expected_draft_revision,3);
      assert.deepEqual(after.plans,before.plans);
      const expected = structuredClone(before.drafts[0].editor);
      expected.smart.economy[0] = {binding_id:readyBinding,reasoning:{kind:'profile',profile:'low'}};
      assert.deepEqual(after.drafts[0].editor,expected);
      assert.equal(after.drafts[0].revision,4);
    });
  }
  await run('fresh-connection-and-check', async () => {
    await setup({language:'en',ready:false,reuse:false,priorDraft:false});
    await begin('en');
    await click(button, 'Create a connection');
    await click(button, 'Alibaba Cloud Model Studio');
    await click(button, 'Token Plan');
    await change(`${dialog} input[type=password]`, 'local-test-input', 'input');
    await evaluate(client, 'productShell.failRecommendationCheck=true');
    await click(button,'Check and continue');
    await waitFor(client, 'productShell.commands.some(c=>c.command==="check_registered_model_connection_v2")');
    await waitFor(client, `[...document.querySelectorAll('${button}')].some(e=>!e.disabled&&e.getClientRects().length&&e.textContent.includes('Check and continue'))`);
    assert.equal(await evaluate(client, `!!document.querySelector('${dialog} option[value^="plan/"]')`),false);
    assert.equal(await evaluate(client, 'productShell.commands.some(c=>c.command==="apply_compute_save")'),false);
    await evaluate(client, 'productShell.failRecommendationCheck=false');
    await click(button,'Check and continue');
    await click(`${dialog} label:has(input[type=checkbox])`,'Qwen4-Max');
    await click(button,'Save connection');
    await position('en');
    await click(button,'Save draft');
    await waitFor(client, 'productShell.desktop.catalog.drafts.length===1');
    assert.equal(await evaluate(client, `${writes}[0].payload.input.expected_draft_revision`),null);
  });
  await run('append-to-saved-connection', async () => {
    await setup({language:'en',ready:false});
    await begin('en');
    await change(`${dialog} select`,savedSource);
    await waitFor(client,`document.querySelector('[data-trial-request-model]')?.value==='qwen4-max'`);
    await click('.trial-reuse-connection button','Check and add');
    await position('en');
    assert.equal(await evaluate(client, 'productShell.commands.filter(c=>c.command==="apply_compute_save").length'),1);
    assert.equal(await evaluate(client, 'productShell.commands.filter(c=>c.command==="register_protected_model_input").length'),0);
  });
  await run('saved-connection-changes-before-save', async () => {
    await setup();
    await begin();
    await click(button, '使用所选接入');
    await position();
    await evaluate(client,`productShell.management.sources.find(s=>s.source_id==='${savedSource}').state='disabled'`);
    await click(button,'保存草稿');
    await waitFor(client,`document.querySelector('${dialog} [role=alert]')?.textContent.includes('所选接入已变化')`);
    assert.equal(await evaluate(client,`${writes}.length`),0);
  });
  await run('back-and-draft-revision-conflict', async () => {
    await setup({language:'en'});
    await begin('en');
    await click(button,'Use selected connection');
    await position('en');
    await click(button,'Back');
    assert.equal(await evaluate(client,`document.querySelector('${dialog} select:has(option[value="economy/0"])').value`),'economy/0');
    await click(button,'Preview replacement');
    await evaluate(client,'productShell.desktop.catalog.drafts[0].revision=4; productShell.desktop.catalog.drafts[0].editor.purpose="External edit"');
    await click(button,'Save draft');
    await waitFor(client,`${writes}.length === 1 && !!document.querySelector('.routing-page [data-error-code="DRAFT_REVISION_STALE"]')`);
    const current = await evaluate(client,'productShell.desktop.catalog');
    assert.equal(current.drafts[0].editor.purpose,'External edit');
    assert.deepEqual(current.plans,await evaluate(client,'productShell.baseline.plans'));
  });
  await run('existing-trial-handoffs', async () => {
    await setup({language:'en'});
    await begin('en',false);
    await click(button,'Use selected connection');
    await waitFor(client,`document.querySelector('.trial-page [data-trial-target=trial]')?.textContent.includes('Qwen4-Max')`);
    assert.equal(await evaluate(client,`${writes}.length`),0);
    await setup({language:'en',route:false});
    await click('.new-model-card:nth-child(2) button','Start configuration');
    assert.equal(await evaluate(client,`[...document.querySelectorAll('${button}')].find(e=>e.textContent.includes('Replace a model')).disabled`),true);
    await click(button,'Create a trial route');
    await waitFor(client,`[...document.querySelectorAll('.trial-model-card')].some(e=>e.textContent.includes('DeepSeek')&&e.getAttribute('aria-pressed')==='true')`);
    assert.equal(await evaluate(client,`${writes}.length`),0);
  });
  await run('read-only', async () => {
    await setup({mutable:false});
    assert.equal(await evaluate(client,`document.querySelector('.new-model-card:first-child .btn-primary').disabled`),true);
    assert.equal(await evaluate(client,`${writes}.length`),0);
  });
} finally {
  client.close();
  await writeFile(path.join(output,'recommendation-setup.json'),JSON.stringify(results,null,2));
}
if (results.some(result=>result.status==='failed')) process.exitCode=1;
