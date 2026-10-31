import { metadataFixtureInvoke } from './model-metadata-fixture';
import { ReadCostFixture, installReadCostClock, readCostResponse } from './trial-read-cost-fixture';
import { TrialFixture, quickStartInvoke } from './trial-quickstart-fixture';
import { NewModels } from '../../../src/features/trial/NewModels';
import { TrustAttention } from '../../../src/features/trust/TrustAttention';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { Sessions } from '../../../src/features/Sessions';
import type { TimelineTurn } from '../../../src/features/RoutingTimeline';
import { Models } from '../../../src/features/models/Models';
import { PlanEditor, type Editor } from '../../../src/plan-editor';
import { PresentationRoot, ProductPage } from '../../../src/ui';
import { readyManagement, dailyPlan, fixturePlanAuthoring, mockProductInvoke } from './product-fixtures';
import type { ModelFeedStatus } from '../../../src/features/model-feed/types';
import type { TrustReport } from '../../../src/features/trust/types';
import '../../../src/occami/styles.css';
const params = new URLSearchParams(location.search);
if (params.get('release-evidence') === 'retention') {
  let clock = Date.now();
  Date.now = () => clock;
  Object.assign(window, { advanceRetentionClock: (ms: number) => { clock += ms; } });
}
const readCost = ['handoff', 'suggestion-cadence'].includes(params.get('view') ?? '');
if (readCost) installReadCostClock();
const language = params.get('lang') === 'zh' ? 'zh' : 'en';
const trace: { command: string; payload: any; at: number }[] = [];
Object.assign(window, { trialTrace: trace });
const feed: ModelFeedStatus = { retrieval_enabled:true, held:{sequence:12, issued_at:'2026-10-18T00:00:00Z', retrieved_at:1792281600000}, highest_accepted_sequence:12, last_attempt:{at:1792281600000,result:'ok',reason:null}, verification:{kind:'validated'}, counts:{descriptive_records:4,qualified_models:4,restricted_models:0}, next_automatic_attempt_at:null, disclosure:'Model information is retrieved automatically from hiroute.ai without account or usage identifiers. You can turn off automatic retrieval.', disclosure_acknowledged:false };
if (params.has('recovery')) { feed.held=null; feed.verification={kind:'discarded',reason:'feed_state_invalid'}; feed.recovery_available=true; }
const featuredCatalog = {
  featured:[
    {id:'featured/flash',kind:'new_model',model_configuration_id:'model.deepseek.v4-1-flash',headline:{en:'DeepSeek V4.1 Flash',zh:'DeepSeek V4.1 Flash'},note:{en:'Cheap and fast. Close to the previous flagship on simple coding tasks.',zh:'便宜、快，编码类简单任务接近上一代主力。'},links:[],included_on:'2026-10-09'},
    {id:'featured/kimi',kind:'new_model',model_configuration_id:'model.trial',headline:{en:'Kimi K3',zh:'Kimi K3'},note:{en:'Steady on long coding sessions. A good first pick for implementation work.',zh:'长程编码稳定，适合先接手实现类任务。'},links:[],included_on:'2026-10-16'},
    {id:'featured/glm',kind:'model_update',model_configuration_id:'model.zhipu.glm-5-3',headline:{en:'GLM-5.3 update',zh:'GLM-5.3 更新'},note:{en:'Faster at reading code and making small edits.',zh:'读代码和小改动速度更快。'},links:[],included_on:'2026-10-02'},
    {id:'featured/luna',kind:'new_model',model_configuration_id:'model.openai.gpt-6-luna',headline:{en:'GPT-6 Luna',zh:'GPT-6 Luna'},note:{en:'Uses new interface features that need HiRoute 0.3.3.',zh:'用到新的接口特性，需要 HiRoute 0.3.3 的适配。'},links:[],included_on:'2026-10-14'},
  ],
  models:[
    {model_configuration_id:'model.deepseek.v4-1-flash',display_name:'DeepSeek V4.1 Flash',origin:{kind:'feed',sequence:12,as_of:'2026-10-09'},identity_stability:'rolling-alias',availability:{kind:'executable'},binding_ids:['binding/bailian/qwen-coder'],facts:{publisher_id:'publisher.deepseek',context_tokens:1000000,tool:true,vision:false,reasoning:{kind:'profiles',profiles:['low','high','max']}}},
    {model_configuration_id:'model.trial',display_name:'Kimi K3',origin:{kind:'feed',sequence:12,as_of:'2026-10-16'},identity_stability:'rolling-alias',availability:{kind:'needs_connection'},binding_ids:[],facts:{publisher_id:'publisher.moonshot',context_tokens:262144,tool:true,vision:false,reasoning:{kind:'toggle'}}},
    {model_configuration_id:'model.zhipu.glm-5-3',display_name:'GLM-5.3',origin:{kind:'embedded',client_version:'0.3.1'},identity_stability:'pinned',availability:{kind:'executable'},binding_ids:['binding/zhipu/glm-5-3'],facts:{publisher_id:'publisher.zhipu',context_tokens:204800,tool:true,vision:false,reasoning:{kind:'toggle'}}},
    {model_configuration_id:'model.openai.gpt-6-luna',display_name:'GPT-6 Luna',origin:{kind:'feed',sequence:12,as_of:'2026-10-14'},identity_stability:'pinned',availability:{kind:'needs_newer_client',version:'0.3.3'},binding_ids:[],facts:{publisher_id:'publisher.openai',context_tokens:400000,tool:true,vision:true,reasoning:null}},
  ],
};
const counts = { executed:58,complete:41,meets_floor:37,partial:4,awaiting_assessment:1,unscored:12,sessions:9 };
const primaryCounts = { executed:19,complete:17,meets_floor:16,partial:0,awaiting_assessment:0,unscored:2,sessions:6 };
const evidenceSessions = ['session/gateway-refactor','session/rust-ownership','session/boundary-test','session/route-review','session/release-notes','session/stream-gap'];
const report: TrustReport = { plan_id:dailyPlan.agent_plan_id, active_revision:8, revision:'active',from_ms:0,to_ms:Date.now(), rows:[
  {branch_id:'smart_saving',group:'regular',model_configuration_id:'qwen3-coder-plus',reasoning_profile_id:'provider-default',attribution:'single',executor:null,identity_stability:'rolling-alias',counts,competence_rate:37/41,scoring_coverage:41/58,execution_share:58/77,protections:[{segment_id:'segment/p1',trigger_segment_id:'segment/t1'},{segment_id:'segment/p2',trigger_segment_id:'segment/t2'},{segment_id:'segment/p3',trigger_segment_id:'segment/t3'}],availability_relays:1,duration_p50_ms:1500,duration_p90_ms:3500,output_tokens_per_second:50},
  {branch_id:'smart_saving',group:'primary',model_configuration_id:'gpt-5.6-sol',reasoning_profile_id:'high',attribution:'single',executor:null,identity_stability:'pinned',counts:primaryCounts,competence_rate:16/17,scoring_coverage:17/19,execution_share:19/77,protections:[],availability_relays:0,duration_p50_ms:4200,duration_p90_ms:9800,output_tokens_per_second:38},
], branches:[{branch_id:'smart_saving',threshold_millis:720,full_history:counts,upward_cohort:counts,upward_from_ms:0,suggestion:{kind:'step_up',threshold_millis:650},replay:{included:58,excluded_by_reason:{heuristic:2},recorded_share_at_current:0.8,estimate_only:true,thresholds:[{threshold_millis:650,regular:49,primary:9,protected_as_primary:3,regular_share:49/58,newly_regular:7}]}}],sessions:evidenceSessions.map((session_id,i)=>({session_id,stages:14-i*2})),session_total:9,executors:[{executor:'deepseek_harness',task_outcomes:{submitted:3,succeeded:2,failed:1}}],announcements:[{model_configuration_id:'qwen3-coder-plus',included_on:'2026-10-10',before:31,announcement_day:2,after:25,executed_version_verified:false}],cost:{kind:'unavailable',reason:'comparison basis not available'} };
const stageAt = (hoursAgo: number) => Date.UTC(2026, 9, 17, 9) - hoursAgo * 3_600_000;
const qualitySamples = ([
  ['segment/t1','session/gateway-refactor',2,0,'没有有效进展，用户反馈'],
  ['segment/t2','session/boundary-test',30,0.3,'边界条件遗漏'],
  ['segment/t3','session/route-review',75,0,'撤回自己的改动两次'],
  ['segment/ok1','session/rust-ownership',5,0.92,'定位准确，修改范围小'],
  ['segment/ok2','session/release-notes',50,0.85,'按要求完成'],
] as const).map(([segment_id,session_id,hours,score,reason],i)=>({segment_id,session_id,plan_id:dailyPlan.agent_plan_id,plan_revision:8,selected_branch_id:'smart_saving',executed_branch_id:'smart_saving',model_configuration_id:'qwen3-coder-plus',native_model:'qwen3-coder-plus',reasoning_profile_id:'provider-default',attribution:'single' as const,first_turn_ordinal:i+1,last_observed_turn_ordinal:i+1,first_at_ms:stageAt(hours),last_at_ms:stageAt(hours)+90_000,history_partial:false,first_request_id:session_id.replace('session/','request/'),last_request_id:session_id.replace('session/','request/'),execution_evidence_available:true,branch_execution:{policy:{name:'smart_saving',floor_millis:600},group:'regular' as const,candidate_index:0},selection:{execution_group:'regular' as const,simple_probability:0.81,simple_threshold_millis:720,selection_reason:'simple_task' as const},assessment:{trigger_request_id:session_id.replace('session/','request/'),assessed_at_ms:stageAt(hours)+120_000,target_from_ordinal:i+1,target_through_ordinal:i+1,score,partial:false,reason,evidence_available:true}}));
const qualityModels = [
  {execution:{group:'regular' as const,candidate_index:0,plan_revision:8,selected_branch_id:'smart_saving',executed_branch_id:'smart_saving',model_configuration_id:'qwen3-coder-plus',profile_digest:null,attribution:'single' as const},native_model:'qwen3-coder-plus',reasoning_profile_id:'provider-default',scored_stage_count:41,unrated_stage_count:17,average_score:0.81},
  {execution:{group:'primary' as const,candidate_index:0,plan_revision:8,selected_branch_id:'smart_saving',executed_branch_id:'smart_saving',model_configuration_id:'gpt-5.6-sol',profile_digest:null,attribution:'single' as const},native_model:'gpt-5.6-sol',reasoning_profile_id:'high',scored_stage_count:17,unrated_stage_count:2,average_score:0.9},
];
const stepDownCounts = { executed:46,complete:40,meets_floor:26,partial:2,awaiting_assessment:0,unscored:6,sessions:7 };
const homeScope = params.get('home-scope');
const homeScopeFrom = homeScope === 'retention' ? Date.UTC(2026, 9, 24, 12, 34, 56) : Date.UTC(2026, 9, 20, 12, 34, 56);
const homeAnnouncements = homeScope && homeScope !== 'no-announcement'
  ? [{ model_configuration_id: homeScope === 'unrelated' ? 'other-branch-model' : 'qwen3-coder-plus', included_on:'2026-10-10', before:31, announcement_day:2, after:25, executed_version_verified:false as const }]
  : [];
const homeSuggestions = [
  {...report,announcements:homeAnnouncements,branches:report.branches.map(b=>({...b,upward_from_ms:homeScope ? homeScopeFrom : b.upward_from_ms,protection_count:2,identity_stabilities:['rolling-alias']}))},
  {...report,plan_id:'plan/orders',announcements:[],branches:[{branch_id:'smart_saving',threshold_millis:800,full_history:stepDownCounts,upward_cohort:stepDownCounts,upward_from_ms:0,suggestion:{kind:'step_down' as const,threshold_millis:900},replay:{included:46,excluded_by_reason:{},recorded_share_at_current:0.61,estimate_only:true as const,thresholds:[{threshold_millis:900,regular:14,primary:32,protected_as_primary:9,regular_share:14/46,newly_regular:0}]},protection_count:9,identity_stabilities:['pinned']}]},
];
const timelineRequests = (params.has('polish') ? Array.from({length:Number(params.get('turns')??2)},(_,i)=>i+1) : params.has('stale-timeline') ? [1,2,3] : [1,2]).map(i => ({request_id:`request-${i}`,started_at_ms:Date.now()-10000+i*1000,duration_ms:1500,native_model:i===1?'primary-model':'trial-model',input_tokens:1000,output_tokens:200,cache_read_tokens:990,cache_read_share:0.99,costs:[{model_configuration_id:i===1?'Primary':'Trial',currency:'USD',valuation_kind:'usage_estimate',known_micros:1250,unknown_contributions:i===1?0:1}],outcome:params.get('request-outcome')??'accepted'}));
let timelinePolls=0; let busyReturned=false;
const timelineTurn = (i:number,updated=false): TimelineTurn => { const turn: TimelineTurn = {turn_id:`request-${i}`,agent_turn_id:`turn-${i}`,segment_id:`stage-${i}`,started_at_ms:timelineRequests[i-1].started_at_ms,finished_at_ms:timelineRequests[i-1].started_at_ms+1500,boundary:i===1?'new_user_message':params.get('boundary')==='rebuilt'?'context_rebuilt':'unknown',plan_id:dailyPlan.agent_plan_id,plan_revision:8,branch_id:'smart_saving',model_configuration_id:i===1?'Primary model with a long readable name':'Trial model',reasoning_profile_id:'high',group:i===1?'primary':'regular',selection:{simple_probability:i===1?0.62:0.94,simple_threshold_millis:900,selection_reason:i===1?'complex_task':'simple_task',competence_trigger:null},tool_continuations:1,relays:[],assessment_state:updated?'scored':'pending',assessment:updated?{score:0.4,partial:false,target_from_ordinal:1,target_through_ordinal:1}:null,competence_floor_millis:500,protected_turn_ids:[],status:'completed',requests:[timelineRequests[i-1]],cost_by_model:timelineRequests[i-1].costs,evidence_partial:false,content_digest:`digest-${i}-${updated}`};
  if(params.has('absorbed-turn')) {
    turn.agent_turn_id=null;turn.status='running';turn.group='regular';turn.model_configuration_id='Trial model';
    turn.requests=[{...turn.requests[0],native_model:'trial-model'}];
  }
  if(params.has('partial-cost')) {
    turn.model_configuration_id='Trial model'; turn.group='regular';
    turn.requests=[{...turn.requests[0],native_model:'trial-model',costs:[{model_configuration_id:'Trial model',currency:'USD',valuation_kind:'api_equivalent',known_micros:i===1?null:1250,unknown_contributions:i===1?1:0}]}];
    turn.cost_by_model=turn.requests[0].costs;
  }
  if(params.has('protection')) {
    const outcome = (['applied','suppressed','retained_unavailable','not_needed','not_considered',null] as const)[(i-1)%6];
    const protectedTurn = outcome==='applied' || outcome==='retained_unavailable';
    turn.group=protectedTurn?'primary':'regular';
    turn.selection!.simple_probability=0.94;
    turn.selection!.selection_reason=protectedTurn?'low_competence':'simple_task';
    turn.selection!.historical_protection=outcome ? {outcome,previous_score:outcome==='not_needed'?0.8:outcome==='not_considered'?null:0.3,competence_floor_millis:500,applicability_probability:outcome==='applied'?0.99:outcome==='suppressed'?0.05:null,applicability_boundary_millis:500} : null;
    turn.requests=[{...turn.requests[0],native_model:protectedTurn?'primary-model':'trial-model',outcome:i%6===0?'failed':'completed'}];
    turn.model_configuration_id=protectedTurn?'Primary model':'Trial model';
    turn.status=i%6===0?'failed':'completed';turn.assessment_state='scored';turn.assessment={score:i%5===0?0.2:0.91,partial:false,target_from_ordinal:i,target_through_ordinal:i};
  }
  return turn;
};
let pendingTimelineTurns: number[] = [];
Object.assign(window, { appendTrialTurns: (count = 1) => {
  for (let n=0;n<count;n++) {
    const i=timelineRequests.length+1;
    timelineRequests.push({...timelineRequests[1],request_id:`request-${i}`,started_at_ms:timelineRequests[0].started_at_ms+i*1000});
    pendingTimelineTurns.push(i);
  }
} });
let failed = false;
mockIPC(async (command, payload: any) => {
  trace.push({command,payload,at:performance.now()});
  if(readCost && command==='observation_read') return readCostResponse(payload,{...report,branches:report.branches.map(b=>({...b,protection_count:0,identity_stabilities:['rolling-alias']}))});
  if(params.get('view')==='quickstart') return quickStartInvoke(command,payload,params);
  if (command === 'worker_executor_availability') return {executors:['deepseek_harness','codex_cli','claude_code','pi','qoder_cli'].map(harness=>({harness,state:harness==='deepseek_harness'?'ready':'unavailable'}))};
  if (command === 'model_feed_status') { if (params.has('error') && !failed) {failed=true;throw new Error('DAEMON_UNAVAILABLE');} return structuredClone(feed); }
  if (command === 'model_feed_refresh') { feed.last_attempt={at:Date.now(),result:'validation',reason:'invalid_payload'}; return structuredClone(feed); }
  if (command === 'model_feed_settings_apply') { if (payload.input.retrieval_enabled !== undefined) feed.retrieval_enabled=payload.input.retrieval_enabled; if (payload.input.acknowledge_disclosure) feed.disclosure_acknowledged=true; return structuredClone(feed); }
  const metadata = metadataFixtureInvoke(command,payload,params); if (metadata !== undefined) return metadata;
  if (command === 'model_feed_remove_apply') { if (params.has('stale')) throw {message_key:'application.error.change_preview_stale'}; feed.held=null; feed.verification={kind:'none'}; feed.recovery_available=false; if (payload.input.also_disable_retrieval) feed.retrieval_enabled=false; return structuredClone(feed); }
  if (command === 'model_availability') {
    if (params.has('current-announcement')) return { ...featuredCatalog, featured: [{ ...featuredCatalog.featured[0],
      kind: 'model_update', headline: { en: 'DeepSeek V4.1 Flash 1017', zh: 'DeepSeek V4.1 Flash 1017' }, included_on: '2026-10-19' }] };
    if (params.has('no-recommendations')) return { featured: [], models: [
      { ...featuredCatalog.models[2], model_configuration_id: 'model.anthropic.old', display_name: 'Older directory model' },
      { ...featuredCatalog.models[0], origin: { kind: 'embedded', client_version: '0.3.1' } },
    ] };
    return ['feed-home', 'home'].includes(params.get('view') ?? '') ? featuredCatalog : { featured: [], models: [] };
  }
  if (command === 'observation_read' && payload.request.intent.view === 'plan_trust_replay') {
    const thresholds: number[]=payload.request.intent.query.thresholds_millis;
    if(params.has('replay-error') && !failed) {failed=true;throw Error('OBSERVATION_SCOPE_TOO_LARGE');}
    return {included:params.has('empty')?0:58,excluded_by_reason:params.has('polish')?{heuristic:2,decision_fallback:1,availability_relay:1}:{heuristic:2},recorded_share_at_current:params.has('empty')?null:0.8,estimate_only:true,thresholds:thresholds.map(threshold=>{
      const regular=params.has('empty')?0:params.has('polish')?Math.max(0,Math.min(58,Math.round((1000-threshold)/8))):49,primary=params.has('empty')?0:58-regular;
      return {threshold_millis:threshold,regular,primary,protected_as_primary:params.has('empty')?0:3,regular_share:params.has('empty')?null:regular/58,primary_share:params.has('empty')?null:primary/58,newly_regular:params.has('empty')?0:7};
    })};
  }
  if (command === 'observation_read' && payload.request.intent.view === 'plan_trust') {
    const value=structuredClone(report); if(params.has('cost')) value.cost={kind:'available',difference_micros:6000,comparison:{currency:'USD',coverage:'complete',request_count:3,compared_request_count:3,missing_request_count:0,excluded_request_count:0,routed_estimate_micros:18000,reference_estimate_micros:12000,compared_routed_subtotal_micros:18000,compared_reference_subtotal_micros:12000,retention_boundary_partial:false,arithmetic_overflow:false}}; if(params.has('division')) value.branches[0].suggestion={kind:'explore_division'}; if (params.has('empty')) {value.rows=[]; value.branches[0].full_history={executed:0,complete:0,meets_floor:0,partial:0,awaiting_assessment:0,unscored:0,sessions:0};}
    if(params.has('cost-partial') || params.has('cost-unknown')) {const known=params.has('cost-partial');value.cost={kind:'available',difference_micros:null,comparison:{currency:'USD',coverage:known?'partial':'unknown',request_count:3,compared_request_count:known?1:0,missing_request_count:known?2:3,excluded_request_count:known?2:3,routed_estimate_micros:null,reference_estimate_micros:null,compared_routed_subtotal_micros:known?6000:null,compared_reference_subtotal_micros:known?4000:null,retention_boundary_partial:false,arithmetic_overflow:false}};}
    if(params.has('cost-routed-only')) value.cost={kind:'available',difference_micros:null,comparison:{currency:'USD',coverage:'unknown',request_count:1,compared_request_count:0,missing_request_count:1,excluded_request_count:0,routed_estimate_micros:18000,reference_estimate_micros:null,same_usage_bounds:{reference_min_micros:92880,reference_max_micros:115738,difference_min_micros:-97738,difference_max_micros:-74880},compared_routed_subtotal_micros:null,compared_reference_subtotal_micros:null,retention_boundary_partial:false,arithmetic_overflow:false}};
    if(params.has('release-evidence')) {
      const q=payload.request.intent.query;
      value.from_ms=params.get('release-evidence')==='retention'?Math.max(q.from_ms??0,Date.now()-7*86_400_000+10):q.from_ms??0;
      value.to_ms=q.to_ms;value.revision=q.revision;
    }
    // Custom extensions have observations, but no built-in threshold policies or suggestions.
    if (params.has('custom')) {
      value.branches=[];
      value.rows.forEach(row => { row.protections=[]; row.competence_rate=null; row.counts.competence_eligible=0; row.counts.meets_floor=0; });
    }
    if (payload.request.intent.query.group_by === 'executor') value.rows.forEach(row => row.executor='deepseek_harness');
    return value;
  }
  if (command === 'observation_read' && params.get('view') === 'timeline') {
    const {view,query}=payload.request.intent;
    if(view==='routing_timeline') {
      if(params.has('scope-error')) throw {code:'SCOPE_TOO_LARGE'};
      if(params.has('busy-read') && !busyReturned) {busyReturned=true;throw {source:'backend',envelope:{error:{code:'OBSERVATION_UNAVAILABLE',retryable:true,details_schema:'hiroute.observation.read-failure/v1',observation_read:{reason:'reader_busy',retry_after_ms:100}}}};}
      if(params.has('timeline-error') && (window as any).timelineFail!==false) throw {code:'OBSERVATION_UNAVAILABLE'};
      if(params.has('timeline-empty')) return {turns:[],watermark:'empty',complete:true};
      timelinePolls++;
      if(params.has('absorbed-turn')) {
        if(timelinePolls===1)return {turns:[timelineTurn(1),timelineTurn(2)],watermark:'watermark-1',complete:true};
        if(timelinePolls===2) {
          const turn=timelineTurn(1,true);turn.agent_turn_id='closed-turn';turn.status='completed';
          turn.requests=[...timelineTurn(1).requests,...timelineTurn(2).requests];
          turn.cost_by_model=turn.requests.flatMap(request=>request.costs);turn.content_digest='closed-with-two-requests';
          return {turns:[turn],watermark:'watermark-2',complete:true};
        }
        return {turns:[],watermark:`watermark-${timelinePolls}`,complete:true};
      }
      if(params.has('polish')) {
        const indices=query.after_watermark==null?timelineRequests.map((_,i)=>i+1):pendingTimelineTurns;
        const turns=indices.map(i=>timelineTurn(i));pendingTimelineTurns=[];
        if(params.has('reason') && turns[0]) turns[0].selection!.selection_reason=params.get('reason')!;
        return {turns,watermark:`watermark-${timelinePolls}`,complete:true};
      }
      if(params.has('stale-timeline') && query.after_watermark==='watermark-1') throw {message_key:'application.error.change_preview_stale'};
      const turns=params.has('stale-timeline') && timelinePolls>1 && query.after_watermark===null ? [timelineTurn(1,true),timelineTurn(2),timelineTurn(3)] : timelinePolls===1?[timelineTurn(1),timelineTurn(2)]:timelinePolls===2?[timelineTurn(1,true)]:[];
      return {turns,watermark:`watermark-${timelinePolls}`,complete:true};
    }
    if(view==='timeline') return {requests:timelineRequests.map(r=>({...r,native_turn_id:r.request_id,within_request_fallback:false,final_native_model:r.native_model,between_turn_model_change:false,previous_native_turn_id:null,previous_turn_model:null,turn_final_native_model:r.native_model})),next_cursor:null};
    if(view==='sessions') return {sessions:[{session_id:'timeline-session',agent_id:'claude-code',request_count:2,fallback_request_count:0,last_request_at_ms:Date.now(),correlation_kind:'confirmed',content_completeness:'complete'}],next_cursor:null};
    if(view==='status') return {running:true,index_running:true,error_count:0};
    if(view==='catalog') return {contents:[],next_cursor:null};
    if(view==='ancestry') return {roots:[],gap:null};
    if(view==='facts') return {facts:[],projection_partial:false,next_cursor:null};
  }
  if (command === 'observation_read' && ['timeline','catalog','content'].includes(payload.request.intent.view)) return mockProductInvoke(command,payload,'ready');
  if(command==='observation_read' && payload.request.intent.view==='plan_quality' && params.has('release-evidence')) {
    const q=payload.request.intent.query, state=params.get('release-evidence');
    const counts={c_decisions:10,requested:10,unavailable:0,no_target_omissions:0,partial_target_omissions:0,inherited:0,required_question_missing:0,gate_opportunities:5,suppressed:state==='zero'?0:5,unavailable_per_requested:0,suppressed_per_opportunity:state==='zero'?0:1,suppressed_per_c_decision:state==='zero'?0:0.5};
    const coveredFrom=state==='retention'?Math.max(q.from_ms??0,Date.now()-7*86_400_000+20):(q.from_ms??0)+(state==='time'?1:0);
    const routing={plan_id:state==='plan'?'other-plan':q.plan_id,session_id:null,plan_revision:state==='revision'?99:q.plan_revision,requested_from_ms:q.from_ms??null,requested_to_ms:q.to_ms,covered_from_ms:coveredFrom,covered_to_ms:q.to_ms,coverage:state==='partial'?'partial':'complete',retained_decisions:10,known_non_c_decisions:0,legacy_or_unknown_decisions:0,unknown_scope_decisions:0,missing_projection_events:0,scan_limited:false,cohorts:state==='empty'?[]:[{cohort:{protocol:'fixture',content_digest:'fixture',bundle_digest:'fixture',scope_id:state==='branch'?'another-branch':'smart_saving',standard:'default',provider:'Fixture service',model:'Judgment model',simple_threshold_millis:720,competence_floor_millis:500},counts}]};
    return {samples:q.cursor?qualitySamples.map(s=>({...s,segment_id:s.segment_id+'-next'})):qualitySamples,summary:{models:qualityModels,scored_stage_count:58,unrated_stage_count:19,session_count:9,available_revisions:[8]},routing_observations:state==='missing'?undefined:routing,next_cursor:state==='retention' && q.execution && !q.cursor?'retained-next':null};
  }
  if (command === 'observation_read' && payload.request.intent.query.execution && !params.has('empty')) {
    const {execution,competence}=payload.request.intent.query;
    const pick=competence==='below_floor'?qualitySamples.filter(s=>s.assessment.score<0.6):competence==='meets_floor'?qualitySamples.filter(s=>s.assessment.score>=0.6):qualitySamples;
    return {samples:pick.filter(s=>s.model_configuration_id===execution.model_configuration_id),summary:{models:qualityModels,scored_stage_count:58,unrated_stage_count:19,session_count:9,available_revisions:[8]},next_cursor:null};
  }
  if (command === 'observation_read') { const models=params.has('empty')?[]:qualityModels; return {samples:[],summary:{models,scored_stage_count:models.reduce((n,m)=>n+m.scored_stage_count,0),unrated_stage_count:models.reduce((n,m)=>n+m.unrated_stage_count,0),session_count:params.has('empty')?0:9,available_revisions:[8]},next_cursor:null}; }
  const value = await mockProductInvoke(command,payload,'ready');
  if(command==='plan_editor_options' && payload?.input?.authoring?.plan_id===plan.agent_plan_id) (value as any).authoring=fixturePlanAuthoring(plan);
  if(command==='plan_editor_options' && params.has('division')) for(const c of (value as any).candidates) if(c.binding_id==='binding/bailian/qwen-coder') {c.model_configuration_id='deepseek-v4.1-flash';c.display_name='DeepSeek trial';}
  return value;
});
const plan = structuredClone(dailyPlan);
plan.desired.delegation_enabled=false;
plan.desired.strategy.judgment!.degree.simple_threshold_millis=720;
plan.desired.strategy.classifier={kind:'decision_service',service:{id:'decision/test',revision:1,name:'Decision model',connection:{kind:'custom',endpoint:'https://decision.example.test',timeout_ms:1000,auth_header:null}}};
if(params.has('native')) plan.desired.strategy.classifier.service.connection={kind:'system_one',provider:'compatible',model:'fixture',endpoint:'https://decision.example.test',timeout_ms:1000,auth_header:{name:'Authorization',value_secret_ref:'fixture'}};
if(params.has('single-group')) plan.desired.strategy.primary=[];
function EditorFixture() {
  const [draft, setDraft] = useState<Editor | null>(null);
  return <PlanEditor key={draft ? 'division' : 'published'} plan={draft ? undefined : plan} initialContent={draft ?? undefined} creating={!!draft} language={language} onCreateDraft={async editor => {
    Object.assign(window, { divisionDraft: editor });
    if (!params.has('review-created-draft')) return false;
    setDraft(editor);
    return true;
  }} onDone={async () => {}} onClose={() => {}} />;
}
function TimelineFixture() {
  const [version, setVersion] = useState(0), [active, setActive] = useState(true);
  Object.assign(window, { refreshTrialTimeline: () => setVersion(v => v + 1), hideTrialTimeline: (hidden: boolean) => setActive(!hidden) });
  return <Sessions language={language} initialSession="timeline-session" initialRequest={params.get('initial-request')} refreshVersion={version} active={active} />;
}
createRoot(document.getElementById('root')!).render(<PresentationRoot language={language} theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={1}>
  {params.get('view')==='feed-home' ? <NewModels language={language} active plans={[]} refreshVersion={0} onTry={()=>{}} /> : params.get('view')==='home' ? <ProductPage title={language==='zh'?'首页':'Home'} subtitle={language==='zh'?'近期使用与需要你处理的事':'Recent work and things that need your attention'}><div className="home-wrap v3-home"><NewModels language={language} active plans={[dailyPlan]} refreshVersion={0} onTry={modelId=>trace.push({command:'try-model',payload:{modelId},at:performance.now()})} onOpenPlan={planId=>trace.push({command:'open-plan',payload:{planId},at:performance.now()})} /><TrustAttention language={language} names={{[dailyPlan.agent_plan_id]:language==='zh'?'日常编码':'Daily coding','plan/orders':language==='zh'?'订单服务维护':'Order service upkeep'}} read={{status:'ready',plans:homeSuggestions,unavailable:[],error:'',refresh:()=>{}}} onAct={(planId,branchId)=>trace.push({command:'review-trust-action',payload:{planId,branchId},at:performance.now()})} onOpen={planId=>trace.push({command:'review-trust',payload:{planId},at:performance.now()})} /></div></ProductPage> : readCost ? <ReadCostFixture language={language} view={params.get('view')!} /> : params.get('view') === 'quickstart' ? <TrialFixture language={language} /> : params.get('view') === 'attention' ? <ProductPage title="Home"><TrustAttention language={language} names={{[dailyPlan.agent_plan_id]:'My daily route'}} read={{status:params.has('error')?'error':'ready',plans:params.has('empty')?[]:[{...report,branches:report.branches.map(b=>({...b,protection_count:0,identity_stabilities:['rolling-alias']}))}],unavailable:params.has('partial-error')?[{plan_id:'plan/too-large',reason:'scope_too_large'}]:[],error:'OBSERVATION_UNAVAILABLE',refresh:()=>trace.push({command:'retry-suggestions',payload:{},at:performance.now()})}} onAct={(planId,branchId)=>trace.push({command:'review-trust-action',payload:{planId,branchId},at:performance.now()})} onOpen={planId=>trace.push({command:'review-trust',payload:{planId},at:performance.now()})} /></ProductPage> : params.get('view') === 'timeline' ? <TimelineFixture /> : params.get('view') === 'editor' ? <ProductPage title="Routing"><EditorFixture /></ProductPage> : <ProductPage title="Models" flush className="hr-models-page"><Models language={language} snapshot={readyManagement} mutable onRefresh={async()=>{}} onPrepareKeyInput={async()=>{throw Error('outside scenario')}} onPreview={async()=>{throw Error('outside scenario')}} onApply={async()=>'saved'} onDiscardProtectedInputs={()=>{}} /></ProductPage>}
</PresentationRoot>);
