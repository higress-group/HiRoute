import React, { useState } from 'react';
import { Disclosure } from '../ui';

export type DecisionContextObservation = {
  schema: 'hiroute.decision-context-observation/v1'; context_digest: string;
  request_source: 'user' | 'metadata' | 'fallback'; question_source: 'user' | 'metadata' | 'fallback';
  request_window_tokens: number; question_window_tokens: number; request_usable_tokens: number; question_usable_tokens: number;
  estimator: string; estimated_request_tokens: number | null; estimated_largest_question_tokens: number | null; encoded_bytes: number | null;
  target_turns_retained: number; target_turns_removed: number; target_steps_retained: number; target_steps_removed: number | null;
  prior_turns_retained: number; prior_turns_removed: number; prior_steps_retained: number; prior_steps_removed: number | null;
  replay_messages_retained: number; replay_messages_omitted: number | null; replay_envelopes_scanned: number; replay_envelopes_unscanned: number | null;
  replay_truncated: boolean; replay_boundary_unavailable: boolean; replay_budget_omitted: boolean; target_partial: boolean;
  limiting_constraint: 'request' | 'question' | 'json_shape'; no_fit_reason: 'mandatory_core' | 'mandatory_shape' | 'accounting' | null;
};
export type RoutingObservation = {
  protocol: string; content_digest: string; bundle_digest: string; provider: string; model: string; standard: string;
  target_segment: string | null; target_complete: boolean; eligibility: string;
  applicability_requested: boolean; applicability_state: string; applicability_reason: string; applicability_probability: number | null;
  protection_action: string; simple_probability: number | null; simple_threshold_millis: number;
  score: number | null; competence_floor_millis: number; applicability_boundary_millis: number;
  request_bytes: number; prior_retained: number; prior_removed: number; stage_retained: number; stage_removed: number;
  context_budget?: DecisionContextObservation | null;
};
type Counts = {
  c_decisions: number; requested: number; unavailable: number; no_target_omissions: number; partial_target_omissions: number;
  inherited: number; required_question_missing: number; gate_opportunities: number; suppressed: number;
  unavailable_per_requested: number | null; suppressed_per_opportunity: number | null; suppressed_per_c_decision: number | null;
};
export type RoutingSummary = {
  plan_id: string | null; session_id: string | null; plan_revision: number | null;
  requested_from_ms: number | null; requested_to_ms: number | null;
  coverage: string; covered_from_ms: number; covered_to_ms: number; retained_decisions: number;
  known_non_c_decisions: number; legacy_or_unknown_decisions: number; unknown_scope_decisions: number; missing_projection_events: number; scan_limited: boolean;
  cohorts: { cohort: { protocol: string; content_digest: string; bundle_digest: string; scope_id: string; standard: string; provider: string; model: string; simple_threshold_millis: number; competence_floor_millis: number }; counts: Counts }[];
};
const rate = (value: number | null) => value == null ? '—' : (value * 100).toFixed(1) + '%';

export function RoutingObservations({ summary, language }: { summary: RoutingSummary | null; language: 'zh' | 'en' }) {
  const [expanded, setExpanded] = useState(false);
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (!summary) return null;
  return <Disclosure className="routing-observations" label={text('历史问题适用性观测', 'Historical problem applicability observations')} language={language} defaultOpen={expanded} onOpenChange={setExpanded}>
    <p className="oc-meta">{text('统计当前计划、版本与时间范围内所有保留的决策，不受模型、阶段评分或分页筛选影响。', 'Counts cover every retained decision in this plan, revision and time range, independently of model, stage-score and page filters.')}</p>
    <p className="oc-meta">{new Date(summary.covered_from_ms).toLocaleString()} – {new Date(summary.covered_to_ms).toLocaleString()} · {text('保留决策 ', 'Retained decisions ')}{summary.retained_decisions} · {text('早期发布及其他路由决策 ', 'Earlier publications and other routing decisions ')}{summary.known_non_c_decisions} · {summary.coverage === 'complete' ? text('覆盖完整', 'Complete coverage') : text('覆盖不完整，比例不可用', 'Partial coverage; rates unavailable')}</p>
    {(summary.legacy_or_unknown_decisions > 0 || summary.unknown_scope_decisions > 0 || summary.missing_projection_events > 0 || summary.scan_limited) && <p className="quality-coverage">{text('来源无法确认的决策 ', 'Decisions of unconfirmed origin ')}{summary.legacy_or_unknown_decisions} · {text('范围未知 ', 'Unknown scope ')}{summary.unknown_scope_decisions} · {text('缺少安全投影的事件 ', 'Events missing safe projection ')}{summary.missing_projection_events}{summary.scan_limited && text(' · 达到读取上限', ' · Read limit reached')}</p>}
    {summary.cohorts.length === 0 && <p className="oc-meta">{text('此范围内暂无历史问题适用性决策记录。', 'No historical problem applicability decisions are available in this scope.')}</p>}
    {summary.cohorts.map(({ cohort, counts }, index) => <section className="routing-cohort" key={index}>
      <strong>{cohort.scope_id} · {cohort.provider} / {cohort.model} · {cohort.standard === 'default' ? text('默认评分', 'Default assessment') : text('自定义评分', 'Custom assessment')}</strong>
      <p>{text('简单阈值 ', 'Simple threshold ')}{cohort.simple_threshold_millis / 1000} · {text('胜任下限 ', 'Competence floor ')}{cohort.competence_floor_millis / 1000}</p>
      <dl className="routing-counts">
        <div><dt>{text('适用性决策 / 同轮继承', 'Applicability decisions / inherited')}</dt><dd>{counts.c_decisions} / {counts.inherited}</dd></div>
        <div><dt>{text('适用性判断不可用 / 实际请求', 'Applicability unavailable / requested')}</dt><dd>{counts.unavailable} / {counts.requested} · {rate(counts.unavailable_per_requested)}</dd></div>
        <div><dt>{text('低分保护被解除 / 已记录保护机会', 'Low-score protection released / recorded opportunities')}</dt><dd>{counts.suppressed} / {counts.gate_opportunities} · {rate(counts.suppressed_per_opportunity)}</dd></div>
        <div><dt>{text('抑制占适用性决策', 'Suppressed per applicability decision')}</dt><dd>{rate(counts.suppressed_per_c_decision)}</dd></div>
        <div><dt>{text('无目标 / 部分目标省略', 'No-target / partial-target omissions')}</dt><dd>{counts.no_target_omissions} / {counts.partial_target_omissions}</dd></div>
        <div><dt>{text('内部缺失必需判断', 'Internally missing required judgment')}</dt><dd>{counts.required_question_missing}</dd></div>
      </dl>
    </section>)}
    <p className="oc-meta">{text('机会按当时实际记录的保护动作统计，可能混合不同时期的规则；解除低分保护不一定避免使用主力模型。这些比例不能比较统一规则下的效果，也不代表准确率或质量提升。零分母与覆盖缺口均显示为 —。', 'Opportunities count protection actions recorded under the rules in effect at the time; a scope may mix earlier and current rules. Releasing low-score protection does not necessarily avoid Primary. These rates do not compare effectiveness under a uniform rule or establish accuracy or quality improvement. A zero denominator or incomplete coverage is shown as —.')}</p>
  </Disclosure>;
}

export function RoutingDecisionDetail({ observation: o, language }: { observation: RoutingObservation; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const actions: Record<string, string> = {
    applied: text('应用历史低分保护', 'Historical protection applied'), suppressed: text('抑制历史低分保护', 'Historical protection suppressed'),
    retained_unavailable: text('适用性判断不可用，保留历史低分保护', 'Applicability unavailable; historical protection retained'),
    not_needed: text('无需历史低分保护', 'Historical protection not needed'), not_considered: text('未进入历史低分保护', 'Historical protection not considered'),
  };
  return <section className="routing-decision-detail">
    <strong>{text('本次历史问题适用性决策', 'This historical problem applicability decision')}</strong><p>{actions[o.protection_action] ?? o.protection_action}</p>
    <p>{text('适用性判断 ', 'Applicability ')}{o.applicability_requested ? text('已请求', 'requested') : text('未请求', 'not requested')} · {o.applicability_state} · {o.applicability_reason}{o.applicability_probability != null && ' · ' + o.applicability_probability.toFixed(3)}</p>
    <p>{text('目标阶段 ', 'Target stage ')}{o.target_segment ?? '—'} · {o.target_complete ? text('完整', 'complete') : text('不可用或不完整', 'unavailable or partial')} · {o.eligibility}</p>
    <p>{text('原始评分 ', 'Raw score ')}{o.score ?? '—'} · {text('当时胜任下限 ', 'Competence floor ')}{o.competence_floor_millis / 1000}</p>
    <p>{text('背景保留 / 移除 ', 'Prior turns retained / removed ')}{o.prior_retained} / {o.prior_removed} · {text('目标保留 / 移除 ', 'Target turns retained / removed ')}{o.stage_retained} / {o.stage_removed} · {o.request_bytes} B</p>
    {o.context_budget && <DecisionContextDetail budget={o.context_budget} language={language} />}
    <p>{o.provider} / {o.model} · {o.standard === 'default' ? text('默认评分', 'Default assessment') : text('自定义评分', 'Custom assessment')}</p>
  </section>;
}

function DecisionContextDetail({ budget: b, language }: { budget: DecisionContextObservation; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const number = (value: number | null) => value == null ? text('未知', 'Unknown') : value.toLocaleString();
  const source = (value: DecisionContextObservation['request_source']) => value === 'user' ? text('自定义', 'Custom') : value === 'metadata' ? text('模型元信息', 'Model metadata') : text('保守默认', 'Conservative default');
  const limit = b.limiting_constraint === 'question' ? text('单个问题上限', 'Per-question limit') : b.limiting_constraint === 'json_shape' ? text('接口结构上限', 'Interface shape limit') : text('总窗口', 'Request window');
  return <Disclosure label={text('上下文分配', 'Context allocation')} language={language}>
    {b.no_fit_reason && <p className="quality-coverage">{text('本地预算无法容纳完整当前输入与决策问题，已进入回退，未调用决策模型。', 'The complete current input and decision questions did not fit the local budget. Routing fell back without calling the decision model.')}</p>}
    <p>{text('总窗口', 'Request window')} {number(b.request_window_tokens)} ({source(b.request_source)}) · {text('每个问题', 'Per question')} {number(b.question_window_tokens)} ({source(b.question_source)})</p>
    <p>{text('扣除预留后可用', 'Usable after reserve')} {number(b.request_usable_tokens)} / {number(b.question_usable_tokens)} · {text('估算总量 / 最大问题', 'Estimated request / largest question')} {number(b.estimated_request_tokens)} / {number(b.estimated_largest_question_tokens)} · {limit}</p>
    <p>{text('目标步骤保留 / 移除', 'Target steps retained / removed')} {number(b.target_steps_retained)} / {number(b.target_steps_removed)} · {text('背景步骤保留 / 移除', 'Prior steps retained / removed')} {number(b.prior_steps_retained)} / {number(b.prior_steps_removed)}</p>
    <p>{text('重放文本保留 / 省略', 'Replay texts retained / omitted')} {number(b.replay_messages_retained)} / {number(b.replay_messages_omitted)} · {text('已扫描 / 未扫描消息', 'Scanned / unscanned envelopes')} {number(b.replay_envelopes_scanned)} / {number(b.replay_envelopes_unscanned)}</p>
    {(b.target_partial || b.replay_truncated || b.replay_boundary_unavailable || b.replay_budget_omitted) && <p className="quality-coverage">{[b.target_partial && text('目标不完整', 'Partial target'), b.replay_truncated && text('重放内容已截取', 'Replay truncated'), b.replay_boundary_unavailable && text('重放边界不可用', 'Replay boundary unavailable'), b.replay_budget_omitted && text('重放内容因预算省略', 'Replay omitted by budget')].filter(Boolean).join(' · ')}</p>}
    <p className="oc-meta">{text('采用转义后 UTF-8 字节的保守估算，预留仅计算一次。未扫描内容的总量显示为未知。', 'Uses conservative escaped UTF-8 byte accounting with one reserve. Totals for unscanned content remain unknown.')}</p>
  </Disclosure>;
}
