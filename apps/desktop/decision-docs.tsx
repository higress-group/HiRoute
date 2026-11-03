// Documentation capture only; current product components and synthetic, read-only IPC.
// This entry is not part of the production build.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { PlanQuality } from './src/features/PlanQuality';
import { DecisionServicesPage } from './src/features/decision-services/DecisionServicesPage';
import { RoutingPage } from './src/product/RoutingPage';
import { HomeNavigation } from './src/features/home/HomeNavigation';
import { PresentationRoot } from './src/ui/PresentationRoot';
import { decisionDocsData, queryDocsSamples, type DocsQualityQuery } from './decision-docs-data';
import { fixturePlanAuthoring } from './tests/v3/browser/product-fixtures';
import { fixtureDecisionContext } from './tests/v3/browser/decision-context-fixture';
import './src/occami/styles.css';

const params = new URLSearchParams(location.search);
const language = params.get('lang') === 'en' ? 'en' : 'zh';
const view = params.get('view') ?? 'quality';
const text = (zh: string, en: string) => language === 'en' ? en : zh;
const { models, services, plan, customPlan, samples, snapshot, options } = decisionDocsData(language);
const captureCalls: string[] = [];
Object.assign(window, { __DECISION_DOCS__: { samples, captureCalls }, __TAURI_INTERNALS__: {
  invoke: async (command: string, args?: { request?: { intent: { view: string; query: DocsQualityQuery } } }) => {
    captureCalls.push(command);
    if (command === 'plan_editor_options') {
      // The editor seeds from the current authoring projection of the selected plan.
      const planId = (args as { input?: { authoring?: { plan_id?: string } } } | undefined)?.input?.authoring?.plan_id;
      return { ...options, authoring: fixturePlanAuthoring([plan, customPlan].find(item => item.agent_plan_id === planId)) };
    }
    // Documentation records use independent keys; digests are fixed placeholders read only by diagnostics.
    if (command === 'decision_services_v3') return { schema: 'hiroute.decision-services/v3', services: services.map((service, index) => ({ service, configuration_digest: `sha256:${String(index + 1).repeat(64)}`, credential_state_revision: `sha256:${'a'.repeat(64)}`, source: null, related_sources: [] })) };
    if (command === 'decision_connection_sources') return { sources: [] };
    if (command === 'agent_model_associations') return { complete: true, agents: [] };
    if (command === 'preview_decision_context') return fixtureDecisionContext((args as unknown as { input: Parameters<typeof fixtureDecisionContext>[0] }).input);
    if (command === 'compute_management_snapshot') return { sources: [{ display_name: 'Bailian', display_template_id: 'bailian-token-plan', models: models.map(m => ({ binding_id: m.model_configuration_id })) }] };
    if (command === 'observation_read' && args?.request?.intent.view === 'plan_quality') return queryDocsSamples(samples, args.request.intent.query);
    // The performance view keeps its stage samples; trust evidence stays empty instead of inventing suggestions.
    if (command === 'observation_read' && args?.request?.intent.view === 'plan_trust') {
      const query = args.request.intent.query as unknown as { plan_id: string };
      const counts = { executed: 0, complete: 0, competence_eligible: 0, meets_floor: 0, partial: 0, awaiting_assessment: 0, unscored: 0, sessions: 0 };
      const branchIds = query.plan_id === customPlan.agent_plan_id ? ['writing', 'review'] : ['smart_saving'];
      return { plan_id: query.plan_id, active_revision: customPlan.agent_plan_revision, revision: 'active', from_ms: 0, to_ms: Date.UTC(2026, 9, 8, 9, 40),
        rows: [], sessions: [], session_total: 0, executors: [], announcements: [],
        branches: branchIds.map(branch_id => ({ branch_id, threshold_millis: 800, full_history: counts, upward_cohort: counts, upward_from_ms: 0,
          suggestion: { kind: 'keep', missing: [] },
          replay: { included: 0, excluded_by_reason: {}, recorded_share_at_current: null, estimate_only: true, thresholds: [] } })),
        cost: { kind: 'unavailable', reason: 'no_recorded_requests' } };
    }
    // The configuration views show the trust control without claiming replayed evidence.
    if (command === 'observation_read' && args?.request?.intent.view === 'plan_trust_replay') {
      const thresholds = (args.request.intent.query as unknown as { thresholds_millis: number[] }).thresholds_millis;
      return { included: 0, excluded_by_reason: {}, recorded_share_at_current: null, estimate_only: true,
        thresholds: thresholds.map(threshold_millis => ({ threshold_millis, regular: 0, primary: 0, protected_as_primary: 0, regular_share: null, primary_share: null, newly_regular: 0 })) };
    }
    throw new Error('DOCUMENTATION_DEMO_NO_LIVE_ACTIONS');
  },
} });
const label = view === 'decision-models' ? text('决策模型', 'Decision models')
  : view === 'config' ? text('按难度分流', 'Difficulty routing')
  : view === 'custom-branches' ? text('自定义分支', 'Custom branches')
  : text('模型表现', 'Model performance');
const compact = view === 'session' || view === 'quality-session';
const selectedPlan = view === 'config' ? plan : customPlan;
const style = document.createElement('style');
// Only the surrounding canvas is styled. Product forms and tables are unchanged.
style.textContent = `html,body,#root {height:100%;overflow:hidden}
.docs-capture {height:100%;padding:16px;background:#e6e9f1}
.docs-capture .app-window {border:1px solid var(--border);border-radius:12px;overflow:hidden}
.docs-label {font-size:12px;color:var(--text-muted);font-weight:400}
.docs-excerpt {height:100%;padding:28px;background:var(--bg)}
.docs-excerpt>main {padding:24px;background:var(--surface);border:1px solid var(--border);border-radius:14px}
.docs-excerpt>.docs-label {margin:0 0 16px}`;
document.head.append(style);
const modelTabs = <nav className="model-category-tabs" aria-label={text('模型类型', 'Model categories')}>
  <button type="button" aria-pressed={false}>{text('通用模型', 'General models')}</button>
  <button type="button" aria-pressed>{text('决策模型', 'Decision models')}</button>
</nav>;
createRoot(document.getElementById('root')!).render(
  <PresentationRoot language={language} theme="light" textScale={1} className={compact ? 'docs-excerpt' : 'docs-capture'}>
    {compact ? <>
      <p className="docs-label">HiRoute · {label}</p>
      <main><section className="editor-section"><div className="editor-section-heading"><div>
        <h3>{text('模型表现', 'Model performance')}</h3>
        <p>{text('文章审稿 · 本会话中各模型的执行阶段与评分覆盖范围。', 'Article review · Execution stages and assessment coverage in this session.')}</p>
      </div></div><PlanQuality sessionId="session-review" language={language} compact /></section></main>
    </> : <div className="app-window">
      <header className="titlebar"><div className="window-controls" aria-hidden="true" /><div className="titlebar-main"><div className="window-title">HiRoute</div><span className="docs-label">{label}</span></div></header>
      <HomeNavigation language={language} current={view === 'decision-models' ? 'models' : 'routing'} serviceReady serviceLabel={text('本机服务', 'Local service')}
        items={[
          { id: 'home', label: text('首页', 'Home'), icon: 'home' },
          { id: 'models', label: text('模型', 'Models'), icon: 'models' },
          { id: 'routing', label: text('智能路由', 'Smart routing'), icon: 'route' },
          { id: 'agents', label: 'Agent', icon: 'agent' },
          { id: 'sessions', label: text('会话', 'Sessions'), icon: 'sessions' },
        ]} onNavigate={() => {}} onOpenSettings={() => {}} />
      <main className="main">{view === 'decision-models' ?
        <DecisionServicesPage language={language} active mutable tabs={modelTabs} plans={snapshot.catalog.plans} /> :
        <RoutingPage language={language} snapshot={snapshot} loading={false} busy={false} mutationAllowed
          initialEditor={{ key: selectedPlan.agent_plan_id, plan: selectedPlan }} onRefresh={async () => {}} onOperation={() => {}} />}
      </main>
    </div>}
  </PresentationRoot>,
);
