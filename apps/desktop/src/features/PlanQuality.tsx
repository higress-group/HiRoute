import React, { useEffect, useMemo, useRef, useState } from 'react';
import { UiIcon } from '../ui';
import { safeDiagnosticCode } from '../error-code';
import { observationRead } from './observation-client';
import { qualityExecutionModels } from './plan-quality-models';
import { PlanQualityStage, type PlanQualitySample } from './PlanQualityStage';
import { qualityBranchLabel, qualityModelRows, qualityReasoningLabel, type PlanQualityModel, type QualityExecution, type QualityModelRow, type QualitySummary } from './plan-quality-state';

export type { PlanQualityModel } from './plan-quality-state';
export type { PlanQualitySample } from './PlanQualityStage';

type Page = { samples: PlanQualitySample[]; summary: QualitySummary; next_cursor?: string | null };
type ScoreFilter = 'all' | 'low' | 'high' | 'unrated';
type Period = 'all' | 'seven_days';
const EMPTY_SUMMARY: QualitySummary = { models: [], scored_stage_count: 0, unrated_stage_count: 0, session_count: 0, available_revisions: [] };

export function PlanQuality({
  planId, sessionId, planRevision, currentModels = [], language, compact = false,
  active = true, refreshVersion = 0, onOpenEvidence,
}: {
  planId?: string | null;
  sessionId?: string | null;
  planRevision?: number | null;
  currentModels?: readonly PlanQualityModel[];
  language: 'zh' | 'en';
  compact?: boolean;
  active?: boolean;
  refreshVersion?: number;
  onOpenEvidence?: (sessionId: string, requestId: string) => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [samples, setSamples] = useState<PlanQualitySample[]>([]);
  const [summary, setSummary] = useState<QualitySummary>(EMPTY_SUMMARY);
  const [executionModels, setExecutionModels] = useState<Record<string, string>>({});
  const [cursor, setCursor] = useState<string | null>(null);
  const [score, setScore] = useState<ScoreFilter>('all');
  const [period, setPeriod] = useState<Period>(compact ? 'all' : 'seven_days');
  const [version, setVersion] = useState('current');
  const [selected, setSelected] = useState<{ key: string; execution: QualityExecution | null } | null>(null);
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  // A detail filter or next page must retain exactly the same time window.
  const window = useMemo(() => {
    const now = Date.now();
    return { from: period === 'seven_days' ? now - 7 * 86_400_000 : undefined, to: now + 1 };
  }, [planId, sessionId, planRevision, period, version, refreshVersion, reload, active]);
  const revision = version === 'current' ? planRevision || null : version === 'all' ? null : Number(version);

  async function load(next: string | null = null) {
    if (!active || (!planId && !sessionId)) return;
    const epoch = ++generation.current;
    setBusy(true); setError('');
    try {
      const page = await observationRead<Page>('plan_quality', {
        plan_id: planId || null, session_id: sessionId || null, plan_revision: revision,
        execution: selected?.execution ?? null,
        from_ms: window.from, to_ms: window.to,
        score_gt: score === 'high' ? 0.5 : null,
        score_lt: score === 'low' ? 0.5 : null,
        unrated_only: score === 'unrated', limit: 20, cursor: next,
      });
      if (epoch !== generation.current) return;
      setSummary(page.summary);
      setSamples(current => next ? [...current, ...page.samples] : page.samples);
      setCursor(page.next_cursor ?? null);
      setLoaded(true);
      // Historical records can lack the native-name projection. Recover only
      // from an exact request in the same session, never a neighbouring request.
      const models = await qualityExecutionModels(page.samples.filter(sample => !sample.native_model), async (session, request) => {
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
    setSummary(EMPTY_SUMMARY);
  }, [planId, sessionId, revision, window]);

  useEffect(() => {
    if (!active) { generation.current++; setBusy(false); return; }
    setSamples([]); setExecutionModels({}); setCursor(null); setLoaded(false);
    if (selected && !selected.execution) { setLoaded(true); return; }
    void load();
    return () => { generation.current++; };
  }, [planId, sessionId, revision, score, selected?.key, window, active]);

  const rows = qualityModelRows(revision === planRevision ? currentModels : [], summary.models);
  const groups = [...new Set(rows.map(row => row.branch))];
  const selectedRow = rows.find(row => row.key === selected?.key);
  const rowName = (row: QualityModelRow) => row.summary?.execution.attribution === 'mixed' ? text('多个执行模型', 'Multiple execution models')
    : row.summary?.execution.attribution === 'unknown' ? text('执行模型未确认', 'Execution model unknown')
    : row.configured?.display_name ?? row.summary?.native_model ?? text('模型名称不可用', 'Model name unavailable');
  const modelNames = new Map(currentModels.map(model => [model.model_configuration_id, model.display_name]));
  const sampleName = (sample: PlanQualitySample) => sample.attribution === 'mixed' ? text('多个执行模型', 'Multiple execution models')
    : sample.attribution === 'unknown' ? text('执行模型未确认', 'Execution model unknown')
    : sample.native_model ?? executionModels[sample.segment_id]
      ?? (sample.model_configuration_id ? modelNames.get(sample.model_configuration_id) : null)
      ?? text('模型名称不可用', 'Model name unavailable');
  const resetDetail = () => { setSelected(null); setScore('all'); };
  const stageCount = summary.scored_stage_count + summary.unrated_stage_count;
  const filter = <label><span className="sr-only">{text('阶段评分筛选', 'Stage score filter')}</span><select className="select quality-score-filter" value={score} onChange={event => setScore(event.target.value as ScoreFilter)}>
    <option value="all">{text('全部阶段', 'All stages')}</option>
    <option value="low">{text('胜任度低于 0.5', 'Competence below 0.5')}</option>
    <option value="high">{text('胜任度高于 0.5', 'Competence above 0.5')}</option>
    <option value="unrated">{text('待评分', 'Unrated')}</option>
  </select></label>;

  return <div className={'plan-quality' + (compact ? ' compact' : '')}>
    <div className="quality-toolbar">
      <div className="quality-scope-meta">{text(stageCount + ' 个执行阶段 · ' + summary.scored_stage_count + ' 已评分 · ' + summary.unrated_stage_count + ' 待评分', stageCount + ' stages · ' + summary.scored_stage_count + ' scored · ' + summary.unrated_stage_count + ' unrated')}</div>
      <div className="quality-filters">
        {!compact && <label><span className="sr-only">{text('时间范围', 'Time range')}</span><select className="select" value={period} onChange={event => { resetDetail(); setPeriod(event.target.value as Period); }}>
          <option value="seven_days">{text('最近 7 天', 'Last 7 days')}</option><option value="all">{text('全部保留期', 'All retained')}</option>
        </select></label>}
        {!compact && planRevision && <label><span className="sr-only">{text('计划版本', 'Plan version')}</span><select className="select" value={version} onChange={event => { resetDetail(); setVersion(event.target.value); }}>
          <option value="current">{text('当前生效 r' + planRevision, 'Active r' + planRevision)}</option>
          {summary.available_revisions.filter(value => value !== planRevision).map(value => <option key={value} value={String(value)}>{text('历史版本 r' + value, 'Revision r' + value)}</option>)}
          <option value="all">{text('全部保留版本', 'All retained revisions')}</option>
        </select></label>}
        <button className="btn btn-quiet quality-refresh" type="button" disabled={busy} onClick={() => setReload(value => value + 1)}><UiIcon name="refresh" />{text('刷新', 'Refresh')}</button>
      </div>
    </div>
    {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('模型表现暂时无法读取。', 'Model performance is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => setReload(value => value + 1)}>{text('重试', 'Retry')}</button></div>}
    {!loaded && busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取模型表现…', 'Reading model performance…')}</p></div>}
    {!compact && <div className="quality-model-scope">{groups.map(branch => <section className="quality-group" key={branch} data-execution-group={branch}>
      <header className="quality-group-heading"><h4>{qualityBranchLabel(branch, language)}</h4><span>{text(rows.filter(row => row.branch === branch).length + ' 个模型配置', rows.filter(row => row.branch === branch).length + ' model configurations')}</span></header>
      <div className="quality-model-columns" aria-hidden="true"><span>{text('模型 / 思考设置', 'Model / Reasoning')}</span><span>{text('平均阶段胜任度', 'Average stage competence')}</span><span>{text('阶段样本', 'Stage samples')}</span><span /></div>
      {rows.filter(row => row.branch === branch).map(row => <div className={'quality-model-row' + (selected?.key === row.key ? ' selected' : '')} key={row.key} data-model-configuration={row.configured?.model_configuration_id ?? row.summary?.execution.model_configuration_id ?? ''}>
        <div className="quality-model-name"><strong>{rowName(row)}</strong><span>{qualityReasoningLabel(row.summary ? row.summary.reasoning_profile_id : row.configured?.reasoning_profile_id, language)}{version === 'all' && row.summary && ' · r' + row.summary.execution.plan_revision}</span></div>
        <div className="quality-model-average">{row.summary?.average_score != null ? <><strong>{row.summary.average_score.toFixed(2)} <small>/ 1</small></strong><div className="quality-score-bar" aria-hidden="true"><span style={{ width: (row.summary.average_score * 100) + '%' }} /></div></> : <span>{row.summary ? text('待评分', 'Unrated') : text('暂无记录', 'No records')}</span>}</div>
        <div className="quality-model-counts"><span>{text((row.summary?.scored_stage_count ?? 0) + ' 已评分', (row.summary?.scored_stage_count ?? 0) + ' scored')}</span><span>{text((row.summary?.unrated_stage_count ?? 0) + ' 待评分', (row.summary?.unrated_stage_count ?? 0) + ' unrated')}</span></div>
        <button type="button" className="btn btn-quiet quality-view-stages" aria-expanded={selected?.key === row.key} onClick={() => { setScore('all'); setSelected(selected?.key === row.key ? null : { key: row.key, execution: row.summary?.execution ?? null }); }}>{selected?.key === row.key ? text('收起阶段', 'Hide stages') : text('查看阶段', 'View stages')}<UiIcon name="chevronRight" /></button>
      </div>)}
    </section>)}</div>}
    {!compact && loaded && groups.length > 0 && <p className="quality-average-note">{text('平均值使用当前范围内各已评分阶段的最新原始评分，待评分阶段不计入平均。', 'Averages use the latest raw assessment of every scored stage in this scope. Unrated stages are excluded.')}</p>}
    {!compact && loaded && !error && groups.length === 0 && <p className="oc-meta">{text('当前范围还没有模型阶段记录。', 'No model-stage records are available for this scope yet.')}</p>}
    {(compact || selected) && <section className="quality-stage-section">
      {!compact && <div className="quality-stage-heading"><div><h4>{selectedRow ? rowName(selectedRow) : text('执行阶段', 'Execution stages')}</h4><span>{text('按实际执行模型与推理配置查看', 'Stages for the actual execution and reasoning configuration')}</span></div>{filter}</div>}
      {compact && stageCount > 20 && <div className="quality-filters">{filter}</div>}
      {loaded && !busy && !error && samples.length === 0 && <p className="oc-meta">{text('没有符合筛选条件的执行阶段。', 'No execution stages match this filter.')}</p>}
      <div className="quality-list">{samples.map(sample => <PlanQualityStage key={sample.segment_id} sample={sample} modelName={sampleName(sample)} language={language} onOpenEvidence={onOpenEvidence} />)}</div>
      {cursor && <button className="btn quality-more" type="button" disabled={busy} onClick={() => void load(cursor)}>{text('加载更多阶段', 'Load more stages')}</button>}
    </section>}
  </div>;
}
