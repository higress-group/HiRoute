import React from 'react';
import { Disclosure, UiIcon } from '../ui';
import { qualityBranchLabel, qualityGroupLabel, qualityReasoningLabel, type BranchPolicy } from './plan-quality-state';

type Assessment = {
  trigger_request_id: string;
  assessed_at_ms: number;
  target_from_ordinal: number;
  target_through_ordinal: number;
  score: number;
  partial: boolean;
  reason?: string | null;
  evidence_available: boolean;
};

export type PlanQualitySample = {
  selection?: { execution_group: 'regular' | 'primary'; simple_probability: number | null; simple_threshold_millis: number | null; selection_reason: 'simple_task' | 'complex_task' | 'low_competence' | 'degree_unavailable' | 'single_group' | 'decision_fallback' | 'availability_relay' | 'heuristic'; competence_trigger?: { score: number; floor_millis: number; segment_id: string } | null } | null;
  branch_execution?: { policy: BranchPolicy; group: 'regular' | 'primary'; candidate_index: number } | null;
  upgrade?: { decision: { segment_id: string; score: number; floor_millis: number; from_group: 'regular' | 'primary' }; trigger_request_id: string } | null;
  segment_id: string;
  session_id: string;
  plan_id: string;
  plan_revision: number;
  selected_branch_id: string;
  executed_branch_id?: string | null;
  model_configuration_id?: string | null;
  profile_digest?: string | null;
  native_model?: string | null;
  reasoning_profile_id?: string | null;
  attribution: 'single' | 'mixed' | 'unknown';
  first_turn_ordinal: number;
  last_observed_turn_ordinal: number;
  first_at_ms: number;
  last_at_ms: number;
  history_partial: boolean;
  first_request_id?: string | null;
  last_request_id?: string | null;
  execution_evidence_available: boolean;
  assessment?: Assessment | null;
};

export function PlanQualityStage({ sample, modelName, language, onOpenEvidence }: {
  sample: PlanQualitySample;
  modelName: string;
  language: 'zh' | 'en';
  onOpenEvidence?: (sessionId: string, requestId: string) => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const assessment = sample.assessment;
  const reliable = assessment && !assessment.partial;
  const executionRequestId = sample.execution_evidence_available ? sample.last_request_id ?? sample.first_request_id : null;
  const triggerRequestId = assessment?.evidence_available ? assessment.trigger_request_id : null;
  const branch = sample.executed_branch_id ?? 'unrecorded';
  return <article className="quality-row" data-stage-id={sample.segment_id}>
    <div className="quality-row-header">
      <div className="quality-row-main">
        <strong>{modelName}</strong>
        <span>{qualityReasoningLabel(sample.reasoning_profile_id, language)}</span>
        <span>{branch === 'smart_saving' ? qualityGroupLabel(branch, sample.branch_execution?.group, language) : sample.branch_execution?.policy.name ?? qualityBranchLabel(branch, language)} · {text('轮次 ' + sample.first_turn_ordinal + '–' + sample.last_observed_turn_ordinal, 'Turns ' + sample.first_turn_ordinal + '–' + sample.last_observed_turn_ordinal)}</span>
      </div>
      <div className="quality-score"><strong>{reliable ? assessment.score.toFixed(2) + ' / 1' : text('未评分', 'Unrated')}</strong><span>{text('胜任度', 'Competence')}</span></div>
    </div>
    <div className="quality-stage-meta">{new Date(sample.last_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')} · {text('计划版本 r' + sample.plan_revision, 'Plan r' + sample.plan_revision)}</div>
    {sample.branch_execution && <p className="quality-coverage">{text('实际执行', 'Actual execution')} · {qualityGroupLabel(branch, sample.branch_execution.group, language)} · {text('候选 ', 'Candidate ')}{sample.branch_execution.candidate_index + 1} · {text('当时胜任下限 ', 'Execution competence floor ')}{sample.branch_execution.policy.floor_millis / 1000}</p>}
    {sample.upgrade && <div className="callout"><UiIcon name="route" /><span>{text(`已触发主力保护 · 当时评分 ${sample.upgrade.decision.score} 低于下限 ${sample.upgrade.decision.floor_millis / 1000}`, `Triggered primary protection · score ${sample.upgrade.decision.score} below floor ${sample.upgrade.decision.floor_millis / 1000}`)}</span></div>}
    <Disclosure className="quality-assessment-details" label={assessment ? text('查看评分依据', 'View assessment evidence') : text('阶段详情', 'Stage details')} language={language}>
      {sample.selection && <div className="quality-selection">
        <strong>{text('阶段开始时的模型选择', 'Model selection at the start of this stage')}</strong>
        <p>{({ simple_task: text('简单任务', 'Simple task'), complex_task: text('复杂任务', 'Complex task'), low_competence: text('上一阶段低分保护', 'Protection after a low stage score'), degree_unavailable: text('程度判断失败', 'Degree decision unavailable'), single_group: text('本分支仅常规组', 'Regular group only'), decision_fallback: text('决策失败兜底', 'Decision failure fallback'), availability_relay: text('沿用本轮故障接力', 'Continue this turn’s fallback'), heuristic: text('启发式规则', 'Heuristic rules') })[sample.selection.selection_reason]} → {qualityGroupLabel(branch, sample.selection.execution_group, language)}</p>
        {sample.selection.simple_probability != null && <p>{text('本次简单概率', 'Simple probability')} {sample.selection.simple_probability.toFixed(3)} · {text('当时阈值', 'Threshold')} {sample.selection.simple_threshold_millis == null ? '—' : sample.selection.simple_threshold_millis / 1000}</p>}
        {sample.selection.competence_trigger && <p>{text('触发评分', 'Trigger score')} {sample.selection.competence_trigger.score} &lt; {sample.selection.competence_trigger.floor_millis / 1000}</p>}
        {sample.branch_execution && sample.selection.execution_group !== sample.branch_execution.group && <p>{text('经候选故障接力后执行', 'Executed after candidate failover')} · {qualityGroupLabel(branch, sample.branch_execution.group, language)} · {text('候选 ', 'Candidate ')}{sample.branch_execution.candidate_index + 1}</p>}
        {sample.first_request_id && sample.execution_evidence_available && <button type="button" className="btn btn-quiet" disabled={!onOpenEvidence} onClick={() => onOpenEvidence?.(sample.session_id, sample.first_request_id!)}>{text('查看选择请求', 'View selection request')}</button>}
      </div>}
      {!assessment && <p className="quality-coverage">{text('尚未产生该阶段的胜任度评分；后续评估触发后才会显示。', 'No competence assessment has been recorded for this stage. A score appears after a subsequent assessment is triggered.')}</p>}
      {assessment && <>
        <p className="quality-coverage">{text('评分覆盖轮次 ' + assessment.target_from_ordinal + '–' + assessment.target_through_ordinal, 'Assessed turns ' + assessment.target_from_ordinal + '–' + assessment.target_through_ordinal)}</p>
        <p className="quality-coverage">{text('原始评分', 'Raw score')} · {assessment.score}{assessment.assessed_at_ms != null && <> · {new Date(assessment.assessed_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')}</>}</p>
        {assessment.reason && <p className="quality-reason">{assessment.reason}</p>}
      </>}
      <div className="quality-flags">
        {sample.history_partial && <span className="badge warn no-dot">{text('历史记录不完整', 'Incomplete history')}</span>}
        {assessment?.partial && <span className="badge warn no-dot">{text('评分证据不完整', 'Partial assessment evidence')}</span>}
        {sample.attribution !== 'single' && <span className="badge warn no-dot">{sample.attribution === 'mixed' ? text('无法归因于单一模型', 'Not attributable to one model') : text('模型归属未确认', 'Model attribution unknown')}</span>}
      </div>
      <p className="quality-coverage">{text('阶段评分用于观察该阶段的执行表现。', 'The score describes execution of this stage.')}</p>
      <div className="quality-diagnostic-identity">
        {sample.executed_branch_id ?? sample.selected_branch_id} · {sample.model_configuration_id ?? 'unknown'} · {sample.profile_digest ?? 'unknown'}
        {sample.native_model && sample.native_model !== modelName && <div>{text('原始模型标识', 'Original model ID')} · {sample.native_model}</div>}
      </div>
      {assessment && <button className="btn btn-quiet" type="button" disabled={!triggerRequestId || !onOpenEvidence} onClick={() => triggerRequestId && onOpenEvidence?.(sample.session_id, triggerRequestId)}>{triggerRequestId ? text('查看后续反馈请求', 'View feedback request') : text('评分触发请求不可用', 'Assessment-trigger request unavailable')}</button>}
    </Disclosure>
    <div className="quality-evidence-actions"><button className="btn btn-quiet" type="button" disabled={!executionRequestId || !onOpenEvidence} onClick={() => executionRequestId && onOpenEvidence?.(sample.session_id, executionRequestId)}><UiIcon name="activity" />{executionRequestId ? text('查看执行过程', 'View execution') : text('执行证据不可用', 'Execution unavailable')}</button></div>
  </article>;
}
