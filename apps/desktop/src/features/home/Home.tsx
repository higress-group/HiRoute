import { nativeConnectionReadText } from '../../native-connection';
import { TrustAttention } from '../trust/TrustAttention';
import type { TrustSuggestionsRead } from '../trust/use-trust-suggestions';
import { useState, type ReactNode } from 'react';
import { ContentExcerpt } from '../Sessions';
import { agentBrandFromId, BrandIcon, Disclosure, formatMoney, ProductPage, UiIcon } from '../../ui';
import { deriveHomePrimaryMode, visibleData } from './state';
import type { HomeProps, HomeReads } from './types';
import { formatCacheHit, formatCacheHitCoverage, formatTokenCount } from '../usage-presentation';

/** v3-shell + occami-core DOM. Only the real read model supplies values. */
export function Home({ language, reads, onAction, hasTasks = false, trustSuggestions, newModels, onTrustAction }: HomeProps & { hasTasks?: boolean; trustSuggestions?: TrustSuggestionsRead; newModels?: ReactNode; onTrustAction?(planId: string, branchId: string): void }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const mode = deriveHomePrimaryMode(reads);
  const compute = visibleData(reads.compute), plans = visibleData(reads.plans), agents = visibleData(reads.agents);
  const activity = visibleData(reads.activity), value = visibleData(reads.value);
  const recent = activity?.sessions.slice(0, 4) ?? [];
  const [sessionRoutes, setSessionRoutes] = useState<Record<string, string>>({});
  const [sessionModels, setSessionModels] = useState<Record<string, string>>({});
  const activityPending = !activity && reads.activity.status === 'loading';
  const activityUnavailable = !activity && reads.activity.status === 'error';
  const done = [Boolean(compute?.sources.length), Boolean(plans?.plans.some(p => p.publication === 'published')), Boolean(agents?.agents.some(a => ['configured', 'verified'].includes(a.model) || ['configured', 'verified'].includes(a.collaboration)))];
  const complete = done.every(Boolean);
  const errors = (Object.keys(reads) as (keyof HomeReads)[]).filter(k => reads[k].status === 'error');
  const unresolvedRequiredRead = (['compute', 'plans', 'agents', 'activity'] as const).some(domain => {
    const read = reads[domain];
    return read.status === 'error' && read.previous === undefined;
  });
  const domainLabels: Record<keyof HomeReads, string> = {
    service: text('服务', 'Service'),
    compute: text('模型', 'Models'),
    plans: text('路由', 'Routing'),
    agents: 'Agent',
    activity: text('会话', 'Sessions'),
    value: text('用量', 'Usage'),
  };
  const steps = [
    [text('添加模型', 'Add models'), text('连接 API，或复用本机订阅', 'Connect an API or reuse a local subscription')],
    [text('创建智能路由', 'Create routing'), text('决定怎样选模型，以及由哪个 Agent 执行任务', 'Choose models and an optional task executor')],
    [text('接入 Agent', 'Connect an agent'), text('选择默认路由或启用 Agent 路由，两者可独立使用', 'Choose a default route or enable Agent routing independently')],
  ];
  const timeLabel = (milliseconds: number | undefined, fallback: string) => {
    if (milliseconds === undefined) return fallback;
    const date = new Date(milliseconds);
    if (!Number.isFinite(date.valueOf())) return fallback;
    const now = new Date();
    const day = (value: Date) => new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
    const days = Math.round((day(now) - day(date)) / 86_400_000);
    if (days === 0) return date.toLocaleTimeString(language === 'zh' ? 'zh-CN' : 'en', { hour: '2-digit', minute: '2-digit', hour12: false });
    if (days === 1) return text('昨天', 'Yesterday');
    return date.toLocaleDateString(language === 'zh' ? 'zh-CN' : 'en', { month: '2-digit', day: '2-digit' });
  };
  const openStep = (index: number) => onAction({kind: index === 0 ? done[0] ? 'view-models' : 'connect-models' : index === 1 ? done[1] ? 'view-routing' : 'create-plan' : 'view-agents', returnTo: 'home'});
  // Returning users see what changed first: new models, then suggestions, then recent work.
  const attention = trustSuggestions && <TrustAttention onAct={onTrustAction} read={trustSuggestions} language={language} names={Object.fromEntries((plans?.plans ?? []).map(plan => [plan.planId, plan.displayName]))} onOpen={planId => onAction({kind:'open-plan',returnTo:'home',planId,performance:true})} />;
  return <ProductPage title={text('首页', 'Home')} subtitle={recent.length ? text('近期使用与需要你处理的事', 'Recent work and things that need your attention') : undefined}>
    <div className="home-wrap v3-home" data-home-mode={mode}>
      {!!errors.length && <div className="callout bad home-read-error" role="alert"><UiIcon name="warning" /><div><strong>{text('部分信息暂时无法读取', 'Some information is temporarily unavailable')}</strong><p>{errors.map(domain => reads[domain]).map(read => read.status === 'error' ? nativeConnectionReadText(read.code, language) : undefined).find(Boolean) ?? text('已有数据会保留；首次读取完成前不会把当前状态当作首次使用。', 'Existing data is retained. The app will not treat this as first use until the initial reads finish.')}</p><div className="actions">{errors.map(domain => <button className="btn btn-quiet" key={domain} type="button" onClick={() => onAction({kind:'retry-read', domain})}>{text('重试', 'Retry')} {domainLabels[domain]}</button>)}</div></div></div>}
      {unresolvedRequiredRead && !recent.length ? <section className="surface v3-ready"><UiIcon name="warning" /><div><strong>{text('暂时无法确认当前配置', 'Current setup cannot be confirmed')}</strong><p>{text('请先重试上方读取；确认实际状态后再继续配置。', 'Retry the reads above before changing setup.')}</p></div></section> : mode === 'loading' && !recent.length ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><span>{text('正在读取本机配置与记录…', 'Reading local configuration and records…')}</span></div> : activityPending && !recent.length ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><span>{text('正在确认会话记录…', 'Checking session records…')}</span></div> : !recent.length ? <>
        <section className="v3-welcome"><div className="eyebrow"><UiIcon name="route" />{text('模型与 Agent 的智能路由', 'Smart routing for models and agents')}</div>
          <h1>{complete ? text('准备好了，回到你常用的 Agent', 'Ready. Open your usual agent.') : text('让合适的模型和 Agent\n处理合适的任务', 'The right models and agents\nfor each task')}</h1>
          <p>{complete ? text('从 Codex 或 Claude Code 开始一次任务。实际使用后，会话与路由结果会出现在这里。', 'Start a task in Codex or Claude Code. Sessions and routing results will appear here.') : text('连接已有订阅或 API，配置一次，在原来的 Agent 中继续工作。', 'Connect subscriptions or APIs. Configure once and keep working in your usual agent.')}</p>
        </section>
        <section className="v3-steps">{steps.map(([title, description], index) => <button type="button" key={index} className={`v3-step ${done[index] ? 'done' : ''} ${!done[index] && done.slice(0,index).every(Boolean) ? 'next' : ''}`} onClick={() => openStep(index)}>
          <span className="v3-step-number">{done[index] ? <UiIcon name="check" /> : index + 1}</span><div><strong>{title}</strong><p>{description}</p><span className="v3-step-cta">{done[index] ? text('查看配置', 'View setup') : text('开始', 'Get started')} <UiIcon name="arrow" /></span></div>
        </button>)}</section>
        {complete && <section className="surface v3-ready"><UiIcon name={activityUnavailable ? 'warning' : 'check'} /><div><strong>{activityUnavailable ? text('模型与路由配置可继续使用', 'Model and routing setup remains available') : text('配置已保存', 'Configuration saved')}</strong><p>{activityUnavailable ? text('会话记录恢复后会在这里显示。', 'Session history will appear here after it becomes available again.') : text('还没有会话记录，不需要在 HiRoute 中再开始一次对话。', 'No sessions yet. You do not need to start another chat in HiRoute.')}</p></div></section>}
      </> : <>
        {newModels}
        {attention}
        <section className="surface"><header className="surface-header"><h2>{text('近期会话', 'Recent sessions')}</h2><button className="btn btn-quiet" type="button" onClick={() => onAction({kind:'view-all-sessions',returnTo:'home'})}>{text('查看全部', 'View all')} <UiIcon name="chevronRight" /></button></header>
          <div className="native-list">{recent.map((session,index) => { const sessionId = session.sessionId; const occurredAt = timeLabel(session.occurredAtMs, session.occurredAtLabel); const fallbackTitle = session.title || [session.agentName, occurredAt].filter(Boolean).join(' · ') || text('会话记录', 'Session'); return <button type="button" className="list-row v3-recent" key={sessionId ?? index} disabled={!sessionId} onClick={() => sessionId && onAction({kind:'open-session',sessionId,returnTo:'home'})}>
            <BrandIcon kind={agentBrandFromId(session.agentId ?? session.agentName ?? '')} size="medium" />
            <div className="row-main"><div className="row-title">{sessionId ? <ContentExcerpt session={sessionId} language={language} fallback={fallbackTitle} limit={48} revision={`${session.occurredAtMs ?? 0}:${session.requestCount ?? 0}`} onModel={model => setSessionModels(current => current[sessionId] === model ? current : { ...current, [sessionId]: model })} onRoute={route => setSessionRoutes(current => current[sessionId] === route ? current : { ...current, [sessionId]: route })} /> : fallbackTitle}</div><div className="row-meta">{[session.agentName, sessionId ? sessionRoutes[sessionId] : undefined, sessionId ? sessionModels[sessionId] : undefined].filter(Boolean).join(' · ')}</div></div>
            {session.modelSwitch === true && <span className="badge info no-dot">{text('请求内回退', 'Request fallback')}</span>}<span className="muted">{occurredAt}</span><UiIcon name="chevronRight" />
          </button>; })}</div>
        </section>
        {value && <Disclosure className="surface v3-usage" label={text('近 7 天用量与费用', 'Usage and cost · last 7 days')} language={language}><div className="v3-usage-body">
          <div><span>{text('输入 Token', 'Input tokens')}</span><strong>{formatTokenCount(value.usage.input, language)}</strong></div>
          <div><span>{text('输出 Token', 'Output tokens')}</span><strong>{formatTokenCount(value.usage.output, language)}</strong></div>
          <div><span>{text('缓存读取 Token', 'Cache-read tokens')}</span><strong>{formatTokenCount(value.usage.cacheRead, language)}</strong></div>
          <div><span>{text('缓存写入 Token', 'Cache-write tokens')}</span><strong>{formatTokenCount(value.usage.cacheWrite, language)}</strong></div>
          <div><span>{text('输入缓存命中率', 'Input cache-hit rate')}</span><strong>{formatCacheHit(value.usage.inputCacheHit, language)}</strong></div>
          <p>{formatCacheHitCoverage(value.usage.inputCacheHit, language)}{value.excludedRequests > 0 ? text(`；已排除 ${value.excludedRequests} 个连接探针。`, `; ${value.excludedRequests} connectivity probes excluded.`) : text('。', '.')}</p>
          {value.money.filter(m => m.usageEstimate != null).map(m => <div key={m.currency}><span>{text('已计价部分的估算费用', 'Estimated cost of priced usage')}</span><strong>{formatMoney(m.currency, m.usageEstimate!)}</strong></div>)}
          {!value.money.some(m => m.usageEstimate != null) && <div><span>{text('尚未计价', 'Unpriced')}</span></div>}
          <p>{value.coverage === 'complete' && value.pending === 0 && !value.provisional && value.unknownTrafficRequests === 0 ? text('Token 来自已记录的上游用量；金额是独立估算，不代表供应商账单。', 'Tokens come from recorded upstream usage; monetary values are separate estimates, not provider bills.') : text('部分用量或请求仍未知；已知 Token 与金额分别展示，不把缺失项当作零。', 'Some usage or requests remain unknown. Known tokens and monetary estimates stay separate; missing values are not zero.')}</p>
        </div></Disclosure>}
      </>}
      {!recent.length && <>{newModels}{attention}</>}
      {hasTasks && <button className="v3-text-link" type="button" onClick={() => onAction({kind:'view-all-tasks',returnTo:'home'})}><UiIcon name="agent" />{text('查看任务记录', 'View task history')}<UiIcon name="arrow" /></button>}
    </div>
  </ProductPage>;
}
