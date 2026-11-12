import { RuntimeFacts, type SafeFact } from './RuntimeFacts';
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '../host/index.ts';
import { Dialog, Disclosure, formatMoney, ProductPage, Toast, UiIcon, useTransientToast } from '../ui';
import { sessionAgentLabel, type CorrelationKind } from './session-correlation';
import { readableEvent, excerpt, excerptAround, groupToolEvents, contentDisplayRuns, coalesceResponseText, isPrivateContentKind, isTechnicalContentKind, summarizeToolArguments, type ReadableEvent } from './session-content';
import { firstRealUserText, splitLeadingAgentContext } from './session-title';
import { formatCacheHit, formatCacheHitCoverage, formatTokenCount, usageValue, type CacheHitSummary, type UsageMetric } from './usage-presentation';
import { nativeConnectionReadText } from '../native-connection';
import { safeDiagnosticCode } from '../error-code';
import { observationRead as read } from './observation-client';
import { observationAccessLost, observationCancelled, observationFailureCode } from './observation-failure';
import { observationRecoveryText, readObservation, readObservationPages, recoverObservation, type ObservationReader, type ObservationRecoveryStatus } from './observation-recovery';
import { PlanQuality } from './PlanQuality';
import { RoutingTimeline, type TimelineAvailability, type TimelineRequest } from './RoutingTimeline';
import { useModelLabels } from './use-model-labels';
type Page<T> = { next_cursor: string | null } & T;
type Summary = { session_id: string; agent_id: string; executor?: string | null; request_count: number; fallback_request_count: number; last_request_at_ms: number; correlation_kind: CorrelationKind; content_completeness: string };
type RoutingContext = { state: 'recorded' | 'unavailable' | 'conflicted'; display_name: string | null; name_state: 'recorded' | 'unavailable'; plan_id?: string | null; plan_revision?: string | null };
type Request = { request_id: string; started_at_ms: number; outcome: string | null; native_turn_id: string | null; within_request_fallback: boolean | null; final_native_model: string | null; between_turn_model_change: boolean | null; previous_native_turn_id: string | null; previous_turn_model: string | null; turn_final_native_model: string | null; routing_context?: RoutingContext; native_auxiliary?: boolean | null };
type Content = { content_id: string; role: string; kind: string; state: string; direction: string; media_type: string; message_occurrence_id: string; message_ordinal?: number; part_ordinal?: number; fork_id?: string; downstream_delivery?: string | null; attempt_id?: string; capture?: { context_only: boolean; boundary_verified: boolean } };
type Hit = { session_id: string; request_id: string; content_id: string; original_text_offset: number };
type Ancestry = { roots: { state: string }[]; gap: string | null };
type ContentState = 'recorded' | 'partial' | 'cleared';
const isCleared = (completeness?: string) => completeness === 'deleted' || completeness === 'expired';
const errorCode = (error: unknown) => observationFailureCode(error) ?? safeDiagnosticCode(error, 'LOCAL_SERVICE_UNAVAILABLE');
const uniqueBy = <T,>(items: T[], id: (item: T) => string | number) => [...new Map(items.map(item => [id(item), item])).values()];
function ReadingRecovery({ status, language }: { status: ObservationRecoveryStatus | undefined; language: 'zh' | 'en' }) {
  if (!status) return null;
  return <p className="oc-meta session-local-feedback" role="status" data-observation-recovery={status}>{observationRecoveryText(status, language)}</p>;
}
function updateReadableEvent(events: Record<string, ReadableEvent>, id: string, event: ReadableEvent | null) {
  if (event) return JSON.stringify(events[id]) === JSON.stringify(event) ? events : { ...events, [id]: event };
  if (!(id in events)) return events;
  const next = { ...events };
  delete next[id];
  return next;
}
function sessionTimeLabel(value: number, language: 'zh' | 'en') {
  const date = new Date(value);
  const current = new Date();
  const day = (input: Date) => new Date(input.getFullYear(), input.getMonth(), input.getDate()).getTime();
  const delta = Math.round((day(current) - day(date)) / 86_400_000);
  if (delta === 0) return date.toLocaleTimeString(language === 'zh' ? 'zh-CN' : 'en', { hour: '2-digit', minute: '2-digit', hour12: false });
  if (delta === 1) return language === 'zh' ? '昨天' : 'Yesterday';
  return `${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}
type ObservationQueryWindow = { from_ms: number; to_ms: number };
// to_ms is exclusive, and a keyset cursor is only valid for the window that produced it, so one query cycle pins one window.
function currentWindow(): ObservationQueryWindow {
  const now = Date.now();
  return { from_ms: now - 7 * 86400000, to_ms: now + 1 };
}
export function Sessions({ initialSession = null, initialRequest = null, language = 'zh', refreshVersion = 0, active = true, onOpenAgents, onReturnToQuality }: { initialSession?: string | null; initialRequest?: string | null; language?: 'zh' | 'en'; refreshVersion?: number; active?: boolean; onOpenAgents?: () => void; onReturnToQuality?: () => void }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [reload, setReload] = useState(0);
  const modelLabels = useModelLabels(active, refreshVersion + reload);
  const sessionQuery = useRef(currentWindow());
  const searchQuery = useRef(currentWindow());
  const timelineQuery = useRef({ from_ms: 0, to_ms: Date.now() + 1, request_id: null as string | null });
  const [sessions, setSessions] = useState<Summary[]>([]);
  const [sessionCursor, setSessionCursor] = useState<string | null>(null);
  const [session, setSession] = useState<string | null>(null);
  const selectedSession = useRef(session);
  useLayoutEffect(() => { selectedSession.current = session; }, [session]);
  const [requests, setRequests] = useState<Request[]>([]);
  const [readingRequests, setReadingRequests] = useState<string[]>([]);
  const [requestCursor, setRequestCursor] = useState<string | null>(null);
  const [request, setRequest] = useState<string | null>(null);
  const [focusedRequest, setFocusedRequest] = useState<string | null>(null);
  const [anchor, setAnchor] = useState<Hit | null>(null);
  const [keyword, setKeyword] = useState('');
  const [hits, setHits] = useState<Hit[]>([]);
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const [partial, setPartial] = useState(false);
  const [listLoaded, setListLoaded] = useState(false);
  const [listBusy, setListBusy] = useState(false);
  const [onlySwitch, setOnlySwitch] = useState(false);
  const [listError, setListError] = useState('');
  const [timelineError, setTimelineError] = useState('');
  const { toast, showToast } = useTransientToast();
  const [sessionContentStates, setSessionContentStates] = useState<Record<string, ContentState>>({});
  const [requestContentStates, setRequestContentStates] = useState<Record<string, ContentState>>({});
  const [sessionModels, setSessionModels] = useState<Record<string, string>>({});
  const [sessionRoutes, setSessionRoutes] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [facts, setFacts] = useState<SafeFact[]>([]);
  const [factsRequest, setFactsRequest] = useState<string | null>(null);
  const [factsOpen, setFactsOpen] = useState(false);
  const [sessionView, setSessionView] = useState<'timeline' | 'conversation' | null>(null);
  const [requestPicked, setRequestPicked] = useState(false);
  const [routing, setRouting] = useState<TimelineAvailability>('loading');
  const [qualityPlan, setQualityPlan] = useState<string | null>(null);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [compact, setCompact] = useState(false);
  const [factsError, setFactsError] = useState('');
  const [factCursor, setFactCursor] = useState<string | null>(null);
  const [factPartial, setFactPartial] = useState(false);
  const [factsBusy, setFactsBusy] = useState(false);
  const [cleanupOpen, setCleanupOpen] = useState(false);
  const [cleanupScope, setCleanupScope] = useState<'content_only' | 'facts_and_content'>('content_only');
  const [cleanupError, setCleanupError] = useState('');
  const [focusedSummary, setFocusedSummary] = useState<Summary | null>(null);
  const generation = useRef(0);
  const timelineGeneration = useRef(0);
  const factGeneration = useRef(0);
  type Owner = 'list' | 'timeline' | 'facts';
  const pendingReads = useRef<Partial<Record<Owner, AbortController>>>({});
  const loadedPages = useRef({ list: 0, timeline: 0, facts: 0 });
  const [recovering, setRecovering] = useState<Partial<Record<Owner, ObservationRecoveryStatus>>>({});
  const [listRebuilding, setListRebuilding] = useState(false);
  const [requestsRebuilding, setRequestsRebuilding] = useState(false);
  const listPane = useRef<HTMLDivElement>(null);
  const listAnchor = useRef<{ id: string; offset: number } | null>(null);
  const searchMounted = useRef(false);
  const keywordTimer = useRef<number | null>(null);
  const initialTarget = useRef(initialSession);
  const initialRequestTarget = useRef(initialRequest);
  function startRead(kind: Owner) {
    pendingReads.current[kind]?.abort();
    const controller = new AbortController();
    pendingReads.current[kind] = controller;
    setRecovering(current => ({ ...current, [kind]: undefined }));
    return controller.signal;
  }
  function recoveryOptions(kind: Owner, signal: AbortSignal, onRebuild?: () => void) {
    return {
      signal, onRebuild,
      onRecovery: (status: ObservationRecoveryStatus) => setRecovering(current => ({ ...current, [kind]: status })),
    };
  }
  function finishRead(kind: Owner, signal: AbortSignal) {
    if (pendingReads.current[kind]?.signal !== signal) return;
    delete pendingReads.current[kind];
    setRecovering(current => ({ ...current, [kind]: undefined }));
  }
  function acceptSummaryVisibility(summaries: Summary[]) {
    // Called only after the read's generation guard. Current metadata supersedes
    // a local cleanup-success hint, including a session revived by new traffic.
    setSessionContentStates(current => {
      if (!summaries.some(item => item.session_id in current)) return current;
      const next = { ...current };
      for (const item of summaries) delete next[item.session_id];
      return next;
    });
  }
  function clearTimelineSelection() {
    pendingReads.current.timeline?.abort();
    pendingReads.current.facts?.abort();
    timelineGeneration.current++;
    setBusy(false); setTimelineError('');
    setRequestsRebuilding(false);
    setSession(null); setRequests([]); setReadingRequests([]); setRequest(null);
    setRequestCursor(null); setFocusedRequest(null); setAnchor(null); setFactsOpen(false);
    setRequestPicked(false);
  }
  function changeOnlySwitch(value: boolean) {
    if (keywordTimer.current !== null) { window.clearTimeout(keywordTimer.current); keywordTimer.current = null; }
    if (value !== onlySwitch) setSessions([]);
    setOnlySwitch(value); setHits([]); setPartial(false); clearTimelineSelection();
    void load(null, { onlySwitch: value });
  }
  async function load(cursor: string | null = null, override?: { keyword?: string; onlySwitch?: boolean }) {
    if (!active || (cursor && pendingReads.current.list)) return;
    const effectiveKeyword = override?.keyword ?? keyword;
    const effectiveOnlySwitch = override?.onlySwitch ?? onlySwitch;
    const searching = Boolean(effectiveKeyword.trim());
    if (!cursor) {
      loadedPages.current.list = 0;
      setListRebuilding(false);
      listAnchor.current = null;
      if (searching) { searchQuery.current = currentWindow(); setSearchCursor(null); setPartial(false); }
      else { sessionQuery.current = currentWindow(); setSessionCursor(null); }
    }
    const queryWindow = searching ? searchQuery.current : sessionQuery.current;
    const signal = startRead('list');
    const epoch = ++generation.current; setListBusy(true); setListError('');
    const query = { ...queryWindow, session_id: null, limit: 50, cursor, agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: effectiveOnlySwitch };
    const options = recoveryOptions('list', signal, () => {
      const pane = listPane.current;
      if (pane && !listAnchor.current) {
        const top = pane.getBoundingClientRect().top;
        const row = [...pane.querySelectorAll<HTMLElement>('[data-session-id]')].find(item => item.getBoundingClientRect().bottom > top);
        if (row?.dataset.sessionId) listAnchor.current = { id: row.dataset.sessionId, offset: row.getBoundingClientRect().top - top };
      }
      setListRebuilding(true);
      setSearchCursor(null); setSessionCursor(null);
      if (searching) setHits([]);
      else setSessions([]);
    });
    try {
      if (searching) {
        const { pages, rebuilt } = await readObservationPages<Page<{ hits: Hit[]; index_partial: boolean }>>(
          'session_search', { ...query, keyword: effectiveKeyword.trim() }, loadedPages.current.list, options,
        );
        if (signal.aborted || epoch !== generation.current) return;
        const matches = pages.flatMap(page => page.hits);
        loadedPages.current.list = (rebuilt ? 0 : loadedPages.current.list) + pages.length;
        setHits(current => uniqueBy(cursor && !rebuilt ? [...current, ...matches] : matches, hit => hit.session_id));
        setSearchCursor(pages.at(-1)!.next_cursor);
        setPartial(pages.some(page => page.index_partial));
        const selected = selectedSession.current;
        if (!cursor || (rebuilt && selected && !matches.some(hit => hit.session_id === selected))) clearTimelineSelection();
      } else {
        const { pages, rebuilt } = await readObservationPages<Page<{ sessions: Summary[] }>>('sessions', query, loadedPages.current.list, options);
        if (signal.aborted || epoch !== generation.current) return;
        const summaries = pages.flatMap(page => page.sessions);
        loadedPages.current.list = (rebuilt ? 0 : loadedPages.current.list) + pages.length;
        acceptSummaryVisibility(summaries);
        setListLoaded(true);
        setSessions(current => uniqueBy(cursor && !rebuilt ? [...current, ...summaries] : summaries, item => item.session_id));
        setSessionCursor(pages.at(-1)!.next_cursor); setHits([]); setPartial(false);
        const selected = selectedSession.current;
        if (rebuilt && selected && !summaries.some(item => item.session_id === selected)) clearTimelineSelection();
      }
    } catch (e) {
      if (!observationCancelled(e, signal) && epoch === generation.current) {
        setListError(errorCode(e)); setSearchCursor(null); setSessionCursor(null);
        if (observationAccessLost(e)) { setHits([]); setSessions([]); clearTimelineSelection(); }
      }
    } finally {
      finishRead('list', signal);
      if (epoch === generation.current) { setListBusy(false); setListRebuilding(false); }
    }
  }
  useLayoutEffect(() => {
    if (!active) return;
    const target = initialTarget.current;
    const targetRequest = initialRequestTarget.current;
    initialTarget.current = null;
    initialRequestTarget.current = null;
    if (target) void timeline(target, null, undefined, targetRequest);
    void load();
    return () => {
      generation.current++;
      for (const controller of Object.values(pendingReads.current)) controller.abort();
      if (keywordTimer.current !== null) {
        window.clearTimeout(keywordTimer.current);
        keywordTimer.current = null;
      }
    };
  }, [reload, refreshVersion, active]);
  useLayoutEffect(() => {
    const pane = listPane.current, anchor = listAnchor.current;
    if (listRebuilding || !pane || !anchor) return;
    const row = [...pane.querySelectorAll<HTMLElement>('[data-session-id]')].find(item => item.dataset.sessionId === anchor.id);
    if (row) pane.scrollTop += row.getBoundingClientRect().top - pane.getBoundingClientRect().top - anchor.offset;
    listAnchor.current = null;
  }, [listRebuilding]);
  // Refresh just the selected summary. List pagination keeps its original window
  // and order; a newer selected row must not restart that query or its selection.
  useEffect(() => {
    if (!active || !session || cleanupOpen || listRebuilding) return;
    let stopped = false, inFlight = false;
    let blocked = false;
    let timer: number | undefined;
    const controller = new AbortController();
    const schedule = () => {
      window.clearTimeout(timer);
      if (!stopped && !blocked && !document.hidden) timer = window.setTimeout(() => void refreshSummary(), 2000);
    };
    async function refreshSummary() {
      if (stopped || blocked || inFlight || document.hidden) return;
      inFlight = true;
      const listGeneration = generation.current;
      const detailGeneration = timelineGeneration.current;
      try {
        const page = await readObservation<Page<{ sessions: Summary[] }>>('sessions', {
          session_id: session, from_ms: 0, to_ms: Date.now() + 1, limit: 1, cursor: null,
          agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: false,
        }, { signal: controller.signal });
        if (stopped || document.hidden || listGeneration !== generation.current || detailGeneration !== timelineGeneration.current) return;
        const updated = page.sessions.find(item => item.session_id === session);
        if (updated) {
          acceptSummaryVisibility([updated]);
          setFocusedSummary(updated);
          setSessions(current => current.map(item => item.session_id === session ? updated : item));
        } else {
          blocked = true;
          setSessions(current => current.filter(item => item.session_id !== session));
          setHits(current => current.filter(hit => hit.session_id !== session));
          clearTimelineSelection();
        }
      } catch (error) {
        blocked = true;
        if (!controller.signal.aborted && observationAccessLost(error)) clearTimelineSelection();
      }
      finally { inFlight = false; schedule(); }
    }
    const visibility = () => { window.clearTimeout(timer); if (document.visibilityState !== 'hidden') void refreshSummary(); };
    document.addEventListener('visibilitychange', visibility);
    if (sessions.some(item => item.session_id === session) || focusedSummary?.session_id === session) schedule();
    else void refreshSummary();
    return () => { stopped = true; controller.abort(); window.clearTimeout(timer); document.removeEventListener('visibilitychange', visibility); };
  }, [session, active, cleanupOpen, listRebuilding, reload, refreshVersion]);
  useEffect(() => {
    if (!searchMounted.current) { searchMounted.current = true; return; }
    if (!active) return;
    const timer = window.setTimeout(() => { keywordTimer.current = null; void load(null, { keyword }); }, 260);
    keywordTimer.current = timer;
    return () => { window.clearTimeout(timer); if (keywordTimer.current === timer) keywordTimer.current = null; };
  }, [keyword]);
  async function timeline(id: string, cursor: string | null = null, hit?: Hit, exactRequest?: string | null) {
    if (!active || (cursor && pendingReads.current.timeline)) return;
    const signal = startRead('timeline');
    const epoch = ++timelineGeneration.current; setBusy(true); setTimelineError('');
    if (!cursor) {
      loadedPages.current.timeline = 0;
      setRequestsRebuilding(false);
      pendingReads.current.facts?.abort();
      timelineQuery.current = { from_ms: 0, to_ms: Date.now() + 1, request_id: exactRequest ?? null };
      const initialRequest = hit?.request_id ?? exactRequest ?? null;
      setFocusedRequest(exactRequest ?? null); setSessionView(hit || exactRequest ? 'conversation' : null);
      setRequestPicked(false);
      setSession(id); setRequests([]); setReadingRequests(initialRequest ? [initialRequest] : []); setRequest(initialRequest); setAnchor(hit ?? null);
      setFactsOpen(false); setFacts([]); setFactsError(''); setFactCursor(null); setFactPartial(false);
      setFocusedSummary(sessions.find(item => item.session_id === id) ?? null);
    }
    try {
      const exactQuery = {
        ...timelineQuery.current,
        session_id: id,
        only_model_switch: false,
      };
      const { pages, rebuilt } = await readObservationPages<Page<{ requests: Request[] }>>('timeline', { ...exactQuery, limit: 50, cursor }, loadedPages.current.timeline,
        recoveryOptions('timeline', signal, () => {
          setRequestsRebuilding(true); setRequests([]); setRequestCursor(null);
          pendingReads.current.facts?.abort(); setFacts([]); setFactCursor(null);
        }));
      if (signal.aborted || epoch !== timelineGeneration.current) return;
      const rows = pages.flatMap(page => page.requests);
      loadedPages.current.timeline = (rebuilt ? 0 : loadedPages.current.timeline) + pages.length;
      if (exactRequest && !rows.some(item => item.request_id === exactRequest)) setTimelineError('OBSERVATION_REQUEST_UNAVAILABLE');
      setRequests(current => uniqueBy(cursor && !rebuilt ? [...current, ...rows] : rows, item => item.request_id));
      setRequestCursor(pages.at(-1)!.next_cursor);
      if (cursor && rebuilt) {
        const valid = new Set(rows.map(item => item.request_id));
        setRequest(current => current && valid.has(current) ? current : rows[0]?.request_id ?? null);
        setReadingRequests(current => current.filter(item => valid.has(item)));
        setAnchor(current => current && valid.has(current.request_id) ? current : null);
      } else if (!cursor && !hit && !exactRequest) {
        const first = rows[0]?.request_id ?? null;
        setRequest(first);
        setReadingRequests(first ? [first] : []);
      }
    } catch (e) {
      if (!observationCancelled(e, signal) && epoch === timelineGeneration.current) {
        setTimelineError(errorCode(e));
        if (observationAccessLost(e)) { setRequests([]); setReadingRequests([]); setRequest(null); setRequestCursor(null); setFactsOpen(false); }
      }
    } finally {
      finishRead('timeline', signal);
      if (epoch === timelineGeneration.current) { setBusy(false); setRequestsRebuilding(false); }
    }
  }
  async function loadFacts(next: string | null = null) {
    if (!active || !request || requestsRebuilding || listRebuilding || (next && pendingReads.current.facts)) return;
    const signal = startRead('facts');
    const selectedRequest = request;
    if (!next) { setFactsRequest(selectedRequest); loadedPages.current.facts = 0; }
    const epoch = ++factGeneration.current; setFactsBusy(true); setFactsError('');
    try {
      const { pages, rebuilt } = await readObservationPages<Page<{ facts: SafeFact[]; projection_partial: boolean }>>(
        'facts', { request_id: selectedRequest, limit: 50, cursor: next }, loadedPages.current.facts,
        recoveryOptions('facts', signal, () => { setFacts([]); setFactCursor(null); }),
      );
      if (signal.aborted || epoch !== factGeneration.current || request !== selectedRequest) return;
      const items = pages.flatMap(page => page.facts);
      loadedPages.current.facts = (rebuilt ? 0 : loadedPages.current.facts) + pages.length;
      setFacts(current => uniqueBy(next && !rebuilt ? [...current, ...items] : items, item => item.sequence));
      setFactCursor(pages.at(-1)!.next_cursor);
      setFactPartial(pages.some(page => page.projection_partial));
    } catch (e) {
      if (!observationCancelled(e, signal) && epoch === factGeneration.current) {
        setFactsError(errorCode(e));
        if (observationAccessLost(e)) { setFacts([]); setFactCursor(null); }
      }
    } finally {
      finishRead('facts', signal);
      if (epoch === factGeneration.current) setFactsBusy(false);
    }
  }
  function openFacts() {
    setFactsOpen(true);
  }
  async function clearSession(data_class: 'content_only' | 'facts_and_content') {
    if (!session) return;
    for (const controller of Object.values(pendingReads.current)) controller.abort();
    setBusy(true); setCleanupError(''); generation.current++; timelineGeneration.current++; factGeneration.current++;
    let refreshAfterCleanup: (() => void) | undefined;
    try {
      const selectedSession = session;
      const result = await invoke<{ cancelled?: boolean; managed_native_cleanup_pending?: boolean; managed_object_cleanup_pending?: boolean; object_cleanup_pending?: boolean }>('observation_delete', { input: { session_id: selectedSession, data_class, language } });
      if (result.cancelled) {
        setCleanupOpen(false);
        showToast(text('已取消清理，记录没有改变。', 'Cleanup cancelled. The session is unchanged.'));
        return;
      }
      window.dispatchEvent(new Event('hiroute-content-invalidated'));
      if (data_class === 'content_only') {
        setSessionContentStates(current => ({ ...current, [selectedSession]: 'cleared' }));
        setRequestContentStates(current => ({ ...current, ...Object.fromEntries(requests.map(item => [item.request_id, 'cleared' as const])) }));
        setRequests([]);
        setRequest(null); setReadingRequests([]);
        refreshAfterCleanup = () => { void timeline(selectedSession); };
      } else {
        setSessionContentStates(current => { const next = { ...current }; delete next[selectedSession]; return next; });
        setSession(null);
        setRequest(null); setReadingRequests([]);
        setRequests([]);
        refreshAfterCleanup = () => { void load(); };
      }
      setCleanupOpen(false);
      setFactsOpen(false);
      showToast(result.managed_native_cleanup_pending || result.managed_object_cleanup_pending || result.object_cleanup_pending
        ? text('已停止展示，部分本地数据待清理。', 'Hidden from view; some local data is still pending cleanup.')
        : text('清理完成。', 'Cleanup completed.'));
    } catch (e) {
      setCleanupError(errorCode(e));
    }
    finally {
      setBusy(false); setListBusy(false); setListRebuilding(false);
      refreshAfterCleanup?.();
    }
  }
  function refresh() {
    if (focusedRequest && session) { initialTarget.current = session; initialRequestTarget.current = focusedRequest; }
    window.dispatchEvent(new Event('hiroute-content-invalidated')); generation.current++; factGeneration.current++; clearTimelineSelection(); setListError(''); setFacts([]); setHits([]); setReload(current => current + 1);
  }
  const selectedSummary = sessions.find(item => item.session_id === session)
    ?? (focusedSummary?.session_id === session ? focusedSummary : undefined);
  const selectedContentCleared = Boolean(session && sessionContentStates[session] === 'cleared') || isCleared(selectedSummary?.content_completeness);
  const agentName = (summary?: Summary) => sessionAgentLabel(summary, language);
  const emptyFirstUse = listLoaded && !listBusy && !listError && !keyword.trim() && !onlySwitch && sessions.length === 0;
  const selectedRequest = requests.find(item => item.request_id === request);
  const displayedRequestIds = request ? (readingRequests.length ? readingRequests : [request]) : [];
  const lastDisplayedIndex = requests.findIndex(item => item.request_id === displayedRequestIds.at(-1));
  const nextRequestId = lastDisplayedIndex >= 0 ? requests[lastDisplayedIndex + 1]?.request_id : undefined;
  const attemptModels = factsRequest === request
    ? [...facts]
      .filter(fact => fact.native_model)
      .sort((left, right) => (left.attempt_ordinal ?? Number.MAX_SAFE_INTEGER) - (right.attempt_ordinal ?? Number.MAX_SAFE_INTEGER) || left.sequence - right.sequence)
      .map(fact => fact.native_model as string)
      .filter((model, index, models) => index === 0 || model !== models[index - 1])
    : [];
  const selectedContentState = request ? requestContentStates[request] ?? 'recorded' : 'recorded';
  const reportRequestContentState = useCallback((requestId: string, state: ContentState) => {
    setRequestContentStates(current => current[requestId] === state ? current : { ...current, [requestId]: state });
  }, []);

  useEffect(() => {
    if (!listLoaded || listBusy || busy || listError || session || keyword.trim() || !sessions[0]) return;
    void timeline(sessions[0].session_id);
  }, [listLoaded, listBusy, busy, listError, session, keyword, onlySwitch, sessions]);

  useLayoutEffect(() => {
    setFacts([]); setFactsError(''); setFactCursor(null); setFactPartial(false);
    setFactsRequest(null);
    factGeneration.current++;
    if (active && !requestsRebuilding && !listRebuilding && (factsOpen || selectedRequest?.within_request_fallback === true) && request) void loadFacts();
    return () => pendingReads.current.facts?.abort();
  }, [active, factsOpen, request, selectedRequest?.within_request_fallback, listRebuilding, requestsRebuilding]);

  // A failed read keeps the tab so its explanation and Retry stay reachable; only a confirmed empty read hides it.
  const routed = routing === 'turns' || routing === 'blocked' || routing === 'unavailable';
  const pane = routed ? sessionView ?? 'timeline' : 'conversation';
  // Loading the first conversation request does not mean the person picked a timeline turn.
  const routingRequestId = requestPicked ? request : focusedRequest ?? anchor?.request_id ?? (sessionView === 'conversation' ? request : null);
  const selectTimelineRequest = (item: TimelineRequest) => {
    setRequestPicked(true); setRequest(item.request_id); setReadingRequests([item.request_id]); setAnchor(null);
    setRequests(current => current.some(r => r.request_id === item.request_id) ? current : [...current, { request_id: item.request_id, started_at_ms: item.started_at_ms, outcome: item.outcome, native_turn_id: null, within_request_fallback: null, final_native_model: item.native_model, between_turn_model_change: null, previous_native_turn_id: null, previous_turn_model: null, turn_final_native_model: null }].sort((a, b) => a.started_at_ms - b.started_at_ms));
  };
  return <ProductPage
    title={text('会话', 'Sessions')}
    subtitle={text('查看 Agent 实际经过 HiRoute 的对话、请求内回退和运行事实', 'View agent conversations, in-request fallbacks and run details captured by HiRoute')}
    actions={<>{session && <button className="btn" type="button" aria-pressed={compact} onClick={() => setCompact(value => !value)}>{compact ? text('退出紧凑视图', 'Exit compact view') : text('紧凑视图', 'Compact view')}</button>}{onReturnToQuality && <button className="btn btn-quiet" type="button" onClick={onReturnToQuality}><UiIcon name="arrowLeft" />{text('返回模型表现', 'Back to model performance')}</button>}{partial && <span className="badge warn">{text('部分正文暂不可搜索', 'Some content is not searchable yet')}</span>}</>}
    flush={!emptyFirstUse}
    className={`sessions-page${active && compact && session ? ' is-compact' : ''}`}
  >
    <Toast notice={toast} />
    {!listLoaded && listBusy && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{recovering.list ? observationRecoveryText(recovering.list, language) : text('正在读取本机会话…', 'Reading local sessions…')}</p></div></div>}
    {active && <MaintenanceStatus key={reload} language={language} />}

    {!listLoaded && listError && !keyword.trim() ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="warning" /></span><h3>{text('暂时无法读取会话', 'Sessions are temporarily unavailable')}</h3><p>{nativeConnectionReadText(listError, language) ?? text('请确认本机服务正在运行，然后重试。', 'Make sure the local service is running, then retry.')}</p><button className="btn btn-primary" type="button" onClick={refresh}>{text('重试', 'Retry')}</button></div></div> : !listLoaded && listBusy ? null : emptyFirstUse ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="sessions" /></span><h3>{text('还没有会话记录', 'No sessions yet')}</h3><p>{text('在已接入的 Agent 中开始使用，经过 HiRoute 的会话会显示在这里。', 'Start in a connected Agent. Sessions routed through HiRoute will appear here.')}</p>{onOpenAgents && <button className="btn btn-primary" type="button" onClick={onOpenAgents}>{text('查看 Agent 接入', 'View Agent connections')}</button>}</div></div> : <div className={`session-layout${session ? ' has-selection' : ''}${factsOpen ? ' with-inspector' : ''}${keyword.trim() ? ' is-searching' : ''}${listError ? ' with-list-error' : ''}`}>
      <aside className="session-master" aria-label={text('会话列表', 'Session list')}>
        <form onSubmit={event => { event.preventDefault(); void load(); }} className="session-filters">
          <label className="search-box"><span className="sr-only">{text('搜索会话正文', 'Search session content')}</span><UiIcon name="search" /><input className="input" value={keyword} maxLength={256} onChange={event => { pendingReads.current.list?.abort(); generation.current++; setListRebuilding(false); clearTimelineSelection(); setSearchCursor(null); setSessionCursor(null); setHits([]); setPartial(false); setListError(''); setListBusy(true); setKeyword(event.target.value); }} placeholder={text('搜索会话正文', 'Search session content')} /></label>
          <div className="segmented" role="group" aria-label={text('会话筛选', 'Session filter')}><button className={`segment${!onlySwitch ? ' active' : ''}`} type="button" aria-pressed={!onlySwitch} onClick={() => changeOnlySwitch(false)}>{text('全部', 'All')}</button><button className={`segment${onlySwitch ? ' active' : ''}`} type="button" aria-pressed={onlySwitch} onClick={() => changeOnlySwitch(true)}>{text('发生过请求内回退', 'In-request fallbacks')}</button></div>
        </form>
        {listError && <div className="callout bad session-local-feedback" role="alert" data-error-code={listError}><UiIcon name="warning" /><div><strong>{keyword.trim() ? text('搜索暂时失败', 'Search is temporarily unavailable') : text('列表没有刷新', 'The list was not refreshed')}</strong><p>{keyword.trim() ? text('请重试，重新读取当前可搜索的会话。', 'Retry to search the currently available sessions.') : text('已加载的会话仍然保留。', 'Previously loaded sessions are still available.')}</p><button className="btn" type="button" disabled={listBusy} onClick={() => void load()}>{text('重试', 'Retry')}</button></div></div>}
        <div className="session-list" ref={listPane}>
          <ReadingRecovery status={recovering.list} language={language} />
          {keyword.trim() ? <>
            {listBusy && !recovering.list && <p className="oc-meta session-local-feedback" role="status"><span className="oc-spinner" />{text('正在搜索…', 'Searching…')}</p>}
            {hits.map(hit => {
              const context = sessions.find(item => item.session_id === hit.session_id);
              return <button className={`session-item${session === hit.session_id ? ' active' : ''}`} key={hit.session_id} data-session-id={hit.session_id} aria-current={session === hit.session_id ? 'page' : undefined} onClick={() => void timeline(hit.session_id, null, hit)}>
                <div className="session-item-top"><span className="muted session-item-time">{context ? sessionTimeLabel(context.last_request_at_ms, language) : text('搜索命中', 'Match')}</span></div>
                <div className="session-item-title"><ContentExcerpt recoverReads active={active} contentCleared={sessionContentStates[hit.session_id] === 'cleared' || isCleared(context?.content_completeness)} hit={hit} language={language} fallback={text('查看命中正文', 'Open matching content')} revision={refreshVersion + reload} /></div>
                <div className="session-item-meta">{context ? <><span>{agentName(context)}</span>{sessionRoutes[context.session_id] && <><span>·</span><span>Plan: {sessionRoutes[context.session_id]}</span></>}{sessionModels[context.session_id] && <><span>·</span><span>{sessionModels[context.session_id]}</span></>}</> : text('点击读取所在会话', 'Open the matching session')}</div>
              </button>;
            })}
            {searchCursor && !listError && <button className="btn session-more" disabled={listBusy} onClick={() => void load(searchCursor)}>{text('加载更多结果', 'Load more results')}</button>}
            {!listBusy && !listError && !hits.length && <div className="empty-state session-list-empty"><div><span className="empty-icon"><UiIcon name="search" /></span><h3>{partial ? text('尚未找到匹配', 'No match found yet') : text('没有匹配的会话', 'No matching sessions')}</h3><p>{partial ? text('部分正文暂不可搜索，可稍后刷新。', 'Some content is not searchable yet. Refresh later.') : text('试试其他关键词。', 'Try another keyword.')}</p></div></div>}
          </> : <>{sessions.map(item => { const contentState = sessionContentStates[item.session_id] ?? (isCleared(item.content_completeness) ? 'cleared' : item.content_completeness === 'complete' ? 'recorded' : 'partial'); return <button className={`session-item${session === item.session_id ? ' active' : ''}`} key={item.session_id} data-session-id={item.session_id} aria-current={session === item.session_id ? 'page' : undefined} onClick={() => void timeline(item.session_id)}><div className="session-item-top"><span className="muted session-item-time">{sessionTimeLabel(item.last_request_at_ms, language)}</span>{contentState === 'cleared' ? <span className="badge warn no-dot">{text('正文已清理', 'Content cleared')}</span> : contentState === 'partial' ? <span className="badge warn no-dot">{text('记录不完整', 'Incomplete record')}</span> : item.fallback_request_count > 0 && <span className="badge warn no-dot">{text('请求内回退', 'Request fallback')}</span>}</div><div className="session-item-title"><ContentExcerpt recoverReads active={active} contentCleared={contentState === 'cleared'} session={item.session_id} language={language} fallback={text('会话记录', 'Session')} limit={48} revision={`${refreshVersion + reload}:${item.last_request_at_ms}:${item.request_count}`} onModel={model => setSessionModels(current => current[item.session_id] === model ? current : { ...current, [item.session_id]: model })} onRoute={route => setSessionRoutes(current => current[item.session_id] === route ? current : { ...current, [item.session_id]: route })} /></div><div className="session-item-meta"><span>{agentName(item)}</span>{sessionRoutes[item.session_id] && <><span>·</span><span>Plan: {sessionRoutes[item.session_id]}</span></>}{sessionModels[item.session_id] && <><span>·</span><span>{sessionModels[item.session_id]}</span></>}{!sessionRoutes[item.session_id] && !sessionModels[item.session_id] && <><span>·</span><span>{item.request_count} {text('个请求', 'requests')}</span></>}</div></button>; })}{sessionCursor && !listError && <button className="btn session-more" disabled={listBusy} onClick={() => void load(sessionCursor)}>{text('加载更多会话', 'Load more sessions')}</button>}{!listBusy && !busy && !listError && listLoaded && !sessions.length && <div className="empty-state session-list-empty"><div><h3>{text('当前筛选没有会话', 'No sessions match this filter')}</h3><p>{text('切换到全部会话。', 'Show all sessions.')}</p></div></div>}</>}
        </div>
      </aside>

      <section className="session-transcript">
        {listRebuilding ? <ReadingRecovery status={recovering.list ?? 'rebuilding'} language={language} /> : session ? <article className="session-detail">
          <div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => { clearTimelineSelection(); if (!listLoaded) void load(); }}><UiIcon name="arrowLeft" />{text('返回会话列表', 'Back to sessions')}</button></div>
          <header className="transcript-head"><div><h2><ContentExcerpt recoverReads active={active} contentCleared={selectedContentCleared} session={session} language={language} fallback={text('会话正文', 'Conversation')} limit={56} revision={`${refreshVersion + reload}:${selectedSummary?.last_request_at_ms}:${selectedSummary?.request_count}`} onModel={model => setSessionModels(current => current[session] === model ? current : { ...current, [session]: model })} onRoute={route => setSessionRoutes(current => current[session] === route ? current : { ...current, [session]: route })} /></h2><p>{[agentName(selectedSummary), sessionRoutes[session] ? `Plan: ${sessionRoutes[session]}` : undefined, selectedSummary ? sessionTimeLabel(selectedSummary.last_request_at_ms, language) : undefined].filter(Boolean).join(' · ')}</p></div><div className="transcript-actions">{routed && <div className="segmented session-view-tabs" role="group" aria-label={text('会话视图', 'Session view')}>{([['timeline', text('路由时间线', 'Routing timeline')], ['conversation', text('对话', 'Conversation')]] as const).map(([value, label]) => <button key={value} type="button" className={`segment${pane === value ? ' active' : ''}`} aria-pressed={pane === value} onClick={() => setSessionView(value)}>{label}</button>)}</div>}<button className="btn" type="button" disabled={!request} onClick={() => factsOpen ? setFactsOpen(false) : openFacts()}><UiIcon name="activity" />{factsOpen ? text('收起运行记录', 'Hide run record') : text('运行记录', 'Run record')}</button>{!factsOpen && <button className="icon-btn" type="button" aria-label={text('清理此会话', 'Clear this session')} disabled={busy} onClick={() => { setCleanupError(''); setCleanupOpen(true); }}><UiIcon name="trash" /></button>}</div></header>
          {focusedRequest && focusedRequest === request && <div className="callout session-detail-feedback session-request-focus" data-request-id={focusedRequest} role="status"><UiIcon name="activity" /><div><strong>{selectedRequest ? text('已定位所选请求', 'Selected request') : timelineError ? text('所选请求不可用', 'Selected request unavailable') : text('正在定位所选请求…', 'Locating selected request…')}</strong>{selectedRequest && <p>{new Date(selectedRequest.started_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')}{selectedRequest.final_native_model && ' · ' + selectedRequest.final_native_model}<br />{text('下方包含本次请求的历史对话和回复。', 'The content below includes this request’s conversation history and response.')}</p>}</div><button className="btn" type="button" onClick={() => void timeline(session)}>{text('查看完整会话', 'View full session')}</button></div>}
          <ReadingRecovery status={recovering.timeline} language={language} />
          {timelineError && <div className="callout bad session-detail-feedback" role="alert" data-error-code={timelineError}><UiIcon name="warning" /><div><strong>{text('会话详情没有刷新', 'The session details were not refreshed')}</strong><p>{nativeConnectionReadText(timelineError, language) ?? (requests.length ? text('已加载的请求仍然保留。', 'Previously loaded requests are still available.') : text('暂时无法读取这条会话。', 'This session is temporarily unavailable.'))}</p><button className="btn" type="button" onClick={() => void timeline(session, null, undefined, focusedRequest)}>{text('重试', 'Retry')}</button></div></div>}
          <div className={`session-routing-and-conversation show-${pane}`}>
          <Disclosure className="native-details session-model-performance" label={text('本会话的模型表现', 'Model performance in this session')} language={language} onOpenChange={setQualityOpen}><PlanQuality key={session} sessionId={session} language={language} compact active={active && qualityOpen} refreshVersion={refreshVersion + reload} onOpenEvidence={(targetSession, targetRequest) => void timeline(targetSession, null, undefined, targetRequest)} /></Disclosure>
          <div className="session-pane" hidden={pane !== 'timeline'}><RoutingTimeline key={session} session={session} modelLabels={modelLabels} contentCleared={selectedContentCleared} clearedRequestIds={Object.keys(requestContentStates).filter(id => requestContentStates[id] === 'cleared')} language={language} active={active} displayed={pane === 'timeline'} refreshVersion={refreshVersion + reload} requestId={routingRequestId} onOpenPerformance={setQualityPlan} onAvailability={setRouting} onSelectRequest={selectTimelineRequest} onViewConversation={item => { selectTimelineRequest(item); setSessionView('conversation'); }} /></div>
          <section className="routing-conversation" hidden={pane !== 'conversation'} aria-label={text('对话', 'Conversation')}>
          {requests.length > 1 && <Disclosure className="native-details request-timeline" label={text(`请求记录（${requests.length}）`, `Requests (${requests.length})`)} language={language}><nav className="native-list" aria-label={text('请求时间线', 'Request timeline')}>{requests.map((item, index) => <button className={`list-row${request === item.request_id ? ' active' : ''}`} key={item.request_id} aria-current={request === item.request_id ? 'true' : undefined} onClick={() => { setRequestPicked(true); setRequest(item.request_id); setReadingRequests([item.request_id]); setAnchor(null); }}><span className="row-main"><span className="row-title">{text('请求', 'Request')} {index + 1}</span><span className="row-meta">{item.routing_context?.state === 'recorded' && item.routing_context.display_name ? item.routing_context.display_name : item.final_native_model ?? text('路由未记录', 'Route not recorded')}{item.within_request_fallback === true ? text(' · 请求内回退', ' · Request fallback') : item.between_turn_model_change === true ? text(' · 跨轮切换', ' · Turn change') : ''}</span></span></button>)}{requestCursor && <button className="btn" disabled={busy} onClick={() => void timeline(session, requestCursor)}>{text('加载更多请求', 'Load more requests')}</button>}</nav></Disclosure>}
          {active && !requestsRebuilding && displayedRequestIds.length ? displayedRequestIds.map((id, index) => <div key={id} className="request-chapter">{index > 0 && <h3>{text('后续请求', 'Following request')} {requests.findIndex(item => item.request_id === id) + 1}</h3>}<RequestBody active={pane === 'conversation'} contentCleared={selectedContentCleared || requestContentStates[id] === 'cleared'} request={id} summary={requests.find(item => item.request_id === id)} attemptModels={id === request ? attemptModels : []} anchor={id === request ? anchor : null} language={language} onOpenFacts={openFacts} onContentState={reportRequestContentState} onNextRequest={index === displayedRequestIds.length - 1 && nextRequestId ? () => { setRequestPicked(true); setReadingRequests(current => [...current, nextRequestId]); setRequest(nextRequestId); setAnchor(null); } : undefined} onMoreRequests={index === displayedRequestIds.length - 1 && !nextRequestId && requestCursor && !busy ? () => void timeline(session, requestCursor) : undefined} /></div>) : !busy && <div className="empty-state transcript-empty"><div><p>{text('此会话尚未取得可显示的请求。', 'No displayable request was retrieved for this session.')}</p></div></div>}
          </section></div>
        </article> : <div className="empty-state session-no-selection"><div><span className="empty-icon"><UiIcon name="sessions" /></span><h3>{text('选择一个会话', 'Select a session')}</h3></div></div>}
      </section>
      {active && !listRebuilding && !requestsRebuilding && factsOpen && <RuntimeFacts language={language} summary={selectedRequest ? { outcome: selectedRequest.outcome, finalModel: selectedRequest.final_native_model, modelFallback: selectedRequest.within_request_fallback } : undefined} facts={facts} partial={factPartial} busy={factsBusy} error={factsError} contentState={selectedContentState} usage={<><ReadingRecovery status={recovering.facts} language={language} /><ObservationValue key={session} session={session} language={language} /></>} next={Boolean(factCursor)} onNext={() => void loadFacts(factsError ? null : factCursor)} onClose={() => setFactsOpen(false)} onClear={() => { setCleanupError(''); setCleanupOpen(true); }} />}
    </div>}
    <Dialog open={Boolean(qualityPlan)} title={text('Plan 模型表现', 'Plan model performance')} closeLabel={text('关闭模型表现', 'Close model performance')} onClose={() => setQualityPlan(null)}>
      {qualityPlan && <PlanQuality planId={qualityPlan} language={language} active={active} refreshVersion={refreshVersion + reload} onOpenEvidence={(targetSession, targetRequest) => { setQualityPlan(null); void timeline(targetSession, null, undefined, targetRequest); }} />}
    </Dialog>
    <Dialog open={cleanupOpen && Boolean(session)} title={text('选择清理范围', 'Choose what to clear')} description={selectedSummary ? `${agentName(selectedSummary)} · ${sessionTimeLabel(selectedSummary.last_request_at_ms, language)}` : text('当前会话', 'Current session')} closeLabel={text('关闭清理范围', 'Close cleanup options')} closeDisabled={busy} onClose={() => { if (!busy) { setCleanupOpen(false); setCleanupError(''); } }} footer={<><button className="btn" type="button" disabled={busy} onClick={() => { setCleanupOpen(false); setCleanupError(''); }}>{text('取消', 'Cancel')}</button><button className="btn btn-danger" type="button" disabled={busy} onClick={() => void clearSession(cleanupScope)}>{busy ? text('正在读取影响范围…', 'Reading impact…') : text('继续', 'Continue')}</button></>}>
      <div className="option-panel" role="radiogroup" aria-label={text('清理范围', 'Cleanup scope')} aria-disabled={busy}>
        <label className="check-row"><input type="radio" name="session-cleanup-scope" value="content_only" disabled={busy} checked={cleanupScope === 'content_only'} onChange={() => { setCleanupScope('content_only'); setCleanupError(''); }} /><div><strong>{text('只清理正文', 'Clear content only')}</strong><span>{text('删除问题、回答和工具内容，保留运行记录。', 'Remove prompts, answers and tool content; keep the run record.')}</span></div></label>
        <label className="check-row"><input type="radio" name="session-cleanup-scope" value="facts_and_content" disabled={busy} checked={cleanupScope === 'facts_and_content'} onChange={() => { setCleanupScope('facts_and_content'); setCleanupError(''); }} /><div><strong>{text('删除这条会话记录', 'Delete this session record')}</strong><span>{text('同时删除此会话的逐请求记录。', 'Also remove this session’s per-request records.')}</span></div></label>
      </div>
      <p className="oc-meta">{text('下一步会显示具体影响范围并请你确认。其他会话与长期金额汇总保留。', 'The next step shows the exact impact for confirmation. Other sessions and long-term amount summaries are preserved.')}</p>
      {cleanupError && <div className="callout bad" role="alert" data-error-code={cleanupError}><UiIcon name="warning" /><span>{text('暂时无法确认清理范围，当前会话没有改变。请重试。', 'The cleanup scope could not be confirmed. The session is unchanged; try again.')}</span></div>}
    </Dialog>
  </ProductPage>;
}
function RequestBody({ request, active, contentCleared, summary, attemptModels, anchor, language, onOpenFacts, onContentState, onNextRequest, onMoreRequests }: { request: string; active: boolean; contentCleared: boolean; summary?: Request; attemptModels: string[]; anchor: Hit | null; language: 'zh' | 'en'; onOpenFacts(): void; onContentState(requestId: string, state: ContentState): void; onNextRequest?: () => void; onMoreRequests?: () => void }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [catalog, setCatalog] = useState<Content[]>([]);
  const [events, setEvents] = useState<Record<string, ReadableEvent>>({});
  const receiveEvent = useCallback((id: string, event: ReadableEvent | null) => setEvents(current => updateReadableEvent(current, id, event)), []);

  const [rootsPartial, setRootsPartial] = useState(false);
  const [ancestryIncomplete, setAncestryIncomplete] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [catalogLoaded, setCatalogLoaded] = useState(false);
  const [recovery, setRecovery] = useState<ObservationRecoveryStatus>();
  const [partRecovery, setPartRecovery] = useState<Record<string, ObservationRecoveryStatus>>({});
  const reportPartRecovery = useCallback((id: string, status?: ObservationRecoveryStatus) => {
    setPartRecovery(current => {
      if (current[id] === status) return current;
      if (status) return { ...current, [id]: status };
      const next = { ...current };
      delete next[id];
      return next;
    });
  }, []);
  const [contentEpoch, setContentEpoch] = useState(0);
  const generation = useRef(0);
  const loadedPages = useRef(0);
  const controller = useRef(new AbortController());
  const ancestryController = useRef<AbortController | null>(null);
  const pendingAncestry = useRef<{ roots: string[]; partial: boolean } | null>(null);
  function discardCatalog() {
    ancestryController.current?.abort();
    pendingAncestry.current = null;
    setCatalog([]); setEvents({}); setPartRecovery({}); setCursor(null); setCatalogLoaded(false);
    setContentEpoch(epoch => epoch + 1);
  }
  async function load(next: string | null = null) {
    if (!active) return;
    const epoch = ++generation.current; setBusy(true); setError(''); setRecovery(undefined);
    const signal = controller.current.signal;
    if (!next) { loadedPages.current = 0; setCatalogLoaded(false); }
    try {
      // Hidden reasoning can span many catalog pages. One reading action must
      // reach something displayable without fetching those private bodies.
      const { pages, rebuilt } = await readObservationPages<Page<{ contents: Content[]; transcript_roots: string[]; roots_partial: boolean }>>(
        'catalog', { request_id: request, limit: 20, cursor: next, include_semantic_metadata: true }, loadedPages.current,
        { signal, onRecovery: setRecovery, onRebuild: discardCatalog },
        page => !page.contents.some(item => displayableContent(item.kind)),
      );
      if (signal.aborted || epoch !== generation.current) return;
      const contents = pages.flatMap(page => page.contents);
      const roots = [...new Set(pages.flatMap(page => page.transcript_roots))];
      const partial = pages.some(page => page.roots_partial);
      const append = Boolean(next && !rebuilt);
      loadedPages.current = (rebuilt ? 0 : loadedPages.current) + pages.length;
      if (!append) setEvents({});
      setCatalog(current => uniqueBy(append ? [...current, ...contents] : contents, item => item.content_id));
      setCursor(pages.at(-1)!.next_cursor);
      setRootsPartial(current => partial || (append && current));
      setCatalogLoaded(true);
      if (!append) {
        setAncestryIncomplete(false);
        void checkAncestry(roots, partial);
      }
    } catch (e) {
      if (!observationCancelled(e, signal) && epoch === generation.current) {
        setError(errorCode(e));
        if (observationAccessLost(e)) discardCatalog();
      }
    } finally { if (epoch === generation.current) { setBusy(false); setRecovery(undefined); } }
  }
  useLayoutEffect(() => {
    function invalidate() {
      generation.current++;
      controller.current.abort();
      discardCatalog();
      if (active && !contentCleared) {
        controller.current = new AbortController();
        void load();
      }
    }
    controller.current = new AbortController();
    if (contentCleared) discardCatalog();
    if (!active) { setBusy(false); setRecovery(undefined); }
    else if (!contentCleared) {
      if (!catalogLoaded) void load();
      else if (pendingAncestry.current) void checkAncestry(pendingAncestry.current.roots, pendingAncestry.current.partial);
    }
    window.addEventListener('hiroute-content-invalidated', invalidate);
    return () => {
      generation.current++;
      controller.current.abort();
      ancestryController.current?.abort();
      window.removeEventListener('hiroute-content-invalidated', invalidate);
    };
  }, [request, contentCleared, active]);
  async function checkAncestry(roots: string[], partial: boolean) {
    ancestryController.current?.abort();
    const owner = new AbortController();
    ancestryController.current = owner;
    const signal = owner.signal;
    if (partial) setAncestryIncomplete(true);
    if (!roots.length) return;
    pendingAncestry.current = { roots, partial };
    const results = await Promise.allSettled(roots.map(transcript_root => readObservation<Ancestry>('ancestry', { request_id: request, transcript_root }, { signal })));
    if (signal.aborted) return;
    pendingAncestry.current = null;
    setAncestryIncomplete(partial || results.some(result => result.status === 'rejected' || Boolean(result.value.gap)));
  }
  const contentIncomplete = catalogLoaded && (rootsPartial || ancestryIncomplete || catalog.some(item => !['available', 'complete'].includes(item.state)));
  const contentState: ContentState = catalog.length > 0 && !cursor && catalog.every(item => isCleared(item.state))
    ? 'cleared'
    : !catalog.length || contentIncomplete ? 'partial' : 'recorded';
  useEffect(() => {
    if (!contentCleared && (catalogLoaded || error)) onContentState(request, contentState);
  }, [request, contentState, contentCleared, catalogLoaded, error, onContentState]);
  // Hide child message caches immediately; preserve navigation to new requests.
  if (contentCleared) return <section className="transcript-body">
    <p className="oc-meta">{text('正文已清理', 'Content cleared')}</p>
    {onNextRequest && <button className="btn" type="button" onClick={onNextRequest}>{text('继续读取下一请求', 'Continue to next request')}</button>}
    {onMoreRequests && <button className="btn" type="button" onClick={onMoreRequests}>{text('加载更多请求', 'Load more requests')}</button>}
  </section>;
  const occurrences = groupOccurrences(catalog.filter(item => displayableContent(item.kind)));
  const displayRuns = contentDisplayRuns(occurrences);
  // Input may replay answers from other models; a request's model and fallback
  // facts only describe its delivered response, never the last visible input.
  const assistantIndexes = occurrences.map((occurrence, index) => occurrence.role === 'assistant' && !occurrence.technical && occurrence.items.every(item => item.direction === 'response_delivered') ? index : -1).filter(index => index >= 0);
  const firstAssistantOccurrenceIndex = occurrences.findIndex(occurrence => occurrence.role === 'assistant');
  const finalAssistantIndex = assistantIndexes.at(-1) ?? -1;
  const toolEvents = groupToolEvents(Object.values(events));
  const renderOccurrence = (occurrence: (typeof occurrences)[number], index: number) => {
    const items = occurrence.items.filter(item => item.content_id !== anchor?.content_id);
    if (!items.length) return null;
    return <React.Fragment key={`${contentEpoch}:${occurrence.key}`}>
      {index === finalAssistantIndex && summary?.within_request_fallback === true && <button className="switch-marker" type="button" onClick={onOpenFacts}><UiIcon name="refresh" /><strong>{attemptModels.length > 1 ? attemptModels.join(' → ') : text('记录到请求内模型回退', 'Recorded model fallback within a request')}</strong><span>{attemptModels.length > 1 ? text('记录到请求内模型回退', 'Recorded model fallback within a request') : text('查看运行记录', 'View run record')}</span><UiIcon name="chevronRight" /></button>}
      <MessageOccurrence onRecovery={reportPartRecovery} active={active} language={language} request={request} items={items} technical={occurrence.technical} modelLabel={index === finalAssistantIndex ? summary?.final_native_model ?? attemptModels.at(-1) : null} onEvent={receiveEvent} />
    </React.Fragment>;
  };
  const toolBlocks = toolEvents.map(event => <div className="tool-block" key={event.logicalId}><strong>{event.name || text('工具调用', 'Tool call')}</strong><span>{event.arguments ? summarizeToolArguments(event.arguments) : event.phase === 'ready' ? text('参数已就绪，执行结果尚未取得。', 'Arguments ready; execution result not received.') : text('已请求，执行结果尚未取得。', 'Requested; execution result not received.')}</span></div>);
  const pendingParts = Object.values(partRecovery);
  const visibleRecovery = pendingParts.includes('slow') ? 'slow' : recovery ?? pendingParts[0];
  return <section className="transcript-body">{error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('正文暂时无法读取，请重试。', 'Content is temporarily unavailable. Try again.')}</span><button className="btn" type="button" disabled={busy} onClick={() => void load(null)}>{text('重试', 'Retry')}</button></div>}
    <ReadingRecovery status={visibleRecovery} language={language} />
    {contentIncomplete && <div className="callout warn completeness-banner" role="status"><UiIcon name="warning" /><div><strong>{text('会话内容的保存记录不完整', 'The saved conversation record is incomplete')}</strong><p>{text('此处只能展示已保存的内容，不代表 Agent 收到的回答不完整。运行记录仍然可用。', 'Only retained content is shown here. This does not establish that the Agent received an incomplete answer. Run records remain available.')}</p></div></div>}
    {anchor && catalog.some(item => item.content_id === anchor.content_id && !isPrivateContentKind(item.kind) && !isCleared(item.state)) && <MessageOccurrence onRecovery={reportPartRecovery} active={active} key={contentEpoch} language={language} request={request} items={[{ content_id: anchor.content_id, role: text('搜索命中', 'Search hit'), kind: 'text', state: 'complete', direction: '', media_type: '', message_occurrence_id: '' }]} anchor={{ contentId: anchor.content_id, offset: anchor.original_text_offset }} />}
    {displayRuns.map(run => <React.Fragment key={run.start}>{run.technical
      ? <Disclosure className="transcript-context" label={text(`系统与工具上下文（${run.end - run.start} 项）`, `System and tool context (${run.end - run.start})`)} language={language}>{occurrences.slice(run.start, run.end).map((occurrence, index) => renderOccurrence(occurrence, index + run.start))}</Disclosure>
      : occurrences.slice(run.start, run.end).map((occurrence, index) => <React.Fragment key={occurrence.key}>{renderOccurrence(occurrence, index + run.start)}{index + run.start === firstAssistantOccurrenceIndex && toolBlocks}</React.Fragment>)}
      {run.technical && firstAssistantOccurrenceIndex >= run.start && firstAssistantOccurrenceIndex < run.end && toolBlocks}
    </React.Fragment>)}
    {cursor && <button className="btn" disabled={busy} onClick={() => void load(cursor)}>{text('继续读取', 'Load more')}</button>}
    {!cursor && catalogLoaded && !busy && !error && onNextRequest && <button className="btn" type="button" onClick={onNextRequest}>{text('继续读取下一请求', 'Continue to next request')}</button>}
    {!cursor && catalogLoaded && !busy && !error && onMoreRequests && <button className="btn" type="button" onClick={onMoreRequests}>{text('加载更多请求', 'Load more requests')}</button>}
    {!catalogLoaded && busy && !recovery && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取正文…', 'Reading content…')}</p></div>}
    {catalogLoaded && !busy && !error && !catalog.length && <div className="callout"><UiIcon name="lock" /><span>{text('正文尚未取得、未捕获或已清理。', 'Content is unavailable, was not captured, or has been cleared.')}</span></div>}
    <div className="callout transcript-privacy"><UiIcon name="lock" /><span>{text('这里仅展示 Gateway 可见的本机会话内容，不包含 Agent 未发送的本地执行。', 'This view contains only local, Gateway-visible content and excludes agent activity that was never sent.')}</span></div>

  </section>;
}

function displayableContent(kind: string): boolean {
  return !isPrivateContentKind(kind) && kind !== 'content_block_started';
}

function groupOccurrences(contents: Content[]): { key: string; role: string; technical: boolean; items: Content[] }[] {
  const grouped = new Map<string, { key: string; role: string; items: Content[] }>();
  for (const item of contents) {
    if (isPrivateContentKind(item.kind)) continue;
    const occurrence = item.message_occurrence_id || item.content_id;
    const key = `${item.direction}\u0000${item.fork_id ?? ''}\u0000${occurrence}`;
    const group = grouped.get(key) ?? { key, role: item.role, items: [] };
    group.items.push(item);
    grouped.set(key, group);
  }
  for (const group of grouped.values()) {
    group.items.sort((left, right) => (left.part_ordinal ?? 0) - (right.part_ordinal ?? 0));
  }
  const runs: { key: string; role: string; technical: boolean; items: Content[] }[] = [];
  for (const group of grouped.values()) {
    for (const item of group.items) {
      const technical = ['system', 'developer', 'tool_definition'].includes(item.role) || isTechnicalContentKind(item.kind);
      const previous = runs.at(-1);
      if (previous?.key.startsWith(`${group.key}\u0000`) && previous.technical === technical) previous.items.push(item);
      else runs.push({ key: `${group.key}\u0000${runs.length}`, role: item.role, technical, items: [item] });
    }
  }
  return runs;
}

function MessageOccurrence({ request, active, items, anchor, language, modelLabel, technical = false, onEvent, onRecovery }: { onEvent?: (id: string, event: ReadableEvent | null) => void; onRecovery: (id: string, status?: ObservationRecoveryStatus) => void; request: string; active: boolean; items: Content[]; anchor?: { contentId: string; offset: number }; language: 'zh' | 'en'; modelLabel?: string | null; technical?: boolean }) {
  const [visibility, setVisibility] = useState<Record<string, boolean>>({});
  const [partIncomplete, setPartIncomplete] = useState<Record<string, boolean>>({});
  const [contextParts, setContextParts] = useState<Record<string, boolean>>({});
  const [events, setEvents] = useState<Record<string, ReadableEvent>>({});
  const reportVisibility = useCallback((id: string, visible: boolean) => setVisibility(current => current[id] === visible ? current : { ...current, [id]: visible }), []);
  const reportIncomplete = useCallback((id: string, incomplete: boolean) => setPartIncomplete(current => current[id] === incomplete ? current : { ...current, [id]: incomplete }), []);
  const reportContext = useCallback((id: string, contextOnly: boolean) => setContextParts(current => current[id] === contextOnly ? current : { ...current, [id]: contextOnly }), []);
  const reportEvent = useCallback((id: string, event: ReadableEvent | null) => {
    setEvents(current => updateReadableEvent(current, id, event));
    if (!event || event.kind === 'tool') onEvent?.(id, event);
  }, [onEvent]);
  const role = items[0]?.role ?? '';
  const importedContext = items.length > 0 && items.every(item => item.capture?.context_only === true);
  const contextOnly = importedContext || role === 'user' && !anchor && items.length > 0 && items.every(item => contextParts[item.content_id] === true);
  const roleLabel = importedContext ? (language === 'zh' ? '历史上下文 · 执行模型未知' : 'Historical context · execution model unknown') : technical ? (language === 'zh' ? '工具上下文' : 'Tool context') : contextOnly ? (language === 'zh' ? 'Agent 附加上下文' : 'Agent-added context') : role === 'assistant' ? (language === 'zh' ? 'Agent 回答' : 'Agent answer') : role === 'user' ? (language === 'zh' ? '你的问题' : 'Your question') : role;
  const potential = items.some(item => !item.media_type.includes('hiroute.model-stream-event') && !item.media_type.includes('hiroute.semantic-terminal'));
  const responseText = coalesceResponseText(Object.values(events));
  const visible = potential || responseText.some(part => part.text.length > 0) || Object.values(visibility).some(Boolean);
  const incomplete = items.some(item => !['available', 'complete'].includes(item.state)) || Object.values(partIncomplete).some(Boolean);
  return <article hidden={!visible} aria-hidden={!visible} className={`message ${!technical && role === 'assistant' ? 'assistant' : !technical && role === 'user' && !contextOnly ? 'user' : 'system'}`}><span className="message-avatar">{technical ? '·' : role === 'assistant' ? 'AI' : role === 'user' && !contextOnly ? 'YOU' : '·'}</span><div><div className="message-role">{roleLabel}{modelLabel && !importedContext && <span className="badge info no-dot">{modelLabel}</span>}{incomplete && <span className="badge warn no-dot">{language === 'zh' ? '内容不完整' : 'Incomplete'}</span>}</div>{items.map(item => <MessagePart onRecovery={onRecovery} active={active} key={item.content_id} request={request} item={item} anchor={anchor?.contentId === item.content_id ? anchor.offset : undefined} language={language} onEvent={reportEvent} onVisibility={reportVisibility} onIncomplete={reportIncomplete} onContext={reportContext} />)}{responseText.map(part => <div className="message-content" key={`${part.kind}-${part.index}-${part.version ?? ''}`}>{part.version && <span className="oc-meta">{language === 'zh' ? (part.version === 'accepted' ? '已接受的内容' : '有冲突的结束版本') : (part.version === 'accepted' ? 'Accepted content' : 'Conflicting final version')}</span>}{part.text}</div>)}</div></article>;
}

function MessagePart({ request, active, item, anchor, language, onEvent, onVisibility, onIncomplete, onContext, onRecovery }: { onEvent?: (id: string, event: ReadableEvent | null) => void; onRecovery: (id: string, status?: ObservationRecoveryStatus) => void; onVisibility(id: string, visible: boolean): void; onIncomplete(id: string, incomplete: boolean): void; onContext(id: string, contextOnly: boolean): void; request: string; active: boolean; item: Content; anchor?: number; language: 'zh' | 'en' }) {
  const [offset, setOffset] = useState(0); const marker = useRef<HTMLElement>(null);
  const [text, setText] = useState(''); const [cursor, setCursor] = useState<string | null>(null);
  const [state, setState] = useState(item.state); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const [recovery, setRecovery] = useState<ObservationRecoveryStatus>();
  useEffect(() => {
    if (!recovery) return;
    onRecovery(item.content_id, recovery);
    return () => onRecovery(item.content_id);
  }, [item.content_id, onRecovery, recovery]);
  const generation = useRef(0); const controller = useRef(new AbortController());
  const loadedPages = useRef(0);
  const loadedScope = useRef<string | null>(null);
  const scope = JSON.stringify([request, item.content_id, anchor]);
  function discardText() {
    loadedScope.current = null;
    setText(''); setCursor(null); onEvent?.(item.content_id, null);
  }
  async function load(next: string | null = null) {
    if (!active) return;
    const epoch = ++generation.current; setBusy(true); setError(''); setRecovery(undefined);
    const signal = controller.current.signal;
    if (!next) loadedPages.current = 0;
    try {
      const { pages, rebuilt } = await readObservationPages<Page<{ state: string; chunks: { text: string; original_byte_offset: number }[] }>>(
        'content', { request_id: request, content_id: item.content_id, cursor: next, anchor_offset: anchor ?? null }, loadedPages.current,
        { signal, onRecovery: setRecovery, onRebuild: discardText },
      );
      if (signal.aborted || epoch !== generation.current) return;
      const page = pages.at(-1)!;
      setState(page.state);
      if (pages.some(item => isCleared(item.state))) { discardText(); loadedScope.current = scope; return; }
      const chunks = uniqueBy(pages.flatMap(item => item.chunks), chunk => chunk.original_byte_offset);
      const chunkText = chunks.map(c => c.text).join('');
      loadedPages.current = (rebuilt ? 0 : loadedPages.current) + pages.length;
      if (!next || rebuilt) setOffset(chunks[0]?.original_byte_offset ?? 0);
      setText(current => next && !rebuilt ? current + chunkText : chunkText);
      setCursor(page.next_cursor);
      loadedScope.current = scope;
    } catch (e) {
      if (!observationCancelled(e, signal) && epoch === generation.current) {
        setError(errorCode(e));
        if (observationAccessLost(e) || observationFailureCode(e) === 'RESOURCE_NOT_FOUND') discardText();
      }
    } finally { if (epoch === generation.current) { setBusy(false); setRecovery(undefined); } }
  }
  useLayoutEffect(() => {
    const invalidate = () => { generation.current++; controller.current.abort(); discardText(); };
    controller.current = new AbortController();
    if (!active) { setBusy(false); setRecovery(undefined); }
    else if (isCleared(item.state)) discardText();
    else if (loadedScope.current !== scope) { discardText(); void load(); }
    window.addEventListener('hiroute-content-invalidated', invalidate);
    return () => {
      generation.current++; controller.current.abort();
      window.removeEventListener('hiroute-content-invalidated', invalidate);
    };
  }, [active, request, item.content_id, item.state, anchor, onEvent]);
  useEffect(() => { marker.current?.scrollIntoView({ block: 'center' }); }, [text]);
  const event = readableEvent(text, item.media_type, item.direction);
  useEffect(() => { if (event && event.kind !== 'reasoning' && onEvent) onEvent(item.content_id, event); }, [text, item.content_id, onEvent]);
  const structuredPending = (item.media_type.includes('hiroute.model-stream-event') || item.media_type.includes('hiroute.semantic-terminal')) && !text && !error;
  const hiddenEvent = Boolean((structuredPending || (event && (event.kind === 'block' || event.kind === 'reasoning' || ((event.kind === 'tool' || event.kind === 'text' || event.kind === 'refusal') && onEvent)))) && anchor === undefined && !error && !cursor);
  useEffect(() => { onVisibility(item.content_id, !hiddenEvent); }, [hiddenEvent, item.content_id, onVisibility]);
  useEffect(() => { onIncomplete(item.content_id, state !== 'available' && state !== 'complete'); }, [item.content_id, onIncomplete, state]);
  const bytes = new TextEncoder().encode(text);
  const position = anchor === undefined ? -1 : anchor - offset;
  const marked = position >= 0 && position < bytes.length;
  const context = item.role === 'user' && anchor === undefined && !event && !cursor && !error ? splitLeadingAgentContext(text) : { context: '', body: text };
  useEffect(() => { onContext(item.content_id, Boolean(context.context && !context.body.trim())); }, [item.content_id, context.context, context.body, onContext]);
  if (isCleared(item.state) || isCleared(state)) return <p className="oc-meta">{language === 'zh' ? '正文已清理' : 'Content cleared'}</p>;
  if (hiddenEvent) return null;
  let visible = marked ? <>{new TextDecoder().decode(bytes.slice(0, position))}<mark ref={marker} aria-label={language === 'zh' ? '搜索命中位置' : 'Search hit position'}>↦</mark>{new TextDecoder().decode(bytes.slice(position))}</> : event && (event.kind === 'text' || event.kind === 'reasoning' || event.kind === 'refusal') ? event.text : event?.kind === 'tool' ? `${event.name || (language === 'zh' ? '工具参数' : 'Tool arguments')} · ${event.phase === 'started' ? (language === 'zh' ? '已请求，结果尚未取得' : 'Requested; result not received') : event.phase === 'ready' ? (language === 'zh' ? '调用参数已就绪，执行结果尚未取得' : 'Call arguments ready; execution result not received') : (language === 'zh' ? '参数片段' : 'Argument fragment')}` : event?.kind === 'block' ? (language === 'zh' ? '内容开始' : 'Content started') : text;
  if (item.role === 'user' && !marked && !event && !cursor) {
    if (context.context) visible = <><Disclosure className="message-context" label={language === 'zh' ? 'Agent 附加上下文' : 'Agent-added context'} language={language}>{context.context}</Disclosure>{context.body}</>;
  }
  return <>{busy && !recovery && <p className="oc-meta" role="status">{ language === 'zh' ? '正在读取正文…' : 'Loading content…'}</p>}{error && <div className="callout bad" role="alert" data-error-code={error}><span>{text ? (language === 'zh' ? '更多正文暂时无法读取，已加载内容仍保留。' : 'More content is temporarily unavailable. Loaded content is retained.') : (language === 'zh' ? '这段正文暂时无法读取。' : 'This content is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => void load(text ? cursor : null)}>{language === 'zh' ? '重试' : 'Retry'}</button></div>}{!event && text && item.media_type.includes('hiroute.model-stream-event') && <p className="oc-meta">{language === 'zh' ? '暂不能结构化显示，以下为已取得的原文。' : 'Structured display is unavailable; captured content follows.'}</p>}{marked && <p className="oc-meta">{language === 'zh' ? '已定位到搜索命中位置。' : 'Located the search match.'}</p>}{text && <div className="message-content">{visible}</div>}{!error && cursor && <button className="btn btn-quiet" disabled={busy} onClick={() => void load(cursor)}>{language === 'zh' ? '继续读取' : 'Load more'}</button>}</>;
}

export function ObservationValue({ session = null, language = 'zh' }: { session?: string | null; language?: 'zh' | 'en' }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  type Amount = { known_sum_micros: number | null; currency: string; valuation_kind: string; coverage: string };
type Value = { pending_requests: number; provisional_requests: number; unknown_traffic_requests: number; excluded_requests: number; amounts: Amount[]; usage: UsageMetric[]; input_cache_hit: CacheHitSummary; archive_boundary_partial: boolean; retention_boundary_partial: boolean };
  const [value, setValue] = useState<Value | null>(null); const [error, setError] = useState(''); const [revision, setRevision] = useState(0);
  const [recovery, setRecovery] = useState<ObservationRecoveryStatus>();
  useLayoutEffect(() => {
    let controller: AbortController | undefined;
    function load() {
      controller?.abort();
      controller = new AbortController();
      const signal = controller.signal;
      setValue(null); setError(''); setRecovery(undefined);
      void readObservation<Value>('home_value', { period: session ? 'seven_days' : 'today', session_id: session, currency: null }, { signal, onRecovery: setRecovery })
        .then(result => { if (!signal.aborted) { setValue(result); setRecovery(undefined); } })
        .catch(e => { if (!observationCancelled(e, signal)) setError(errorCode(e)); });
    }
    load();
    window.addEventListener('hiroute-content-invalidated', load);
    return () => { controller?.abort(); window.removeEventListener('hiroute-content-invalidated', load); };
  }, [session, revision]);
  const coverage = (value?.retention_boundary_partial || value?.usage.some(metric => metric.coverage !== 'complete') || value?.amounts.some(amount => amount.coverage !== 'complete') || value?.input_cache_hit.coverage !== 'complete')
    ? text('部分已记录', 'Partially recorded') : text('已记录', 'Recorded');
  if (error) return <section className="fact-section observation-value"><h4>{text('用量与费用', 'Usage and cost')}</h4><div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('用量暂时无法读取。', 'Usage is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => setRevision(current => current + 1)}>{text('重试', 'Retry')}</button></div></section>;
  if (!value) return <section className="fact-section observation-value"><h4>{text('用量与费用', 'Usage and cost')}</h4><div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{recovery ? observationRecoveryText(recovery, language) : text('正在读取用量…', 'Reading usage…')}</p></div></section>;
  const cacheHit = value.input_cache_hit;
  const ratio = formatCacheHit(cacheHit, language);
  const ratioBasis = cacheHit.state === 'available' && cacheHit.cache_read_tokens !== null && cacheHit.total_input_tokens !== null
    ? ` (${formatTokenCount(cacheHit.cache_read_tokens, language)} / ${formatTokenCount(cacheHit.total_input_tokens, language)})`
    : '';
  const gapDetails = [
    cacheHit.missing_attempt_count > 0 ? text(`${cacheHit.missing_attempt_count} 个 Attempt 缺少输入或缓存读取`, `${cacheHit.missing_attempt_count} attempts lack input or cache-read usage`) : '',
    cacheHit.invalid_attempt_count > 0 ? text(`${cacheHit.invalid_attempt_count} 个 Attempt 的缓存读取大于总输入`, `${cacheHit.invalid_attempt_count} attempts report cache read above total input`) : '',
    cacheHit.arithmetic_overflow ? text('统计发生算术溢出', 'Aggregation overflowed') : '',
    cacheHit.archive_coverage_partial ? text('旧归档缺少成对缓存口径', 'Older archives lack paired cache evidence') : '',
    value.retention_boundary_partial ? text('所选范围超出会话明细保留期，旧用量不可按会话恢复', 'The selected range exceeds session detail retention; older usage cannot be recovered for this session') : '',
    value.unknown_traffic_requests > 0 ? text(`${value.unknown_traffic_requests} 个请求尚未完成流量分类`, `${value.unknown_traffic_requests} requests remain unclassified`) : '',
    value.excluded_requests > 0 ? text(`已排除 ${value.excluded_requests} 个连接探针`, `${value.excluded_requests} connectivity probes excluded`) : '',
  ].filter(Boolean);
  return <section className="fact-section observation-value"><h4>{text('用量与费用', 'Usage and cost')}</h4><dl className="fact-kv">
    <dt>{text('输入 Token', 'Input tokens')}</dt><dd>{formatTokenCount(usageValue(value.usage, 'input'), language)}</dd>
    <dt>{text('输出 Token', 'Output tokens')}</dt><dd>{formatTokenCount(usageValue(value.usage, 'output'), language)}</dd>
    <dt>{text('缓存读取 Token', 'Cache-read tokens')}</dt><dd>{formatTokenCount(usageValue(value.usage, 'cache_read'), language)}</dd>
    <dt>{text('缓存写入 Token', 'Cache-write tokens')}</dt><dd>{formatTokenCount(usageValue(value.usage, 'cache_write'), language)}</dd>
    <dt>{text('输入缓存命中率', 'Input cache-hit rate')}</dt><dd>{ratio}{ratioBasis}<span className="oc-meta"> · {formatCacheHitCoverage(cacheHit, language)}</span></dd>
    {value.amounts.length ? value.amounts.map((amount, index) => <React.Fragment key={`${amount.currency}-${amount.valuation_kind}-${index}`}><dt>{amount.valuation_kind === 'usage_estimate' ? text('估算调用成本', 'Estimated call cost') : amount.valuation_kind === 'api_equivalent' ? text('API 等价值', 'API-equivalent value') : text('已计价金额', 'Priced amount')}</dt><dd>{amount.known_sum_micros === null ? text('尚未计价', 'Unpriced') : formatMoney(amount.currency, (amount.known_sum_micros / 1_000_000).toFixed(2))}</dd></React.Fragment>) : <><dt>{text('已计价金额', 'Priced amount')}</dt><dd>{text('尚未计价', 'Unpriced')}</dd></>}
  </dl><p className="oc-meta">{text('Token 来自已记录的上游用量；金额是独立估算，不代表供应商账单。', 'Tokens come from recorded upstream usage; monetary values are separate estimates, not provider bills.')}{coverage !== text('已记录', 'Recorded') ? ` ${coverage}${text('。', '.')}` : ''}</p>{gapDetails.length > 0 && <p className="oc-meta">{gapDetails.join(text('；', '; '))}{text('。', '.')}</p>}{(value.pending_requests > 0 || value.provisional_requests > 0 || value.archive_boundary_partial) && <p className="oc-meta">{text('部分请求仍在整理，数值可能稍后更新。', 'Some requests are still being prepared, so values may update later.')}</p>}</section>;
}

function MaintenanceStatus({ language }: { language: 'zh' | 'en' }) {
  const [status, setStatus] = useState<{ running: boolean; index_running: boolean; error_count: number } | null>(null);
  useLayoutEffect(() => {
    const controller = new AbortController();
    const signal = controller.signal;
    void readObservation<{ running: boolean; index_running: boolean; error_count: number }>('status', {}, { signal })
      .then(value => { if (!signal.aborted) setStatus(value); })
      .catch(() => { /* Maintenance is optional feedback for the page. */ });
    return () => controller.abort();
  }, []);
  if (!status) return null;
  return !status.running || !status.index_running || status.error_count > 0 ? <div className="callout warn session-page-feedback" role="status"><UiIcon name="clock" /><span>{language === 'zh' ? '部分会话仍在整理，搜索或清理结果可能稍后更新。' : 'Some sessions are still being prepared. Search or cleanup results may update shortly.'}</span></div> : null;
}

type TitleScanState = { sawContent: boolean; allDeleted: boolean; incomplete: boolean; inputUnconfirmed?: boolean };
type InputSelection = { version: number; evidence: 'native_matched' | 'wire_observed' | 'unavailable'; message_ordinal: number | null; source_unverified: boolean };
const TITLE_CONTENT_LIMIT = 1024 * 1024;

async function completeTitlePart(requestId: string, contentId: string, signal: AbortSignal, readOne: ObservationReader): Promise<{ text: string | null; incomplete: boolean }> {
  let cursor: string | null = null;
  let text = '';
  let byteCount = 0;
  const seen = new Set<string>();
  do {
    const page: { state: string; chunks: { text: string }[]; next_cursor: string | null } = await readOne('content', { request_id: requestId, content_id: contentId, cursor, anchor_offset: null });
    if (signal.aborted || ['deleted', 'expired'].includes(page.state)) return { text: null, incomplete: true };
    const next = page.chunks.map(chunk => chunk.text).join('');
    byteCount += new TextEncoder().encode(next).length;
    if (byteCount > TITLE_CONTENT_LIMIT) return { text: null, incomplete: true };
    text += next;
    cursor = page.next_cursor;
    if (cursor && seen.has(cursor)) return { text: null, incomplete: true };
    if (cursor) seen.add(cursor);
    if (!cursor) return { text, incomplete: !['available', 'complete'].includes(page.state) };
  } while (!signal.aborted);
  return { text: null, incomplete: true };
}

async function firstRequestTitle(requestId: string, signal: AbortSignal, state: TitleScanState, readOne: ObservationReader, latest = false): Promise<string | null> {
  let cursor: string | null = null;
  const contents: Content[] = [];
  const seen = new Set<string>();
  let selection: InputSelection | undefined;
  let firstPage = true;
  let inputAnchor: { source_request_id: string; message_occurrence_id: string } | undefined;
  let sourceRequest = requestId;
  do {
    const page: { contents: Content[]; roots_partial?: boolean; next_cursor?: string | null; input_selection?: InputSelection; input_anchor?: { source_request_id: string; message_occurrence_id: string } } = await readOne('catalog', { request_id: requestId, limit: 200, cursor, ...(latest ? { include_input_selection: true, include_semantic_metadata: true } : {}) });
    if (firstPage) { selection = page.input_selection; inputAnchor = page.input_anchor; }
    firstPage = false;
    contents.push(...page.contents);
    state.sawContent ||= page.contents.length > 0;
    state.allDeleted &&= page.contents.every(item => ['deleted', 'expired'].includes(item.state));
    state.incomplete ||= Boolean(page.roots_partial) || page.contents.some(item => !['available', 'complete', 'deleted', 'expired'].includes(item.state));
    cursor = page.next_cursor ?? null;
    if (cursor && seen.has(cursor)) { state.incomplete = true; break; }
    if (cursor) seen.add(cursor);
  } while (cursor && !signal.aborted);

  if (latest && (!selection || selection.version !== 1 || !['native_matched', 'wire_observed'].includes(selection.evidence)
    || !Number.isSafeInteger(selection.message_ordinal) || selection.message_ordinal === null
    || selection.message_ordinal < 0 || selection.message_ordinal > 65_535)) {
    // Older records did not retain a selected-input witness. The original
    // transcript stays readable; a late User envelope is not a substitute.
    state.inputUnconfirmed = true;
    return null;
  }

  if (latest && inputAnchor && inputAnchor.source_request_id !== requestId && !contents.some(item => item.message_occurrence_id === inputAnchor?.message_occurrence_id)) {
    // The server returns a one-hop source only while it remains authorized and
    // live. Never choose a different User message when that source disappears.
    contents.length = 0;
    sourceRequest = inputAnchor.source_request_id;
    const sourceSeen = new Set<string>();
    do {
      const page: { contents: Content[]; roots_partial?: boolean; next_cursor?: string | null } = await readOne('catalog', { request_id: sourceRequest, limit: 200, cursor, include_semantic_metadata: true });
      const selected = page.contents.filter(item => item.message_occurrence_id === inputAnchor?.message_occurrence_id);
      contents.push(...selected);
      state.sawContent ||= selected.length > 0;
      state.allDeleted &&= selected.every(item => ['deleted', 'expired'].includes(item.state));
      state.incomplete ||= Boolean(page.roots_partial) || selected.some(item => !['available', 'complete', 'deleted', 'expired'].includes(item.state));
      cursor = page.next_cursor ?? null;
      if (cursor && sourceSeen.has(cursor)) { state.incomplete = true; break; }
      if (cursor) sourceSeen.add(cursor);
    } while (cursor && !signal.aborted);
    state.incomplete ||= contents.length === 0;
  }

  const groups = new Map<string, Content[]>();
  for (const content of contents) {
    if (content.role !== 'user' || content.kind !== 'text' || ['deleted', 'expired', 'unavailable'].includes(content.state)) continue;
    if (latest && (content.direction !== 'request_input' || (sourceRequest === requestId ? content.message_ordinal !== selection?.message_ordinal : content.message_occurrence_id !== inputAnchor?.message_occurrence_id))) continue;
    const key = `${content.direction}\u0000${content.fork_id ?? ''}\u0000${content.message_occurrence_id || content.content_id}`;
    const group = groups.get(key) ?? [];
    group.push(content);
    groups.set(key, group);
  }
  const messages = [...groups.values()].sort((left, right) => (left[0]?.message_ordinal ?? 0) - (right[0]?.message_ordinal ?? 0));
  for (const message of messages) {
    message.sort((left, right) => (left.part_ordinal ?? 0) - (right.part_ordinal ?? 0));
    const parts: string[] = [];
    let readable = true;
    for (const content of message) {
      const part = await completeTitlePart(sourceRequest, content.content_id, signal, readOne);
      state.incomplete ||= part.incomplete;
      if (part.text === null) { readable = false; break; }
      parts.push(part.text);
    }
    if (!readable) continue;
    const title = firstRealUserText([parts.join('')]);
    if (title !== null) return title;
  }
  return null;
}

/** `request` uses the recorded selected-input position; it never guesses from late User context. */
export function ContentExcerpt({ session, request, requestRunning, hit, language, fallback = '', limit = 160, revision = 0, active = true, contentCleared = false, recoverReads = false, onCompleteness, onContentState, onModel, onRoute }: { session?: string; request?: string; requestRunning?: boolean; hit?: Hit; language: 'zh' | 'en'; fallback?: string; limit?: number; revision?: number | string; active?: boolean; contentCleared?: boolean; recoverReads?: boolean; onCompleteness?: (incomplete: boolean) => void; onContentState?: (state: ContentState) => void; onModel?: (model: string) => void; onRoute?: (route: string) => void }) {
  const node = useRef<HTMLSpanElement>(null);
  const key = JSON.stringify([session, request, hit?.request_id, hit?.content_id, hit?.original_text_offset, limit, revision]);
  type ExcerptStatus = 'loading' | 'recovering' | 'pending' | 'ready' | 'absent' | 'cleared' | 'error' | 'unconfirmed';
  const [result, setResult] = useState<{ key: string; value: string; status: ExcerptStatus }>({ key, value: '', status: 'loading' });
  const current = result.key === key ? result : { key, value: '', status: 'loading' as const };
  const { value, status } = current;
  const setValue = (value: string) => setResult(previous => ({ ...(previous.key === key ? previous : current), key, value }));
  const setStatus = (status: ExcerptStatus) => setResult(previous => ({ ...(previous.key === key ? previous : current), key, status }));
  const callbacks = useRef({ onCompleteness, onContentState, onModel, onRoute });
  callbacks.current = { onCompleteness, onContentState, onModel, onRoute };
  useEffect(() => { setResult({ key, value: '', status: 'loading' }); }, [key]);
  useLayoutEffect(() => {
    if (contentCleared) {
      setValue(''); setStatus('cleared');
      callbacks.current.onCompleteness?.(false);
      callbacks.current.onContentState?.('cleared');
      return;
    }
    let stopped = false, visible = false, inFlight = false, resolved = false;
    let timer: number | undefined, controller: AbortController | undefined;
    let retryMs = 2000;
    const eligible = () => !stopped && active && visible && document.visibilityState !== 'hidden';
    const schedule = () => {
      window.clearTimeout(timer);
      if (eligible() && !resolved) {
        timer = window.setTimeout(() => void load(), retryMs);
        retryMs = Math.min(retryMs * 2, 30000);
      }
    };
    function report(title: string | null, state: TitleScanState) {
      const cleared = state.sawContent && state.allDeleted && !state.incomplete;
      resolved = cleared || (state.sawContent && !state.incomplete);
      setValue(title === null ? '' : excerpt(title, limit));
      setStatus(cleared ? 'cleared' : !resolved ? 'pending' : state.inputUnconfirmed ? 'unconfirmed' : title !== null ? 'ready' : 'absent');
      callbacks.current.onCompleteness?.(!resolved);
      callbacks.current.onContentState?.(cleared ? 'cleared' : resolved ? 'recorded' : 'partial');
    }
    async function load() {
      if (!eligible() || inFlight || resolved) return;
      inFlight = true;
      controller = new AbortController();
      const signal = controller.signal;
      const toMs = Date.now() + 1;
      async function scan(readOne: ObservationReader) {
        if (hit) {
          const page = await readOne<{ state?: string; chunks: { text: string; original_byte_offset: number }[] }>('content', { request_id: hit.request_id, content_id: hit.content_id, cursor: null, anchor_offset: hit.original_text_offset });
          if (signal.aborted) return;
          const cleared = ['deleted', 'expired'].includes(page.state ?? '');
          resolved = cleared || ['available', 'complete'].includes(page.state ?? '');
          setValue(cleared ? '' : excerptAround(page.chunks.map(c => c.text).join(''), Math.max(0, hit.original_text_offset - (page.chunks[0]?.original_byte_offset ?? 0)), limit));
          setStatus(cleared ? 'cleared' : resolved ? 'ready' : 'pending');
          callbacks.current.onCompleteness?.(!resolved);
          callbacks.current.onContentState?.(cleared ? 'cleared' : resolved ? 'recorded' : 'partial');
          return;
        }
        const state: TitleScanState = { sawContent: false, allDeleted: true, incomplete: false };
        if (request) {
          const title = await firstRequestTitle(request, signal, state, readOne, true);
          if (!signal.aborted) report(title, state);
          return;
        }
        if (!session) { resolved = true; return; }
        let cursor: string | null = null;
        let firstRequest = true;
        const seen = new Set<string>();
        do {
          const timeline: Page<{ requests: Request[] }> = await readOne('timeline', { session_id: session, limit: 50, cursor, from_ms: 0, to_ms: toMs, agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: false });
          if (signal.aborted) return;
          for (const item of timeline.requests) {
            // Only affirmative native-purpose evidence excludes a title input.
            // Direct request/hit navigation still exposes the original content.
            if (item.native_auxiliary === true) continue;
            if (firstRequest) {
              firstRequest = false;
              if (item.final_native_model) callbacks.current.onModel?.(item.final_native_model);
              const routing = item.routing_context;
              if (routing?.state === 'recorded' && routing.name_state === 'recorded' && routing.display_name) callbacks.current.onRoute?.(routing.display_name);
            }
            const title = await firstRequestTitle(item.request_id, signal, state, readOne);
            if (signal.aborted) return;
            if (title !== null) { report(title, state); return; }
          }
          cursor = timeline.next_cursor;
          if (cursor && seen.has(cursor)) { state.incomplete = true; break; }
          if (cursor) seen.add(cursor);
        } while (cursor && !signal.aborted);
        if (!signal.aborted) report(null, state);
      }
      try {
        if (recoverReads) await recoverObservation(scan, {
          signal,
          onRecovery: status => { if (status === 'slow') setStatus('recovering'); },
          onRebuild: () => setValue(''),
        });
        else await scan((view, query) => read(view, query, signal));
      } catch (error) {
        if (!observationCancelled(error, signal)) {
          if (recoverReads) {
            resolved = true;
            if (observationAccessLost(error) || observationFailureCode(error) === 'RESOURCE_NOT_FOUND') setValue('');
          }
          setStatus('error');
          callbacks.current.onCompleteness?.(true);
          callbacks.current.onContentState?.('partial');
        }
      } finally { inFlight = false; schedule(); }
    }
    const visibility = () => {
      window.clearTimeout(timer);
      if (!eligible()) { if (!recoverReads || !active || !visible) controller?.abort(); }
      else if (!resolved) { retryMs = 2000; void load(); }
    };
    // Abort synchronously on privacy invalidation: a pending old response must
    // never repopulate the body while React schedules the next render.
    const invalidate = () => {
      controller?.abort(); window.clearTimeout(timer); resolved = false; retryMs = 2000;
      setValue(''); setStatus('pending');
      if (!inFlight) void load();
    };
    const observer = new IntersectionObserver(entries => {
      visible = entries.some(entry => entry.isIntersecting);
      visibility();
    });
    if (node.current) observer.observe(node.current);
    document.addEventListener('visibilitychange', visibility);
    window.addEventListener('hiroute-content-invalidated', invalidate);
    return () => {
      stopped = true; controller?.abort(); window.clearTimeout(timer); observer.disconnect();
      document.removeEventListener('visibilitychange', visibility);
      window.removeEventListener('hiroute-content-invalidated', invalidate);
    };
  }, [session, request, hit?.request_id, hit?.content_id, hit?.original_text_offset, limit, revision, active, contentCleared, recoverReads]);
  // Mask cached text in the same render that receives a visibility change.
  const currentStatus = contentCleared ? 'cleared' : status;
  const directRequest = Boolean(request && !hit);
  const statusText = currentStatus === 'cleared' ? (language === 'zh' ? '正文已清理' : 'Content cleared')
    : currentStatus === 'unconfirmed' ? (language === 'zh' ? '本轮输入位置未记录，可查看原始对话' : 'This turn’s input position was not recorded; view the original conversation')
    : currentStatus === 'recovering' ? observationRecoveryText('slow', language)
    : currentStatus === 'loading' ? (language === 'zh' ? '正在读取消息…' : 'Loading message…')
      : directRequest && currentStatus === 'error' ? (language === 'zh' ? '消息暂时无法读取' : 'Message could not be read')
        : directRequest && currentStatus === 'pending' && requestRunning === true ? (language === 'zh' ? '消息记录正在准备…' : 'Message record pending…')
        : directRequest && currentStatus === 'pending' && requestRunning === false ? (language === 'zh' ? '消息记录尚不完整' : 'Message record is incomplete')
        : directRequest && currentStatus === 'pending' ? (language === 'zh' ? '这一轮的消息暂不可用' : 'This turn’s message is unavailable')
        : currentStatus === 'error' ? (language === 'zh' ? '正文暂时无法读取' : 'Content is temporarily unavailable')
          : currentStatus === 'pending' ? (language === 'zh' ? '正文尚未就绪' : 'Content is not ready yet') : undefined;
  return <span ref={node} data-content-state={currentStatus} title={statusText}>{(contentCleared ? '' : value) || ((request || hit || currentStatus === 'cleared') && statusText) || fallback || (hit ? (language === 'zh' ? '查看命中正文' : 'Open matching content') : '')}</span>;
}
