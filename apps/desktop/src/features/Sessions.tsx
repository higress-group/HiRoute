import { RuntimeFacts, type SafeFact } from './RuntimeFacts';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Dialog, Disclosure, formatMoney, ProductPage, Toast, UiIcon, useTransientToast } from '../ui';
import type { CorrelationKind } from './session-correlation';
import { readableEvent, excerpt, excerptAround, groupToolEvents, contentDisplayRuns, coalesceResponseText, isPrivateContentKind, isTechnicalContentKind, summarizeToolArguments, type ReadableEvent } from './session-content';
import { firstRealUserText, splitLeadingAgentContext } from './session-title';
import { formatCacheHit, formatCacheHitCoverage, formatTokenCount, usageValue, type CacheHitSummary, type UsageMetric } from './usage-presentation';
import { safeDiagnosticCode } from '../error-code';
import { observationRead as read } from './observation-client';
import { PlanQuality } from './PlanQuality';
type Page<T> = { next_cursor: string | null } & T;
type Summary = { session_id: string; agent_id: string; request_count: number; fallback_request_count: number; last_request_at_ms: number; correlation_kind: CorrelationKind };
type RoutingContext = { state: 'recorded' | 'unavailable' | 'conflicted'; display_name: string | null; name_state: 'recorded' | 'unavailable'; plan_id?: string | null; plan_revision?: string | null };
type Request = { request_id: string; started_at_ms: number; outcome: string | null; native_turn_id: string | null; within_request_fallback: boolean | null; final_native_model: string | null; between_turn_model_change: boolean | null; previous_native_turn_id: string | null; previous_turn_model: string | null; turn_final_native_model: string | null; routing_context?: RoutingContext };
type Content = { content_id: string; role: string; kind: string; state: string; direction: string; media_type: string; message_occurrence_id: string; message_ordinal?: number; part_ordinal?: number; fork_id?: string; downstream_delivery?: string | null };
type Hit = { session_id: string; request_id: string; content_id: string; original_text_offset: number };
type Ancestry = { roots: { state: string }[]; gap: string | null };
type ContentState = 'recorded' | 'partial' | 'cleared';
const errorCode = (error: unknown) => safeDiagnosticCode(error, 'LOCAL_SERVICE_UNAVAILABLE');
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
export function Sessions({ initialSession = null, initialRequest = null, language = 'zh', refreshVersion = 0, active = true, onOpenAgents }: { initialSession?: string | null; initialRequest?: string | null; language?: 'zh' | 'en'; refreshVersion?: number; active?: boolean; onOpenAgents?: () => void }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [reload, setReload] = useState(0);
  const sessionQuery = useRef(currentWindow());
  const searchQuery = useRef(currentWindow());
  const timelineQuery = useRef<ObservationQueryWindow>({ from_ms: 0, to_ms: Date.now() + 1 });
  const [sessions, setSessions] = useState<Summary[]>([]);
  const [sessionCursor, setSessionCursor] = useState<string | null>(null);
  const [session, setSession] = useState<string | null>(null);
  const [requests, setRequests] = useState<Request[]>([]);
  const [readingRequests, setReadingRequests] = useState<string[]>([]);
  const [requestCursor, setRequestCursor] = useState<string | null>(null);
  const [request, setRequest] = useState<string | null>(null);
  const [anchor, setAnchor] = useState<Hit | null>(null);
  const [keyword, setKeyword] = useState('');
  const [hits, setHits] = useState<Hit[]>([]);
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const [partial, setPartial] = useState(false);
  const [listLoaded, setListLoaded] = useState(false);
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
  const searchMounted = useRef(false);
  const initialTarget = useRef(initialSession);
  const initialRequestTarget = useRef(initialRequest);
  async function load(cursor: string | null = null, override?: { keyword?: string; onlySwitch?: boolean }) {
    const effectiveKeyword = override?.keyword ?? keyword;
    const effectiveOnlySwitch = override?.onlySwitch ?? onlySwitch;
    const searching = Boolean(effectiveKeyword.trim());
    if (!cursor) {
      if (searching) { searchQuery.current = currentWindow(); setSearchCursor(null); }
      else { sessionQuery.current = currentWindow(); setSessionCursor(null); }
    }
    const queryWindow = searching ? searchQuery.current : sessionQuery.current;
    const epoch = ++generation.current; setBusy(true); setListError('');
    try {
      if (searching) {
        const page = await read<Page<{ hits: Hit[]; index_partial: boolean; budget_exhausted: boolean }>>('search', { ...queryWindow, session_id: null, keyword: effectiveKeyword.trim(), limit: 50, cursor, agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: effectiveOnlySwitch });
        if (epoch !== generation.current) return;
        setHits(current => cursor ? [...current, ...page.hits] : page.hits); setSearchCursor(page.next_cursor); setPartial(page.index_partial || page.budget_exhausted);
        if (!cursor) {
          const first = page.hits[0];
          if (first) void timeline(first.session_id, null, first);
          else {
            timelineGeneration.current++;
            setSession(null); setRequests([]); setReadingRequests([]); setRequest(null); setAnchor(null); setFactsOpen(false);
          }
        }
      } else {
        const page = await read<Page<{ sessions: Summary[] }>>('sessions', { ...queryWindow, session_id: null, limit: 50, cursor, agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: effectiveOnlySwitch });
        if (epoch !== generation.current) return;
        setListLoaded(true); setSessions(current => cursor ? [...current, ...page.sessions] : page.sessions); setSessionCursor(page.next_cursor); setHits([]); setPartial(false);
      }
    } catch (e) { if (epoch === generation.current) setListError(errorCode(e)); }
    finally { if (epoch === generation.current) setBusy(false); }
  }
  useEffect(() => {
    if (!active) return;
    const target = initialTarget.current;
    const targetRequest = initialRequestTarget.current;
    initialTarget.current = null;
    initialRequestTarget.current = null;
    if (target) void timeline(target, null, undefined, targetRequest);
    void load();
    return () => { generation.current++; };
  }, [reload, refreshVersion, active]);
  useEffect(() => {
    if (!searchMounted.current) { searchMounted.current = true; return; }
    const timer = window.setTimeout(() => void load(null, { keyword }), 260);
    return () => window.clearTimeout(timer);
  }, [keyword]);
  async function timeline(id: string, cursor: string | null = null, hit?: Hit, exactRequest?: string | null) {
    const epoch = ++timelineGeneration.current; setBusy(true); setTimelineError('');
    if (!cursor) {
      timelineQuery.current = { from_ms: 0, to_ms: Date.now() + 1 };
      const initialRequest = hit?.request_id ?? exactRequest ?? null;
      setSession(id); setRequests([]); setReadingRequests(initialRequest ? [initialRequest] : []); setRequest(initialRequest); setAnchor(hit ?? null);
      setFactsOpen(false); setFacts([]); setFactsError(''); setFactCursor(null); setFactPartial(false);
      setFocusedSummary(sessions.find(item => item.session_id === id) ?? null);
    }
    try {
      const exactQuery = {
        ...timelineQuery.current,
        session_id: id,
        request_id: !cursor ? exactRequest ?? null : null,
        only_model_switch: false,
      };
      const [page, summaryPage] = await Promise.all([
        read<Page<{ requests: Request[] }>>('timeline', { ...exactQuery, limit: 50, cursor }),
        cursor || sessions.some(item => item.session_id === id)
          ? Promise.resolve(null)
          : read<Page<{ sessions: Summary[] }>>('sessions', { ...exactQuery, limit: 1, cursor: null }),
      ]);
      if (epoch !== timelineGeneration.current) return;
      if (!cursor && summaryPage?.sessions[0]) setFocusedSummary(summaryPage.sessions[0]);
      setRequests(current => cursor ? [...current, ...page.requests] : page.requests); setRequestCursor(page.next_cursor);
      if (!cursor && !hit && !exactRequest) {
        const first = page.requests[0]?.request_id ?? null;
        setRequest(first);
        setReadingRequests(first ? [first] : []);
      }
    } catch (e) { if (epoch === timelineGeneration.current) setTimelineError(errorCode(e)); }
    finally { if (epoch === timelineGeneration.current) setBusy(false); }
  }
  async function loadFacts(next: string | null = null) {
    if (!request) return;
    const selectedRequest = request;
    if (!next) setFactsRequest(selectedRequest);
    const epoch = ++factGeneration.current; setFactsBusy(true); setFactsError('');
    try {
      const page = await read<Page<{ facts: SafeFact[]; projection_partial: boolean }>>('facts', { request_id: selectedRequest, limit: 50, cursor: next });
      if (epoch !== factGeneration.current || request !== selectedRequest) return;
      setFacts(current => next ? [...current, ...page.facts] : page.facts);
      setFactCursor(page.next_cursor);
      setFactPartial(page.projection_partial);
    } catch (e) {
      if (epoch === factGeneration.current) setFactsError(errorCode(e));
    } finally {
      if (epoch === factGeneration.current) setFactsBusy(false);
    }
  }
  function openFacts() {
    setFactsOpen(true);
  }
  async function clearSession(data_class: 'content_only' | 'facts_and_content') {
    if (!session) return;
    setBusy(true); setCleanupError(''); generation.current++; timelineGeneration.current++; factGeneration.current++;
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
        await timeline(selectedSession);
      } else {
        setSessionContentStates(current => { const next = { ...current }; delete next[selectedSession]; return next; });
        setSession(null);
        setRequest(null); setReadingRequests([]);
        setRequests([]);
        await load();
      }
      setCleanupOpen(false);
      setFactsOpen(false);
      showToast(result.managed_native_cleanup_pending || result.managed_object_cleanup_pending || result.object_cleanup_pending
        ? text('已停止展示，部分本地数据待清理。', 'Hidden from view; some local data is still pending cleanup.')
        : text('清理完成。', 'Cleanup completed.'));
    } catch (e) {
      setCleanupError(errorCode(e));
    }
    finally { setBusy(false); }
  }
  function refresh() { window.dispatchEvent(new Event('hiroute-content-invalidated')); generation.current++; timelineGeneration.current++; factGeneration.current++; setListError(''); setTimelineError(''); setSession(null); setRequest(null); setReadingRequests([]); setRequests([]); setFactsOpen(false); setFacts([]); setHits([]); setReload(current => current + 1); }
  const selectedSummary = sessions.find(item => item.session_id === session)
    ?? (focusedSummary?.session_id === session ? focusedSummary : undefined);
  const agentName = (id?: string) => !id
    ? text('未关联 Agent', 'Unlinked Agent')
    : id.toLowerCase().includes('codex') ? 'Codex'
      : id.toLowerCase().includes('claude') ? 'Claude Code'
        : text('本机 Agent', 'Local Agent');
  const emptyFirstUse = listLoaded && !busy && !listError && !keyword.trim() && !onlySwitch && sessions.length === 0;
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
    if (!listLoaded || busy || listError || session || keyword.trim() || !sessions[0]) return;
    void timeline(sessions[0].session_id);
  }, [listLoaded, busy, listError, session, keyword, onlySwitch, sessions]);

  useEffect(() => {
    setFacts([]); setFactsError(''); setFactCursor(null); setFactPartial(false);
    setFactsRequest(null);
    factGeneration.current++;
    if ((factsOpen || selectedRequest?.within_request_fallback === true) && request) void loadFacts();
  }, [factsOpen, request, selectedRequest?.within_request_fallback]);

  return <ProductPage
    title={text('会话', 'Sessions')}
    subtitle={text('查看 Agent 实际经过 HiRoute 的对话、请求内回退和运行事实', 'View agent conversations, in-request fallbacks and run details captured by HiRoute')}
    actions={partial ? <span className="badge warn">{text('搜索结果可能不完整', 'Search may be incomplete')}</span> : undefined}
    flush={!emptyFirstUse}
    className="sessions-page"
  >
    <Toast notice={toast} />
    {!listLoaded && busy && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取本机会话…', 'Reading local sessions…')}</p></div></div>}
    <MaintenanceStatus key={reload} language={language} />

    {!listLoaded && listError ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="warning" /></span><h3>{text('暂时无法读取会话', 'Sessions are temporarily unavailable')}</h3><p>{text('请确认本机服务正在运行，然后重试。', 'Make sure the local service is running, then retry.')}</p><button className="btn btn-primary" type="button" onClick={refresh}>{text('重试', 'Retry')}</button></div></div> : !listLoaded && busy ? null : emptyFirstUse ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="sessions" /></span><h3>{text('还没有会话记录', 'No sessions yet')}</h3><p>{text('在已接入的 Agent 中开始使用，经过 HiRoute 的会话会显示在这里。', 'Start in a connected Agent. Sessions routed through HiRoute will appear here.')}</p>{onOpenAgents && <button className="btn btn-primary" type="button" onClick={onOpenAgents}>{text('查看 Agent 接入', 'View Agent connections')}</button>}</div></div> : <div className={`session-layout${session ? ' has-selection' : ''}${factsOpen ? ' with-inspector' : ''}`}>
      <aside className="session-master" aria-label={text('会话列表', 'Session list')}>
        <form onSubmit={event => { event.preventDefault(); void load(); }} className="session-filters">
          <label className="search-box"><span className="sr-only">{text('搜索会话正文', 'Search session content')}</span><UiIcon name="search" /><input className="input" value={keyword} maxLength={256} onChange={event => { generation.current++; setBusy(false); setKeyword(event.target.value); }} placeholder={text('搜索会话正文', 'Search session content')} /></label>
          <div className="segmented" role="group" aria-label={text('会话筛选', 'Session filter')}><button className={`segment${!onlySwitch ? ' active' : ''}`} type="button" aria-pressed={!onlySwitch} onClick={() => { setOnlySwitch(false); setSession(null); setRequest(null); setFactsOpen(false); void load(null, { onlySwitch: false }); }}>{text('全部', 'All')}</button><button className={`segment${onlySwitch ? ' active' : ''}`} type="button" aria-pressed={onlySwitch} onClick={() => { setOnlySwitch(true); setSession(null); setRequest(null); setFactsOpen(false); void load(null, { onlySwitch: true }); }}>{text('发生过请求内回退', 'In-request fallbacks')}</button></div>
        </form>
        {listError && <div className="callout bad session-local-feedback" role="alert" data-error-code={listError}><UiIcon name="warning" /><div><strong>{text('列表没有刷新', 'The list was not refreshed')}</strong><p>{text('已加载的会话仍然保留。', 'Previously loaded sessions are still available.')}</p><button className="btn" type="button" onClick={() => void load()}>{text('重试', 'Retry')}</button></div></div>}
        <div className="session-list">
          {keyword.trim() ? <>{hits.map((hit, index) => { const context = sessions.find(item => item.session_id === hit.session_id); return <button className={`session-item${session === hit.session_id ? ' active' : ''}`} key={`${hit.content_id}-${hit.original_text_offset}-${index}`} aria-current={session === hit.session_id ? 'page' : undefined} onClick={() => void timeline(hit.session_id, null, hit)}><div className="session-item-top"><span className="muted session-item-time">{context ? sessionTimeLabel(context.last_request_at_ms, language) : text('搜索命中', 'Match')}</span></div><div className="session-item-title"><ContentExcerpt hit={hit} language={language} fallback={text('查看命中正文', 'Open matching content')} revision={refreshVersion + reload} /></div><div className="session-item-meta">{context ? <><span>{agentName(context.agent_id)}</span>{sessionRoutes[context.session_id] && <><span>·</span><span>{sessionRoutes[context.session_id]}</span></>}{sessionModels[context.session_id] && <><span>·</span><span>{sessionModels[context.session_id]}</span></>}</> : text('点击读取所在会话', 'Open the matching session')}</div></button>; })}{searchCursor && <button className="btn session-more" disabled={busy} onClick={() => void load(searchCursor)}>{text('加载更多结果', 'Load more results')}</button>}{!busy && !listError && !hits.length && <div className="empty-state session-list-empty"><div><span className="empty-icon"><UiIcon name="search" /></span><h3>{text('没有匹配的会话', 'No matching sessions')}</h3><p>{text('试试其他关键词。', 'Try another keyword.')}</p></div></div>}</> : <>{sessions.map(item => { const contentState = sessionContentStates[item.session_id]; return <button className={`session-item${session === item.session_id ? ' active' : ''}`} key={item.session_id} aria-current={session === item.session_id ? 'page' : undefined} onClick={() => void timeline(item.session_id)}><div className="session-item-top"><span className="muted session-item-time">{sessionTimeLabel(item.last_request_at_ms, language)}</span>{contentState === 'cleared' ? <span className="badge warn no-dot">{text('正文已清理', 'Content cleared')}</span> : contentState === 'partial' ? <span className="badge warn no-dot">{text('内容不完整', 'Incomplete')}</span> : item.fallback_request_count > 0 && <span className="badge warn no-dot">{text('请求内回退', 'Request fallback')}</span>}</div><div className="session-item-title"><ContentExcerpt session={item.session_id} language={language} fallback={text('会话记录', 'Session')} limit={48} revision={`${refreshVersion + reload}:${item.last_request_at_ms}:${item.request_count}`} onModel={model => setSessionModels(current => current[item.session_id] === model ? current : { ...current, [item.session_id]: model })} onRoute={route => setSessionRoutes(current => current[item.session_id] === route ? current : { ...current, [item.session_id]: route })} onContentState={state => setSessionContentStates(current => current[item.session_id] === state ? current : { ...current, [item.session_id]: state })} /></div><div className="session-item-meta"><span>{agentName(item.agent_id)}</span>{sessionRoutes[item.session_id] && <><span>·</span><span>{sessionRoutes[item.session_id]}</span></>}{sessionModels[item.session_id] && <><span>·</span><span>{sessionModels[item.session_id]}</span></>}{!sessionRoutes[item.session_id] && !sessionModels[item.session_id] && <><span>·</span><span>{item.request_count} {text('个请求', 'requests')}</span></>}</div></button>; })}{sessionCursor && <button className="btn session-more" disabled={busy} onClick={() => void load(sessionCursor)}>{text('加载更多会话', 'Load more sessions')}</button>}{!busy && !listError && listLoaded && !sessions.length && <div className="empty-state session-list-empty"><div><h3>{text('当前筛选没有会话', 'No sessions match this filter')}</h3><p>{text('切换到全部会话。', 'Show all sessions.')}</p></div></div>}</>}
        </div>
      </aside>

      <section className="session-transcript">
        {session ? <article className="session-detail">
          <div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => { timelineGeneration.current++; factGeneration.current++; setSession(null); setRequest(null); setFactsOpen(false); if (!listLoaded) void load(); }}><UiIcon name="arrowLeft" />{text('返回会话列表', 'Back to sessions')}</button></div>
          <header className="transcript-head"><div><h2><ContentExcerpt session={session} language={language} fallback={text('会话正文', 'Conversation')} limit={56} revision={refreshVersion + reload} onModel={model => setSessionModels(current => current[session] === model ? current : { ...current, [session]: model })} onRoute={route => setSessionRoutes(current => current[session] === route ? current : { ...current, [session]: route })} /></h2><p>{[agentName(selectedSummary?.agent_id), sessionRoutes[session], selectedSummary ? sessionTimeLabel(selectedSummary.last_request_at_ms, language) : undefined].filter(Boolean).join(' · ')}</p></div><div className="transcript-actions"><button className="btn" type="button" disabled={!request} onClick={() => factsOpen ? setFactsOpen(false) : openFacts()}><UiIcon name="activity" />{factsOpen ? text('收起运行记录', 'Hide run record') : text('运行记录', 'Run record')}</button>{!factsOpen && <button className="icon-btn" type="button" aria-label={text('清理此会话', 'Clear this session')} disabled={busy} onClick={() => { setCleanupError(''); setCleanupOpen(true); }}><UiIcon name="trash" /></button>}</div></header>
          {timelineError && <div className="callout bad session-detail-feedback" role="alert" data-error-code={timelineError}><UiIcon name="warning" /><div><strong>{text('会话详情没有刷新', 'The session details were not refreshed')}</strong><p>{requests.length ? text('已加载的请求仍然保留。', 'Previously loaded requests are still available.') : text('暂时无法读取这条会话。', 'This session is temporarily unavailable.')}</p><button className="btn" type="button" onClick={() => void timeline(session)}>{text('重试', 'Retry')}</button></div></div>}
          <Disclosure className="native-details session-model-performance" label={text('模型表现', 'Model performance')} language={language}><PlanQuality sessionId={session} language={language} compact onOpenEvidence={(targetSession, targetRequest) => void timeline(targetSession, null, undefined, targetRequest)} /></Disclosure>
          {requests.length > 1 && <Disclosure className="native-details request-timeline" label={text(`请求记录（${requests.length}）`, `Requests (${requests.length})`)} language={language}><nav className="native-list" aria-label={text('请求时间线', 'Request timeline')}>{requests.map((item, index) => <button className={`list-row${request === item.request_id ? ' active' : ''}`} key={item.request_id} aria-current={request === item.request_id ? 'true' : undefined} onClick={() => { setRequest(item.request_id); setReadingRequests([item.request_id]); setAnchor(null); }}><span className="row-main"><span className="row-title">{text('请求', 'Request')} {index + 1}</span><span className="row-meta">{item.routing_context?.state === 'recorded' && item.routing_context.display_name ? item.routing_context.display_name : item.final_native_model ?? text('路由未记录', 'Route not recorded')}{item.within_request_fallback === true ? text(' · 请求内回退', ' · Request fallback') : item.between_turn_model_change === true ? text(' · 跨轮切换', ' · Turn change') : ''}</span></span></button>)}{requestCursor && <button className="btn" disabled={busy} onClick={() => void timeline(session, requestCursor)}>{text('加载更多请求', 'Load more requests')}</button>}</nav></Disclosure>}
          {displayedRequestIds.length ? displayedRequestIds.map((id, index) => <div key={id} className="request-chapter">{index > 0 && <h3>{text('后续请求', 'Following request')} {requests.findIndex(item => item.request_id === id) + 1}</h3>}<RequestBody request={id} summary={requests.find(item => item.request_id === id)} attemptModels={id === request ? attemptModels : []} anchor={id === request ? anchor : null} language={language} onOpenFacts={openFacts} onContentState={reportRequestContentState} onNextRequest={index === displayedRequestIds.length - 1 && nextRequestId ? () => { setReadingRequests(current => [...current, nextRequestId]); setRequest(nextRequestId); setAnchor(null); } : undefined} onMoreRequests={index === displayedRequestIds.length - 1 && !nextRequestId && requestCursor ? () => void timeline(session, requestCursor) : undefined} /></div>) : !busy && <div className="empty-state transcript-empty"><div><p>{text('此会话尚未取得可显示的请求。', 'No displayable request was retrieved for this session.')}</p></div></div>}
        </article> : <div className="empty-state session-no-selection"><div><span className="empty-icon"><UiIcon name="sessions" /></span><h3>{text('选择一个会话', 'Select a session')}</h3></div></div>}
      </section>
      {factsOpen && <RuntimeFacts language={language} summary={selectedRequest ? { outcome: selectedRequest.outcome, finalModel: selectedRequest.final_native_model, modelFallback: selectedRequest.within_request_fallback } : undefined} facts={facts} partial={factPartial} busy={factsBusy} error={factsError} contentState={selectedContentState} usage={<ObservationValue session={session} language={language} />} next={Boolean(factCursor)} onNext={() => void loadFacts(factsError ? null : factCursor)} onClose={() => setFactsOpen(false)} onClear={() => { setCleanupError(''); setCleanupOpen(true); }} />}
    </div>}
    <Dialog open={cleanupOpen && Boolean(session)} title={text('选择清理范围', 'Choose what to clear')} description={selectedSummary ? `${agentName(selectedSummary.agent_id)} · ${sessionTimeLabel(selectedSummary.last_request_at_ms, language)}` : text('当前会话', 'Current session')} closeLabel={text('关闭清理范围', 'Close cleanup options')} closeDisabled={busy} onClose={() => { if (!busy) { setCleanupOpen(false); setCleanupError(''); } }} footer={<><button className="btn" type="button" disabled={busy} onClick={() => { setCleanupOpen(false); setCleanupError(''); }}>{text('取消', 'Cancel')}</button><button className="btn btn-danger" type="button" disabled={busy} onClick={() => void clearSession(cleanupScope)}>{busy ? text('正在读取影响范围…', 'Reading impact…') : text('继续', 'Continue')}</button></>}>
      <div className="option-panel" role="radiogroup" aria-label={text('清理范围', 'Cleanup scope')} aria-disabled={busy}>
        <label className="check-row"><input type="radio" name="session-cleanup-scope" value="content_only" disabled={busy} checked={cleanupScope === 'content_only'} onChange={() => { setCleanupScope('content_only'); setCleanupError(''); }} /><div><strong>{text('只清理正文', 'Clear content only')}</strong><span>{text('删除问题、回答和工具内容，保留运行记录。', 'Remove prompts, answers and tool content; keep the run record.')}</span></div></label>
        <label className="check-row"><input type="radio" name="session-cleanup-scope" value="facts_and_content" disabled={busy} checked={cleanupScope === 'facts_and_content'} onChange={() => { setCleanupScope('facts_and_content'); setCleanupError(''); }} /><div><strong>{text('删除这条会话记录', 'Delete this session record')}</strong><span>{text('同时删除此会话的逐请求记录。', 'Also remove this session’s per-request records.')}</span></div></label>
      </div>
      <p className="oc-meta">{text('下一步会显示具体影响范围并请你确认。其他会话与长期金额汇总保留。', 'The next step shows the exact impact for confirmation. Other sessions and long-term amount summaries are preserved.')}</p>
      {cleanupError && <div className="callout bad" role="alert" data-error-code={cleanupError}><UiIcon name="warning" /><span>{text('暂时无法确认清理范围，当前会话没有改变。请重试。', 'The cleanup scope could not be confirmed. The session is unchanged; try again.')}</span></div>}
    </Dialog>
  </ProductPage>;
}
function RequestBody({ request, summary, attemptModels, anchor, language, onOpenFacts, onContentState, onNextRequest, onMoreRequests }: { request: string; summary?: Request; attemptModels: string[]; anchor: Hit | null; language: 'zh' | 'en'; onOpenFacts(): void; onContentState(requestId: string, state: ContentState): void; onNextRequest?: () => void; onMoreRequests?: () => void }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [catalog, setCatalog] = useState<Content[]>([]);
  const [events, setEvents] = useState<Record<string, ReadableEvent>>({});
  const receiveEvent = useCallback((id: string, event: ReadableEvent) => setEvents(current => JSON.stringify(current[id]) === JSON.stringify(event) ? current : { ...current, [id]: event }), []);

  const [rootsPartial, setRootsPartial] = useState(false);
  const [ancestryIncomplete, setAncestryIncomplete] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [catalogLoaded, setCatalogLoaded] = useState(false);
  const generation = useRef(0);
  const controller = useRef(new AbortController());
  async function load(next: string | null = null) {
    const epoch = ++generation.current; setBusy(true); setError('');
    if (!next) setCatalogLoaded(false);
    try {
      const page = await read<Page<{ contents: Content[]; transcript_roots: string[]; roots_partial: boolean }>>('catalog', { request_id: request, limit: 20, cursor: next }, controller.current.signal);
      if (epoch !== generation.current) return;
      if (!next) setEvents({});
      setCatalog(current => next ? [...current, ...page.contents] : page.contents);
      setCursor(page.next_cursor);
      setRootsPartial(page.roots_partial);
      setCatalogLoaded(true);
      if (!next) {
        setAncestryIncomplete(false);
        void checkAncestry(page.transcript_roots, page.roots_partial, epoch);
      }
    } catch (e) { if (epoch === generation.current) setError(errorCode(e)); }
    finally { if (epoch === generation.current) setBusy(false); }
  }
  useEffect(() => {
    controller.current = new AbortController(); void load();
    return () => { generation.current++; controller.current.abort(); };
  }, [request]);
  async function checkAncestry(roots: string[], partial: boolean, epoch: number) {
    if (partial) setAncestryIncomplete(true);
    if (!roots.length) return;
    const results = await Promise.allSettled(roots.map(transcript_root => read<Ancestry>('ancestry', { request_id: request, transcript_root }, controller.current.signal)));
    if (epoch !== generation.current || controller.current.signal.aborted) return;
    setAncestryIncomplete(partial || results.some(result => result.status === 'rejected' || Boolean(result.value.gap)));
  }
  const contentIncomplete = catalogLoaded && (rootsPartial || ancestryIncomplete || catalog.some(item => !['available', 'complete'].includes(item.state)));
  const contentState: ContentState = catalog.length > 0 && !cursor && catalog.every(item => item.state === 'deleted')
    ? 'cleared'
    : !catalog.length || contentIncomplete ? 'partial' : 'recorded';
  useEffect(() => {
    if (catalogLoaded || error) onContentState(request, contentState);
  }, [request, contentState, catalogLoaded, error, onContentState]);
  const occurrences = groupOccurrences(catalog.filter(item => !isPrivateContentKind(item.kind)));
  const displayRuns = contentDisplayRuns(occurrences);
  const assistantIndexes = occurrences.map((occurrence, index) => occurrence.role === 'assistant' && !occurrence.technical ? index : -1).filter(index => index >= 0);
  const firstAssistantOccurrenceIndex = occurrences.findIndex(occurrence => occurrence.role === 'assistant');
  const firstAssistantIndex = assistantIndexes[0] ?? -1;
  const finalAssistantIndex = assistantIndexes.at(-1) ?? -1;
  const toolEvents = groupToolEvents(Object.values(events));
  const renderOccurrence = (occurrence: (typeof occurrences)[number], index: number) => {
    const items = occurrence.items.filter(item => item.content_id !== anchor?.content_id);
    if (!items.length) return null;
    return <React.Fragment key={occurrence.key}>
      {index === finalAssistantIndex && summary?.within_request_fallback === true && <button className="switch-marker" type="button" onClick={onOpenFacts}><UiIcon name="refresh" /><strong>{attemptModels.length > 1 ? attemptModels.join(' → ') : text('记录到请求内模型回退', 'Recorded model fallback within a request')}</strong><span>{attemptModels.length > 1 ? text('记录到请求内模型回退', 'Recorded model fallback within a request') : text('查看运行记录', 'View run record')}</span><UiIcon name="chevronRight" /></button>}
      <MessageOccurrence language={language} request={request} items={items} technical={occurrence.technical} modelLabel={index === finalAssistantIndex ? summary?.final_native_model ?? attemptModels.at(-1) : index === firstAssistantIndex && attemptModels.length > 1 ? attemptModels[0] : null} onEvent={receiveEvent} />
    </React.Fragment>;
  };
  const toolBlocks = toolEvents.map(event => <div className="tool-block" key={event.logicalId}><strong>{event.name || text('工具调用', 'Tool call')}</strong><span>{event.arguments ? summarizeToolArguments(event.arguments) : event.phase === 'ready' ? text('参数已就绪，执行结果尚未取得。', 'Arguments ready; execution result not received.') : text('已请求，执行结果尚未取得。', 'Requested; execution result not received.')}</span></div>);
  return <section className="transcript-body">{error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('正文暂时无法读取，请重试。', 'Content is temporarily unavailable. Try again.')}</span><button className="btn" type="button" disabled={busy} onClick={() => void load(null)}>{text('重试', 'Retry')}</button></div>}
    {contentIncomplete && <div className="callout warn completeness-banner" role="status"><UiIcon name="warning" /><div><strong>{text('本次会话内容不完整', 'This session is incomplete')}</strong><p>{text('只展示 Gateway 已收到并交付的内容；运行记录仍然可用。', 'Only content received and delivered by the Gateway is shown. Run records remain available.')}</p></div></div>}
    {anchor && catalog.some(item => item.content_id === anchor.content_id && !isPrivateContentKind(item.kind)) && <MessageOccurrence language={language} request={request} items={[{ content_id: anchor.content_id, role: text('搜索命中', 'Search hit'), kind: 'text', state: 'complete', direction: '', media_type: '', message_occurrence_id: '' }]} anchor={{ contentId: anchor.content_id, offset: anchor.original_text_offset }} />}
    {displayRuns.map(run => <React.Fragment key={run.start}>{run.technical
      ? <Disclosure className="transcript-context" label={text(`系统与工具上下文（${run.end - run.start} 项）`, `System and tool context (${run.end - run.start})`)} language={language}>{occurrences.slice(run.start, run.end).map((occurrence, index) => renderOccurrence(occurrence, index + run.start))}</Disclosure>
      : occurrences.slice(run.start, run.end).map((occurrence, index) => <React.Fragment key={occurrence.key}>{renderOccurrence(occurrence, index + run.start)}{index + run.start === firstAssistantOccurrenceIndex && toolBlocks}</React.Fragment>)}
      {run.technical && firstAssistantOccurrenceIndex >= run.start && firstAssistantOccurrenceIndex < run.end && toolBlocks}
    </React.Fragment>)}
    {cursor && <button className="btn" disabled={busy} onClick={() => void load(cursor)}>{text('继续读取', 'Load more')}</button>}
    {!cursor && catalogLoaded && !busy && !error && onNextRequest && <button className="btn" type="button" onClick={onNextRequest}>{text('继续读取下一请求', 'Continue to next request')}</button>}
    {!cursor && catalogLoaded && !busy && !error && onMoreRequests && <button className="btn" type="button" onClick={onMoreRequests}>{text('加载更多请求', 'Load more requests')}</button>}
    {!catalogLoaded && busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取正文…', 'Reading content…')}</p></div>}
    {catalogLoaded && !busy && !error && !catalog.length && <div className="callout"><UiIcon name="lock" /><span>{text('正文尚未取得、未捕获或已清理。', 'Content is unavailable, was not captured, or has been cleared.')}</span></div>}
    <div className="callout transcript-privacy"><UiIcon name="lock" /><span>{text('这里仅展示 Gateway 可见的本机会话内容，不包含 Agent 未发送的本地执行。', 'This view contains only local, Gateway-visible content and excludes agent activity that was never sent.')}</span></div>

  </section>;
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

function MessageOccurrence({ request, items, anchor, language, modelLabel, technical = false, onEvent }: { onEvent?: (id: string, event: ReadableEvent) => void; request: string; items: Content[]; anchor?: { contentId: string; offset: number }; language: 'zh' | 'en'; modelLabel?: string | null; technical?: boolean }) {
  const [visibility, setVisibility] = useState<Record<string, boolean>>({});
  const [partIncomplete, setPartIncomplete] = useState<Record<string, boolean>>({});
  const [contextParts, setContextParts] = useState<Record<string, boolean>>({});
  const [events, setEvents] = useState<Record<string, ReadableEvent>>({});
  const reportVisibility = useCallback((id: string, visible: boolean) => setVisibility(current => current[id] === visible ? current : { ...current, [id]: visible }), []);
  const reportIncomplete = useCallback((id: string, incomplete: boolean) => setPartIncomplete(current => current[id] === incomplete ? current : { ...current, [id]: incomplete }), []);
  const reportContext = useCallback((id: string, contextOnly: boolean) => setContextParts(current => current[id] === contextOnly ? current : { ...current, [id]: contextOnly }), []);
  const reportEvent = useCallback((id: string, event: ReadableEvent) => {
    setEvents(current => JSON.stringify(current[id]) === JSON.stringify(event) ? current : { ...current, [id]: event });
    if (event.kind === 'tool') onEvent?.(id, event);
  }, [onEvent]);
  const role = items[0]?.role ?? '';
  const contextOnly = role === 'user' && !anchor && items.length > 0 && items.every(item => contextParts[item.content_id] === true);
  const roleLabel = technical ? (language === 'zh' ? '工具上下文' : 'Tool context') : contextOnly ? (language === 'zh' ? 'Agent 附加上下文' : 'Agent-added context') : role === 'assistant' ? (language === 'zh' ? 'Agent 回答' : 'Agent answer') : role === 'user' ? (language === 'zh' ? '你的问题' : 'Your question') : role;
  const potential = items.some(item => !item.media_type.includes('hiroute.model-stream-event'));
  const responseText = coalesceResponseText(Object.values(events));
  const visible = potential || responseText.some(part => part.text.length > 0) || Object.values(visibility).some(Boolean);
  const incomplete = items.some(item => !['available', 'complete'].includes(item.state)) || Object.values(partIncomplete).some(Boolean);
  return <article hidden={!visible} aria-hidden={!visible} style={visible ? undefined : { display: 'none' }} className={`message ${!technical && role === 'assistant' ? 'assistant' : !technical && role === 'user' && !contextOnly ? 'user' : 'system'}`}><span className="message-avatar">{technical ? '·' : role === 'assistant' ? 'AI' : role === 'user' && !contextOnly ? 'YOU' : '·'}</span><div><div className="message-role">{roleLabel}{modelLabel && <span className="badge info no-dot">{modelLabel}</span>}{incomplete && <span className="badge warn no-dot">{language === 'zh' ? '内容不完整' : 'Incomplete'}</span>}</div>{items.map(item => <MessagePart key={item.content_id} request={request} item={item} anchor={anchor?.contentId === item.content_id ? anchor.offset : undefined} language={language} onEvent={reportEvent} onVisibility={reportVisibility} onIncomplete={reportIncomplete} onContext={reportContext} />)}{responseText.map(part => <div className="message-content" key={`${part.kind}-${part.index}`}>{part.text}</div>)}</div></article>;
}

function MessagePart({ request, item, anchor, language, onEvent, onVisibility, onIncomplete, onContext }: { onEvent?: (id: string, event: ReadableEvent) => void; onVisibility(id: string, visible: boolean): void; onIncomplete(id: string, incomplete: boolean): void; onContext(id: string, contextOnly: boolean): void; request: string; item: Content; anchor?: number; language: 'zh' | 'en' }) {
  const [offset, setOffset] = useState(0); const marker = useRef<HTMLElement>(null);
  const [text, setText] = useState(''); const [cursor, setCursor] = useState<string | null>(null);
  const [state, setState] = useState(item.state); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const generation = useRef(0); const controller = useRef(new AbortController());
  async function load(next: string | null = null) {
    const epoch = ++generation.current; setBusy(true); setError('');
    try {
      const page = await read<Page<{ state: string; chunks: { text: string; original_byte_offset: number }[] }>>('content', { request_id: request, content_id: item.content_id, cursor: next, anchor_offset: anchor ?? null }, controller.current.signal);
      if (epoch !== generation.current) return;
      const chunkText = page.chunks.map(c => c.text).join('');
      if (!next) setOffset(page.chunks[0]?.original_byte_offset ?? 0);
      setText(current => next ? current + chunkText : chunkText);
      setCursor(page.next_cursor); setState(page.state);
    } catch (e) { if (epoch === generation.current) setError(errorCode(e)); }
    finally { if (epoch === generation.current) setBusy(false); }
  }
  useEffect(() => { controller.current = new AbortController(); void load(); return () => { generation.current++; controller.current.abort(); }; }, [request, item.content_id, anchor]);
  useEffect(() => { marker.current?.scrollIntoView({ block: 'center' }); }, [text]);
  const event = readableEvent(text, item.media_type, item.direction);
  useEffect(() => { if (event && event.kind !== 'reasoning' && onEvent) onEvent(item.content_id, event); }, [text, item.content_id, onEvent]);
  const structuredPending = item.media_type.includes('hiroute.model-stream-event') && !text && !error;
  const hiddenEvent = Boolean((structuredPending || (event && (event.kind === 'block' || event.kind === 'reasoning' || ((event.kind === 'tool' || event.kind === 'text' || event.kind === 'refusal') && onEvent)))) && anchor === undefined && !error && !cursor);
  useEffect(() => { onVisibility(item.content_id, !hiddenEvent); }, [hiddenEvent, item.content_id, onVisibility]);
  useEffect(() => { onIncomplete(item.content_id, state !== 'available' && state !== 'complete'); }, [item.content_id, onIncomplete, state]);
  const bytes = new TextEncoder().encode(text);
  const position = anchor === undefined ? -1 : anchor - offset;
  const marked = position >= 0 && position < bytes.length;
  const context = item.role === 'user' && anchor === undefined && !event && !cursor && !error ? splitLeadingAgentContext(text) : { context: '', body: text };
  useEffect(() => { onContext(item.content_id, Boolean(context.context && !context.body.trim())); }, [item.content_id, context.context, context.body, onContext]);
  if (hiddenEvent) return null;
  let visible = marked ? <>{new TextDecoder().decode(bytes.slice(0, position))}<mark ref={marker} aria-label={language === 'zh' ? '搜索命中位置' : 'Search hit position'}>↦</mark>{new TextDecoder().decode(bytes.slice(position))}</> : event && (event.kind === 'text' || event.kind === 'reasoning' || event.kind === 'refusal') ? event.text : event?.kind === 'tool' ? `${event.name || (language === 'zh' ? '工具参数' : 'Tool arguments')} · ${event.phase === 'started' ? (language === 'zh' ? '已请求，结果尚未取得' : 'Requested; result not received') : event.phase === 'ready' ? (language === 'zh' ? '调用参数已就绪，执行结果尚未取得' : 'Call arguments ready; execution result not received') : (language === 'zh' ? '参数片段' : 'Argument fragment')}` : event?.kind === 'block' ? (language === 'zh' ? '内容开始' : 'Content started') : text;
  if (item.role === 'user' && !marked && !event && !cursor) {
    if (context.context) visible = <><Disclosure className="message-context" label={language === 'zh' ? 'Agent 附加上下文' : 'Agent-added context'} language={language}>{context.context}</Disclosure>{context.body}</>;
  }
  return <>{busy && <p className="oc-meta" role="status">{language === 'zh' ? '正在读取正文…' : 'Loading content…'}</p>}{error && <div className="callout bad" role="alert" data-error-code={error}><span>{text ? (language === 'zh' ? '更多正文暂时无法读取，已加载内容仍保留。' : 'More content is temporarily unavailable. Loaded content is retained.') : (language === 'zh' ? '这段正文暂时无法读取。' : 'This content is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => void load(text ? cursor : null)}>{language === 'zh' ? '重试' : 'Retry'}</button></div>}{!event && text && item.media_type.includes('hiroute.model-stream-event') && <p className="oc-meta">{language === 'zh' ? '暂不能结构化显示，以下为已取得的原文。' : 'Structured display is unavailable; captured content follows.'}</p>}{marked && <p className="oc-meta">{language === 'zh' ? '已定位到搜索命中位置。' : 'Located the search match.'}</p>}{text && <div className="message-content">{visible}</div>}{!error && cursor && <button className="btn btn-quiet" disabled={busy} onClick={() => void load(cursor)}>{language === 'zh' ? '继续读取' : 'Load more'}</button>}</>;
}

export function ObservationValue({ session = null, language = 'zh' }: { session?: string | null; language?: 'zh' | 'en' }) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  type Amount = { known_sum_micros: number | null; currency: string; valuation_kind: string; coverage: string };
type Value = { pending_requests: number; provisional_requests: number; unknown_traffic_requests: number; excluded_requests: number; amounts: Amount[]; usage: UsageMetric[]; input_cache_hit: CacheHitSummary; archive_boundary_partial: boolean; retention_boundary_partial: boolean };
  const [value, setValue] = useState<Value | null>(null); const [error, setError] = useState(''); const [revision, setRevision] = useState(0);
  useEffect(() => { let alive = true; setValue(null); setError(''); void read<Value>('home_value', { period: session ? 'seven_days' : 'today', session_id: session, currency: null }).then(result => { if (alive) setValue(result); }).catch(e => { if (alive) setError(errorCode(e)); }); return () => { alive = false; }; }, [session, revision]);
  const coverage = (value?.retention_boundary_partial || value?.usage.some(metric => metric.coverage !== 'complete') || value?.amounts.some(amount => amount.coverage !== 'complete') || value?.input_cache_hit.coverage !== 'complete')
    ? text('部分已记录', 'Partially recorded') : text('已记录', 'Recorded');
  if (error) return <section className="fact-section observation-value"><h4>{text('用量与费用', 'Usage and cost')}</h4><div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('用量暂时无法读取。', 'Usage is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => setRevision(current => current + 1)}>{text('重试', 'Retry')}</button></div></section>;
  if (!value) return <section className="fact-section observation-value"><h4>{text('用量与费用', 'Usage and cost')}</h4><div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取用量…', 'Reading usage…')}</p></div></section>;
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
  const [error, setError] = useState('');
  useEffect(() => { let alive = true; void read<{ running: boolean; index_running: boolean; error_count: number }>('status', {}).then(value => { if (alive) setStatus(value); }).catch(e => { if (alive) setError(errorCode(e)); }); return () => { alive = false; }; }, []);
  if (error) return null;
  if (!status) return null;
  return !status.running || !status.index_running || status.error_count > 0 ? <div className="callout warn session-page-feedback" role="status"><UiIcon name="clock" /><span>{language === 'zh' ? '部分会话仍在整理，搜索或清理结果可能稍后更新。' : 'Some sessions are still being prepared. Search or cleanup results may update shortly.'}</span></div> : null;
}

type TitleScanState = { sawContent: boolean; allDeleted: boolean; incomplete: boolean };
const TITLE_CONTENT_LIMIT = 1024 * 1024;

async function completeTitlePart(requestId: string, contentId: string, signal: AbortSignal): Promise<{ text: string | null; incomplete: boolean }> {
  let cursor: string | null = null;
  let text = '';
  let byteCount = 0;
  const seen = new Set<string>();
  do {
    const page: { state: string; chunks: { text: string }[]; next_cursor: string | null } = await read('content', { request_id: requestId, content_id: contentId, cursor, anchor_offset: null }, signal);
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

async function firstRequestTitle(requestId: string, signal: AbortSignal, state: TitleScanState): Promise<string | null> {
  let cursor: string | null = null;
  const contents: Content[] = [];
  const seen = new Set<string>();
  do {
    const page: { contents: Content[]; roots_partial?: boolean; next_cursor?: string | null } = await read('catalog', { request_id: requestId, limit: 200, cursor }, signal);
    contents.push(...page.contents);
    state.sawContent ||= page.contents.length > 0;
    state.allDeleted &&= page.contents.every(item => item.state === 'deleted');
    state.incomplete ||= Boolean(page.roots_partial) || page.contents.some(item => !['available', 'complete', 'deleted'].includes(item.state));
    cursor = page.next_cursor ?? null;
    if (cursor && seen.has(cursor)) { state.incomplete = true; break; }
    if (cursor) seen.add(cursor);
  } while (cursor && !signal.aborted);

  const groups = new Map<string, Content[]>();
  for (const content of contents) {
    if (content.role !== 'user' || content.kind !== 'text' || ['deleted', 'expired', 'unavailable'].includes(content.state)) continue;
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
      const part = await completeTitlePart(requestId, content.content_id, signal);
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

export function ContentExcerpt({ session, hit, language, fallback = '', limit = 160, revision = 0, onCompleteness, onContentState, onModel, onRoute }: { session?: string; hit?: Hit; language: 'zh' | 'en'; fallback?: string; limit?: number; revision?: number | string; onCompleteness?: (incomplete: boolean) => void; onContentState?: (state: ContentState) => void; onModel?: (model: string) => void; onRoute?: (route: string) => void }) {
  const node = useRef<HTMLSpanElement>(null);
  const [value, setValue] = useState('');
  const [invalidation, setInvalidation] = useState(0);
  useEffect(() => { const invalidate = () => { setValue(''); setInvalidation(v => v + 1); }; window.addEventListener('hiroute-content-invalidated', invalidate); return () => window.removeEventListener('hiroute-content-invalidated', invalidate); }, []);
  useEffect(() => {
    const controller = new AbortController(); let started = false;
    setValue('');
    async function load() {
      try {
        if (hit) {
          const page = await read<{ state?: string; chunks: { text: string; original_byte_offset: number }[] }>('content', { request_id: hit.request_id, content_id: hit.content_id, cursor: null, anchor_offset: hit.original_text_offset }, controller.signal);
          if (controller.signal.aborted) return;
          setValue(excerptAround(page.chunks.map(c => c.text).join(''), Math.max(0, hit.original_text_offset - (page.chunks[0]?.original_byte_offset ?? 0)), limit));
          if (typeof page.state === 'string') {
            const incomplete = !['available', 'complete'].includes(page.state);
            onCompleteness?.(incomplete);
            if (page.state === 'deleted') onContentState?.('cleared');
            else if (incomplete) onContentState?.('partial');
          }
          return;
        }
        if (!session) return;
        const state: TitleScanState = { sawContent: false, allDeleted: true, incomplete: false };
        let cursor: string | null = null;
        const toMs = Date.now() + 1;
        let firstRequest = true;
        const seen = new Set<string>();
        do {
          const timeline: Page<{ requests: Request[] }> = await read('timeline', { session_id: session, limit: 50, cursor, from_ms: 0, to_ms: toMs, agent_id: null, plan_id: null, native_model: null, outcome: null, only_model_switch: false }, controller.signal);
          for (const request of timeline.requests) {
            if (firstRequest) {
              firstRequest = false;
              if (request.final_native_model) onModel?.(request.final_native_model);
              const routing = request.routing_context;
              if (routing?.state === 'recorded' && routing.name_state === 'recorded' && routing.display_name) onRoute?.(routing.display_name);
            }
            const title = await firstRequestTitle(request.request_id, controller.signal, state);
            if (title !== null) {
              if (!controller.signal.aborted) setValue(excerpt(title, limit));
              onCompleteness?.(state.incomplete);
              onContentState?.(state.incomplete ? 'partial' : 'recorded');
              return;
            }
          }
          cursor = timeline.next_cursor;
          if (cursor && seen.has(cursor)) { state.incomplete = true; break; }
          if (cursor) seen.add(cursor);
        } while (cursor && !controller.signal.aborted);
        if (!controller.signal.aborted) {
          onCompleteness?.(state.incomplete);
          onContentState?.(state.sawContent && state.allDeleted ? 'cleared' : !state.sawContent || state.incomplete ? 'partial' : 'recorded');
        }
      } catch {
        // Keep the semantic row visible, while making the missing content
        // explicit instead of silently leaving it indistinguishable from a
        // successfully recorded fallback title.
        if (!controller.signal.aborted) {
          onCompleteness?.(true);
          onContentState?.('partial');
        }
      }
    }
    const observer = new IntersectionObserver(entries => { if (!started && entries.some(e => e.isIntersecting)) { started = true; void load(); } });
    if (node.current) observer.observe(node.current);
    return () => { controller.abort(); observer.disconnect(); };
  }, [session, hit?.request_id, hit?.content_id, hit?.original_text_offset, invalidation, limit, revision]);
  return <span ref={node}>{value || fallback || (hit ? (language === 'zh' ? '查看命中正文' : 'Open matching content') : '')}</span>;
}
