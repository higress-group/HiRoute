import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { RoutingTimeline, type TimelineTurn } from '../../../src/features/RoutingTimeline';
import { useModelLabels } from '../../../src/features/use-model-labels';
import { PresentationRoot, ProductPage } from '../../../src/ui';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const language = params.get('lang') === 'zh' ? 'zh' : 'en';
const trace: { command: string; query?: any; input?: any }[] = [];
let phase = 'pending', dirty = false;
let releaseSources: (() => void) | null = null;
const candidates = [
  { model_configuration_id:'model-00000001', binding_id:'binding-a', display_name:'Astra' },
  { model_configuration_id:'model-00000002', binding_id:'binding-b', display_name:'Astra' },
];
function turn(n: number): TimelineTurn {
  const scenario = params.get('scenario');
  const pending = n === 1 && phase === 'pending' && !['mixed','unknown','no-target'].includes(scenario ?? '');
  const mixed = n === 1 && scenario === 'mixed';
  const unknown = n === 1 && scenario === 'unknown';
  const id = n % 2 ? candidates[0].model_configuration_id : candidates[1].model_configuration_id;
  const request = { request_id:`request-${n}`, started_at_ms:1790000000000+n*1000, duration_ms:pending ? null : 1000, native_model:pending ? null : `native-${n % 2}`, input_tokens:pending ? null : 20, output_tokens:pending ? null : 0, cache_read_tokens:null, cache_read_share:null, costs:[], outcome:pending ? null : 'accepted' };
  const second = { ...request, request_id:`request-${n}-second`, native_model:'native-second' };
  const selection = pending ? null : { execution_group:'regular' as const, simple_probability:.8, simple_threshold_millis:800, selection_reason:'simple_task', ...(scenario === 'no-target' ? { routing_observation:{eligibility:'no_target'}, historical_protection:{outcome:'not_considered' as const, previous_score:null, competence_floor_millis:500, applicability_probability:null, applicability_boundary_millis:500} } : {}) };
  return { turn_id:`turn-${n}`, agent_turn_id:`stage-${n}`, segment_id:null, started_at_ms:request.started_at_ms, finished_at_ms:pending ? null : request.started_at_ms+1000, boundary:'new_user_message', plan_id:'plan', plan_revision:1, branch_id:'smart', model_configuration_id:pending || mixed || unknown ? null : id, reasoning_profile_id:'provider-default', group:pending || unknown ? null : 'regular', selection, tool_continuations:0, relays:[], assessment_state:'pending', assessment:null, competence_floor_millis:500, protected_turn_ids:[], status:pending ? 'running' : 'completed', requests:mixed ? [request,second] : [request], cost_by_model:[], evidence_partial:false, content_digest:`digest-${n}-${phase}`, presentation:{ group_state:pending ? 'pending' : unknown ? 'unavailable' : 'resolved', execution_attribution:pending || unknown ? 'unknown' : mixed ? 'mixed' : 'single', request_models:mixed ? [{request_id:request.request_id,model_configuration_id:candidates[0].model_configuration_id},{request_id:second.request_id,model_configuration_id:candidates[1].model_configuration_id}] : [{request_id:request.request_id,model_configuration_id:pending || unknown ? null : id}] } };
}
const count = ['pending','mixed','unknown','no-target'].includes(params.get('scenario') ?? '') ? 1 : 12;
Object.assign(window, { routingPresentation: { trace, resolve: () => { phase = 'resolved'; dirty = true; document.dispatchEvent(new Event('visibilitychange')); }, invalidate: () => window.dispatchEvent(new Event('hiroute-native-connection-unavailable')), releaseSources: () => releaseSources?.() } });
mockIPC(async (command, payload: any) => {
  if (command === 'plan_editor_options') { trace.push({command,input:payload.input}); if (params.has('labels-error')) throw Error('unavailable'); return { candidates:params.has('one-name') ? candidates.slice(0,1) : candidates }; }
  if (command === 'compute_management_snapshot') {
    trace.push({command});
    if (params.has('delay-source')) await new Promise<void>(resolve => { releaseSources = resolve; });
    return { sources:[{source_id:'a',display_name:'First',models:[{binding_id:'binding-a'}]},{source_id:'b',display_name:'Second',models:[{binding_id:'binding-b'}]}] };
  }
  if (command !== 'observation_read') throw Error(`Unexpected ${command}`);
  const { view, query } = payload.request.intent;
  trace.push({command:view,query});
  if (view === 'catalog') {
    if (params.has('body-error')) throw Error('unavailable');
    if (params.has('body-loading')) return await new Promise(() => {});
    if (params.has('body-cleared')) return { contents:[{content_id:'c',role:'user',kind:'text',state:'deleted',direction:'request_input',media_type:'text/plain',message_occurrence_id:'m'}], roots_partial:false, next_cursor:null };
    return { contents:[], roots_partial:true, next_cursor:null };
  }
  if (view !== 'routing_timeline') throw Error(`Unexpected ${view}`);
  if (query.after_watermark == null) return { turns:Array.from({length:count}, (_, i) => turn(i+1)), watermark:'watermark-1', complete:true };
  if (dirty) { dirty = false; return { turns:[turn(1)], watermark:'watermark-2', complete:true }; }
  return { turns:[], watermark:'watermark-2', complete:true };
});
function Fixture() {
  const [request, select] = useState<string | null>(null);
  const labels = useModelLabels(true, 0);
  return <PresentationRoot theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={1} language={language}><ProductPage title={language === 'zh' ? '会话' : 'Sessions'}><div className="session-routing-and-conversation"><RoutingTimeline session="presentation-session" language={language} active displayed refreshVersion={0} requestId={request} modelLabels={labels} onSelectRequest={value => select(value.request_id)} onOpenPerformance={() => {}} /></div></ProductPage></PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Fixture />);
