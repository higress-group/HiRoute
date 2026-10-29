import { useState, type ReactNode } from 'react';
import { ContentExcerpt } from '../Sessions';
import { agentDisplayName } from '../agents/ecosystems';
import { Disclosure } from '../../ui';
import { trustLevelLabel } from './levels';
import { formatPresentedCostAmount, presentTrustCost, trustCostUnavailableReason } from './cost-presentation';
import { qualityBranchLabel, type PlanQualityModel } from '../plan-quality-state';
import type { TrustBranch, TrustCounts, TrustReport, TrustRow } from './types';
import './trust.css';

type Language = 'zh' | 'en';
const ratio = (part: number, whole: number) => whole ? `${Math.round(part / whole * 100)}%` : '—';
const rolling = ['rolling-alias', 'dynamic-route', 'unknown'];
const regularRows = (report: TrustReport) => report.rows.filter(row => row.group === 'regular');
const sum = (rows: TrustRow[], pick: (row: TrustRow) => number) => rows.reduce((total, row) => total + pick(row), 0);
type SuggestionBanner = { tone: string; badge: string; title: string; body: string; extra: string; action: ReactNode; threshold?: string; replay?: string; newlyRegular?: string };

/** One banner per routed branch: the suggestion, the facts behind it and an optional draft action. */
export function TrustSuggestions({ report, language, currentModels = [], onThreshold, onDivision, onOpenAgent }: {
  report: TrustReport; language: Language; currentModels?: readonly PlanQualityModel[]; onThreshold?(branch: string, threshold: number): void; onDivision?(branch: string): void; onOpenAgent?(agent: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const named = report.branches.length > 1;
  const configuredByBranch = new Map<string, PlanQualityModel[]>();
  for (const model of currentModels) {
    if (model.plan_revision !== report.active_revision || model.group !== 'regular') continue;
    const configured = configuredByBranch.get(model.branch_id) ?? [];
    configured.push(model); configuredByBranch.set(model.branch_id, configured);
  }
  // Announcements cover the report's whole catalog, while editor options can omit
  // published bindings or contain newer model IDs. Only an empty global list proves
  // that no model-update cutoff applies without the published per-branch identities.
  const noModelUpdates = report.announcements.length === 0;
  function facts(counts: TrustCounts, upward: boolean) {
    return t(`完整评分阶段 ${counts.complete}（要求 ≥30）· 胜任率 ${ratio(counts.meets_floor, counts.competence_eligible ?? counts.complete)}（${upward ? '要求 ≥85%' : '低于 70% 时建议下调'}）· 评分覆盖 ${ratio(counts.complete, counts.executed)}${upward ? '（要求 ≥60%）' : ''} · 来自 ${counts.sessions} 个会话${upward ? '（要求 ≥3）' : ''}`,
      `${counts.complete} fully scored stages (≥30 required) · ${ratio(counts.meets_floor, counts.competence_eligible ?? counts.complete)} competence (${upward ? '≥85% required' : 'below 70% suggests stepping down'}) · ${ratio(counts.complete, counts.executed)} scored${upward ? ' (≥60% required)' : ''} · ${counts.sessions} sessions${upward ? ' (≥3 required)' : ''}`);
  }
  function banner(branch: TrustBranch): SuggestionBanner {
    const s = branch.suggestion;
    const level = 'threshold_millis' in s ? trustLevelLabel(s.threshold_millis, language) : '';
    const threshold = 'threshold_millis' in s ? t(`简单概率 ≥ ${(s.threshold_millis / 1000).toFixed(2)}`, `P(simple) ≥ ${(s.threshold_millis / 1000).toFixed(2)}`) : '';
    const join = (...parts: string[]) => parts.filter(Boolean).join(t('', ' '));
    const replay = 'threshold_millis' in s ? branch.replay.thresholds.find(row => row.threshold_millis === s.threshold_millis) : undefined;
    const replayText = replay?.regular_share != null ? t(`按你的记录回放，这一档约 ${Math.round(replay.regular_share * 100)}% 的阶段会交给常规模型，${replay.protected_as_primary} 个仍由主力保护。`, `Replaying your records, about ${Math.round(replay.regular_share * 100)}% of stages would go to Regular; ${replay.protected_as_primary} stay protected by Primary.`) : '';
    const fresh = replay && replay.newly_regular > 0 ? t(`${replay.newly_regular} 个阶段之前由主力处理，没有常规模型的胜任证据。`, `${replay.newly_regular} of them were handled by Primary and carry no Regular competence evidence.`) : '';
    if (s.kind === 'keep') {
      const need: Record<string, string> = {
        complete_scores: t(`至少 30 个评分完整的阶段（现在 ${branch.upward_cohort.complete}）`, `at least 30 fully scored stages (now ${branch.upward_cohort.complete})`),
        scoring_coverage: t(`评分覆盖至少 60%（现在 ${ratio(branch.upward_cohort.complete, branch.upward_cohort.executed)}）`, `at least 60% scored (now ${ratio(branch.upward_cohort.complete, branch.upward_cohort.executed)})`),
        contributing_sessions: t(`来自至少 3 个会话（现在 ${branch.upward_cohort.sessions}）`, `from at least 3 sessions (now ${branch.upward_cohort.sessions})`),
      };
      const missing = s.missing.filter(code => need[code]).map(code => need[code]);
      return { tone: '', badge: t('保持当前档位', 'Keep this level'), title: s.missing.includes('active_revision_only') ? t('建议只针对当前生效的版本', 'Suggestions cover only the active version') : missing.length ? t('证据还不够，暂不建议调整', 'Not enough evidence to suggest a change') : t('当前档位合适，保持即可', 'The current level fits; keeping it is fine'),
        body: missing.length ? t(`升档需要${missing.join('、')}。`, `Stepping up needs ${missing.join(', ')}.`) : facts(branch.upward_cohort, true), extra: '', action: null };
    }
    if (s.kind === 'step_up') return { tone: 'info', badge: t('可以上调', 'Step up'), title: t(`可以调到「${level}」`, `You can move to ${level}`), body: facts(branch.upward_cohort, true), threshold, replay: replayText, newlyRegular: fresh, extra: '',
      action: onThreshold && <button type="button" className="btn btn-primary" onClick={() => onThreshold(branch.branch_id, s.threshold_millis)}>{t('按建议打开草稿', 'Open draft with this level')}</button> };
    if (s.kind === 'step_down') return { tone: 'warn', badge: t('建议下调', 'Step down'), title: t(`建议调回「${level}」`, `Consider lowering trust to ${level}`), body: facts(branch.full_history, false), threshold, replay: replayText, extra: '',
      action: onThreshold && <button type="button" className="btn btn-primary" onClick={() => onThreshold(branch.branch_id, s.threshold_millis)}>{t('按建议打开草稿', 'Open draft with this level')}</button> };
    if (s.kind === 'explore_division') return { tone: 'cyan', badge: t('可以分工', 'Divide work'), title: t('已经放手，可以试试按任务分工', 'Hands-off already; you can try dividing work by task'), body: facts(branch.upward_cohort, true), extra: '',
      action: onDivision && <button type="button" className="btn btn-primary" onClick={() => onDivision(branch.branch_id)}>{t('查看分工草稿', 'Review division draft')}</button> };
    if (s.kind === 'return_to_previous_routing') return { tone: 'bad', badge: t('建议回退', 'Go back'), title: t('常规模型胜任率偏低，可以回到之前的路由', 'Regular competence is low. You can return to previous routing'), body: facts(branch.full_history, false), extra: '',
      action: <>{(report.previous_routing ?? []).map(target => <button type="button" className="btn btn-primary" key={`${target.agent}/${target.presets.join(',')}`} disabled={!onOpenAgent} onClick={() => onOpenAgent?.(target.agent)}>{t('查看之前的路由', 'Review previous routing')} · {agentDisplayName(target.agent, language)}</button>)}</> };
    return { tone: 'bad', badge: t('建议换模型', 'Change model'), title: t('常规模型胜任率偏低，可以更换常规模型', 'Regular competence is low. Consider changing the Regular model'), body: join(facts(branch.full_history, false), t('尚未确认有可恢复的历史映射。', 'No recoverable historical mapping is confirmed.')), extra: '', action: null };
  }
  return <>{report.branches.map(branch => {
    const b = banner(branch);
    const configured = configuredByBranch.get(branch.branch_id) ?? [];
    const category = configured.find(model => model.branch_name)?.branch_name
      ?? (branch.branch_id === 'smart_saving' ? qualityBranchLabel(branch.branch_id, language) : branch.branch_id);
    const fromDate = new Date(branch.upward_from_ms);
    const from = Number.isSafeInteger(branch.upward_from_ms) && branch.upward_from_ms > 0 && Number.isFinite(fromDate.getTime())
      ? fromDate.toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US') : null;
    return <section key={branch.branch_id} className={`trust-suggestion ${b.tone}`} aria-label={t('档位建议', 'Level suggestion')}>
      <div className="trust-suggestion-main">
        <div className="trust-suggestion-line"><span className={`badge no-dot ${b.tone}`}>{b.badge}</span><strong>{named ? `${branch.branch_id} · ` : ''}{b.title}</strong></div>
        {b.threshold && <p className="trust-suggestion-note trust-suggestion-threshold">{b.threshold}</p>}
        <p>{b.body}</p>
        <p className="trust-suggestion-sample"><strong>{t(`升档样本：${branch.upward_cohort.complete} / 30`, `Step-up samples: ${branch.upward_cohort.complete} / 30`)}</strong><span>{t(`当前版本 · ${category} · 常规组 · 完整评分`, `Active revision · ${category} · Regular · Fully scored`)}</span></p>
        <Disclosure className="trust-evidence-scope" label={t('样本范围', 'Evidence scope')} language={language}>
          <p>{t('升档使用当前生效版本、本分类常规组的保留记录，与页面展示的时间筛选无关。下调也只使用该版本和分类的保留记录。', 'Step-up uses retained records for the active revision and this category’s Regular group, independently of the page’s display period. Step-down also uses retained records for that revision and category.')}</p>
          {noModelUpdates ? <p>{t('此报告没有模型更新公告；升档范围受保留期限制。', 'No model-update announcements are present in this report; retention bounds the step-up scope.')}</p>
            : <p>{t('模型更新是否适用于本分类尚未确认；实际起点已包含保留期和可能适用的更新截点。', 'Model-update basis is unconfirmed for this category; the effective start includes retention and any applicable update cutoff.')}</p>}
          <p>{from ? t(`升档样本实际起点：${from}。`, `Effective step-up sample start: ${from}.`) : t('升档样本从可用保留记录的起点开始。', 'Step-up samples begin at the start of available retained records.')}</p>
        </Disclosure>
        {b.replay && <p className="trust-suggestion-replay">{b.replay}</p>}
        {b.newlyRegular && <p className="trust-suggestion-newly-regular">{b.newlyRegular}</p>}
        {b.extra && <p className="trust-suggestion-note">{b.extra}</p>}
        <p className="trust-suggestion-note">{t(`当前「${trustLevelLabel(branch.threshold_millis, language)}」· 只是建议，发布前不会改变路由；保持当前档位也是正常选择。`, `Now ${trustLevelLabel(branch.threshold_millis, language)} · A suggestion only; routing changes only when you publish, and keeping this level is fine.`)}</p>
      </div>
      {b.action && <div className="trust-suggestion-actions">{b.action}</div>}
    </section>;
  })}</>;
}

/** Headline figures for the Regular group of a difficulty-routing or two-group branch. */
export function TrustTiles({ report, language }: { report: TrustReport; language: Language }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const regular = regularRows(report);
  if (!regular.length) return null;
  const total = sum(report.rows, row => row.counts.executed);
  const executed = sum(regular, row => row.counts.executed), complete = sum(regular, row => row.counts.complete), meets = sum(regular, row => row.counts.meets_floor);
  const eligible = sum(regular, row => row.counts.competence_eligible ?? row.counts.complete);
  const unscored = executed - complete;
  const protections = sum(report.rows, row => row.protections.length), relays = sum(report.rows, row => row.availability_relays);
  const cost = presentTrustCost(report.cost);
  const money = (entry: Extract<typeof cost, { kind: 'available' }>['routed'], signed = false) => cost.kind === 'available'
    ? `${formatPresentedCostAmount(entry, cost.currency, language, signed)}${entry.kind === 'range' ? t('（范围）', ' (Range)') : ''}` : '';
  return <div className="trust-tiles">
    <div className="trust-tile"><span>{t('交给常规模型的阶段', 'Stages given to Regular')}</span><strong>{ratio(executed, total)}</strong><small>{executed} / {total}</small></div>
    <div className="trust-tile"><span>{t('常规模型胜任率', 'Regular competence')}</span><strong>{ratio(meets, eligible)}</strong><small>{t(`${meets} / ${eligible} 达到下限 · ${unscored} 未完整评分`, `${meets} / ${eligible} meet the floor · ${unscored} not fully scored`)}</small></div>
    <div className="trust-tile"><span>{t('低分保护', 'Low-score protection')}</span><strong>{t(`${protections} 次`, String(protections))}</strong><small>{t(`另有 ${relays} 次故障接力，不算没做好`, `${relays} availability relays are not counted as failures`)}</small></div>
    <div className="trust-tile"><span>{t('常规模型阶段估算费用', 'Estimated Regular cost')}</span>{cost.kind === 'available' ? <><strong>{money(cost.routed)}</strong><small>{t(`同样用量的主力参考：${money(cost.reference)}`, `Primary reference for the same usage: ${money(cost.reference)}`)}</small><small>{t(`估算差额：${money(cost.difference, true)}`, `Estimated difference: ${money(cost.difference, true)}`)}</small></> : <small>{t('费用对比暂不可用：', 'Cost comparison unavailable: ')}{trustCostUnavailableReason(cost.reason, language)}</small>}</div>
  </div>;
}

/** Disclosures that apply to every row: provider alias updates and editorial update announcements. */
export function TrustScopeNotes({ report, names, language }: { report: TrustReport; names: Map<string, string>; language: Language }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const notes = [
    ...report.announcements.map(a => t(`${names.get(a.model_configuration_id) ?? a.model_configuration_id} 在 ${a.included_on} 有更新公告：之前 ${a.before} 个阶段，当天 ${a.announcement_day} 个，之后 ${a.after} 个。公告日期不代表供应商实际部署时间。`, `${names.get(a.model_configuration_id) ?? a.model_configuration_id} has an update announced on ${a.included_on}: ${a.before} stages before, ${a.announcement_day} that day, ${a.after} after. The date does not establish when the provider deployed it.`)),
    report.rows.some(row => rolling.includes(row.identity_stability)) ? t('供应商可能在不改模型名的情况下更新模型，这些记录可能混合多个版本，实际执行版本未经确认。', 'Providers can update a model without changing its name. These records may mix versions; the executed version is unverified.') : '',
  ].filter(Boolean);
  return notes.length ? <div className="trust-scope-notes">{notes.map(note => <p key={note}>{note}</p>)}</div> : null;
}

/** Task outcomes of delegated executors; groups handled different tasks and are not ranked. */
export function ExecutorOutcomes({ report, language }: { report: TrustReport; language: Language }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (!report.executors.length) return null;
  return <section className="trust-card"><h3>{t('执行 Agent 的任务', 'Execution agent tasks')}</h3>
    {report.executors.map(group => { const o = group.task_outcomes; return <div className="trust-card-row" key={group.executor}><strong>{agentDisplayName(group.executor, language)}</strong><span>{t(`新提交 ${o.submitted ?? 0} · 完成 ${o.succeeded ?? 0} · 失败 ${o.failed ?? 0} · 取消 ${o.cancelled ?? 0} · 运行中 ${o.running ?? 0} · 未知 ${o.unknown ?? 0} · 继续 ${o.continued ?? 0}`, `Submitted ${o.submitted ?? 0} · succeeded ${o.succeeded ?? 0} · failed ${o.failed ?? 0} · cancelled ${o.cancelled ?? 0} · running ${o.running ?? 0} · unknown ${o.unknown ?? 0} · continued ${o.continued ?? 0}`)}</span></div>; })}
    <p className="field-help">{t('按任务首次提交时间计数，显示最新运行状态。各组处理的任务不同，不作排名；任务完成不等于胜任，没有评分仍算未评分。', 'Counted by first submission, with the latest run status. Groups handled different tasks and are not ranked. Completion is not competence; no score still means unscored.')}</p>
  </section>;
}

export function TrustSessions({ report, language, onOpenSession, onViewAll }: { report: TrustReport; language: Language; onOpenSession?(session: string): void; onViewAll?(): void }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [limit, setLimit] = useState(8);
  if (!report.sessions.length) return null;
  return <section className="trust-card trust-sessions">
    <div className="trust-card-head"><h3>{t(`证据来自你的 ${report.session_total} 个会话`, `Evidence from ${report.session_total} of your sessions`)}</h3>{onViewAll && <button type="button" className="btn btn-quiet" onClick={onViewAll}>{t('查看全部', 'View all')}</button>}</div>
    <div className="trust-session-grid">{report.sessions.slice(0, limit).map(session => <button type="button" className="trust-session" key={session.session_id} disabled={!onOpenSession} onClick={() => onOpenSession?.(session.session_id)}><strong><ContentExcerpt session={session.session_id} language={language} fallback={t('会话记录', 'Session')} limit={48} /></strong><span>{t(`${session.stages} 个阶段`, `${session.stages} stages`)}</span></button>)}</div>
    {limit < report.sessions.length && <button type="button" className="btn btn-quiet" onClick={() => setLimit(n => n + 8)}>{t('查看更多', 'Show more')}</button>}
  </section>;
}
