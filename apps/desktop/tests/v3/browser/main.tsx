import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { Agents } from '../../../src/agents';
import { Sessions } from '../../../src/features/Sessions';
import { Home, HomeNavigation, type HomeAction, type HomeNavigationItem } from '../../../src/features/home';
import { ModelManagementPage } from '../../../src/product/ModelManagementPage';
import { RoutingPage } from '../../../src/product/RoutingPage';
import { SettingsPage } from '../../../src/product/SettingsPage';
import { PresentationRoot, parseLanguage, parseTextScale, parseTheme, resolveLanguage, resolveTheme, type Language, type LanguagePreference, type TextScale, type ThemePreference } from '../../../src/ui';
import { dailyPlan, fixtureTrace, mockProductInvoke, productHomeReads, productTaskRead, readyAgents, readyDesktop, resetFixtureTrace, type ProductScenario } from './product-fixtures';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const scenarioNames: ProductScenario[] = ['fresh', 'ready', 'collaboration', 'gap', 'configured_no_sessions', 'drift', 'cooling', 'free_exhausted', 'api_failure', 'unknown_model', 'agent_blocked', 'task_cancelled'];
const rawScenario = params.get('scenario');
const scenario = (rawScenario === 'daily'
  ? 'ready'
  : rawScenario === 'configured'
    ? 'configured_no_sessions'
    : scenarioNames.includes(rawScenario as ProductScenario)
      ? rawScenario
      : 'ready') as ProductScenario;
const availablePages = ['home', 'models', 'routing', 'agents', 'sessions', 'settings'] as const;
type Page = typeof availablePages[number];
const pageParam = params.get('page');
const initialPage: Page = availablePages.includes(pageParam as Page) ? pageParam as Page : 'home';
const freshProduct = scenario === 'fresh';

resetFixtureTrace();
(window as typeof window & { __HIRouteFixtureTrace?: typeof fixtureTrace }).__HIRouteFixtureTrace = fixtureTrace;
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: { writeText: async (value: string) => { fixtureTrace.clipboard.push(value); } },
});
mockIPC((command, payload) => mockProductInvoke(command, payload as Record<string, any> | undefined, scenario));

const wait = (milliseconds: number) => new Promise(resolve => window.setTimeout(resolve, milliseconds));
const clickButton = (label: string) => {
  const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.includes(label));
  button?.click();
  return button;
};
function setNativeValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function items(language: Language): HomeNavigationItem[] {
  return language === 'zh'
    ? [
        { id: 'home', label: '首页', icon: 'home' },
        { id: 'models', label: '模型', icon: 'models' },
        { id: 'routing', label: '智能路由', icon: 'route' },
        { id: 'agents', label: 'Agent', icon: 'agent' },
        { id: 'sessions', label: '会话', icon: 'sessions' },
      ]
    : [
        { id: 'home', label: 'Home', icon: 'home' },
        { id: 'models', label: 'Models', icon: 'models' },
        { id: 'routing', label: 'Smart routing', icon: 'route' },
        { id: 'agents', label: 'Agent', icon: 'agent' },
        { id: 'sessions', label: 'Sessions', icon: 'sessions' },
      ];
}

function Harness() {
  const [languagePreference, setLanguagePreference] = useState<LanguagePreference>(() => parseLanguage(params.get('lang'), 'zh'));
  const language = resolveLanguage(languagePreference, navigator.language);
  const [theme, setTheme] = useState<ThemePreference>(() => parseTheme(params.get('theme')));
  const [textScale, setTextScale] = useState<TextScale>(() => parseTextScale(params.get('scale')));
  const [page, setPage] = useState<Page>(initialPage);
  const [newRouteFromModel, setNewRouteFromModel] = useState<string | null>(null);
  const reads = productHomeReads(scenario);
  const taskRead = productTaskRead(scenario);
  const serviceReady = reads.service.status === 'ready' && reads.service.data.daemon === 'running' && reads.service.data.gateway === 'ready';
  const pageLabels: Record<Page, [string, string]> = { home: ['首页', 'Home'], models: ['模型', 'Models'], routing: ['智能路由', 'Smart routing'], agents: ['Agent', 'Agent'], sessions: ['会话', 'Sessions'], settings: ['设置', 'Settings'] };
  const labels = language === 'zh' ? { service: '仅在本机运行' } : { service: 'Running locally' };
  const action = (_value: HomeAction) => undefined;

  useEffect(() => {
    const capture = params.get('capture');
    let active = true;
    delete document.documentElement.dataset.hirouteCaptureReady;
    async function prepareCapture() {
      await wait(120);
      if (!active) return;
      if (capture === 'api_failure') {
        clickButton('添加模型');
        await wait(120);
        clickButton('连接 API');
        await wait(120);
        const password = document.querySelector<HTMLInputElement>('.modal input[type="password"]');
        if (password) setNativeValue(password, 'fixture-secret');
        await wait(40);
        clickButton('检查接入');
      } else if (capture === 'all_candidates') {
        await wait(420);
        clickButton('添加模型');
      } else if (capture === 'session_drawer') {
        await wait(180);
        clickButton('运行记录');
      } else if (capture === 'task_cancelled') {
        clickButton('任务记录');
      } else if (capture === 'task_routing_only') {
        await wait(600);
        clickButton('保存配置');
      }
      await wait(260);
      if (active) document.documentElement.dataset.hirouteCaptureReady = 'true';
    }
    void prepareCapture().catch(error => {
      document.documentElement.dataset.hirouteCaptureError = error instanceof Error ? error.message : String(error);
    });
    return () => { active = false; delete document.documentElement.dataset.hirouteCaptureReady; };
  }, []);

  const content = page === 'home' ? <Home language={language} reads={reads} hasTasks={taskRead.status === 'ready' && taskRead.tasks.length > 0} onAction={action} />
    : page === 'models' ? <ModelManagementPage language={language} active={page === 'models'} trustedAuthority refreshVersion={0} startAdding={params.get('modal') === 'add'} plans={freshProduct ? [] : readyDesktop.catalog.plans} agents={freshProduct ? [] : readyAgents.agents} onOpenPlan={() => setPage('routing')} onCreatePlan={bindingId => { setNewRouteFromModel(bindingId); setPage('routing'); }} onOperation={() => undefined} onRecoveryRefresh={() => undefined} onChanged={() => undefined} />
      : page === 'routing' ? <RoutingPage language={language} snapshot={freshProduct ? { ...readyDesktop, catalog: { plans: [], drafts: [] } } : readyDesktop} agentSnapshot={freshProduct ? { ...readyAgents, agents: [] } : readyAgents} loading={false} busy={false} initialEditor={newRouteFromModel ? { key: 'new/from-model', initialBindingId: newRouteFromModel } : freshProduct ? null : { key: dailyPlan.agent_plan_id, plan: dailyPlan }} onRefresh={async () => undefined} onOperation={() => undefined} onOpenAgent={() => setPage('agents')} />
    : page === 'agents' ? <Agents language={language} initialAgentId={params.get('capture') === 'task_routing_only' ? 'agent_claude_default' : null} initialFacet="collaboration" initialTab={scenario === 'task_cancelled' ? 'tasks' : 'configuration'} initialTaskId={scenario === 'task_cancelled' ? 'task/fix-empty-list' : null} taskRead={taskRead} onTaskCancel={async (_task, onAccepted) => { fixtureTrace.commands.push('fixture_task_cancel'); onAccepted?.('cancelling'); await wait(1200); return 'cancelled'; }} onOpenTaskSession={() => setPage('sessions')} onCreatePlan={() => setPage('routing')} onMutation={() => undefined} />
          : page === 'sessions' ? <Sessions language={language} initialSession={scenario === 'gap' ? 'session/stream-gap' : null} onOpenAgents={() => setPage('agents')} />
            : <SettingsPage active language={language} languagePreference={languagePreference} theme={theme} textScale={textScale} serviceLabel={labels.service} serviceReady={serviceReady} onLanguageChange={setLanguagePreference} onThemeChange={setTheme} onTextScaleChange={setTextScale} onOpenSessions={() => setPage('sessions')} />;

  return <PresentationRoot language={language} theme={resolveTheme(theme, matchMedia('(prefers-color-scheme: dark)').matches)} textScale={textScale}>
    <div className="app-window">
      <header className="titlebar">
        <div className="window-controls" />
        <div className="titlebar-main"><div className="window-title"><span>HiRoute</span><span>/</span><span>{pageLabels[page][language === 'zh' ? 0 : 1]}</span></div></div>
      </header>
      <HomeNavigation language={language} items={items(language)} current={page} serviceLabel={labels.service} serviceReady={serviceReady} onNavigate={id => setPage(id as Page)} onOpenSettings={() => setPage('settings')} />
      <main className="main">
        {content}
      </main>
    </div>
  </PresentationRoot>;
}

createRoot(document.getElementById('root')!).render(<Harness />);
