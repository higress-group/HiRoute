import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { Sessions, ContentExcerpt } from '../../../src/features/Sessions';
import { PresentationRoot } from '../../../src/ui';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const language = params.get('lang') === 'zh' ? 'zh' : 'en';
const baseTime = Date.now();
let now = baseTime;
Date.now = () => now;
const realSet = window.setTimeout.bind(window), realClear = window.clearTimeout.bind(window);
const timers = new Map<number, { at: number; callback: () => void }>();
let serial = 90000, hidden = false;
window.setTimeout = ((callback: () => void, delay: number) => {
  if (delay < 2000 || delay > 30000) return realSet(callback, delay);
  const id = ++serial; timers.set(id, { at: now + delay, callback }); return id;
}) as typeof window.setTimeout;
window.clearTimeout = ((id: number) => { if (!timers.delete(id)) realClear(id); }) as typeof window.clearTimeout;
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => hidden ? 'hidden' : 'visible' });
Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
const fixture = {
  phase: 0, rootsPartial: false, onlyResponse: false, partPartial: false, body: params.get('body') ?? 'pending', deferSummary: false, deferContent: false,
  clearedSessions: new Map<string, string>(), deletedRequests: new Set<string>(), revivedSessions: new Set<string>(),
  clear: (session: string, state = 'deleted') => {
    fixture.clearedSessions.set(session, state);
    fixture.deletedRequests.add(`request/${session}`);
  },
  revive: (session: string) => {
    fixture.clearedSessions.delete(session);
    fixture.revivedSessions.add(session);
  },
  reads: [] as { view: string; query: any; at: number }[], releases: [] as (() => void)[],
  advance: (ms: number) => {
    now += ms;
    for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.callback(); }
  },
  visibility: (value: boolean) => { hidden = value; document.dispatchEvent(new Event('visibilitychange')); },
  invalidate: () => window.dispatchEvent(new Event('hiroute-content-invalidated')),
  release: () => { for (const release of fixture.releases.splice(0)) release(); },
};
if (params.has('cleared')) fixture.clear('session-1');
Object.assign(window, { sessionRefreshFixture: fixture });
const summary = (id: string, phase = fixture.phase) => ({ session_id: id, agent_id: 'claude-code', request_count: fixture.revivedSessions.has(id) ? 13 : phase ? 18 : 12, fallback_request_count: 0, last_request_at_ms: fixture.revivedSessions.has(id) ? now : phase ? baseTime + 65000 : baseTime - 60000, correlation_kind: 'native_session', content_completeness: fixture.clearedSessions.get(id) ?? (phase || fixture.revivedSessions.has(id) ? 'complete' : 'partial') });
const request = (session: string) => ({ request_id: `request/${session}`, started_at_ms: baseTime - 60000, outcome: 'completed', native_turn_id: 'turn/one', within_request_fallback: false, final_native_model: 'provider/exact-model', between_turn_model_change: false, previous_native_turn_id: null, previous_turn_model: null, turn_final_native_model: 'provider/exact-model', routing_context: { state: 'recorded', name_state: 'recorded', display_name: 'Daily coding trial', plan_id: 'plan/trial', plan_revision: '1' } });
const body = (requestId: string, state = fixture.body) => ({ state: fixture.partPartial ? 'partial' : ['deleted', 'expired'].includes(state) ? state : 'complete', chunks: state === 'available' ? [{ text: `Ready user content for ${requestId}`, original_byte_offset: 0 }] : [], next_cursor: null });
mockIPC(async (command, payload: any) => {
  if (command === 'observation_delete') { fixture.clear(payload.input.session_id); return {}; }
  if (command !== 'observation_read') throw Error(`Unexpected command ${command}`);
  const { view, query } = payload.request.intent;
  fixture.reads.push({ view, query, at: now });
  if (view === 'sessions') {
    if (query.session_id) {
      const result = { sessions: [summary(query.session_id)], next_cursor: null };
      if (fixture.deferSummary) return await new Promise(resolve => fixture.releases.push(() => resolve(result)));
      return result;
    }
    return { sessions: query.cursor ? [summary('older-page')] : Array.from({ length: 12 }, (_, index) => summary(`session-${index + 1}`)), next_cursor: query.cursor ? null : 'page-two' };
  }
  const sessionRequests = (id: string) => fixture.revivedSessions.has(id)
    ? [{ ...request(id), request_id: `request/${id}/new`, started_at_ms: baseTime + 65000 }, request(id)] : [request(id)];
  if (view === 'timeline') return { requests: sessionRequests(query.session_id), next_cursor: null };
  if (view === 'routing_timeline') return {
    turns: params.has('routed') ? sessionRequests(query.session_id).map(item => ({
      turn_id: item.request_id, agent_turn_id: `agent-turn/${item.request_id}`, segment_id: null, started_at_ms: item.started_at_ms,
      finished_at_ms: item.started_at_ms + 1000, boundary: 'new_user_message', plan_id: null, plan_revision: null,
      branch_id: null, model_configuration_id: 'provider/exact-model', reasoning_profile_id: null, group: 'regular',
      selection: null, tool_continuations: 0, relays: [], assessment_state: 'unrated', assessment: null,
      competence_floor_millis: null, protected_turn_ids: [], status: 'completed',
      requests: [{ ...item, duration_ms: 1000, native_model: 'provider/exact-model', input_tokens: null, output_tokens: null, cache_read_tokens: null, cache_read_share: null, costs: [] }],
      transitions: [], cost_by_model: [], evidence_partial: false, content_digest: 'unchanged-digest',
    })) : [], complete: true, watermark: 'constant-timeline-digest',
  };
  if (view === 'catalog') {
    if (fixture.body === 'error') throw { code: 'OBSERVATION_READ_TIMEOUT' };
    // Production cleanup removes catalog rows and transcript roots; it does not
    // leave a synthetic entry whose state is deleted.
    const cleared = fixture.deletedRequests.has(query.request_id);
    return { contents: cleared || fixture.body === 'pending' ? [] : [{ content_id: 'content/one', role: fixture.onlyResponse ? 'assistant' : 'user', kind: 'text', state: 'complete', direction: 'request_input', media_type: 'text/plain', message_occurrence_id: 'message/one', message_ordinal: 1, part_ordinal: 1 }], transcript_roots: [], roots_partial: cleared || fixture.rootsPartial, next_cursor: null,
      ...(query.include_input_selection ? { input_selection: { version: 1, evidence: 'native_matched', message_ordinal: 1, source_unverified: true } } : {}) };
  }
  if (view === 'content') {
    const result = body(query.request_id, fixture.deletedRequests.has(query.request_id) ? 'deleted' : fixture.body);
    if (fixture.deferContent) return await new Promise(resolve => fixture.releases.push(() => resolve(result)));
    return result;
  }
  if (view === 'ancestry') return { roots: [], gap: null };
  if (view === 'facts') return { facts: [], projection_partial: false, next_cursor: null };
  if (view === 'status') return { running: true, index_running: true, error_count: 0 };
  throw Error(`Unexpected view ${view}`);
});
function Fixture() {
  const [active, setActive] = useState(true);
  return <PresentationRoot theme={params.get('theme') === 'dark' ? 'dark' : 'light'} language={language} textScale={1}>
    <button id="toggle-active" className="btn" onClick={() => setActive(value => !value)}>Toggle active page</button>
    {params.get('mode') === 'excerpt' ? <main style={{ padding: 24, height: 'calc(100vh - 64px)', overflowY: 'auto' }}><h1>Selected request</h1><p className="selected-excerpt"><ContentExcerpt request="request/selected" active={active} language={language} revision="unchanged-digest" fallback="No user message recorded" /></p><div style={{ height: 1400 }} /><p>Offscreen target</p></main> : <Sessions language={language} active={active} />}
  </PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Fixture />);
