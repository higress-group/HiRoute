import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { Sessions } from '../../../src/features/Sessions';
import { Home, HomeNavigation } from '../../../src/features/home';
import type { HomeReads } from '../../../src/features/home/types';
import { projectHomeActivity } from '../../../src/product/home-projections';
import { PresentationRoot } from '../../../src/ui';
import '../../../src/occami/styles.css';

type Summary = { session_id: string; agent_id: string; request_count: number; fallback_request_count: number; first_request_at_ms: number; last_request_at_ms: number; unknown_model_request_count: number; correlation_kind: string };
type RoutingContext = { state: 'recorded'; display_name: string; name_state: 'recorded'; plan_id: string; plan_revision: string };
type RequestRow = { request_id: string; session_id: string; started_at_ms: number; outcome: string | null; attempted_model_count: number; final_native_model: string | null; within_request_fallback: boolean | null; between_turn_model_change: boolean | null; routing_context: RoutingContext | null };
type Hit = { session_id: string; request_id: string; content_id: string; original_text_offset: number };
type Handler = (payload: Record<string, any>) => unknown;

const digestOf = (view: string, query: Record<string, any>) => [view, query.from_ms, query.to_ms, query.session_id ?? '', query.keyword ?? '', query.limit, query.only_model_switch ? 1 : 0, query.agent_id ?? '', query.plan_id ?? '', query.native_model ?? '', query.outcome ?? ''].join('|');
const inWindow = (startedAt: number, query: Record<string, any>) => startedAt >= query.from_ms && startedAt < query.to_ms;

const control = {
  requests: {} as Record<string, RequestRow[]>,
  catalog: {} as Record<string, any[]>,
  texts: {} as Record<string, string>,
  hits: [] as Hit[],
  views: {} as Record<string, (query: Record<string, any>) => unknown>,
  handlers: {} as Record<string, Handler>,
  commands: [] as { command: string; payload: Record<string, any> }[],
  defaultRead: (view: string, query: Record<string, any>): unknown => observationRead(view, query),
  makeRequest: (sessionId: string, index: number, at: number, overrides: Partial<RequestRow> = {}): RequestRow => ({
    request_id: `request/${sessionId.split('/').at(-1)}/${index}`,
    session_id: sessionId,
    started_at_ms: at,
    outcome: null,
    attempted_model_count: 1,
    final_native_model: null,
    within_request_fallback: null,
    between_turn_model_change: null,
    routing_context: { state: 'recorded', display_name: '日常编码', name_state: 'recorded', plan_id: 'plan/fixture', plan_revision: '1' },
    ...overrides,
  }),
  bumpRefresh: () => {},
  showHome: () => {},
  showSessions: () => {},
  reset: () => {},
};

// Mirrors the daemon: a session is visible only through requests inside the query window,
// and its aggregates are computed over those in-window requests alone.
function sessionSummaries(query: Record<string, any>): Summary[] {
  return Object.keys(control.requests).flatMap(sessionId => {
    const rows = control.requests[sessionId].filter(row => inWindow(row.started_at_ms, query));
    if (!rows.length || (query.session_id && query.session_id !== sessionId)) return [];
    const fallback = rows.filter(row => row.within_request_fallback === true).length;
    if (query.only_model_switch && !fallback) return [];
    return [{
      session_id: sessionId,
      agent_id: '',
      request_count: rows.length,
      fallback_request_count: fallback,
      first_request_at_ms: Math.min(...rows.map(row => row.started_at_ms)),
      last_request_at_ms: Math.max(...rows.map(row => row.started_at_ms)),
      unknown_model_request_count: rows.filter(row => row.attempted_model_count === 0).length,
      correlation_kind: 'agent_supplied',
    }];
  }).sort((left, right) => right.last_request_at_ms - left.last_request_at_ms);
}
// The daemon binds keyset cursors to the normalized query (including from_ms/to_ms), so a
// cursor presented with a different window is refused as stale.
function paginate<T>(items: T[], query: Record<string, any>, digest: string): { page: T[]; next_cursor: string | null } {
  let offset = 0;
  if (query.cursor) {
    const token = JSON.parse(String(query.cursor)) as { d: string; o: number };
    if (token.d !== digest) throw { code: 'CHANGE_PREVIEW_STALE' };
    offset = token.o;
  }
  const limit = Number(query.limit ?? 50);
  return { page: items.slice(offset, offset + limit), next_cursor: offset + limit < items.length ? JSON.stringify({ d: digest, o: offset + limit }) : null };
}
function observationRead(view: string, query: Record<string, any>) {
  switch (view) {
    case 'sessions': {
      const digest = digestOf('sessions', query);
      const { page, next_cursor } = paginate(sessionSummaries(query), query, digest);
      return { next_cursor, sessions: page };
    }
    case 'search': {
      const digest = digestOf('search', query);
      const keyword = String(query.keyword ?? '');
      const visible = new Set(sessionSummaries({ ...query, only_model_switch: false, session_id: null }).map(row => row.session_id));
      const matched = control.hits.filter(hit => visible.has(hit.session_id) && (control.texts[hit.content_id] ?? '').includes(keyword));
      const { page, next_cursor } = paginate(matched, query, digest);
      return { hits: page, next_cursor, index_partial: false, budget_exhausted: false };
    }
    case 'timeline': {
      const digest = digestOf('timeline', query);
      const rows = (control.requests[query.session_id] ?? []).filter(row => inWindow(row.started_at_ms, query)).sort((left, right) => left.started_at_ms - right.started_at_ms);
      const { page, next_cursor } = paginate(rows, query, digest);
      return { next_cursor, requests: page };
    }
    case 'catalog': return { contents: control.catalog[query.request_id] ?? [], transcript_roots: [], roots_partial: false, next_cursor: null };
    case 'content': return { state: 'complete', chunks: [{ text: control.texts[query.content_id] ?? '', original_byte_offset: 0 }], next_cursor: null };
    case 'facts': return { facts: [], projection_partial: false, next_cursor: null };
    case 'plan_quality': return { samples: [], next_cursor: null };
    case 'home_value': return {
      pending_requests: 0,
      provisional_requests: 0,
      unknown_traffic_requests: 0,
      excluded_requests: 1,
      amounts: [],
      usage: [
        { metric: 'input', known_sum: 1_100, coverage: 'complete' },
        { metric: 'output', known_sum: 75, coverage: 'complete' },
        { metric: 'cache_read', known_sum: 190, coverage: 'complete' },
        { metric: 'cache_write', known_sum: 0, coverage: 'complete' },
      ],
      input_cache_hit: { state: 'available', ratio_basis_points: 1727, cache_read_tokens: 190, total_input_tokens: 1_100, eligible_attempt_count: 2, total_attempt_count: 2, zero_input_attempt_count: 0, missing_attempt_count: 0, invalid_attempt_count: 0, arithmetic_overflow: false, archive_coverage_partial: false, coverage: 'complete' },
      archive_boundary_partial: false,
      retention_boundary_partial: false,
    };
    case 'ancestry': return { roots: [], gap: null };
    case 'status': return { running: true, index_running: true, error_count: 0 };
    default: throw new Error(`Unexpected session-window view: ${view}`);
  }
}

Object.assign(window, { sessionWindow: control });

mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, any>;
  control.commands.push({ command, payload: args });
  if (control.handlers[command]) return control.handlers[command](args);
  if (command === 'observation_read') {
    const intent = args.request?.intent ?? {};
    const view = String(intent.view ?? '');
    const query = (intent.query ?? {}) as Record<string, any>;
    if (control.views[view]) return control.views[view](query);
    return observationRead(view, query);
  }
  throw new Error(`Unexpected session-window command: ${command}`);
});

const navItems = [
  { id: 'home', label: '首页', icon: 'home' },
  { id: 'models', label: '模型', icon: 'models' },
  { id: 'routing', label: '智能路由', icon: 'route' },
  { id: 'agents', label: 'Agent', icon: 'agent' },
  { id: 'sessions', label: '会话', icon: 'sessions' },
] as const;

function Harness() {
  const [generation, setGeneration] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const [homeVisible, setHomeVisible] = useState(false);
  control.bumpRefresh = () => setRefresh(value => value + 1);
  control.showHome = () => setHomeVisible(true);
  control.showSessions = () => setHomeVisible(false);
  control.reset = () => {
    control.requests = {};
    control.catalog = {};
    control.texts = {};
    control.hits = [];
    control.views = {};
    control.handlers = {};
    control.commands = [];
    setHomeVisible(false);
    setRefresh(0);
    setGeneration(value => value + 1);
  };
  const homeActivity = projectHomeActivity({
    sessions: Object.entries(control.requests).map(([sessionId, requests]) => ({
      session_id: sessionId, agent_id: '', first_request_at_ms: requests[0]?.started_at_ms ?? 0,
      last_request_at_ms: requests.at(-1)?.started_at_ms ?? 0, request_count: requests.length,
      fallback_request_count: 0, unknown_model_request_count: 0, correlation_kind: 'agent_supplied',
    })),
    next_cursor: null,
  });
  const homeReads: HomeReads = {
    service: { status: 'ready', data: { daemon: 'running', gateway: 'ready', recoveryReady: true } },
    compute: { status: 'ready', data: { candidates: [], sources: [], saveAttempts: [], subscriptionChecks: [] } },
    plans: { status: 'ready', data: { plans: [], drafts: [] } },
    agents: { status: 'ready', data: { agents: [] } },
    activity: { status: 'ready', data: homeActivity },
    value: { status: 'loading' },
  };
  return <PresentationRoot language="zh" theme="dark" textScale={1}>
    <div className="app-window">
      <header className="titlebar"><div className="window-controls" /><div className="titlebar-main"><div className="window-title"><span>HiRoute</span><span>/</span><span>会话</span></div></div></header>
      <HomeNavigation language="zh" items={[...navItems]} current="sessions" serviceLabel="仅在本机运行" serviceReady onNavigate={() => {}} onOpenSettings={() => {}} />
      <main className="main"><div>
        <p className="sr-only">组件测试：所有 IPC 为 mock；不连接 Tauri、daemon 或真实数据。</p>
        {homeVisible && <Home language="zh" reads={homeReads} onAction={() => {}} />}
        <div hidden={homeVisible}><Sessions key={generation} active={!homeVisible} language="zh" refreshVersion={refresh} onOpenAgents={() => {}} /></div>
      </div></main>
    </div>
  </PresentationRoot>;
}

createRoot(document.getElementById('root')!).render(<Harness />);
