import { useState } from 'react';
import { TrialQuickStart } from '../../../src/features/trial/TrialQuickStart';
import { NewModels } from '../../../src/features/trial/NewModels';
import { readyDesktop, readyAgents, readyManagement, dailyPlan, fixturePlanAuthoring, mockProductInvoke } from './product-fixtures';
import { PlanEditor } from '../../../src/plan-editor';
import type { DesktopOperation, DesktopSnapshot } from '../../../src/product/home-projections';
import type { ModelFeedStatus } from '../../../src/features/model-feed/types';
import { trialAgentRead } from './trial-agent-fixture';
import type { Agent } from '../../../src/features/agents/types';
const desktop = structuredClone(readyDesktop); desktop.catalog = { plans: [], drafts: [] }; desktop.pending = null;
const agents = structuredClone(readyAgents); agents.agents = [{...agents.agents.find(a=>a.agent_id==='agent_claude_default')!,context_id:'agent-context/claude/default',settings:{state:'not_configured',restore_point_ref:null,model_verified:false}}];agents.plans.plans=[];
const additionalFamily = new URLSearchParams(location.search).get('agent-family');
if (['dsh', 'pi', 'qoder', 'opencode'].includes(additionalFamily ?? '')) agents.agents.push({
 ...structuredClone(agents.agents[0]), agent_id:`agent_${additionalFamily}_default`, context_id:`agent-context/${additionalFamily}/default`,
});
const retiredContexts = new Map<string, Agent>();
if (additionalFamily === 'codex') agents.agents.push({
 ...structuredClone(readyAgents.agents.find(a=>a.agent_id==='agent_codex_default')!),
 context_id:'agent-context/codex/root',
 codex_access:{codex_home:'/workspace/codex',slot_id:'codex',root_context_id:'agent-context/codex/root',profile_context_id:'agent-context/codex/profile',selected_mode:'root',slot_occupied:true,target_file:'config.toml',profile_name:'hiroute',commands:{},pending_operation:null,access_revoked:false,conflict_fields:[]},
});
const decision = {id:'decision/trial',revision:1,name:'Saved decision model',connection:{kind:'custom',endpoint:'https://decision.example.test',timeout_ms:10000}};
let observed:(DesktopOperation & {accepted_digest?:string})|null=null;
let publishLost=false;
const trialWrites = { pending: [] as { action: string; release(): void }[] };
Object.assign(window, { trialWrites });
const feedStatus: ModelFeedStatus = {retrieval_enabled:true,held:{sequence:12,issued_at:'2026-10-16T00:00:00Z',retrieved_at:1792108800000},highest_accepted_sequence:12,last_attempt:{at:1792108800000,result:'ok',reason:null},verification:{kind:'validated'},counts:{descriptive_records:2,qualified_models:2,restricted_models:0},next_automatic_attempt_at:null,disclosure:'Model information is retrieved automatically from hiroute.ai without account or usage identifiers.',disclosure_acknowledged:false};
const catalog: { model_configuration_id: string; display_name: string; binding_ids: string[]; [key: string]: unknown }[] = [
 ...readyManagement.sources.flatMap(source=>source.models.map(model=>({model_configuration_id:model.catalog_configuration_id??model.upstream_model_id,display_name:model.display_name,origin:{kind:'embedded',client_version:'0.3.1'},identity_stability:'rolling-alias',availability:{kind:'executable'},binding_ids:[model.binding_id],facts:{publisher_id:model.binding_id.includes('qwen')?'publisher.alibaba':'publisher.openai',context_tokens:262144,tool:true,vision:false,reasoning:{kind:'toggle'}}}))),
 {model_configuration_id:'model.trial',display_name:'Kimi K3',origin:{kind:'feed',sequence:12,as_of:'2026-10-16'},identity_stability:'rolling-alias',availability:{kind:'needs_connection'},binding_ids:[],facts:{publisher_id:'publisher.moonshot',context_tokens:262144,tool:true,vision:false,reasoning:{kind:'toggle'}}},
 {model_configuration_id:'model.openai.gpt-6-luna',display_name:'GPT-6 Luna',origin:{kind:'feed',sequence:12,as_of:'2026-10-14'},identity_stability:'pinned',availability:{kind:'needs_newer_client',version:'0.3.3'},binding_ids:[],facts:{publisher_id:'publisher.openai',context_tokens:400000,tool:true,vision:true,reasoning:null}},
];
const featured = ['model.trial','model.openai.gpt-6-luna'].map(id=>({id:`featured/${id}`,kind:'new_model',model_configuration_id:id,headline:{en:id,zh:id},note:{en:'',zh:''},links:[],included_on:'2026-10-16'}));
export const trialModel = catalog.find(m=>m.binding_ids.includes('binding/bailian/qwen-coder'))!;
export const primaryModel = catalog.find(m=>m.binding_ids.includes('binding/codex/gpt-5.6-sol'))!;
const sameNames = new URLSearchParams(location.search).has('same-names');
if (new URLSearchParams(location.search).has('long-model-names')) {
 trialModel.display_name = '试'.repeat(80);
 primaryModel.display_name = '🚀'.repeat(80);
}
if (new URLSearchParams(location.search).has('unsafe-model-name')) trialModel.display_name='https://example.invalid';
const trialManagement = structuredClone(readyManagement);
const setupParams = new URLSearchParams(location.search);
// This registered fixture represents the catalog member selected on entry.
// The saved-only variant instead exercises a user-declared, unlinked model.
if (setupParams.has('saved-only')) trialManagement.sources[1].models[0].membership = 'user_declared';
else trialManagement.sources[1].models[0].catalog_configuration_id = trialModel.model_configuration_id;
if (setupParams.has('catalog-unadmitted-user')) trialManagement.sources[1].provenance='user_configured';
const setupControl = { failNextManagement: false, holdNextManagement: false, holdNextDecision: false, pending: [] as (()=>void)[], pendingDecision: [] as (()=>void)[],
 snapshotFailures: [] as string[], catalogAdmission: !setupParams.has('catalog-unadmitted-user'),
 recommendationReads: 0, recommendationFailures: 0, holdNextRecommendation: false, pendingRecommendations: [] as (()=>void)[],
 pendingApply: [] as (()=>void)[],
 pendingCheck: [] as (()=>void)[], checkCount: 0,
 checkedSubscription: null as any,
 trialReasoningMode: setupParams.has('no-trial-reasoning-default') ? 'toggle' : 'default',
 appendChange: null as any,
 setTrialAvailability(available:boolean) { trialManagement.sources[1].models[0].presentation!.availability=available?'available':'disabled'; },
 changeSaved(action:'rename'|'disable'|'remove') {
  const source=trialManagement.sources.find(item=>item.source_id==='source/fixture/new');
  if (!source) return;
  if (action==='remove') trialManagement.sources=trialManagement.sources.filter(item=>item.source_id!==source.source_id);
  else if (action==='disable') source.state='disabled';
  else { source.display_name='Renamed synthetic API'; source.revision++; }
 },
 release() { this.pending.shift()?.(); },
 releaseApply() { this.pendingApply.shift()?.(); },
 releaseDecision() { this.pendingDecision.shift()?.(); },
 releaseCheck() { this.pendingCheck.shift()?.(); },
 releaseRecommendation() { this.pendingRecommendations.shift()?.(); },
 replacePending() { if (desktop.pending) desktop.pending.idempotency_key='synthetic-other-operation'; },
};
if (setupParams.has('disabled-trial')) setupControl.setTrialAvailability(false);
if (setupParams.has('append-fixture')) trialManagement.sources[1].actions.push('append_models');
if (setupParams.has('subscription-append')) trialManagement.sources[0].actions.push('append_models');
if (setupParams.has('unlinked-existing-collision')) {
 const source=structuredClone(trialManagement.sources[1]);
 source.source_id='source/fixture/new'; source.revision=1; source.display_name='Saved synthetic API';
 source.models=[{...source.models[0],model_ref:'model-ref/qwen3-coder-plus',binding_id:'binding/fixture/saved-model',revision:1,upstream_model_id:'fixture-unlinked-model',display_name:'Saved synthetic model',catalog_configuration_id:undefined,membership:'user_declared'}];
 trialManagement.sources.push(source);
}
Object.assign(window,{trialSetupControl:setupControl});
if (sameNames) {
 const duplicate = structuredClone(trialManagement.sources[0]);
 duplicate.source_id = 'source/second';
 duplicate.display_name = 'A second provider with a deliberately long connection name';
 duplicate.models = [{...duplicate.models[0], model_ref:'model-ref/second', binding_id:'binding/second/same-name', catalog_configuration_id:'model.same-name'}];
 trialManagement.sources.push(duplicate);
 const sameSource = structuredClone(trialManagement.sources[1]);
 sameSource.source_id = 'source/bailian/second';
 sameSource.models = [{...sameSource.models[0], model_ref:'model-ref/qwen-second', binding_id:'binding/bailian/qwen-second'}];
 trialManagement.sources.push(sameSource);
 catalog.push({...structuredClone(primaryModel),model_configuration_id:'model.same-name',binding_ids:['binding/second/same-name']});
 trialModel.binding_ids.push('binding/bailian/qwen-second');
}
if (new URLSearchParams(location.search).has('feed-trial')) trialModel.origin = {kind:'feed',sequence:12,as_of:'2026-10-16'};
Object.assign(window,{trialModel,primaryModel});
const agentControl = {
 hold: new URLSearchParams(location.search).has('agent-hold'), failNext: false,
 pending: [] as { context: string; release(): void }[],
 mappedContexts: [] as string[], rejectedContexts: [] as string[],
 setTrusted(value: boolean) { agents.trusted_authority=value; },
 setCodexMode(mode: 'root' | 'profile') {
  const agent=agents.agents.find(a=>a.agent_id==='agent_codex_default')!;
  retiredContexts.set(agent.context_id!,{...structuredClone(agent),settings:{state:'restored',restore_point_ref:null,model_verified:false}});
  agent.context_id=`agent-context/codex/${mode}`;
  agent.codex_access={...agent.codex_access!,selected_mode:mode};
 },
 setClaudePlan(name: string) {
  const agent = agents.agents[0];
  agent.settings = {state:'configured',restore_point_ref:null,model_verified:false,current_selection:{mode:'claude_launcher',surfaces:['claude_cli'],fixed_models:[],preset_mappings:{opus:{kind:'plan',plan_id:name},sonnet:{kind:'preserve_native'},haiku:{kind:'preserve_native'}}}};
  agents.plans.plans.push({...structuredClone(dailyPlan),agent_plan_id:name,desired:{...structuredClone(dailyPlan.desired),display_name:name}});
 },
};
Object.assign(window,{trialAgentControl:agentControl});
export async function quickStartInvoke(command:string,payload:any,params:URLSearchParams) {
 if(command==='trial_agent_choices') return trialAgentRead(agents,command,payload);
 if(command==='trial_agent_model') {
  if(agentControl.failNext){agentControl.failNext=false;throw Error('AGENT_STATUS_UNAVAILABLE');}
  const retired=retiredContexts.get(payload.contextId);
  const result=trialAgentRead(retired ? {...agents,agents:[retired]} : agents,command,payload);
  if(agentControl.hold) await new Promise<void>(release=>agentControl.pending.push({context:payload.contextId,release}));
  return result;
 }
 if(command==='trial_setup_snapshot') {
  const failure=setupControl.snapshotFailures.shift(); if(failure) throw {source:'native',code:failure};
  const options:any=await quickStartInvoke('plan_editor_options',{input:{authoring:{},requirements:{},native_selections:{},suggest_free:false}},params);
  const catalogAtCapture=structuredClone(catalog);
  const management:any=await quickStartInvoke('compute_management_snapshot',{},params);
  management.sources=management.sources.map((source:any)=>({source_id:source.source_id,revision:source.revision,display_name:source.display_name,provenance:source.provenance,access_kind:source.connection_identity?.access_kind??'unknown',state:source.state,actions:source.actions,models:source.models.map((member:any)=>({model_ref:member.model_ref,binding_id:member.binding_id,revision:member.revision,upstream_model_id:member.upstream_model_id,display_name:member.display_name,catalog_configuration_id:member.catalog_configuration_id,membership:member.membership,availability:member.presentation?.availability??'unknown',reason_code:member.presentation?.reason_code,catalog_admitted:setupControl.catalogAdmission && !!member.catalog_configuration_id && options.candidates.some((candidate:any)=>candidate.binding_id===member.binding_id && candidate.routable && candidate.model_configuration_id===member.catalog_configuration_id) && !!catalogAtCapture.find(item=>item.model_configuration_id===member.catalog_configuration_id)?.binding_ids.includes(member.binding_id)}))}));
  management.retained_api_connections=[];
  if (params.has('production-capture')) management.retained_api_connections.push(structuredClone((window as any).trialRetainedCapture.connection));
  if (params.has('retained-api')) {
   const source=management.sources.find((value:any)=>value.source_id==='source/bailian/coding');
   const member=source.models[0];
   management.retained_api_connections.push({source_id:source.source_id,source_revision:source.revision,binding_id:member.binding_id,binding_revision:member.revision,model_configuration_id:params.has('retained-mismatch')?'model.different':trialModel.model_configuration_id,display_name:'Retained fixture API'});
   if (!params.has('retained-v2-owner')) management.sources=management.sources.filter((value:any)=>value.source_id!==source.source_id);
  }
  return {schema:'hiroute.trial-setup-snapshot/v1',options,management};
 }
 if(command==='model_availability') {
  setupControl.recommendationReads++;
  if ((params.has('recommendations-fail') && setupControl.recommendationReads===2) || setupControl.recommendationFailures>0) {
   if (setupControl.recommendationFailures>0) setupControl.recommendationFailures--;
   throw Error('SYNTHETIC_RECOMMENDATION_FAILURE');
  }
  const value=structuredClone({featured:params.has('saved-only')?[]:featured,models:params.has('saved-only')?[]:catalog});
  if (setupControl.holdNextRecommendation) {setupControl.holdNextRecommendation=false;await new Promise<void>(release=>setupControl.pendingRecommendations.push(release));}
  return value;
 }
 if(command==='desktop_snapshot') return structuredClone(desktop);
 if(command==='compute_management_snapshot') {
  const value=structuredClone(trialManagement);
  if (setupControl.failNextManagement) { setupControl.failNextManagement=false; throw Error('SYNTHETIC_MANAGEMENT_READ_FAILED'); }
  if (setupControl.holdNextManagement) { setupControl.holdNextManagement=false; await new Promise<void>(release=>setupControl.pending.push(release)); }
  return value;
 }
 if(command==='model_feed_status') return structuredClone(feedStatus);
 if(command==='model_feed_settings_apply') {if(payload.input.retrieval_enabled!==undefined) feedStatus.retrieval_enabled=payload.input.retrieval_enabled; if(payload.input.acknowledge_disclosure) feedStatus.disclosure_acknowledged=true; return structuredClone(feedStatus);}
 if(command==='agent_snapshot') return structuredClone(agents);
 if(command==='decision_services') {
  if (setupControl.holdNextDecision) { setupControl.holdNextDecision=false; await new Promise<void>(release=>setupControl.pendingDecision.push(release)); }
  const native=params.has('native')?{...decision,connection:{kind:'system_one',provider:'compatible',model:'fixture',endpoint:'https://decision.example.test',timeout_ms:10000,auth_header:{name:'Authorization',value_secret_ref:'fixture'}}}:decision;
  return {services:params.has('custom-service')?[native,{...decision,id:'decision/custom-fixture',name:'Saved extension'}]:[native]};
 }
 if(command==='plan_editor_options') {
  const value:any = await mockProductInvoke(command,payload,'ready');
  if (params.has('production-capture')) value.candidates.push(structuredClone((window as any).trialRetainedCapture.candidate));
  if (payload?.input?.authoring?.plan_id === 'plan/trial') value.authoring=fixturePlanAuthoring(desktop.catalog.plans.find(plan=>plan.agent_plan_id==='plan/trial'));
  if (trialManagement.sources.some(source=>source.source_id==='source/fixture/new')) {
   value.candidates.push({binding_id:'binding/fixture/saved-model',model_configuration_id:params.has('same-model-save')?trialModel.model_configuration_id:params.has('saved-registered') || params.has('saved-id-collision') || params.has('unlinked-existing-collision')?'model.trial':'fixture-unlinked-model',display_name:'Saved synthetic model',reasoning:{kind:'fixed',profile:'provider-default'},billing_class:'paid',routable:true,ingress_protocols:['responses','messages']});
   if (params.has('saved-multiple') || params.has('saved-extra-binding')) value.candidates.push({binding_id:'binding/fixture/saved-second',model_configuration_id:'fixture-second-model',display_name:'Second synthetic model',reasoning:{kind:'fixed',profile:'provider-default'},billing_class:'paid',routable:true,ingress_protocols:['responses','messages']});
  }
  if (trialManagement.sources[1].models.some(member=>member.binding_id==='binding/fixture/appended')) value.candidates.push({binding_id:'binding/fixture/appended',model_configuration_id:'fixture-added-model',display_name:'Added synthetic model',reasoning:{kind:'fixed',profile:'provider-default'},billing_class:'paid',routable:true,ingress_protocols:['responses','messages']});
  if (trialManagement.sources[0].models.some(member=>member.binding_id==='binding/fixture/sub-added')) value.candidates.push({binding_id:'binding/fixture/sub-added',model_configuration_id:'fixture-sub-added',display_name:'Added subscription model',reasoning:{kind:'fixed',profile:'provider-default'},billing_class:'subscription',routable:true,ingress_protocols:['responses']});
  if (setupControl.trialReasoningMode==='toggle') value.candidates.find((candidate:any)=>candidate.binding_id==='binding/bailian/qwen-coder').reasoning={kind:'toggle'};
  const primary = value.candidates.find((candidate:any) => candidate.binding_id === 'binding/codex/gpt-5.6-sol');
  if (params.has('long-model-names') || params.has('unsafe-model-name')) {
   value.candidates.find((candidate:any)=>candidate.binding_id==='binding/bailian/qwen-coder').display_name=trialModel.display_name;
   primary.display_name=primaryModel.display_name;
  }
  if (primary?.reasoning.kind === 'discrete' && !params.has('no-reasoning-default')) primary.reasoning.default_profile = 'high';
  if (sameNames) {
   value.candidates.push({...structuredClone(primary),binding_id:'binding/second/same-name',model_configuration_id:'model.same-name'});
   const trial = value.candidates.find((candidate:any) => candidate.binding_id === 'binding/bailian/qwen-coder');
   value.candidates.push({...structuredClone(trial),binding_id:'binding/bailian/qwen-second'});
  }
  if (payload?.input?.preview_agent === 'dsh') {
   value.codex_capabilities=null; value.claude_capabilities=null;
   if (payload.input.editor) value.dsh_capabilities={state:'available',protocol:'responses',context_window:64000,max_output_tokens:4096};
  }
  if (!('quickStartAuthoringJudgment' in window)) Object.assign(window, { quickStartAuthoringJudgment: structuredClone((value as any).authoring.editor.smart.judgment) });
  return value;
 }
 if(command==='observe_operation') {
  if(params.has('lost-apply') && desktop.pending && observed) desktop.pending.operation_id=observed.operation_id;
  return observed;
 }
 if(command==='preview_plan_editor') {
  const i=payload.input;
  if (params.has('hold-writes')) await new Promise<void>(release => trialWrites.pending.push({ action:i.action, release }));
  observed={operation_id:`operation/${i.action}`,state:'succeeded',sequence:1,cancellable:false,safe_error_code:null};
  desktop.pending={plan_id:i.plan_id??i.draft_id,operation_id:observed.operation_id,idempotency_key:'fixture'};
  if(i.action==='save_draft') desktop.catalog.drafts=[{draft_id:i.draft_id,revision:1,editor:structuredClone(i.editor)}];
  if(i.action==='publish') {
   const plan={...structuredClone(dailyPlan),agent_plan_id:'plan/trial',model_alias:i.editor.custom_alias,head:{head_revision:1,status:'enabled'},agent_plan_revision:1,desired:{...structuredClone(dailyPlan.desired),display_name:i.editor.display_name,purpose:i.editor.purpose,mode:i.editor.mode,strategy:{mode:'smart_saving',...i.editor.smart}}};
   desktop.catalog.plans=[plan];desktop.catalog.drafts=[];agents.plans.plans=[plan];
   if(params.has('lost')&&!publishLost){publishLost=true;throw Error('TRANSPORT_LOST_AFTER_COMMIT');}
  }
  return {state:'succeeded',operation:observed};
 }
 if(command==='preview_agent_settings') {
  const codex=agents.agents.find(a=>a.agent_id==='agent_codex_default');
  if(codex && payload.input.spec.context_id.startsWith('agent-context/codex/') && payload.input.spec.context_id!==codex.context_id) {
   agentControl.rejectedContexts.push(payload.input.spec.context_id);
   return {preview:{applicable:false,blockers:[{reason:'codex_slot_conflict'}]}};
  }
  const agent=agents.agents.find(a=>a.context_id===payload.input.spec.context_id)!;
  agentControl.mappedContexts.push(payload.input.spec.context_id);
  const selection=structuredClone(payload.input.spec.model.settings);
  // Match the native snapshot's map order rather than echoing the form object.
  if (selection.mode==='claude_launcher') {
   const p=selection.preset_mappings;
   selection.preset_mappings={haiku:p.haiku,opus:p.opus,sonnet:p.sonnet};
  }
  agent.settings={state:'configured',restore_point_ref:'restore/test',model_verified:false,operation_id:'operation/map',operation_state:'succeeded',current_selection:selection};
  return {preview:{applicable:true,blockers:[]},mutation:{state:'applied',operation:{operation_id:'operation/map',state:'succeeded',sequence:1,cancellable:false}}};
 }
 if(command==='observation_read') {
  if (payload.request.intent.view==='plan_trust_replay') return {included:0,excluded_by_reason:{},recorded_share_at_current:null,estimate_only:true,thresholds:payload.request.intent.query.thresholds_millis.map((threshold:number)=>({threshold_millis:threshold,regular:0,primary:0,protected_as_primary:0,regular_share:null,primary_share:null,newly_regular:0}))};
  return {rows:[],branches:[],sessions:[],announcements:[]};
 }
 if(command==='compute_connection_options') { const value:any=await mockProductInvoke(command,payload,'ready'); for(const option of value.options) if(option.connection_option_id==='kimi.open-platform.cn.v1') option.model_configuration_ids=[params.has('additional-connection')?trialModel.model_configuration_id:'model.trial']; return value; }
 if(command==='check_registered_model_connection' && params.has('saved-registered')) {
  const value:any=await mockProductInvoke(command,payload,'ready');
  setupControl.checkCount++;
  if (params.has('check-race') && setupControl.checkCount===1) {
   value.observed_model_ids=[];
   await new Promise<void>(release=>setupControl.pendingCheck.push(release));
  } else value.observed_model_ids=['qwen3-coder-plus'];
  return value;
 }
 if(command==='check_model_connection' && params.has('saved-custom')) return mockProductInvoke(command,payload,'unknown_model');
 if(command==='compute_subscriptions' && params.has('subscription-append')) {
  const result:any=await mockProductInvoke(command,payload,'ready');
  if (setupControl.checkedSubscription) result.candidates=[structuredClone(setupControl.checkedSubscription.checked_candidate)];
  return result;
 }
 if(command==='check_subscription' && params.has('subscription-append')) {
  const result:any=await mockProductInvoke(command,payload,'ready');
  result.checked_candidate.existing_source_id='source/codex/subscription';
  result.checked_candidate.models=[...trialManagement.sources[0].models.map(member=>({model_ref:member.model_ref,upstream_model_id:member.upstream_model_id,display_name:member.display_name,membership:member.membership,selectable:true})),{model_ref:'model-ref/sub-added',upstream_model_id:'fixture-sub-added',display_name:'Added subscription model',membership:'observed',selectable:true}];
  setupControl.checkedSubscription=result;
  return result;
 }
 if(command==='get_subscription_check_result' && params.has('subscription-append') && setupControl.checkedSubscription) return structuredClone(setupControl.checkedSubscription);
 if(command==='check_saved_model_connection' && params.has('append-fixture')) {
  const result:any=await mockProductInvoke(command,payload,'ready');
  result.candidate.models.push({model_ref:'model-ref/append-new',upstream_model_id:'fixture-added-model',display_name:'Added synthetic model',membership:'observed',selectable:true});
  return result;
 }
 if(command==='preview_compute_save' && (params.has('append-fixture') || params.has('subscription-append'))) {
  setupControl.appendChange=structuredClone(payload.change);
  return mockProductInvoke(command,payload,'ready');
 }
 if(command==='apply_compute_save' && params.has('append-fixture') && setupControl.appendChange?.edit?.action==='append_models') {
  const source=trialManagement.sources[1];
  if (!source.models.some(member=>member.binding_id==='binding/fixture/appended')) {
   source.revision++;
   source.models.push({...structuredClone(source.models[0]),model_ref:'model-ref/append-new',binding_id:'binding/fixture/appended',revision:1,upstream_model_id:'fixture-added-model',display_name:'Added synthetic model',catalog_configuration_id:undefined,membership:'observed'});
  }
  return mockProductInvoke(command,payload,'ready');
 }
 if(command==='apply_compute_save' && params.has('subscription-append') && setupControl.appendChange?.edit?.action==='append_models') {
  const source=trialManagement.sources[0];
  if (!source.models.some(member=>member.binding_id==='binding/fixture/sub-added')) {
   source.revision++;
   source.models.push({...structuredClone(source.models[0]),model_ref:'model-ref/sub-added',binding_id:'binding/fixture/sub-added',revision:1,upstream_model_id:'fixture-sub-added',display_name:'Added subscription model',catalog_configuration_id:undefined,membership:'observed',native_reasoning:{kind:'fixed',profile:'provider-default'}});
  }
  return mockProductInvoke(command,payload,'ready');
 }
 if(command==='get_compute_save_result' && params.has('subscription-append')) return {disposition:'saved',source_id:'source/codex/subscription',saved_revision:trialManagement.sources[0].revision,bindings:trialManagement.sources[0].models.map(member=>({model_ref:member.model_ref,binding_id:member.binding_id,revision:member.revision}))};
 if(command==='apply_compute_save' && params.has('lost-apply')) {
  const request=payload.request;
  desktop.pending={plan_id:'compute-management',operation_id:null,idempotency_key:request.idempotency_key,operation_kind:'ApplyComputeSave',accepted_digest:request.accept_digest} as typeof desktop.pending;
  observed={operation_id:'operation/fixture-model-save',state:'succeeded',sequence:1,cancellable:false,safe_error_code:null,accepted_digest:request.accept_digest};
  throw Error('SYNTHETIC_REPLY_LOST_AFTER_ACCEPTANCE');
 }
 if(command==='apply_compute_save' && params.has('hold-apply')) {
  const result=await mockProductInvoke(command,payload,'ready');
  await new Promise<void>(release=>setupControl.pendingApply.push(release));
  return result;
 }
 if(command==='get_compute_save_result' && (params.has('saved-user-declared') || params.has('saved-registered'))) {
  // Keep the receipt, management snapshot and editor candidates separate, as
  // in the observed save: an available declaration has no catalog identity.
  const binding = {model_ref:params.has('saved-custom')?'model-ref/custom-0':'model-ref/qwen3-coder-plus', binding_id:'binding/fixture/saved-model', revision:params.has('saved-revision-mismatch')?2:1};
  if (!trialManagement.sources.some(source=>source.source_id==='source/fixture/new')) {
   const source = structuredClone(trialManagement.sources[1]);
   source.source_id='source/fixture/new'; source.revision=1; source.display_name='Saved synthetic API'; source.state='ready';
   source.models=[{...source.models[0], ...binding, revision:1, upstream_model_id:'fixture-unlinked-model', display_name:'Saved synthetic model', catalog_configuration_id:params.has('saved-registered')?(params.has('same-model-save')?trialModel.model_configuration_id:'model.trial'):undefined, membership:params.has('saved-registered')?'observed':'user_declared'}];
   if (params.has('saved-multiple') || params.has('saved-extra-binding')) source.models.push({...structuredClone(source.models[0]),model_ref:'model-ref/qwen3-max',binding_id:'binding/fixture/saved-second',upstream_model_id:'fixture-second-model',display_name:'Second synthetic model'});
   trialManagement.sources.push(source);
   if (params.has('saved-registered')) catalog.find(m=>m.model_configuration_id===(params.has('same-model-save')?trialModel.model_configuration_id:'model.trial'))!.binding_ids.push(binding.binding_id);
  }
  if (params.has('saved-read-failure')) setupControl.failNextManagement=true;
  const bindings = params.has('saved-multiple') || params.has('saved-extra-binding') ? [binding,{model_ref:'model-ref/qwen3-max',binding_id:'binding/fixture/saved-second',revision:1}] : [binding];
  return {disposition:'saved', source_id:'source/fixture/new', saved_revision:1, management_state:'ready', candidate:{candidate_ref:'candidate/protected',candidate_revision:params.has('saved-candidate-mismatch')?3:2}, bindings};
 }
 return mockProductInvoke(command,payload,'ready');
}
export function TrialFixture({language}:{language:'zh'|'en'}) {
 const [currentLanguage,setCurrentLanguage]=useState(language);
 const [snapshot,setSnapshot]=useState<DesktopSnapshot>(structuredClone(desktop));
 const [operation,setOperation]=useState<DesktopOperation|null>(null);
 const [open,setOpen]=useState(false);
 const [reopened,setReopened]=useState(false);
 Object.assign(window,{reopenPublishedTrial:()=>setReopened(true),setTrialLanguage:setCurrentLanguage});
 if (reopened && desktop.catalog.plans[0]) return <div className="trial-page-host" style={{height:'100vh'}}><PlanEditor plan={desktop.catalog.plans[0]} language={currentLanguage} onDone={async()=>{}} onClose={()=>setReopened(false)} /></div>;
 return open ? <div className="trial-page-host" style={{height:'100vh'}}><TrialQuickStart initialModelId={trialModel.model_configuration_id} initialBindingId="binding/bailian/qwen-coder" language={currentLanguage} mutable snapshot={snapshot} operation={operation} onOperation={setOperation} onRefresh={async()=>{setSnapshot(structuredClone(desktop));setOperation(observed);}} onClose={()=>setOpen(false)} onOpenAgent={()=>{}} /></div> : <NewModels language={currentLanguage} active plans={snapshot.catalog.plans} refreshVersion={0} onTry={()=>setOpen(true)} />;
}
