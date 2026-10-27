import { ExecutorOutcomes, TrustScopeNotes, TrustSessions, TrustSuggestions, TrustTiles } from './trust/TrustEvidencePanel';
import { TrustCost } from './trust/TrustCost';
import { usePlanTrust, type TrustGrouping } from './trust/use-plan-trust';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { TrustCounts, TrustReport, TrustRow } from './trust/types';
import { UiIcon } from '../ui';
import { ContentExcerpt } from './Sessions';
import { safeDiagnosticCode } from '../error-code';
import { observationRead } from './observation-client';
import { qualityExecutionModels } from './plan-quality-models';
import { planQualityWindow } from './plan-quality-window';
import { PlanQualityStage, type PlanQualitySample } from './PlanQualityStage';
import { RoutingObservations, type RoutingSummary } from './RoutingObservations';
import { releasedProtectionEvidence, type ReleasedProtectionEvidence } from './trust/protection-evidence';
import { qualityRowSection, qualityRowSectionLabel, qualityGroupLabel, qualityModelRows, qualityNativeModelName, qualityReasoningLabel, type PlanQualityModel, type QualityExecution, type QualityModelRow, type QualitySummary } from './plan-quality-state';

export type { PlanQualityModel } from './plan-quality-state';
export type { PlanQualitySample } from './PlanQualityStage';

type Page = { samples: PlanQualitySample[]; summary: QualitySummary; routing_observations?: RoutingSummary; next_cursor?: string | null };
type ScoreFilter = 'all' | 'low' | 'high' | 'unrated';
type Period = 'all' | 'seven_days' | 'one_day';
const EMPTY_SUMMARY: QualitySummary = { models: [], scored_stage_count: 0, unrated_stage_count: 0, session_count: 0, available_revisions: [] };

export function PlanQuality({
  planId, sessionId, planRevision, currentModels = [], language, compact = false,
  active = true, refreshVersion = 0, onOpenEvidence, onThreshold, onDivision, divisionBranches, onOpenAgent,
}: {
  planId?: string | null;
  sessionId?: string | null;
  planRevision?: number | null;
  currentModels?: readonly PlanQualityModel[];
  language: 'zh' | 'en';
  compact?: boolean;
  active?: boolean;
  refreshVersion?: number;
  onThreshold?: (branch: string, threshold: number) => void;
  onDivision?: (branch: string) => void; onOpenAgent?: (agent: string) => void;
  divisionBranches?: readonly string[];
  onOpenEvidence?: (sessionId: string, requestId: string) => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [samples, setSamples] = useState<PlanQualitySample[]>([]);
  const [summary, setSummary] = useState<QualitySummary>(EMPTY_SUMMARY);
  const [routingSummary, setRoutingSummary] = useState<RoutingSummary | null>(null);
  const [executionModels, setExecutionModels] = useState<Record<string, string>>({});
  const [cursor, setCursor] = useState<string | null>(null);
  const [score, setScore] = useState<ScoreFilter>('all');
  const [period, setPeriod] = useState<Period>(compact ? 'all' : 'seven_days');
  const [version, setVersion] = useState('current');
  const [selected, setSelected] = useState<{ key: string; execution: QualityExecution | null; trustRow?: { branch_id: string; group: TrustRow["group"]; model_configuration_id: string | null; reasoning_profile_id: string | null; attribution: TrustRow["attribution"]; executor: string | null; group_by_executor: boolean }; label?: string } | null>(null);
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const [groupBy, setGroupBy] = useState<TrustGrouping>('model');
  const generation = useRef(0);
  // A detail filter or next page must retain exactly the same time window.
  const window = useMemo(() => {
    return planQualityWindow(period, Date.now());
  }, [planId, sessionId, planRevision, period, version, refreshVersion, reload, active, groupBy]);
  const trustScope = !compact && Boolean(planId) && !sessionId;
  // A supplied revision pins the loaded Plan. Other entries resolve active once
  // through the trust report; an unresolved active scope never means all versions.
  const trust = usePlanTrust({ planId: trustScope ? planId : null,
    revision: version === 'current' ? planRevision ?? 'active' : version === 'all' ? 'all' : Number(version),
    fromMs: window.from, toMs: window.to, active, groupBy, reload });
  const report = trust.report;
  const activeRevision = planRevision ?? report?.active_revision ?? null;
  const revision = version === 'current' ? activeRevision : version === 'all' ? null : Number(version);
  const revisionReady = !trustScope || version !== 'current' || revision !== null;
  const readScope = useMemo(() => ({ window, revision }), [window, revision]);
  const pairedScope = useRef<typeof readScope | null>(null);

  async function load(next: string | null = null, captureRouting = false) {
    if (!active || !revisionReady || (!planId && !sessionId)) return;
    const epoch = ++generation.current;
    if (captureRouting) pairedScope.current = readScope;
    setBusy(true); setError('');
    try {
      const page = await observationRead<Page>('plan_quality', {
        include_routing_observations: true,
        plan_id: planId || null, session_id: sessionId || null, plan_revision: revision,
        execution: selected?.execution ?? null,
        trust_row: selected?.trustRow ?? null,
        from_ms: window.from, to_ms: window.to,
        competence: score === 'high' ? 'meets_floor' : score === 'low' ? 'below_floor' : null,
        unrated_only: score === 'unrated', limit: 20, cursor: next,
      });
      // Aggregate routing evidence belongs to the base read paired with PlanTrust.
      // Detail reads can be clipped by a later retention clock and must not replace it.
      if (captureRouting && pairedScope.current === readScope) setRoutingSummary(page.routing_observations ?? null);
      if (epoch !== generation.current) return;
      setSummary(page.summary);
      const stages = selected && !selected.execution && !selected.trustRow ? [] : page.samples;
      setSamples(current => next ? [...current, ...stages] : stages);
      setCursor(selected && !selected.execution && !selected.trustRow ? null : page.next_cursor ?? null);
      setLoaded(true);
      // Historical records can lack the native-name projection. Recover only
      // from an exact request in the same session, never a neighbouring request.
      const models = await qualityExecutionModels(stages.filter(sample => !sample.native_model), async (session, request) => {
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
    setRoutingSummary(null);
    pairedScope.current = null;
    return () => { pairedScope.current = null; };
  }, [planId, sessionId, readScope]);

  useEffect(() => {
    if (!active || !revisionReady) { generation.current++; setBusy(false); setLoaded(false); return; }
    setSamples([]); setExecutionModels({}); setCursor(null); setLoaded(false);
    void load(null, pairedScope.current !== readScope);
    return () => { generation.current++; };
  }, [planId, sessionId, readScope, revisionReady, score, selected?.key, active]);

  const rows = qualityModelRows(revision !== null && revision === planRevision ? currentModels : [], summary.models);
  const groups = [...new Set(rows.map(qualityRowSection))];
  const selectedRow = rows.find(row => row.key === selected?.key);
  const rowName = (row: QualityModelRow) => row.summary?.execution.attribution === 'mixed' ? text('多个执行模型', 'Multiple execution models')
    : row.summary?.execution.attribution === 'unknown' ? text('执行模型未确认', 'Execution model unknown')
    : row.configured?.display_name ?? qualityNativeModelName(row.summary?.native_model ?? text('模型名称不可用', 'Model name unavailable'));
  const modelNames = new Map(currentModels.map(model => [model.model_configuration_id, model.display_name]));
  const sampleName = (sample: PlanQualitySample) => sample.attribution === 'mixed' ? text('多个执行模型', 'Multiple execution models')
    : sample.attribution === 'unknown' ? text('执行模型未确认', 'Execution model unknown')
    : qualityNativeModelName(sample.native_model ?? executionModels[sample.segment_id]
      ?? (sample.model_configuration_id ? modelNames.get(sample.model_configuration_id) : null)
      ?? text('模型名称不可用', 'Model name unavailable'));
  const resetDetail = () => { setSelected(null); setScore('all'); };
  const stageCount = summary.scored_stage_count + summary.unrated_stage_count;
  const filter = <label><span className="sr-only">{text('阶段评分筛选', 'Stage score filter')}</span><select className="select quality-score-filter" value={score} onChange={event => setScore(event.target.value as ScoreFilter)}>
    <option value="all">{text('全部阶段', 'All stages')}</option>
    <option value="low">{text('低于当时胜任下限', 'Below the execution competence floor')}</option>
    <option value="high">{text('达到当时胜任下限', 'Meets the execution competence floor')}</option>
    <option value="unrated">{text('未评分', 'Unrated')}</option>
  </select></label>;

  const manualDivisionBranches = divisionBranches ?? report?.branches.map(branch => branch.branch_id) ?? [];
  const percent = (value: number | null | undefined) => value == null ? '—' : `${Math.round(value * 100)}%`;
  const seconds = (ms: number | null | undefined) => ms == null ? '—' : text(`${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} 秒`, `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`);
  const groupOf = (row: QualityModelRow) => row.summary?.execution.group ?? row.configured?.group ?? null;
  // Trust rows aggregate a model across the scope; join them only where one table row owns that identity.
  const trustRow = (row: QualityModelRow) => {
    if (!report || version === 'all' || (row.summary && row.summary.execution.attribution !== 'single')) return null;
    const model = row.summary?.execution.model_configuration_id ?? row.configured?.model_configuration_id ?? null;
    const reasoning = row.summary ? row.summary.reasoning_profile_id : row.configured?.reasoning_profile_id ?? null;
    const matches = report.rows.filter(item => item.attribution === 'single' && item.branch_id === row.branch && item.group === groupOf(row) && item.model_configuration_id === model && item.reasoning_profile_id === reasoning);
    return matches.length === 1 ? matches[0] : null;
  };
  const selectTrustRow = (row: TrustRow) => {
    const value = { branch_id: row.branch_id, group: row.group, model_configuration_id: row.model_configuration_id, reasoning_profile_id: row.reasoning_profile_id, attribution: row.attribution, executor: row.executor, group_by_executor: true };
    setSelected(selected?.key === JSON.stringify(value) ? null : { key: JSON.stringify(value), execution: null, trustRow: value, label: modelNames.get(row.model_configuration_id ?? '') ?? text('所选执行组', 'Selected execution group') }); setScore('all');
  };
  const regularRow = rows.find(row => groupOf(row) === 'regular' && row.summary);
  const showLowStages = regularRow && (() => { setSelected({ key: regularRow.key, execution: regularRow.summary!.execution }); setScore('low'); });
  const triggers = useMemo(() => new Set(report?.rows.flatMap(row => row.protections.map(item => item.trigger_segment_id)) ?? []), [report]);
  const [caught, setCaught] = useState<PlanQualitySample[] | null>(null);
  const regularExecution = regularRow?.summary?.execution ?? null;
  const regularKey = regularRow?.key ?? '';
  // The low-score stages that handed the next agent turn to Primary, newest first.
  useEffect(() => {
    setCaught(null);
    if (!active || !planId || !report || groupBy !== 'model' || !regularExecution || !triggers.size) return;
    let current = true;
    observationRead<Page>('plan_quality', { plan_id: planId, session_id: null, plan_revision: revision, execution: regularExecution, trust_row: null, from_ms: window.from, to_ms: window.to, competence: 'below_floor', unrated_only: false, limit: 20, cursor: null })
      .then(page => { if (current) setCaught(page.samples.filter(sample => triggers.has(sample.segment_id)).slice(0, 4)); })
      .catch(() => { if (current) setCaught([]); });
    return () => { current = false; };
  }, [active, planId, report, groupBy, regularKey, revision, window, triggers]);
  const trustRowName = (row: TrustRow) => row.attribution !== 'single' ? text(row.attribution === 'mixed' ? '多个执行模型' : '执行模型未确认', row.attribution === 'mixed' ? 'Multiple execution models' : 'Execution model unknown') : modelNames.get(row.model_configuration_id ?? '') ?? row.model_configuration_id ?? text('未知模型', 'Unknown model');
  const groupTone = (group: 'regular' | 'primary' | null) => group === 'regular' ? 'cyan' : group === 'primary' ? 'purple' : '';
  // Partial, awaiting and unscored stages stay distinct: none of them counts as complete evidence.
  const stageStates = (counts: TrustCounts) => <span className="quality-stage-states">{([
    ['complete', counts.complete, '完整评分', 'complete'], ['partial', counts.partial, '部分评分', 'partial'],
    ['awaiting', counts.awaiting_assessment, '等待评分', 'awaiting assessment'], ['unscored', counts.unscored, '未评分', 'unscored'],
  ] as const).map(([state, value, zh, en]) => <span key={state} data-state={state}>{text(`${value} ${zh}`, `${value} ${en}`)}</span>)}</span>;
  const columns = <div className="quality-table-head" aria-hidden="true"><span>{text('模型 / 思考设置', 'Model / reasoning')}</span><span>{text('平均阶段胜任度', 'Average stage competence')}</span><span>{text('胜任率', 'Competence rate')}</span><span>{text('阶段占比', 'Stage share')}</span><span>{text('阶段样本', 'Stage samples')}</span><span>{text('耗时中位数', 'Median duration')}</span><span /></div>;
  const stageSection = (compact || selected) && <section className="quality-stage-section">
    {!compact && <div className="quality-stage-heading"><div><h4>{selected?.label ?? (selectedRow ? rowName(selectedRow) : text('执行阶段', 'Execution stages'))}</h4><span>{text('按实际执行模型与推理配置查看', 'Stages for the actual execution and reasoning configuration')}</span></div><div className="quality-filters">{filter}<button type="button" className="btn btn-quiet" onClick={resetDetail}>{text('收起', 'Close')}</button></div></div>}
    {compact && stageCount > 20 && <div className="quality-filters">{filter}</div>}
    {loaded && !busy && !error && samples.length === 0 && <p className="oc-meta">{text('没有符合筛选条件的执行阶段。', 'No execution stages match this filter.')}</p>}
    <div className="quality-list">{samples.map(sample => <PlanQualityStage key={sample.segment_id} sample={sample} modelName={sampleName(sample)} activeRevision={activeRevision} language={language} onOpenEvidence={onOpenEvidence} />)}</div>
    {cursor && <button className="btn quality-more" type="button" disabled={busy} onClick={() => void load(cursor)}>{text('加载更多阶段', 'Load more stages')}</button>}
  </section>;

  return <div className={'plan-quality' + (compact ? ' compact' : '')}>
    <div className="quality-toolbar">
      <div className="quality-scope-meta">{text(`${stageCount} 个执行阶段 · ${summary.scored_stage_count} 已评分 · ${summary.unrated_stage_count} 未评分${report ? ` · 证据来自 ${report.session_total} 个会话` : ''}`, `${stageCount} stages · ${summary.scored_stage_count} scored · ${summary.unrated_stage_count} unrated${report ? ` · evidence from ${report.session_total} sessions` : ''}`)}</div>
      <div className="quality-filters">
        {!compact && <label><span className="sr-only">{text('时间范围', 'Time range')}</span><select className="select" value={period} onChange={event => { resetDetail(); setPeriod(event.target.value as Period); }}>
          <option value="seven_days">{text('最近 7 天', 'Last 7 days')}</option><option value="one_day">{text('最近 24 小时', 'Last 24 hours')}</option><option value="all">{text('全部保留期', 'All retained')}</option>
        </select></label>}
        {!compact && planId && <label><span className="sr-only">{text('路由版本', 'Route version')}</span><select className="select" value={version} onChange={event => { resetDetail(); setVersion(event.target.value); }}>
          <option value="current">{text('当前生效版本', 'Active version')}</option>
          {summary.available_revisions.filter(value => value !== activeRevision).map(value => <option key={value} value={String(value)}>{text('历史版本 ' + value, 'Earlier version ' + value)}</option>)}
          <option value="all">{text('全部保留版本', 'All retained versions')}</option>
        </select></label>}
        <button className="btn btn-quiet quality-refresh" type="button" disabled={busy} onClick={() => setReload(value => value + 1)}><UiIcon name="refresh" />{text('刷新', 'Refresh')}</button>
      </div>
    </div>
    {trust.error && <div className="callout warn" role="alert" data-error-code={trust.error}><span>{trust.error.toLowerCase().includes('scope_too_large') ? text('证据超过读取上限。建议需要本版本的全部保留记录；缩短显示时间不一定能减少建议计算范围。', 'Evidence exceeds the read limit. Suggestions need the full retained version; narrowing the displayed window may not reduce that scope.') : text('证据暂时无法读取，当前档位未改变。', 'Evidence is unavailable. Your trust level has not changed.')}</span><button className="btn" type="button" onClick={() => setReload(value => value + 1)}>{text('重试', 'Retry')}</button></div>}
    {trustScope && !report && !trust.error && <p className="quality-status" role="status">{text('正在读取完整范围的证据…', 'Reading the complete evidence scope…')}</p>}
    {report && <TrustSuggestions report={report} language={language} onThreshold={onThreshold} onDivision={onDivision} onOpenAgent={onOpenAgent} />}
    {report && groupBy === 'model' && <TrustTiles report={report} language={language} />}
    {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{text('模型表现暂时无法读取。', 'Model performance is temporarily unavailable.')}</span><button className="btn" type="button" onClick={() => setReload(value => value + 1)}>{text('重试', 'Retry')}</button></div>}
    {!loaded && busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取模型表现…', 'Reading model performance…')}</p></div>}
    <RoutingObservations key={planId ?? sessionId ?? ''} summary={routingSummary} language={language} />
    {!compact && <section className="quality-tables">
      <div className="quality-tables-head"><h3>{text('各模型的表现', 'Performance by model')}</h3>
        {trustScope && <div className="segmented" role="group" aria-label={text('观察分组', 'Evidence grouping')}>{([['model', text('按模型', 'By model')], ['executor', text('按执行 Agent', 'By execution agent')]] as const).map(([value, label]) => <button key={value} type="button" className={`segment${groupBy === value ? ' active' : ''}`} aria-pressed={groupBy === value} onClick={() => { resetDetail(); setGroupBy(value); }}>{label}</button>)}</div>}
      </div>
      <div className="quality-model-scope">
      {groupBy === 'model' && groups.map(branch => {
        const members = rows.filter(row => qualityRowSection(row) === branch);
        const names = [...new Set(members.map(rowName))];
        return <section className="quality-group" key={branch} data-execution-group={branch}>
          <header className="quality-group-heading"><h4>{qualityRowSectionLabel(members[0], language)}{names.length === 1 ? ` → ${names[0]}` : ''}</h4><span>{text(members.length + ' 个模型配置', members.length + (members.length === 1 ? ' model configuration' : ' model configurations'))}</span></header>
          <div className="quality-table">{columns}{members.map(row => { const evidence = trustRow(row); const tone = groupTone(groupOf(row)); return <div className={'quality-model-row' + (selected?.key === row.key ? ' selected' : '')} key={row.key} data-model-configuration={row.configured?.model_configuration_id ?? row.summary?.execution.model_configuration_id ?? ''}>
            <div className="quality-model-name"><strong>{rowName(row)}</strong><span className="quality-model-tags"><span className={`badge no-dot ${tone}`}>{qualityGroupLabel(row.branch, groupOf(row), language)}</span><span>{qualityReasoningLabel(row.summary ? row.summary.reasoning_profile_id : row.configured?.reasoning_profile_id, language)}{version === 'all' && row.summary && text(' · 版本 ', ' · version ') + row.summary.execution.plan_revision}</span></span></div>
            <div className="quality-model-average" data-label={text('平均阶段胜任度', 'Average stage competence')}>{row.summary?.average_score != null ? <><strong>{row.summary.average_score.toFixed(2)} <small>/ 1</small></strong><div className={`quality-score-bar ${tone}`} aria-hidden="true"><svg viewBox="0 0 100 4" preserveAspectRatio="none"><rect width={Math.max(0, Math.min(100, row.summary.average_score * 100))} height="4" /></svg></div></> : <span>{row.summary?.execution.attribution === 'mixed' ? text('不归属单模型', 'No single-model attribution') : row.summary ? text('未评分', 'Unrated') : text('暂无记录', 'No records')}</span>}</div>
            <span data-label={text('胜任率', 'Competence rate')}>{percent(evidence?.competence_rate)}</span>
            <span data-label={text('阶段占比', 'Stage share')}>{percent(evidence?.execution_share)}</span>
            <span className="quality-model-counts" data-label={text('阶段样本', 'Stage samples')}>{evidence ? <><span className="quality-row-coverage">{text(`评分覆盖 ${percent(evidence.scoring_coverage)}`, `${percent(evidence.scoring_coverage)} scoring coverage`)}</span>{stageStates(evidence.counts)}</> : <><span>{text((row.summary?.scored_stage_count ?? 0) + ' 已评分', (row.summary?.scored_stage_count ?? 0) + ' scored')}</span><span>{text((row.summary?.unrated_stage_count ?? 0) + ' 未评分', (row.summary?.unrated_stage_count ?? 0) + ' unrated')}</span></>}</span>
            <span data-label={text('耗时中位数', 'Median duration')}>{seconds(evidence?.duration_p50_ms)}</span>
            <button type="button" className="btn btn-quiet quality-view-stages" aria-expanded={selected?.key === row.key} onClick={() => { setScore('all'); setSelected(selected?.key === row.key ? null : { key: row.key, execution: row.summary?.execution ?? null }); }}>{selected?.key === row.key ? text('收起阶段', 'Hide stages') : text('查看阶段', 'View stages')}<UiIcon name="chevronRight" /></button>
          </div>; })}</div>
        </section>;
      })}
      {groupBy === 'executor' && report && <section className="quality-group" data-execution-group="executor">
        <header className="quality-group-heading"><h4>{text('按执行 Agent', 'By execution agent')}</h4><span>{text(`${report.rows.length} 个执行组`, `${report.rows.length} execution groups`)}</span></header>
        <div className="quality-table"><div className="quality-table-head" aria-hidden="true"><span>{text('模型 · 执行 Agent', 'Model · execution agent')}</span><span>{text('评分覆盖', 'Scoring coverage')}</span><span>{text('胜任率', 'Competence rate')}</span><span>{text('阶段占比', 'Stage share')}</span><span>{text('阶段样本', 'Stage samples')}</span><span>{text('耗时中位数', 'Median duration')}</span><span /></div>
          {report.rows.map(row => { const key = JSON.stringify({ branch_id: row.branch_id, group: row.group, model_configuration_id: row.model_configuration_id, reasoning_profile_id: row.reasoning_profile_id, attribution: row.attribution, executor: row.executor, group_by_executor: true }); return <div className={'quality-model-row' + (selected?.key === key ? ' selected' : '')} key={key}>
            <div className="quality-model-name"><strong>{trustRowName(row)}</strong><span className="quality-model-tags"><span className={`badge no-dot ${groupTone(row.group)}`}>{qualityGroupLabel(row.branch_id, row.group, language)}</span><span>{row.executor ?? text('执行 Agent 未知', 'Unknown execution agent')}</span></span></div>
            <span data-label={text('评分覆盖', 'Scoring coverage')}>{percent(row.scoring_coverage)}</span>
            <span data-label={text('胜任率', 'Competence rate')}>{percent(row.competence_rate)}</span>
            <span data-label={text('阶段占比', 'Stage share')}>{percent(row.execution_share)}</span>
            <span className="quality-model-counts" data-label={text('阶段样本', 'Stage samples')}><span>{text(`${row.counts.executed} 次执行`, `${row.counts.executed} executed`)}</span>{stageStates(row.counts)}</span>
            <span data-label={text('耗时中位数', 'Median duration')}>{seconds(row.duration_p50_ms)}</span>
            <button type="button" className="btn btn-quiet quality-view-stages" aria-expanded={selected?.key === key} onClick={() => selectTrustRow(row)}>{selected?.key === key ? text('收起阶段', 'Hide stages') : text('查看阶段', 'View stages')}<UiIcon name="chevronRight" /></button>
          </div>; })}
        </div>
      </section>}
      </div>
      {report && <TrustScopeNotes report={report} names={modelNames} language={language} />}
      {loaded && groups.length > 0 && groupBy === 'model' && <p className="quality-average-note">{text('平均值使用当前范围内各已评分阶段的最新原始评分，未评分阶段不计入平均。', 'Averages use the latest raw assessment of every scored stage in this scope. Unrated stages are excluded.')}</p>}
      {loaded && !error && groups.length === 0 && <p className="oc-meta">{text('当前范围还没有模型阶段记录。', 'No model-stage records are available for this scope yet.')}</p>}
    </section>}
    {stageSection}
    {report && groupBy === 'model' && <div className="trust-card-pair">
      <ProtectionCard report={report} released={releasedProtectionEvidence(report, routingSummary)} caught={caught} language={language} onShowLow={showLowStages || undefined} onOpenEvidence={onOpenEvidence} />
      <TrustCost value={report.cost} language={language} />
    </div>}
    {report && groupBy === 'executor' && <ExecutorOutcomes report={report} language={language} />}
    {report && <TrustSessions report={report} language={language} onOpenSession={onOpenEvidence ? session => onOpenEvidence(session, '') : undefined} />}
    {onDivision && manualDivisionBranches.length > 0 && <div className="quality-footer-links"><span>{text('想直接分工？', 'Want to divide work directly?')}</span>{manualDivisionBranches.map(branch => <button key={branch} type="button" className="btn btn-quiet" onClick={() => onDivision(branch)}>{text('创建分工草稿', 'Create a division draft')}{manualDivisionBranches.length > 1 ? ` · ${branch}` : ''}</button>)}<span>{text('打开草稿不代表已有证据支持扩大试用范围。', 'Opening a draft does not mean the evidence supports wider use.')}</span></div>}
  </div>;
}

function ProtectionCard({ report, released, caught, language, onShowLow, onOpenEvidence }: { report: TrustReport; released: ReleasedProtectionEvidence; caught: PlanQualitySample[] | null; language: 'zh' | 'en'; onShowLow?: () => void; onOpenEvidence?: (sessionId: string, requestId: string) => void }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const protections = report.rows.reduce((total, row) => total + row.protections.length, 0);
  const relays = report.rows.reduce((total, row) => total + row.availability_relays, 0);
  const releaseReason = released.kind === 'available' ? text('之前的问题已修正或与当前工作无关，允许新模型继续。', 'Earlier problems were fixed or unrelated, allowing Regular to continue.') : {
    missing: text('历史保护记录尚不可用。', 'Historical protection records are not available yet.'),
    partial: text('历史保护记录覆盖不完整。', 'Historical protection records have incomplete coverage.'),
    plan: text('历史保护记录与当前路由范围不同。', 'Historical protection records cover a different route scope.'),
    revision: text('历史保护记录与当前路由版本不同。', 'Historical protection records cover a different route version.'),
    time: text('历史保护记录与当前时间范围不同。', 'Historical protection records cover a different time range.'),
    branch: text('历史保护记录与当前分支范围不同。', 'Historical protection records cover different branches.'),
  }[released.reason];
  const time = (ms: number) => new Date(ms).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false });
  return <section className="trust-card trust-protection">
    <h3>{text(`被拦下的 ${protections} 次`, `${protections} stages caught`)}</h3>
    <p>{protections ? text('这些轮次因低分保护而交给主力模型，计数来自实际发生的记录。', 'These turns went to Primary through low-score protection; the count comes from recorded routing.') : text('还没有因为低分转交主力的阶段。', 'No stage has been handed to Primary after a low score yet.')}</p>
    <div className="trust-release" data-release-state={released.kind}>
      <strong>{released.kind === 'available' ? text(`解除 ${released.count} 次`, `${released.count} protections released`) : text('解除次数 —', 'Protections released —')}</strong>
      <p>{releaseReason}</p>
      {released.kind === 'available' && <p className="field-help">{text('解除按路由判断次数统计，与上方保护阶段数分开。', 'Releases count routing decisions; protections above count stages.')}</p>}
    </div>
    {caught && caught.length > 0 && <ul className="trust-caught">{caught.map(sample => <li key={sample.segment_id}>
      <div><strong><ContentExcerpt session={sample.session_id} language={language} fallback={text('会话记录', 'Session')} limit={40} /></strong>
        <span>{time(sample.first_at_ms)} · {text(`评分 ${sample.assessment?.score.toFixed(2) ?? '—'}`, `score ${sample.assessment?.score.toFixed(2) ?? '—'}`)}{sample.assessment?.reason ? text(`（${sample.assessment.reason}）`, ` (${sample.assessment.reason})`) : ''} → {text('下一轮主力', 'next turn Primary')}</span></div>
      {onOpenEvidence && <button type="button" className="btn btn-quiet" onClick={() => onOpenEvidence(sample.session_id, sample.last_request_id ?? sample.first_request_id ?? '')}>{text('查看证据', 'View evidence')}</button>}
    </li>)}</ul>}
    <p className="field-help">{text(`另有 ${relays} 次故障接力：模型暂时不可用时换用其他模型，不算没做好。`, `${relays} availability relays switched models while one was unavailable; they are not counted as failures.`)}</p>
    {onShowLow && protections > 0 && <button type="button" className="btn btn-quiet" onClick={onShowLow}>{text('查看全部低分阶段', 'View all low-score stages')}<UiIcon name="chevronRight" /></button>}
  </section>;
}
