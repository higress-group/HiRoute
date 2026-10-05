import React from 'react';
import { Disclosure, UiIcon } from '../ui';
import { qualityBranchLabel, qualityReasoningLabel } from './plan-quality-state';

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
  const executionRequestId = sample.execution_evidence_available ? sample.last_request_id ?? sample.first_request_id : null;
  const triggerRequestId = assessment?.evidence_available ? assessment.trigger_request_id : null;
  const branch = sample.executed_branch_id ?? 'unrecorded';
  return <article className="quality-row" data-stage-id={sample.segment_id}>
    <div className="quality-row-header">
      <div className="quality-row-main">
        <strong>{modelName}</strong>
        <span>{qualityReasoningLabel(sample.reasoning_profile_id, language)}</span>
        <span>{qualityBranchLabel(branch, language)} · {text('轮次 ' + sample.first_turn_ordinal + '–' + sample.last_observed_turn_ordinal, 'Turns ' + sample.first_turn_ordinal + '–' + sample.last_observed_turn_ordinal)}</span>
      </div>
      <div className="quality-score"><strong>{assessment ? assessment.score.toFixed(2) + ' / 1' : text('未评分', 'Unrated')}</strong><span>{text('阶段胜任度', 'Stage competence')}</span></div>
    </div>
    <div className="quality-stage-meta">{new Date(sample.last_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')} · {text('计划版本 r' + sample.plan_revision, 'Plan r' + sample.plan_revision)}</div>
    <Disclosure className="quality-assessment-details" label={assessment ? text('评分详情', 'Assessment details') : text('阶段详情', 'Stage details')} language={language}>
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
      {assessment && <button className="btn btn-quiet" type="button" disabled={!triggerRequestId || !onOpenEvidence} onClick={() => triggerRequestId && onOpenEvidence?.(sample.session_id, triggerRequestId)}>{triggerRequestId ? text('打开评分触发请求', 'Open assessment-trigger request') : text('评分触发请求不可用', 'Assessment-trigger request unavailable')}</button>}
    </Disclosure>
    <div className="quality-evidence-actions"><button className="btn btn-quiet" type="button" disabled={!executionRequestId || !onOpenEvidence} onClick={() => executionRequestId && onOpenEvidence?.(sample.session_id, executionRequestId)}><UiIcon name="activity" />{executionRequestId ? text('执行证据', 'Execution evidence') : text('执行证据不可用', 'Execution unavailable')}</button></div>
  </article>;
}
