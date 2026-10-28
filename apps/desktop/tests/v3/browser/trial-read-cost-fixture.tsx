import { useState } from 'react';
import { TrialSessionHandoff } from '../../../src/features/trial/TrialSessionHandoff';
import { useTrustSuggestions, type TrustSuggestionPlan } from '../../../src/features/trust/use-trust-suggestions';
import { TrustAttention } from '../../../src/features/trust/TrustAttention';
import { Sessions } from '../../../src/features/Sessions';

const timers = new Map<number, { interval: number; callback: () => void; once?: boolean }>();
let serial = 70000, hidden = false;
export const readCostState = { found: false, fail: false, confirmed: Date.now(), hold: new URLSearchParams(location.search).has('hold-session-read'), completed: 0, pending: [] as (() => void)[] };
export function installReadCostClock() {
  const set = window.setInterval.bind(window), clear = window.clearInterval.bind(window);
  const timeout = window.setTimeout.bind(window), clearTimeout = window.clearTimeout.bind(window);
  window.setInterval = ((callback: () => void, interval: number) => {
    if (![5000,300000].includes(interval)) return set(callback, interval);
    const id = ++serial; timers.set(id,{interval,callback}); return id;
  }) as typeof window.setInterval;
  window.clearInterval = ((id: number) => { if (!timers.delete(id)) clear(id); }) as typeof window.clearInterval;
  // Suggestions schedule their next read after completion, rather than overlapping intervals.
  window.setTimeout = ((callback: () => void, interval: number) => {
    if (interval !== 300000) return timeout(callback, interval);
    const id = ++serial; timers.set(id, { interval, callback, once: true }); return id;
  }) as typeof window.setTimeout;
  window.clearTimeout = ((id: number) => { if (!timers.delete(id)) clearTimeout(id); }) as typeof window.clearTimeout;
  Object.defineProperty(document,'hidden',{configurable:true,get:()=>hidden});
  Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>hidden?'hidden':'visible'});
  Object.assign(window,{readCostState, tickReadCost:(elapsed:number)=>{for(const [id,timer] of [...timers])if(elapsed>=timer.interval){if(timer.once)timers.delete(id);timer.callback();}}, hideReadCost:(value:boolean)=>{hidden=value;document.dispatchEvent(new Event('visibilitychange'));},readCostTimers:timers});
}
export function ReadCostFixture({language,view}:{language:'zh'|'en';view:string}) {
  const [open,setOpen]=useState(true),[session,setSession]=useState<string|null>(null),[revision,setRevision]=useState(1),[home,setHome]=useState(true);
  const read=useTrustSuggestions(view==='suggestion-cadence',revision,home);
  return <div className="new-models"><button className="btn" onClick={()=>setOpen(false)}>Close waiting</button><button className="btn" onClick={()=>setRevision(v=>v+1)}>Publish fixture</button><button className="btn" onClick={()=>setHome(v=>!v)}>Toggle Home</button>
    {view==='suggestion-cadence'?<TrustAttention read={read} language={language} names={{}} onOpen={()=>{}} />: session ? <Sessions language={language} initialSession={session} /> : open && <TrialSessionHandoff planId="plan/trial" confirmedAt={readCostState.confirmed} language={language} onWatch={setSession} />}
  </div>;
}
export async function readCostResponse(payload:any, plan:TrustSuggestionPlan) {
  const {view,query}=payload.request.intent;
  if(view==='plan_trust_suggestions') return {suggestions:[plan],unavailable:[{plan_id:'plan/oversized',reason:'scope_too_large'}]};
  if(view==='sessions') {
    if (readCostState.hold) await new Promise<void>(resolve => readCostState.pending.push(resolve));
    if(readCostState.fail)throw Error('OBSERVATION_UNAVAILABLE');
    const records=[{session_id:'trial-before-confirmation',agent_id:null,last_request_at_ms:readCostState.confirmed-1},...(readCostState.found?[{session_id:'trial-found',agent_id:null,last_request_at_ms:Math.max(readCostState.confirmed,query.to_ms-1)}]:[])];
    readCostState.completed++;
    return {sessions:records.filter(row=>row.last_request_at_ms>=query.from_ms && row.last_request_at_ms<query.to_ms).map(row=>({...row,request_count:1,fallback_request_count:0,correlation_kind:'unknown',content_completeness:'unknown'})),next_cursor:null};
  }
  if(view==='routing_timeline')return {turns:[],watermark:'fixture',complete:true};
  if(view==='timeline')return {requests:[],next_cursor:null};
  if(view==='status')return {running:true,index_running:true,error_count:0};
  if(view==='catalog')return {contents:[],next_cursor:null};
  if(view==='ancestry')return {roots:[],gap:null};
  if(view==='facts')return {facts:[],projection_partial:false};
  throw Error(`Unexpected read ${view} ${JSON.stringify(query)}`);
}
