import { useState } from 'react';
import { TrialQuickStart } from '../../../src/features/trial/TrialQuickStart';
import { NewModels } from '../../../src/features/trial/NewModels';
import { readyDesktop, readyAgents, readyManagement, dailyPlan, mockProductInvoke } from './product-fixtures';
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
let observed:DesktopOperation|null=null;
let publishLost=false;
const feedStatus: ModelFeedStatus = {retrieval_enabled:true,held:{sequence:12,issued_at:'2026-10-16T00:00:00Z',retrieved_at:1792108800000},highest_accepted_sequence:12,last_attempt:{at:1792108800000,result:'ok',reason:null},verification:{kind:'validated'},counts:{descriptive_records:2,qualified_models:2,restricted_models:0},next_automatic_attempt_at:null,disclosure:'Model information is retrieved automatically from hiroute.ai without account or usage identifiers.',disclosure_acknowledged:false};
const catalog: { model_configuration_id: string; display_name: string; binding_ids: string[]; [key: string]: unknown }[] = [
 ...readyManagement.sources.flatMap(source=>source.models.map(model=>({model_configuration_id:model.catalog_configuration_id??model.upstream_model_id,display_name:model.display_name,origin:{kind:'embedded',client_version:'0.3.1'},identity_stability:'rolling-alias',availability:{kind:'executable'},binding_ids:[model.binding_id],facts:{publisher_id:model.binding_id.includes('qwen')?'publisher.alibaba':'publisher.openai',context_tokens:262144,tool:true,vision:false,reasoning:{kind:'toggle'}}}))),
 {model_configuration_id:'model.trial',display_name:'Kimi K3',origin:{kind:'feed',sequence:12,as_of:'2026-10-16'},identity_stability:'rolling-alias',availability:{kind:'needs_connection'},binding_ids:[],facts:{publisher_id:'publisher.moonshot',context_tokens:262144,tool:true,vision:false,reasoning:{kind:'toggle'}}},
 {model_configuration_id:'model.openai.gpt-6-luna',display_name:'GPT-6 Luna',origin:{kind:'feed',sequence:12,as_of:'2026-10-14'},identity_stability:'pinned',availability:{kind:'needs_newer_client',version:'0.3.3'},binding_ids:[],facts:{publisher_id:'publisher.openai',context_tokens:400000,tool:true,vision:true,reasoning:null}},
];
const featured = ['model.trial','model.openai.gpt-6-luna'].map(id=>({id:`featured/${id}`,kind:'new_model',model_configuration_id:id,headline:{en:id,zh:id},note:{en:'',zh:''},links:[],included_on:'2026-10-16'}));
export const trialModel = catalog.find(m=>m.binding_ids.includes('binding/bailian/qwen-coder'))!;
export const primaryModel = catalog.find(m=>m.binding_ids.includes('binding/codex/gpt-5.6-sol'))!;
if (new URLSearchParams(location.search).has('feed-trial')) trialModel.origin = {kind:'feed',sequence:12,included_on:'2026-10-16'};
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
 if(command==='model_availability') return {featured:params.has('saved-only')?[]:featured,models:params.has('saved-only')?[]:catalog};
 if(command==='desktop_snapshot') return structuredClone(desktop);
 if(command==='model_feed_status') return structuredClone(feedStatus);
 if(command==='model_feed_settings_apply') {if(payload.input.retrieval_enabled!==undefined) feedStatus.retrieval_enabled=payload.input.retrieval_enabled; if(payload.input.acknowledge_disclosure) feedStatus.disclosure_acknowledged=true; return structuredClone(feedStatus);}
 if(command==='agent_snapshot') return structuredClone(agents);
 if(command==='decision_services') return {services:[params.has('native')?{...decision,connection:{kind:'system_one',provider:'compatible',model:'fixture',endpoint:'https://decision.example.test',timeout_ms:10000,auth_header:{name:'Authorization',value_secret_ref:'fixture'}}}:decision]};
 if(command==='plan_editor_options') {
  const value = await mockProductInvoke(command,payload,'ready');
  if (!('quickStartAuthoringJudgment' in window)) Object.assign(window, { quickStartAuthoringJudgment: structuredClone((value as any).authoring.editor.smart.judgment) });
  return value;
 }
 if(command==='observe_operation') return observed;
 if(command==='preview_plan_editor') {
  const i=payload.input;
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
 if(command==='observation_read') return {rows:[],branches:[],sessions:[],announcements:[]};
 if(command==='compute_connection_options') { const value:any=await mockProductInvoke(command,payload,'ready'); for(const option of value.options) if(option.connection_option_id==='kimi.open-platform.cn.v1') option.model_configuration_ids=['model.trial']; return value; }
 return mockProductInvoke(command,payload,'ready');
}
export function TrialFixture({language}:{language:'zh'|'en'}) {
 const [snapshot,setSnapshot]=useState<DesktopSnapshot>(structuredClone(desktop));
 const [operation,setOperation]=useState<DesktopOperation|null>(null);
 const [open,setOpen]=useState(false);
 return open ? <div className="trial-page-host" style={{height:'100vh'}}><TrialQuickStart initialModelId={trialModel.model_configuration_id} initialBindingId="binding/bailian/qwen-coder" language={language} mutable snapshot={snapshot} operation={operation} onOperation={setOperation} onRefresh={async()=>{setSnapshot(structuredClone(desktop));setOperation(observed);}} onClose={()=>setOpen(false)} onOpenAgent={()=>{}} /></div> : <NewModels language={language} active plans={snapshot.catalog.plans} refreshVersion={0} onTry={()=>setOpen(true)} />;
}
