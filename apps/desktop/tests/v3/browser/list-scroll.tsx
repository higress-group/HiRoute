// Real product pages with synthetic IPC data; no daemon or provider calls.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { ModelManagementPage } from '../../../src/product/ModelManagementPage';
import { RoutingPage } from '../../../src/product/RoutingPage';
import { PresentationRoot, parseTextScale } from '../../../src/ui';
import { dailyPlan, mockProductInvoke, readyDesktop, readyManagement } from './product-fixtures';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const page = params.get('page') === 'routing' ? 'routing' : 'models';
const warning = params.get('warning') === 'true';
const count = params.get('empty') === 'true' ? 0 : 40;
const management = structuredClone(readyManagement);
const source = management.sources[0];
source.models = Array.from({ length: count }, (_, i) => ({
  ...source.models[0], model_ref: `scroll-model-${i + 1}`, binding_id: `scroll-binding-${i + 1}`,
  display_name: `Scroll model ${String(i + 1).padStart(2, '0')}`,
}));
management.sources = count ? [source] : [];
const plans = Array.from({ length: count }, (_, i) => ({
  ...dailyPlan, agent_plan_id: `scroll-route-${i + 1}`, model_alias: `scroll-route-${i + 1}`,
  desired: { ...dailyPlan.desired, display_name: `Scroll route ${String(i + 1).padStart(2, '0')}` },
}));
const snapshot = {
  ...readyDesktop, catalog: { plans, drafts: [] },
  catalog_error: warning ? 'SCROLL_FIXTURE_UNAVAILABLE' : null,
  trusted_authority: !warning,
};
mockIPC((command, payload) => command === 'compute_management_snapshot'
  ? management : mockProductInvoke(command, payload as Record<string, any> | undefined, 'ready'));

createRoot(document.getElementById('root')!).render(
  <PresentationRoot language="en" theme="dark" textScale={parseTextScale(params.get('scale'))}>
    <div className="app-window">
      <header className="titlebar" />
      <aside className="sidebar">Scroll regression fixture</aside>
      <main className="main"><div>
        {page === 'models'
          ? <ModelManagementPage language="en" active trustedAuthority={!warning} refreshVersion={0}
              onOperation={() => undefined} onRecoveryRefresh={() => undefined} onChanged={() => undefined} />
          : <RoutingPage language="en" snapshot={snapshot} loading={false} busy={false}
              onRefresh={async () => undefined} onOperation={() => undefined} />}
      </div></main>
    </div>
  </PresentationRoot>,
);
