import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { formatMoney } from '../ui';
import { safeDiagnosticCode } from '../error-code';
import { observationBusyRetryMs } from './observation-failure';
import { observationRead } from './observation-client';
import { formatTokenCount } from './usage-presentation';
import { ContentExcerpt } from './Sessions';
import { selectionReasonLabel } from './selection-reasons';
import './routing-timeline.css';

type Cost = { model_configuration_id: string | null; currency: string | null; valuation_kind: string | null; known_micros: number | null; unknown_contributions: number };
export type TimelineRequest = { request_id: string; started_at_ms: number; duration_ms: number | null; native_model: string | null; input_tokens: number | null; output_tokens: number | null; cache_read_tokens: number | null; cache_read_share: number | null; costs: Cost[]; outcome: string | null };
export type TimelineProtection = {
  outcome: 'applied' | 'suppressed' | 'retained_unavailable' | 'not_needed' | 'not_considered';
  previous_score: number | null; competence_floor_millis: number;
  applicability_probability: number | null; applicability_boundary_millis: number;
};
export type TimelineTurn = {
  turn_id: string; agent_turn_id: string | null; segment_id: string | null; started_at_ms: number; finished_at_ms: number | null;
  boundary: 'new_user_message' | 'context_rebuilt' | 'unknown'; plan_id: string | null; plan_revision: number | null;
  branch_id: string | null; model_configuration_id: string | null; reasoning_profile_id: string | null; group: string | null;
  selection: { execution_group?: 'regular' | 'primary'; extension_reason?: string; simple_probability: number | null; simple_threshold_millis: number | null; selection_reason: string; competence_trigger?: { score: number; floor_millis: number } | null; historical_protection?: TimelineProtection | null } | null;
  tool_continuations: number; relays: { request_id: string; from_model: string | null; to_model: string | null }[];
  assessment_state: string; assessment: { score: number; partial: boolean; target_from_turn_id?: string; target_through_turn_id?: string; target_from_ordinal: number; target_through_ordinal: number } | null;
  competence_floor_millis: number | null; protected_turn_ids: string[]; status: string; requests: TimelineRequest[];
  transitions?: { kind: string; request_id: string; previous_request_id?: string | null; model_changed?: boolean | null; reasoning_changed?: boolean | null; model: { native_model: string | null }; previous_model: { native_model: string | null } | null; completeness: string }[];
  cost_by_model: Cost[]; evidence_partial: boolean; content_digest: string;
};
type TimelinePage = { turns: TimelineTurn[]; watermark: string; complete: boolean };
/** What the latest read established. Only a successful read without turns is `empty`. */
export type TimelineAvailability = 'loading' | 'empty' | 'turns' | 'blocked' | 'unavailable';
type Lane = 'regular' | 'primary' | 'unknown';

const laneOf = (turn: TimelineTurn): Lane => turn.group === 'regular' || turn.group === 'primary' ? turn.group : 'unknown';
const modelOf = (turn: TimelineTurn) => [...turn.requests].reverse().find(request => request.native_model)?.native_model ?? turn.model_configuration_id;
const failed = (request: TimelineRequest) => request.outcome !== null && !['accepted', 'completed', 'success', 'succeeded'].includes(request.outcome);
const TICKS = 18;
const REQUESTS = 6;

/** `active` allows reads at all; `displayed` adds continuing polling. Otherwise one probe settles availability. */
export function RoutingTimeline({ session, language, active, displayed, refreshVersion, requestId, contentCleared = false, clearedRequestIds = [], onSelectRequest, onOpenPerformance, onViewConversation, onAvailability }: {
  session: string; language: 'zh' | 'en'; active: boolean; displayed: boolean; refreshVersion: number; requestId: string | null; contentCleared?: boolean; clearedRequestIds?: readonly string[];
  onSelectRequest(request: TimelineRequest): void; onOpenPerformance(plan: string): void;
  onViewConversation?(request: TimelineRequest): void; onAvailability?(availability: TimelineAvailability): void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [turns, setTurns] = useState<TimelineTurn[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState(false);
  const [retry, setRetry] = useState(0);
  const [allRequests, setAllRequests] = useState(false);
  const [arrivals, setArrivals] = useState(new Set<string>());
  const [unseen, setUnseen] = useState<string[]>([]);
  const [viewport, setViewport] = useState({ from: 0, to: 1 });
  const [overviewFocus, setOverviewFocus] = useState<string | null>(null);
  const watermark = useRef<string | null>(null);
  const blockedScope = useRef(false);
  const probed = useRef(false);
  const lastRead = useRef(-Infinity);
  const failures = useRef(0);
  const stored = useRef(new Map<string, TimelineTurn>());
  const scroller = useRef<HTMLDivElement>(null);
  const timeline = useRef<HTMLElement>(null);
  const pendingFocus = useRef<{ from: Element; owner: string; attribute: 'data-overview-turn' | 'data-turn-id' } | null>(null);
  useEffect(() => { blockedScope.current = false; setError(''); }, [session]);
  useEffect(() => { watermark.current = null; probed.current = false; lastRead.current = -Infinity; failures.current = 0; stored.current.clear(); setTurns([]); setSelected(null); setOverviewFocus(null); setLoaded(false); setArrivals(new Set()); setUnseen([]); }, [session, refreshVersion]);
  useEffect(() => {
    if (!arrivals.size) return;
    const timer = window.setTimeout(() => setArrivals(new Set()), 600);
    return () => window.clearTimeout(timer);
  }, [arrivals]);
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController(); let inFlight = false; let timer = 0;
    const schedule = (delay: number) => { window.clearTimeout(timer); if (!controller.signal.aborted) timer = window.setTimeout(() => void load(), delay); };
    async function load() {
      if (inFlight || blockedScope.current || controller.signal.aborted || document.visibilityState !== 'visible' || (!displayed && probed.current)) return;
      inFlight = true;
      try {
        const page = await observationRead<TimelinePage>('routing_timeline', { session_id: session, after_watermark: watermark.current, limit: 50 }, controller.signal);
        if (controller.signal.aborted) return;
        let changed = false;
        const fresh = watermark.current !== null && probed.current ? page.turns.filter(turn => !stored.current.has(turn.turn_id)).map(turn => turn.turn_id) : [];
        const owners = new Map(page.turns.flatMap(turn => turn.requests.map(request => [request.request_id, turn.turn_id] as const)));
        const absorbed = new Map<string, string>();
        for (const id of stored.current.keys()) {
          const owner = owners.get(id);
          if (owner && owner !== id) { stored.current.delete(id); absorbed.set(id, owner); changed = true; }
        }
        if (absorbed.size) {
          const focused = document.activeElement;
          if (focused && timeline.current?.contains(focused)) {
            const overview = focused.closest('[data-overview-turn]');
            const control = overview ?? focused.closest('.routing-turn[data-turn-id]');
            const attribute = overview ? 'data-overview-turn' : 'data-turn-id';
            const owner = absorbed.get(control?.getAttribute(attribute) ?? '');
            if (owner) pendingFocus.current = { from: focused, owner, attribute };
          }
          setSelected(current => current ? absorbed.get(current) ?? current : null);
          setOverviewFocus(current => current ? absorbed.get(current) ?? current : null);
          setArrivals(current => new Set([...current].filter(id => !absorbed.has(id))));
          setUnseen(current => current.filter(id => !absorbed.has(id)));
        }
        for (const turn of page.turns) { if (stored.current.get(turn.turn_id)?.content_digest !== turn.content_digest) { stored.current.set(turn.turn_id, turn); changed = true; } }
        if (fresh.length) { setArrivals(previous => new Set([...previous, ...fresh])); setUnseen(previous => [...new Set([...previous, ...fresh])]); }
        if (changed) setTurns([...stored.current.values()].sort((a, b) => a.started_at_ms - b.started_at_ms || a.turn_id.localeCompare(b.turn_id)));
        watermark.current = page.watermark; failures.current = 0; setError(''); setLoaded(true);
        if (!page.complete) schedule(0);
        else { probed.current = true; if (displayed) schedule(2000); }
      } catch (e) {
        if (controller.signal.aborted) return;
        const code = safeDiagnosticCode(e, 'OBSERVATION_UNAVAILABLE');
        const busyDelay = observationBusyRetryMs(e);
        if (busyDelay !== null) { schedule(busyDelay); return; }
        blockedScope.current = code.toLowerCase().includes('scope_too_large');
        const stale = code.toLowerCase().includes('stale') && watermark.current !== null;
        if (stale) { watermark.current = null; stored.current.clear(); setTurns([]); setLoaded(false); setArrivals(new Set()); setUnseen([]); }
        setError(code);
        if (stale) schedule(0);
        else {
          probed.current = true;
          // Keep one visible read in flight; repeated transient failures back off,
          // while actual capacity limits require an explicit retry.
          const delay = Math.min(30_000, 2000 * 2 ** Math.min(failures.current++, 4));
          if (displayed && !blockedScope.current) schedule(delay);
        }
      } finally { inFlight = false; lastRead.current = performance.now(); }
    }
    const visible = () => { if (document.visibilityState === 'visible') void load(); else window.clearTimeout(timer); };
    const invalidate = () => { controller.abort(); stored.current.clear(); watermark.current = null; probed.current = false; lastRead.current = -Infinity; setTurns([]); setLoaded(false); setArrivals(new Set()); setUnseen([]); setRetry(n => n + 1); };
    // Showing the pane continues the two-second cadence instead of repeating the probe that just ran.
    const since = performance.now() - lastRead.current;
    document.addEventListener('visibilitychange', visible); window.addEventListener('hiroute-content-invalidated', invalidate);
    if (displayed && probed.current && since < 2000) schedule(2000 - since); else void load();
    return () => { controller.abort(); window.clearTimeout(timer); document.removeEventListener('visibilitychange', visible); window.removeEventListener('hiroute-content-invalidated', invalidate); };
  }, [session, active, displayed, refreshVersion, retry]);
  useLayoutEffect(() => {
    const focus = pendingFocus.current;
    pendingFocus.current = null;
    if (!focus || !displayed || !timeline.current) return;
    // A removed control loses focus to the body. Never take it from another control.
    if (document.activeElement !== document.body && document.activeElement !== focus.from) return;
    [...timeline.current.querySelectorAll<HTMLElement | SVGGElement>(`[${focus.attribute}]`)]
      .find(element => element.getAttribute(focus.attribute) === focus.owner)?.focus();
  }, [turns, displayed]);
  useEffect(() => { const turn = turns.find(t => t.requests.some(r => r.request_id === requestId)); if (turn) setSelected(turn.turn_id); }, [requestId, turns]);
  const blocked = error.toLowerCase().includes('scope_too_large');
  const availability: TimelineAvailability = turns.length ? 'turns' : blocked ? 'blocked' : error ? 'unavailable' : loaded ? 'empty' : 'loading';
  useEffect(() => { onAvailability?.(availability); }, [availability]);
  const markVisible = useCallback(() => {
    const pane = scroller.current;
    if (!pane || !pane.clientWidth) return;
    const viewport = pane.getBoundingClientRect();
    const labelWidth = pane.querySelector('.routing-lane-label')?.getBoundingClientRect().width ?? 0;
    const elements = [...pane.querySelectorAll<HTMLElement>('[data-turn-id]')];
    const indices: number[] = [];
    const visible = new Set(elements.filter((element, index) => {
      const bounds = element.getBoundingClientRect();
      const inside = bounds.right > viewport.left + labelWidth && bounds.left < viewport.right;
      if (inside) indices.push(index);
      return inside;
    }).map(element => element.dataset.turnId));
    if (indices.length) {
      const from = indices[0], to = indices[indices.length - 1] + 1;
      setViewport(previous => previous.from === from && previous.to === to ? previous : { from, to });
    }
    setUnseen(previous => { const next = previous.filter(id => !visible.has(id)); return next.length === previous.length ? previous : next; });
  }, []);
  // Set the initial position before interaction; later arrivals retain an explicitly selected turn.
  useLayoutEffect(() => {
    if (!displayed || !scroller.current) return;
    if (!selected) { scroller.current.scrollLeft = scroller.current.scrollWidth; setUnseen([]); }
    markVisible();
  }, [turns.length, selected, displayed, markVisible]);
  useEffect(() => {
    if (!displayed || !scroller.current) return;
    const observer = new ResizeObserver(markVisible); observer.observe(scroller.current);
    return () => observer.disconnect();
  }, [displayed, markVisible, turns.length > 0]);
  useEffect(() => setAllRequests(false), [selected]);

  const selectedTurn = turns.find(t => t.turn_id === selected) ?? turns[turns.length - 1];
  const overviewTabStop = turns.some(turn => turn.turn_id === overviewFocus) ? overviewFocus : selectedTurn?.turn_id;
  const coordinates = new Map(turns.map((turn, i) => [turn.turn_id, i + 1]));
  const index = (turn: TimelineTurn) => coordinates.get(turn.turn_id)!;
  const assessmentCoordinates = new Map<string, number | null>();
  for (const turn of turns) {
    // Assessments refer to authoritative AgentTurnId, while timeline selection
    // uses the opening RequestId. Ambiguous retained identities cannot locate a score.
    if (turn.agent_turn_id) assessmentCoordinates.set(turn.agent_turn_id, assessmentCoordinates.has(turn.agent_turn_id) ? null : index(turn));
  }
  const boundary = (turn: TimelineTurn) => turn.boundary === 'new_user_message' ? text('新消息', 'New message') : turn.boundary === 'context_rebuilt' ? text('上下文重建', 'Context rebuilt') : text('边界未确认', 'Boundary unknown');
  const laneName = (lane: Lane) => lane === 'primary' ? text('主力', 'Primary') : lane === 'regular' ? text('常规', 'Regular') : text('分组未记录', 'Group unknown');
  const tone = (lane: Lane) => lane === 'regular' ? 'cyan' : lane === 'primary' ? 'purple' : '';
  const selectedLane = (turn: TimelineTurn): Lane => turn.selection?.execution_group ?? 'unknown';
  const reason = (turn: TimelineTurn) => selectionReasonLabel(turn.selection?.selection_reason ?? '', language);
  const fixed = (millis: number | null | undefined) => millis == null ? '—' : (millis / 1000).toFixed(2);
  const probability = (turn: TimelineTurn) => turn.selection?.simple_probability == null ? text('简单概率未知', 'P(simple) unknown') : `${text('简单概率', 'P(simple)')} ${turn.selection.simple_probability.toFixed(2)}`;
  const below = (turn: TimelineTurn) => Boolean(turn.assessment && turn.competence_floor_millis != null && turn.assessment.score * 1000 < turn.competence_floor_millis);
  const scoreTone = (turn: TimelineTurn) => turn.assessment ? below(turn) ? 'bad' : turn.assessment.partial ? 'warn' : 'good' : '';
  const score = (turn: TimelineTurn) => turn.assessment ? `${turn.assessment.partial ? text('部分评分', 'Partial') : text('评分', 'Score')} ${turn.assessment.score.toFixed(2)}`
    : turn.status === 'running' ? text('进行中…', 'Running…') : turn.assessment_state === 'pending' ? text('等待评分', 'Awaiting score') : text('未评分', 'Unscored');
  const amount = (micros: number | null, currency: string | null) => micros === null || !currency ? text('金额未知', 'Amount unknown') : formatMoney(currency, (micros / 1_000_000).toFixed(micros >= 10_000 ? 3 : 4));
  const costBasis = (cost: Cost) => cost.valuation_kind === 'api_equivalent' ? text('API 等价估算', 'API equivalent estimate') : text('估算', 'estimate');
  const costAmount = (cost: Cost) => `${cost.valuation_kind === 'api_equivalent' ? `${costBasis(cost)} ` : ''}${amount(cost.known_micros, cost.currency)}${cost.unknown_contributions ? text(' + 未知部分', ' + unknown') : ''}`;
  const percent = (share: number | null) => share === null ? text('未知', 'unknown') : `${Math.round(share * 100)}%`;
  const tokens = (count: number | null) => count === null ? '—' : formatTokenCount(count, language);

  const grouped = turns.some(turn => laneOf(turn) !== 'unknown');
  const lanes = (['regular', 'primary', 'unknown'] as Lane[]).filter(lane => lane === 'unknown' ? turns.some(turn => laneOf(turn) === 'unknown') : grouped);
  const laneModel = (lane: Lane) => { const latest = [...turns].reverse().find(turn => laneOf(turn) === lane); return latest ? modelOf(latest) : null; };
  const names = new Map<string, string>();
  for (const turn of turns) if (turn.model_configuration_id) names.set(turn.model_configuration_id, modelOf(turn) ?? turn.model_configuration_id);
  const totals = new Map<string, { cost: Cost; lane: Lane }>();
  for (const turn of turns) for (const cost of turn.cost_by_model) {
    const key = `${cost.model_configuration_id}\u0000${cost.currency}\u0000${cost.valuation_kind}`;
    const total = totals.get(key);
    if (!total) totals.set(key, { cost: { ...cost }, lane: laneOf(turn) });
    else {
      const sum = (total.cost.known_micros ?? 0) + (cost.known_micros ?? 0);
      const known = total.cost.known_micros !== null || cost.known_micros !== null;
      const overflow = known && !Number.isSafeInteger(sum);
      total.cost = { ...total.cost, known_micros: known && !overflow ? sum : null, unknown_contributions: total.cost.unknown_contributions + cost.unknown_contributions + Number(overflow) };
    }
  }
  const last = turns[turns.length - 1];
  const running = last?.status === 'running';
  const protectionLabel = (turn: TimelineTurn) => {
    switch (turn.selection?.historical_protection?.outcome) {
      case 'applied': return text('低分保护', 'Low-score protection');
      case 'suppressed': return text('已解除保护', 'Protection released');
      case 'retained_unavailable': return text('判断不可用，保留保护', 'Judgment unavailable; protection retained');
      case 'not_needed': return text('无需低分保护', 'Protection not needed');
      case 'not_considered': return text('未进入保护判断', 'Protection not considered');
      default: return '';
    }
  };
  function selectTurn(turn: TimelineTurn, navigate = false) {
    setSelected(turn.turn_id);
    if (turn.requests[0]) onSelectRequest(turn.requests[0]);
    if (!navigate || !scroller.current) return;
    const pane = scroller.current;
    const element = [...pane.querySelectorAll<HTMLElement>('[data-turn-id]')].find(node => node.dataset.turnId === turn.turn_id);
    if (!element) return;
    const bounds = element.getBoundingClientRect(), outer = pane.getBoundingClientRect();
    const labels = pane.querySelector('.routing-lane-label')?.getBoundingClientRect().width ?? 0;
    const left = pane.scrollLeft + bounds.left - outer.left - labels - (pane.clientWidth - labels - bounds.width) / 2;
    pane.scrollTo({ left, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  }

  function historicalExplanation(turn: TimelineTurn) {
    if (turn.selection?.selection_reason === 'extension_decision' || turn.selection?.selection_reason === 'decision_unavailable') return <p>{explanation(turn)}</p>;
    const history = turn.selection?.historical_protection;
    if (!history) return <><p>{explanation(turn)}</p><p className="muted">{text('历史保护结果未记录。', 'Historical protection result was not recorded.')}</p></>;
    const p = turn.selection?.simple_probability, threshold = turn.selection?.simple_threshold_millis;
    const simple = p != null && threshold != null ? p * 1000 >= threshold : null;
    const task = simple == null ? text('难度判断不可用', 'Difficulty judgment unavailable') : simple ? text('简单', 'Simple') : text('复杂', 'Complex');
    const a = history.applicability_probability;
    const applicability = history.outcome === 'applied' ? text('之前的问题仍相关', 'The earlier problem still matters')
      : history.outcome === 'suppressed' ? text('之前的问题已修正或不再相关', 'The earlier problem was fixed or no longer matters')
      : history.outcome === 'retained_unavailable' ? text('无法判断之前的问题，保留保护', 'Earlier problem judgment unavailable; protection retained')
      : history.outcome === 'not_needed' ? text('无需历史低分保护', 'Historical protection not needed') : text('未进入历史低分保护判断', 'Historical protection was not considered');
    return <div className="routing-judgments">
      <p>{text('当前工作：', 'Current task: ')}{task}{p != null && <> ({p.toFixed(2)}{threshold != null && <> {simple ? '≥' : '<'} {fixed(threshold)}</>})</>}</p>
      <p>{text('上一阶段：', 'Previous stage: ')}{history.previous_score == null ? text('评分未记录', 'Score not recorded') : history.previous_score.toFixed(2)} · {text('胜任下限 ', 'Competence floor ')}{fixed(history.competence_floor_millis)}</p>
      <p>{history.outcome === 'not_needed' && a != null ? <>{text('之前的问题相关概率 ', 'Earlier problem relevance ')}{a.toFixed(2)} · {applicability}</> : <>{applicability}{a != null && <> ({a.toFixed(2)})</>}</>}</p>
      <strong>→ {laneName(selectedLane(turn))}</strong>
    </div>;
  }

  function explanation(turn: TimelineTurn) {
    const target = laneName(selectedLane(turn));
    const p = turn.selection?.simple_probability, threshold = turn.selection?.simple_threshold_millis;
    const trigger = turn.selection?.competence_trigger;
    switch (turn.selection?.selection_reason) {
      case 'extension_decision': return `${text('由决策扩展选择', 'Selected by the decision extension')}${turn.selection.extension_reason ? ` · ${turn.selection.extension_reason}` : ''} → ${target}.`;
      case 'decision_unavailable': return text(`扩展未提供有效模型组，保留任务类别并使用主力 → ${target}。`, `The extension did not return a valid group; keep the category and use primary → ${target}.`);
      case 'simple_task': return text(`简单概率 ${p?.toFixed(2) ?? '—'} ≥ 阈值 ${fixed(threshold)}，没有适用的低分 → ${target}。`, `P(simple) ${p?.toFixed(2) ?? '—'} ≥ threshold ${fixed(threshold)} with no applicable low score → ${target}.`);
      case 'complex_task': return text(`简单概率 ${p?.toFixed(2) ?? '—'} < 阈值 ${fixed(threshold)} → ${target}。`, `P(simple) ${p?.toFixed(2) ?? '—'} < threshold ${fixed(threshold)} → ${target}.`);
      case 'low_competence': return trigger ? text(`上一阶段评分 ${trigger.score.toFixed(2)} 低于下限 ${fixed(trigger.floor_millis)} → ${target}。`, `The previous stage scored ${trigger.score.toFixed(2)}, below the ${fixed(trigger.floor_millis)} floor → ${target}.`) : text(`上一阶段低于胜任下限 → ${target}。`, `The previous stage was below the competence floor → ${target}.`);
      default: return `${reason(turn)}${p != null ? ` · ${text('简单概率', 'P(simple)')} ${p.toFixed(2)}` : ''}${threshold != null ? ` · ${text('阈值', 'threshold')} ${fixed(threshold)}` : ''} → ${target}${text('。', '.')}`;
    }
  }
  function scoreExplanation(turn: TimelineTurn) {
    if (turn.assessment) {
      const from = assessmentCoordinates.get(turn.assessment.target_from_turn_id ?? '');
      const through = assessmentCoordinates.get(turn.assessment.target_through_turn_id ?? '');
      // Producer ordinals can restart across epochs; only exact identities share
      // the same coordinates as the overview, lane headings and selected detail.
      const covered = typeof from === 'number' && typeof through === 'number' && from <= through
        ? from === through ? text(`覆盖轮次 T${from}`, `covers turn T${from}`) : text(`覆盖轮次 T${from}–T${through}`, `covers turns T${from}–T${through}`)
        : text('覆盖范围包含当前已加载范围外或身份未确认的轮次', 'covers turns outside the loaded range or without a confirmed identity');
      const floor = turn.competence_floor_millis != null ? text(`，下限 ${fixed(turn.competence_floor_millis)}`, `, floor ${fixed(turn.competence_floor_millis)}`) : '';
      const protectedNext = turn.protected_turn_ids.length ? text('低于下限，下一轮交给主力。', ' Below the floor, so the next agent turn went to Primary.') : '';
      const attributionNote = !turn.model_configuration_id
        ? text('整体阶段评分，不计入单模型能力。', ' Overall stage score; excluded from individual model capability.') : '';
      return text(`阶段评分${covered}${floor}。${turn.assessment.partial ? '证据不完整。' : ''}${attributionNote}${protectedNext}`, `This stage score ${covered}${floor}.${turn.assessment.partial ? ' Evidence is incomplete.' : ''}${attributionNote}${protectedNext}`);
    }
    if (turn.status === 'running') return text('这一轮还在进行，结束后才评分。', 'This agent turn is still running; it is scored after it ends.');
    if (turn.assessment_state === 'pending') return text('下一轮开始后，根据你的反馈给这一轮评分。', 'Scored from your feedback once the next agent turn starts.');
    return text('这一轮没有评分。', 'This agent turn was not scored.');
  }
  const status = (turn: TimelineTurn) => ({ running: text('运行中', 'Running'), completed: text('完成', 'Completed'), failed: text('失败', 'Failed'), interrupted: text('中断', 'Interrupted') } as Record<string, string>)[turn.status] ?? text('状态未知', 'Status unknown');
  const transition = (kind: string) => ({ initial_selection: text('首次选择', 'Initial selection'), between_turn_change: text('跨轮模型切换', 'Between-turn model change'), continuation_boundary: text('续接边界', 'Continuation boundary'), within_request_fallback: text('请求内模型回退', 'Within-request fallback'), credential_relay: text('凭据接力', 'Credential relay'), reasoning_change: text('推理设置变化', 'Reasoning change'), unknown: text('切换证据尚不完整', 'Transition evidence incomplete') } as Record<string, string>)[kind] ?? text('切换类型未知', 'Unknown transition');

  const detail = selectedTurn && (() => {
    const lane = laneOf(selectedTurn);
    const requests = allRequests ? selectedTurn.requests : selectedTurn.requests.slice(0, REQUESTS);
    const changes = (selectedTurn.transitions ?? []).filter(change => !['unchanged', 'initial_selection'].includes(change.kind));
    return <div className="routing-turn-detail">
      <div className="routing-detail-main">
        <div className="routing-detail-head">
          <strong>T{index(selectedTurn)}</strong><span className="badge no-dot">{boundary(selectedTurn)}</span><span className={`badge no-dot ${tone(lane)}`}>{text('模型：', 'Model: ')}{modelOf(selectedTurn) ?? text('未记录', 'unknown')}</span>
          <span className="routing-detail-actions">
            {onViewConversation && selectedTurn.requests[0] && <button type="button" className="btn btn-quiet" onClick={() => onViewConversation(selectedTurn.requests[0])}>{text('查看对话', 'View conversation')}</button>}
            {selectedTurn.plan_id && <button type="button" className="btn btn-quiet" onClick={() => onOpenPerformance(selectedTurn.plan_id!)}>{text('查看模型表现', 'View model performance')}</button>}
          </span>
        </div>
        <p className="routing-boundary-basis muted">{selectedTurn.boundary === 'new_user_message'
          ? text('依据：会话第一轮，或网关确认对话延续并重新判断。客户端自动追加的后台消息也可能满足此规则。', 'Basis: the first session turn, or a fresh decision on gateway-confirmed continuing history. Client-generated background messages can also meet this rule.')
          : selectedTurn.boundary === 'context_rebuilt' ? text('上下文中的消息步骤比上一份可比较记录少，按压缩归因；已重新判断。回退或共享会话的后台、子代理请求也可能满足此条件。', 'The context has fewer message steps than the prior comparable checkpoint, so this is attributed to compaction and judged again. Rewinds or background and subagent requests sharing a session can also meet this condition.')
          : text('没有足够记录确认这一轮的起因。', 'The retained records do not establish what started this turn.')}</p>
        {selectedTurn.requests[0] && <blockquote className="routing-quote"><ContentExcerpt active={active && displayed} contentCleared={contentCleared || clearedRequestIds.includes(selectedTurn.requests[0].request_id)} request={selectedTurn.requests[0].request_id} language={language} fallback={text('这一轮的消息暂不可用', 'This turn’s message is unavailable')} limit={140} revision={selectedTurn.content_digest} /></blockquote>}
        <div className="routing-detail-block"><span>{text('阶段开始时的模型选择', 'Model selection at the start of this stage')}</span>{historicalExplanation(selectedTurn)}</div>
        <div className="routing-detail-block"><span>{text('胜任评分', 'Competence score')}</span><div><span className={`routing-score ${scoreTone(selectedTurn)}`}>{score(selectedTurn)}</span></div><p>{scoreExplanation(selectedTurn)}</p></div>
        <p className="routing-detail-facts">{[
          status(selectedTurn),
          text(`${selectedTurn.tool_continuations} 次工具续轮`, `${selectedTurn.tool_continuations} tool continuation${selectedTurn.tool_continuations === 1 ? '' : 's'}`),
          selectedTurn.relays.length ? text(`${selectedTurn.relays.length} 次请求内接力`, `${selectedTurn.relays.length} relay${selectedTurn.relays.length === 1 ? '' : 's'} within requests`) : '',
          selectedTurn.reasoning_profile_id ? text(`思考 ${selectedTurn.reasoning_profile_id}`, `reasoning ${selectedTurn.reasoning_profile_id}`) : '',
          selectedTurn.evidence_partial ? text('证据不完整', 'Incomplete evidence') : '',
        ].filter(Boolean).join(' · ')}</p>
        {changes.map((change, i) => {
          const withinTurn = Boolean(change.previous_request_id && selectedTurn.requests.some(request => request.request_id === change.previous_request_id));
          const label = change.kind !== 'unknown' || !withinTurn ? transition(change.kind)
            : change.completeness !== 'complete' ? text('请求间切换证据尚不完整', 'Request transition evidence incomplete')
            : change.model_changed ? text('本轮请求间模型变化', 'Model change between requests in this turn')
            : change.reasoning_changed ? text('本轮请求间推理设置变化', 'Reasoning change between requests in this turn')
            : text('本轮请求间变化未分类', 'Unclassified transition between requests in this turn');
          return <p key={`transition-${i}`} className="routing-detail-facts">{label}{change.previous_model?.native_model && ` · ${change.previous_model.native_model} → ${change.model.native_model ?? '?'}`}</p>;
        })}
        {selectedTurn.relays.map((relay, i) => <p key={`relay-${i}`} className="routing-detail-facts">{text('接力', 'Relay')} · {relay.from_model ?? '?'} → {relay.to_model ?? '?'}</p>)}
      </div>
      <div className="routing-detail-requests">
        <span className="routing-detail-label">{text(`请求（${selectedTurn.requests.length}）`, `Requests (${selectedTurn.requests.length})`)}</span>
        <div className="routing-turn-requests">{requests.map((request, i) => <button className={`routing-request${request.request_id === requestId ? ' selected' : ''}${failed(request) ? ' failed' : ''}`} type="button" key={request.request_id} onClick={() => onSelectRequest(request)}>
          <span className="routing-request-index">#{i + 1}</span>
          <span className="routing-request-main"><span>{text('输入', 'In')} {tokens(request.input_tokens)} · {text('输出', 'Out')} {tokens(request.output_tokens)}</span><small>{[request.native_model ?? text('模型未记录', 'Model unknown'), request.duration_ms === null ? '' : `${(request.duration_ms / 1000).toFixed(1)} s`, request.costs.map(costAmount).join(' · '), failed(request) ? text('失败', 'Failed') : ''].filter(Boolean).join(' · ')}</small></span>
          <span className={`badge no-dot ${request.cache_read_share !== null && request.cache_read_share >= 0.5 ? 'good' : ''}`}>{text('缓存', 'Cache')} {percent(request.cache_read_share)}</span>
        </button>)}</div>
        {!allRequests && selectedTurn.requests.length > REQUESTS && <button type="button" className="btn btn-quiet" onClick={() => setAllRequests(true)}>{text(`显示全部 ${selectedTurn.requests.length} 个请求`, `Show all ${selectedTurn.requests.length} requests`)}</button>}
        {selectedTurn.cost_by_model.length > 0 && <p className="routing-detail-facts">{text('这一轮按模型估算：', 'This turn by model: ')}{selectedTurn.cost_by_model.map(cost => `${names.get(cost.model_configuration_id ?? '') ?? cost.model_configuration_id ?? text('模型未知', 'unknown model')} ${costAmount(cost)}`).join(' · ')}</p>}
      </div>
    </div>;
  })();

  return <section className="routing-timeline" ref={timeline} aria-label={text('路由时间线', 'Routing timeline')}>
    <div className="routing-live" role="status"><span className={`routing-live-dot${running && !error ? ' running' : ''}`} />{error ? text('更新延迟', 'Update delayed') : running ? text(`实时 · 第 ${turns.length} 轮正在执行`, `Live · agent turn ${turns.length} is running`) : text('实时', 'Live')}<span className="muted">{error ? blocked ? text('等待手动重试', 'Waiting for Retry') : text('可见时自动重试', 'Retries automatically while visible') : text('可见时每 2 秒更新', 'Updates every 2 s while visible')}</span></div>
    {error && <div className="callout warn" role="alert" data-error-code={error}><span>{blocked ? text('这条会话超出完整读取范围，暂时无法显示时间线。', 'This session exceeds the complete-read limit; its timeline is unavailable.') : turns.length ? text('更新暂时不可用，显示上次读取的内容。', 'Updates are temporarily unavailable. Showing the last successful read.') : text('暂时无法读取，正在自动重试。', 'The read is temporarily unavailable. Retrying automatically.')}</span><button className="btn" onClick={() => { blockedScope.current = false; failures.current = 0; lastRead.current = -Infinity; setRetry(n => n + 1); }}>{text('重试', 'Retry')}</button></div>}
    {!loaded && !error && <p className="muted" role="status">{text('正在读取轮次…', 'Reading turns…')}</p>}
    {loaded && !turns.length && <p className="muted">{text('还没有可显示的轮次。', 'No recorded turns yet.')}</p>}
    {turns.length > 0 && <div className="routing-tiles">
      {[...totals.values()].map(({ cost, lane }) => <div className="routing-tile" key={`${cost.model_configuration_id}/${cost.currency}/${cost.valuation_kind}`}><span>{names.get(cost.model_configuration_id ?? '') ?? cost.model_configuration_id ?? text('模型未知', 'Unknown model')} · {costBasis(cost)}</span><strong className={tone(lane)}>{amount(cost.known_micros, cost.currency)}</strong>{cost.unknown_contributions > 0 && <small>{cost.known_micros === null ? text('记录不足，暂不能估算', 'Not enough evidence to estimate') : text('已知小计 · 另有未知部分', 'Known subtotal · Plus an unknown portion')}</small>}</div>)}
      <div className="routing-tile"><span>{text('Agent 轮次', 'Agent turns')}</span><strong>{turns.length}</strong><small>{text(`${turns.reduce((n, turn) => n + turn.requests.length, 0)} 个请求`, `${turns.reduce((n, turn) => n + turn.requests.length, 0)} requests`)}</small></div>
    </div>}
    {turns.length > 0 && <div className="routing-lanes-card">
      <div className="routing-overview">
        <div className="routing-overview-head"><strong>{text(`${turns.length} 轮总览`, `${turns.length} turns at a glance`)}</strong><span>{text('● 低分 / 失败 · ◆ 保护 · ✓ 解除', '● Low score / failure · ◆ Protected · ✓ Released')}</span></div>
        <svg className="routing-overview-track" role="group" aria-label={text('轮次总览，选择以定位', 'Turn overview; select to navigate')}>
          {turns.map((turn, i) => {
            const outcome = turn.selection?.historical_protection?.outcome;
            const protectedTurn = outcome === 'applied' || outcome === 'retained_unavailable' || turn.selection?.selection_reason === 'low_competence';
            const problem = below(turn) || turn.status === 'failed' || turn.requests.some(failed);
            const label = [`T${i + 1}`, laneName(laneOf(turn)), problem ? text('低分或失败', 'Low score or failure') : '', protectionLabel(turn)].filter(Boolean).join(' · ');
            return <g key={turn.turn_id} className={`routing-overview-turn ${tone(laneOf(turn))}`} role="button" tabIndex={overviewTabStop === turn.turn_id ? 0 : -1} aria-label={label} aria-pressed={selectedTurn?.turn_id === turn.turn_id} data-overview-turn={turn.turn_id} data-protection={outcome} data-problem={problem || undefined} onFocus={() => setOverviewFocus(turn.turn_id)} onClick={() => { setOverviewFocus(turn.turn_id); selectTurn(turn, true); }} onKeyDown={event => {
              const next = event.key === 'Home' ? 0 : event.key === 'End' ? turns.length - 1
                : event.key === 'ArrowRight' || event.key === 'ArrowDown' ? (i + 1) % turns.length
                : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? (i + turns.length - 1) % turns.length : null;
              if (next !== null) {
                event.preventDefault();
                event.currentTarget.parentElement?.querySelectorAll<SVGGElement>('.routing-overview-turn')[next]?.focus();
              }
              if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectTurn(turn, true); }
            }}>
              <title>{label}</title>
              <rect className="routing-overview-mark" x={`${i * 100 / turns.length}%`} y="10" width={`${100 / turns.length}%`} height="16" rx="3" />
              {problem && <circle className="routing-overview-problem" cx={`${(i + .5) * 100 / turns.length}%`} cy="5" r="2.5" />}
              {(protectedTurn || outcome === 'suppressed') && <text className={`routing-overview-outcome${outcome === 'suppressed' ? ' released' : ''}`} x={`${(i + .5) * 100 / turns.length}%`} y="35" textAnchor="middle">{outcome === 'suppressed' ? '✓' : '◆'}</text>}
            </g>;
          })}
          <rect className="routing-overview-window" x={`${Math.min(viewport.from, turns.length - 1) * 100 / turns.length}%`} y="8" width={`${Math.max(1, Math.min(viewport.to, turns.length) - Math.min(viewport.from, turns.length - 1)) * 100 / turns.length}%`} height="20" rx="4" />
        </svg>
      </div>
      {selected && unseen.length > 0 && <div className="routing-lanes-notice" role="status"><button type="button" className="btn routing-new-turns" onClick={() => {
        scroller.current?.scrollTo({ left: scroller.current.scrollWidth, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
        setUnseen([]);
      }}>{text(`${unseen.length} 个新轮次 →`, `${unseen.length} new turn${unseen.length === 1 ? '' : 's'} →`)}</button></div>}
      <div className="routing-lanes-scroll" ref={scroller} onScroll={markVisible}>
        <div className="routing-lanes" role="group" aria-label={text('Agent 轮次', 'Agent turns')} data-lanes={lanes.length}>
          <div className="routing-lane-corner" />
          {lanes.map(lane => <div key={lane} className={`routing-lane-label ${tone(lane)}`}><strong>{laneName(lane)}</strong>{laneModel(lane) && <span>{text('模型：', 'Model: ')}{laneModel(lane)}</span>}</div>)}
          {/* One column per agent turn; the grid flows by column, so focus and reading order follow time. */}
          {turns.map(turn => <Fragment key={turn.turn_id}>
            <div className="routing-turn-head" title={new Date(turn.started_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')}><strong>T{index(turn)}</strong>{turn.boundary !== 'unknown' && <span className="badge no-dot">{boundary(turn)}</span>}</div>
            {lanes.map(lane => <div className={`routing-lane-cell${laneOf(turn) !== lane ? ' routing-cell-empty' : ''}`} key={lane} aria-hidden={laneOf(turn) !== lane || undefined}>{laneOf(turn) === lane && <button type="button" className={`routing-turn ${tone(lane)}${selectedTurn?.turn_id === turn.turn_id ? ' selected' : ''}`} data-turn-id={turn.turn_id} data-new={arrivals.has(turn.turn_id) || undefined} aria-pressed={selectedTurn?.turn_id === turn.turn_id} onClick={() => selectTurn(turn)}>
              <span className="sr-only">{text(`轮次 ${index(turn)}`, `Turn ${index(turn)}`)}</span>
              <strong className="routing-turn-reason">{reason(turn)}</strong>
              <span className="routing-turn-meta">{probability(turn)}</span>
              {['suppressed', 'retained_unavailable'].includes(turn.selection?.historical_protection?.outcome ?? '') && <span className={`badge no-dot routing-protection-badge ${turn.selection?.historical_protection?.outcome === 'suppressed' ? 'good' : 'warn'}`}>{protectionLabel(turn)}</span>}
              <span className="routing-ticks" aria-label={text(`${turn.requests.length} 个请求`, `${turn.requests.length} requests`)}>{turn.requests.slice(0, TICKS).map(request => <i key={request.request_id} className={failed(request) ? 'failed' : undefined} />)}{turn.requests.length > TICKS && <em>+{turn.requests.length - TICKS}</em>}</span>
              <span className={`routing-score ${scoreTone(turn)}`}>{score(turn)}</span>
              {(turn.selection?.competence_trigger || turn.protected_turn_ids.length > 0 || turn.evidence_partial || turn.relays.length > 0) && <span className="routing-turn-flags">
                {turn.selection?.competence_trigger && <span className="badge warn no-dot">{text('胜任度保护', 'Competence protection')}</span>}
                {turn.protected_turn_ids.length > 0 && <span className="badge no-dot">{text('触发下一轮保护', 'Protected the next turn')}</span>}
                {turn.relays.length > 0 && <span className="badge no-dot">{text(`接力 ${turn.relays.length}`, `${turn.relays.length} relay${turn.relays.length === 1 ? '' : 's'}`)}</span>}
                {turn.evidence_partial && <span className="badge warn no-dot">{text('证据不完整', 'Incomplete evidence')}</span>}
              </span>}
            </button>}</div>)}
          </Fragment>)}
        </div>
      </div>
      <p className="routing-legend"><span><i />{text('请求', 'Request')}</span><span><i className="failed" />{text('失败', 'Failed')}</span><span className="muted">{text('同一轮内不换模型，保住缓存；新消息或上下文重建时重新判断。点一轮查看详情。', 'The model stays fixed within a turn to preserve the cache; a new message or rebuilt context triggers a fresh decision. Select a turn for details.')}</span></p>
    </div>}
    {detail}
  </section>;
}
