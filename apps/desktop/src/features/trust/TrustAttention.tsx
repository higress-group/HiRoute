import type { TrustSuggestionsRead } from './use-trust-suggestions';
import { trustLevelLabel } from './levels';
import './trust.css';

const rollingStability = ['rolling-alias', 'dynamic-route', 'unknown'];
const percent = (part: number, whole: number) => whole ? `${Math.round(part / whole * 100)}%` : '—';

export function TrustAttention({ read, language, names, onOpen, onAct }: { read: TrustSuggestionsRead; language: 'zh' | 'en'; names: Record<string, string>; onOpen(planId: string): void; onAct?(planId: string, branchId: string): void }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const items = read.plans.flatMap(plan => plan.branches.filter(branch => branch.suggestion.kind !== 'keep').map(branch => ({ plan, branch })));
  const pending = read.pending?.length ?? 0;
  return <section className="surface trust-attention" aria-label={text('可选的路由调整', 'Optional routing adjustments')}>
    <header className="surface-header"><h2>{text('需要你处理的事', 'Needs your attention')}</h2><span className="home-section-note">{text('根据你自己会话里的记录给出，只是建议', 'Based on your own sessions. Suggestions only')}</span></header>
    {read.status === 'ready' && !items.length && !read.unavailable.length && !pending && <p className="trust-attention-empty">{text('当前没有路由调整建议。保持当前档位即可。', 'No routing adjustments are suggested. Keeping the current level is a valid choice.')}</p>}
    {read.status === 'ready' && pending > 0 && <p className="trust-attention-empty" role="status">{text(`正在继续读取其余 ${pending} 个路由的建议…`, `Continuing to read suggestions for ${pending} more ${pending === 1 ? 'route' : 'routes'}…`)}</p>}
    {read.status === 'loading' && !items.length && <p className="trust-attention-empty" role="status">{text('正在读取路由证据…', 'Reading routing evidence…')}</p>}
    {read.status === 'error' && <div className="callout warn" role="alert" data-error-code={read.error}><span>{text('建议暂时无法刷新。已有建议可能过时，请重试后再决定。', 'Suggestions could not refresh. Loaded suggestions may be outdated; retry before deciding.')}</span><button className="btn" onClick={read.refresh}>{text('重试', 'Retry')}</button></div>}
    {read.unavailable.map(item => <div key={item.plan_id} className="callout warn" role="alert"><span>{names[item.plan_id] ?? item.plan_id} · {item.reason === 'scope_too_large' ? text('此路由的证据超出读取范围，其他建议仍可查看。', 'Evidence for this route exceeds the read limit. Other suggestions remain available.') : text('此路由的建议暂时无法读取。', 'Suggestions for this route are temporarily unavailable.')}</span><button className="btn" onClick={read.refresh}>{text('重试', 'Retry')}</button></div>)}
    {items.map(({ plan, branch }) => {
      const suggestion = branch.suggestion;
      const upward = suggestion.kind === 'step_up' || suggestion.kind === 'explore_division';
      const cohort = upward ? branch.upward_cohort : branch.full_history;
      const target = 'threshold_millis' in suggestion ? trustLevelLabel(suggestion.threshold_millis, language) : '';
      const replay = 'threshold_millis' in suggestion ? branch.replay.thresholds.find(r => r.threshold_millis === suggestion.threshold_millis) : undefined;
      const kind = suggestion.kind === 'step_up' ? ['info', text('可以上调', 'Step up'), text(`可以把信任度调到「${target}」`, `You can raise trust to ${target}`), text('打开调整草稿', 'Open change draft')]
        : suggestion.kind === 'step_down' ? ['warn', text('建议下调', 'Step down'), text(`建议把信任度调回「${target}」`, `Consider lowering trust to ${target}`), text('打开调整草稿', 'Open change draft')]
        : suggestion.kind === 'explore_division' ? ['cyan', text('可以分工', 'Divide work'), text('可以按任务分工，让常规模型接手一类任务', 'You can divide work by task and give one kind to Regular'), text('打开分工草稿', 'Open division draft')]
        : suggestion.kind === 'return_to_previous_routing' ? ['bad', text('建议回退', 'Go back'), text('常规模型胜任率偏低，可以回到之前的路由', 'Regular competence is low. You can return to previous routing'), text('查看之前的路由', 'Review previous routing')]
        : ['bad', text('建议换模型', 'Change model'), text('常规模型胜任率偏低，可以更换常规模型', 'Regular competence is low. You can change the Regular model'), text('打开路由编辑', 'Open routing editor')];
      const facts = [
        text(`胜任率 ${percent(cohort.meets_floor, cohort.complete)}（${cohort.meets_floor} / ${cohort.complete} 个评分阶段达到下限）`, `Competence ${percent(cohort.meets_floor, cohort.complete)} (${cohort.meets_floor} of ${cohort.complete} scored stages meet the floor)`),
        text(`评分覆盖 ${percent(cohort.complete, cohort.executed)}`, `${percent(cohort.complete, cohort.executed)} scored`),
        text(`来自 ${cohort.sessions} 个会话`, `${cohort.sessions} sessions`),
        text(`低分保护 ${branch.protection_count} 次`, `${branch.protection_count} protections`),
      ].join(' · ');
      const replayText = replay?.regular_share != null ? text(`按你的记录回放，这一档约 ${Math.round(replay.regular_share * 100)}% 的阶段会交给常规模型。`, `Replaying your records, this level would give about ${Math.round(replay.regular_share * 100)}% of stages to Regular.`) : '';
      const scope = [
        upward && plan.announcements.length > 0 && branch.upward_from_ms > 0 ? text(`只看 ${new Date(branch.upward_from_ms).toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric' })} 模型更新公告之后的记录`, `Only records after the ${new Date(branch.upward_from_ms).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })} model update announcement`) : '',
        branch.identity_stabilities.some(stability => rollingStability.includes(stability)) ? text('同名模型可能被供应商更新，记录可能混合多个版本', 'The provider may update this model name; records may mix versions') : '',
      ].filter(Boolean).join(' · ');
      return <article key={`${plan.plan_id}/${branch.branch_id}`} className="trust-attention-item">
        <div className="trust-attention-main">
          <div className="trust-attention-line"><span className={`badge no-dot ${kind[0]}`}>{kind[1]}</span><strong>{names[plan.plan_id] ?? plan.plan_id}</strong><span>{text(`当前「${trustLevelLabel(branch.threshold_millis, language)}」`, `Now ${trustLevelLabel(branch.threshold_millis, language)}`)}</span></div>
          <p className="trust-attention-title">{kind[2]}</p>
          <p className="trust-attention-facts">{facts}{replayText && <>{language === 'zh' ? '。' : '. '}{replayText}</>}</p>
          {scope && <p className="trust-attention-scope">{scope}</p>}
        </div>
        <div className="trust-attention-actions"><button className="btn" disabled={read.status !== 'ready'} onClick={() => onOpen(plan.plan_id)}>{text('查看依据', 'View evidence')}</button>{onAct && <button className="btn btn-primary" disabled={read.status !== 'ready'} onClick={() => onAct(plan.plan_id, branch.branch_id)}>{kind[3]}</button>}</div>
      </article>;
    })}
  </section>;
}
