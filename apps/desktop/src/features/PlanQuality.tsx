import React, { useEffect, useRef, useState } from 'react';
import { UiIcon } from '../ui';
import { safeDiagnosticCode } from '../error-code';
import { observationRead } from './observation-client';
import { qualityExecutionModels } from './plan-quality-models';

type Assessment = {
  trigger_request_id: string;
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

export type PlanQualityModel = {
  model_configuration_id: string;
  display_name: string;
};

type Page = { samples: PlanQualitySample[]; next_cursor?: string | null };
type ScoreFilter = 'all' | 'low' | 'high';
type Period = 'all' | 'seven_days';
type VersionScope = 'current' | 'retained';

export function PlanQuality({
  planId,
  sessionId,
  planRevision,
  currentModels = [],
  language,
  compact = false,
  onOpenEvidence,
}: {
  planId?: string | null;
  sessionId?: string | null;
  planRevision?: number | null;
  currentModels?: readonly PlanQualityModel[];
  language: 'zh' | 'en';
  compact?: boolean;
  onOpenEvidence?: (sessionId: string, requestId: string) => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [samples, setSamples] = useState<PlanQualitySample[]>([]);
  const [executionModels, setExecutionModels] = useState<Record<string, string>>({});
  const [cursor, setCursor] = useState<string | null>(null);
  const [score, setScore] = useState<ScoreFilter>('all');
  const [period, setPeriod] = useState<Period>(compact ? 'all' : 'seven_days');
  const [versionScope, setVersionScope] = useState<VersionScope>('current');
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const queryWindow = useRef<{ from?: number; to: number } | null>(null);

  async function load(next: string | null = null) {
    if (!planId && !sessionId) return;
    const epoch = ++generation.current;
    setBusy(true); setError('');
    if (!next || !queryWindow.current) {
      const now = Date.now();
      queryWindow.current = {
        from: period === 'seven_days' ? now - 7 * 86_400_000
          : undefined,
        to: now + 1,
      };
    }
    const window = queryWindow.current;
    try {
      const page = await observationRead<Page>('plan_quality', {
        plan_id: planId || null,
        session_id: sessionId || null,
        plan_revision: planRevision && versionScope === 'current' ? planRevision : null,
        model_configuration_id: null,
        from_ms: window.from,
        to_ms: window.to,
        score_gt: score === 'high' ? 0.5 : null,
        score_lt: score === 'low' ? 0.5 : null,
        limit: 20,
        cursor: next,
      });
      if (epoch !== generation.current) return;
      setSamples(current => next ? [...current, ...page.samples] : page.samples);
      setCursor(page.next_cursor ?? null);
      setLoaded(true);
      const models = await qualityExecutionModels(page.samples, async (session, request) => {
        const result = await observationRead<{ requests: { session_id: string; request_id: string; final_native_model: string | null }[] }>('timeline', {
          session_id: session, request_id: request, from_ms: 0, to_ms: window.to,
          only_model_switch: false, limit: 1, cursor: null,
        });
        return result.requests;
      });
      if (epoch === generation.current) setExecutionModels(current => next ? { ...current, ...models } : models);
    } catch (cause) {
      if (epoch === generation.current) setError(safeDiagnosticCode(cause, 'LOCAL_SERVICE_UNAVAILABLE'));
    } finally {
      if (epoch === generation.current) setBusy(false);
    }
  }

  useEffect(() => {
    setSamples([]); setExecutionModels({}); setCursor(null); setLoaded(false);
    queryWindow.current = null;
    void load();
    return () => { generation.current++; };
  }, [planId, sessionId, planRevision, score, period, versionScope]);

  const scoreText = (sample: PlanQualitySample) => sample.assessment
    ? `${sample.assessment.score.toFixed(2)} / 1`
    : text('未评分', 'Not rated');
  const branchText = (sample: PlanQualitySample) => {
    const branch = sample.executed_branch_id ?? sample.selected_branch_id;
    const label = branch === 'smart_saving_simple' ? text('省钱分支', 'Economy branch')
      : branch === 'smart_saving_complex' ? text('主力分支', 'Primary branch') : text('其他分支', 'Other branch');
    return sample.executed_branch_id ? label : `${label} · ${text('实际分支未记录', 'Execution branch not recorded')}`;
  };
  const currentModelNames = new Map(currentModels.map(model => [model.model_configuration_id, model.display_name]));

  return <div className={`plan-quality${compact ? ' compact' : ''}`}>
    {!compact && currentModels.length > 0 && <div className="quality-model-scope">
      <span className="quality-model-scope-label">{text('当前生效版本模型', 'Models in the active revision')}</span>
      <div className="quality-model-list">{currentModels.map(model => <span className="badge no-dot" key={model.model_configuration_id} title={model.model_configuration_id}>{model.display_name}</span>)}</div>
    </div>}
    {!compact && <div className="quality-filters">
      <label><span className="sr-only">{text('评分筛选', 'Score filter')}</span><select className="select" value={score} onChange={event => setScore(event.target.value as ScoreFilter)}><option value="all">{text('全部评分', 'All scores')}</option><option value="low">{text('胜任度低于 0.5', 'Competence below 0.5')}</option><option value="high">{text('胜任度高于 0.5', 'Competence above 0.5')}</option></select></label>
      <label><span className="sr-only">{text('时间范围', 'Time range')}</span><select className="select" value={period} onChange={event => setPeriod(event.target.value as Period)}><option value="seven_days">{text('最近 7 天', 'Last 7 days')}</option><option value="all">{text('全部保留期', 'All retained')}</option></select></label>
      {planRevision && <label><span className="sr-only">{text('计划版本', 'Plan version')}</span><select className="select" value={versionScope} onChange={event => setVersionScope(event.target.value as VersionScope)}><option value="current">{text(`当前版本 r${planRevision}`, `Current r${planRevision}`)}</option><option value="retained">{text('全部保留版本', 'All retained versions')}</option></select></label>}
    </div>}
    {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('运行表现暂时无法读取。', 'Runtime performance is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => void load()}>{text('重试', 'Retry')}</button></div>}
    {!loaded && busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取模型表现…', 'Reading model performance…')}</p></div>}
    {loaded && !busy && !error && samples.length === 0 && <p className="oc-meta">{text('当前范围还没有模型阶段记录。', 'No model-stage records are available for this scope yet.')}</p>}
    <div className="quality-list">{samples.map(sample => {
      const executionRequestId = sample.execution_evidence_available ? sample.last_request_id ?? sample.first_request_id : null;
      const feedbackRequestId = sample.assessment?.evidence_available ? sample.assessment.trigger_request_id : null;
      const modelName = sample.attribution === 'mixed' ? text('多个执行模型', 'Multiple execution models')
        : sample.attribution === 'unknown' ? text('执行模型未确认', 'Execution model unknown')
        : executionModels[sample.segment_id]
          ?? (sample.model_configuration_id ? currentModelNames.get(sample.model_configuration_id) : null)
          ?? (busy ? text('正在读取模型名称…', 'Reading model name…') : text('模型名称不可用', 'Model name unavailable'));
      return <article className="quality-row" key={sample.segment_id}>
        <div className="quality-row-main"><strong title={sample.model_configuration_id ?? undefined}>{modelName}</strong><span title={sample.executed_branch_id ?? sample.selected_branch_id}>{branchText(sample)} · {text(`轮次 ${sample.first_turn_ordinal}–${sample.last_observed_turn_ordinal}`, `turn ${sample.first_turn_ordinal}–${sample.last_observed_turn_ordinal}`)}</span><span>{new Date(sample.last_at_ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en')} · {text(`计划版本 r${sample.plan_revision}`, `plan r${sample.plan_revision}`)}</span></div>
        <div className="quality-score"><strong>{scoreText(sample)}</strong><span>{text('胜任度', 'Competence')}</span></div>
        <div className="quality-flags">
          {sample.history_partial && <span className="badge warn no-dot" title={text('该阶段的执行历史存在记录缺口；这不等于 Agent 收到的回答不完整。', 'The recorded execution history has gaps; this does not establish that the Agent received an incomplete answer.')}>{text('历史记录不完整', 'Incomplete history')}</span>}
          {sample.assessment?.partial && <span className="badge warn no-dot" title={text('评分依据仅覆盖该阶段的部分证据。', 'The assessment is based on only part of the evidence for this stage.')}>{text('评分证据不完整', 'Partial assessment evidence')}</span>}
          {sample.attribution !== 'single' && <span className="badge warn no-dot">{sample.attribution === 'mixed' ? text('无法归因于单一模型', 'Not attributable to one model') : text('模型归属未确认', 'Model attribution unknown')}</span>}
        </div>
        {!sample.assessment && <p className="quality-coverage">{text('尚未产生该阶段的胜任度评分；后续评估触发后才会显示。', 'No competence assessment has been recorded for this stage. A score appears after a subsequent assessment is triggered.')}</p>}
        {sample.assessment && <p className="quality-coverage">{text(`评分覆盖轮次 ${sample.assessment.target_from_ordinal}–${sample.assessment.target_through_ordinal}`, `Assessed turns ${sample.assessment.target_from_ordinal}–${sample.assessment.target_through_ordinal}`)}</p>}
        {sample.assessment?.reason && <p className="quality-reason">{sample.assessment.reason}</p>}
        <div className="quality-evidence-actions"><button className="btn btn-quiet" type="button" disabled={!executionRequestId || !onOpenEvidence} onClick={() => executionRequestId && onOpenEvidence?.(sample.session_id, executionRequestId)}>{executionRequestId ? text('执行证据', 'Execution evidence') : text('执行证据不可用', 'Execution unavailable')}</button>{sample.assessment && <button className="btn btn-quiet" type="button" disabled={!feedbackRequestId || !onOpenEvidence} onClick={() => feedbackRequestId && onOpenEvidence?.(sample.session_id, feedbackRequestId)}>{feedbackRequestId ? text('评分触发反馈', 'Assessment feedback') : text('反馈不可用', 'Feedback unavailable')}</button>}</div>
      </article>;
    })}</div>
    {cursor && <button className="btn" type="button" disabled={busy} onClick={() => void load(cursor)}>{text('加载更多阶段', 'Load more stages')}</button>}
  </div>;
}
