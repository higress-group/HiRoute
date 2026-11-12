import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { RoutingTimeline, type TimelineTurn } from '../../../src/features/RoutingTimeline';
import { PresentationRoot, ProductPage } from '../../../src/ui';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const language = params.get('lang') === 'zh' ? 'zh' : 'en';
const scenario = params.get('scenario') ?? 'coordinates';
const reads: { at: number; error: string | null; after: string | null }[] = [];
const model = 'managed-provider/exact-model-id';
const turns = Array.from({ length: 18 }, (_, offset): TimelineTurn => {
  const n = offset + 1;
  const request = { request_id: `request-${n}`, started_at_ms: 1_790_000_000_000 + n * 1000, duration_ms: 500, native_model: model, input_tokens: 1000, output_tokens: 20, cache_read_tokens: 800, cache_read_share: .8, costs: [], outcome: 'completed' };
  return { turn_id: `turn-${n}`, agent_turn_id: `agent-turn-${n}`, segment_id: `segment-${n}`, started_at_ms: request.started_at_ms, finished_at_ms: request.started_at_ms + 500, boundary: 'new_user_message', plan_id: 'plan-1', plan_revision: 2, branch_id: 'smart', model_configuration_id: 'configuration-1', reasoning_profile_id: null, group: n % 2 ? 'regular' : 'primary', selection: { simple_probability: .8, simple_threshold_millis: 700, selection_reason: 'simple_task' }, tool_continuations: 0, relays: [], assessment_state: 'scored', assessment: { score: .4, partial: false, target_from_ordinal: 2, target_through_ordinal: 2, target_from_turn_id: n === 15 ? 'outside-retained-scope' : 'agent-turn-14', target_through_turn_id: n === 15 ? 'agent-turn-15' : 'agent-turn-14' }, competence_floor_millis: 500, protected_turn_ids: [], status: 'completed', requests: [request], transitions: [], cost_by_model: [], evidence_partial: false, content_digest: `digest-${n}` };
});
// The two known endpoints share a turn, so this gap is a request comparison,
// not a claim that the completed cross-turn comparison is missing.
turns[13].requests.push({ ...turns[13].requests[0], request_id: 'request-14-extra' });
turns[13].transitions = [
  { kind: 'unknown', request_id: 'request-14-extra', previous_request_id: 'request-14', model: { native_model: model }, previous_model: { native_model: model }, completeness: 'unknown' },
  { kind: 'between_turn_change', request_id: 'request-14-extra', previous_request_id: 'request-13', model: { native_model: model }, previous_model: { native_model: 'earlier-model' }, completeness: 'complete' },
  { kind: 'within_request_fallback', request_id: 'request-14-extra', previous_request_id: 'request-14', model: { native_model: model }, previous_model: { native_model: 'failed-model' }, completeness: 'complete' },
];
turns[13].transitions.push({ kind: 'unknown', request_id: 'request-14-extra', previous_request_id: 'request-14', model: { native_model: model }, previous_model: { native_model: 'different-model' }, model_changed: true, completeness: 'complete' });
turns[16].agent_turn_id = turns[17].agent_turn_id = 'duplicate-agent-turn';
turns[15].assessment = { ...turns[15].assessment!, target_from_turn_id: 'duplicate-agent-turn', target_through_turn_id: 'duplicate-agent-turn' };
turns[14].transitions = [{ kind: 'unknown', request_id: 'request-15', previous_request_id: 'outside-retained-request', model: { native_model: model }, previous_model: null, completeness: 'unknown' }];
if (scenario === 'extension-late-assessment') {
  Object.assign(turns[13], {
    group: 'regular',
    selection: { execution_group: 'regular', selection_reason: 'extension_decision', extension_reason: 'Extension chose regular', simple_probability: null, simple_threshold_millis: null },
    assessment_state: 'unrated', assessment: null, competence_floor_millis: null,
  });
}
Object.assign(window, { presentationFixture: { reads, turns } });
mockIPC((command, payload: any) => {
  if (command !== 'observation_read') throw new Error(`Unexpected ${command}`);
  const { view, query } = payload.request.intent;
  if (view === 'catalog') return { entries: [], next_cursor: null };
  if (view !== 'routing_timeline') throw new Error(`Unexpected observation ${view}`);
  const count = reads.length;
  const error = ['timeout', 'extension-late-assessment'].includes(scenario) && (count === 1 || count === 2) ? 'timeout' : scenario === 'scope' && count === 0 ? 'scope' : null;
  reads.push({ at: performance.now(), error, after: query.after_watermark });
  if (error === 'timeout') throw { source: 'backend', envelope: { error: {
    code: 'OBSERVATION_UNAVAILABLE', retryable: true, details_schema: 'hiroute.observation.read-failure/v1',
    observation_read: { reason: 'timeout', retry_after_ms: 100, action: 'retry' },
  } } };
  if (error === 'scope') throw { code: 'SCOPE_TOO_LARGE', retryable: true };
  if (scenario === 'stale' || scenario === 'stale-absorption') {
    if (count === 0) return { turns, watermark: 'old-snapshot', complete: true };
    if (query.after_watermark === 'old-snapshot') throw { code: 'CHANGE_PREVIEW_STALE', retryable: false };
    const retained = scenario === 'stale' ? turns.slice(1) : turns.flatMap(turn => {
      if (turn.turn_id === 'turn-14') return [];
      return [turn.turn_id === 'turn-13' ? { ...turn, requests: [...turn.requests, ...turns[13].requests], content_digest: 'absorbed-turn' } : turn];
    });
    if (query.after_watermark === null) return { turns: retained.slice(0, 10), watermark: 'new-snapshot', complete: false };
    return { turns: query.after_watermark === 'new-snapshot' ? retained.slice(10) : [], watermark: 'fresh-end', complete: true };
  }
  if (scenario === 'extension-late-assessment' && count >= 3) {
    turns[13] = { ...turns[13], assessment_state: 'scored', assessment: { score: .1, partial: false, target_from_ordinal: 2, target_through_ordinal: 2, target_from_turn_id: 'agent-turn-14', target_through_turn_id: 'agent-turn-14' }, content_digest: 'extension-late-score' };
    return { turns: query.after_watermark == null ? turns : count === 3 ? [turns[13]] : [], watermark: `watermark-${count}`, complete: true };
  }
  return { turns: count === 0 || scenario === 'scope' ? turns : [], watermark: `watermark-${count}`, complete: true };
});
function Fixture() {
  const [requestId, select] = useState<string | null>(null);
  const [refreshVersion, refresh] = useState(0);
  return <PresentationRoot theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={1} language={language}><ProductPage title={language === 'zh' ? '会话' : 'Sessions'}>{scenario === 'extension-late-assessment' && <button id="refresh-timeline" className="btn" onClick={() => refresh(value => value + 1)}>Refresh timeline</button>}<div className="session-routing-and-conversation"><RoutingTimeline session="presentation-session" language={language} active displayed refreshVersion={refreshVersion} requestId={requestId} onSelectRequest={request => select(request.request_id)} onOpenPerformance={() => {}} /></div></ProductPage></PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Fixture />);
