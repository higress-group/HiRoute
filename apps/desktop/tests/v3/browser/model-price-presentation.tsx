// Real model management and price editor; IPC is controlled and makes no provider calls.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { ModelManagementPage } from '../../../src/product/ModelManagementPage';
import { PresentationRoot } from '../../../src/ui';
import { mockProductInvoke, readyManagement } from './product-fixtures';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const language = params.get('lang') === 'zh' ? 'zh' : 'en';
const theme = params.get('theme') === 'dark' ? 'dark' : 'light';
const management = structuredClone(readyManagement);
const kind = params.get('kind');
const source = management.sources[kind === 'subscription' ? 0 : kind === 'free' ? 2 : 1];
source.models = [source.models[0]];
if (kind !== 'subscription' && kind !== 'free') {
  source.models[0].presentation!.price_contexts = [{ currency: 'USD', valuation_kind: 'api_equivalent' }];
}
management.sources = [source];
const rateCases: Record<string, [string | null, string | null, string | null, string | null]> = {
  unknown: [null, null, null, null],
  input: ['0.8', null, null, null],
  output: [null, '2', null, null],
  zero_input: ['0', '2', null, null],
  zero_output: ['0.8', '0', null, null],
  known_cache: ['0.8', '2', '0.05', '0.01'],
};
const rates = rateCases[params.get('rates') ?? 'unknown'];
const fixture = { requests: [] as { command: string; payload: any }[], failNext: params.has('fail') };
Object.assign(window, { modelPriceFixture: fixture });
mockIPC((command, payload: any) => {
  fixture.requests.push({ command, payload });
  if (command === 'compute_management_snapshot') return structuredClone(management);
  if (command === 'effective_price_query') {
    if (fixture.failNext) { fixture.failNext = false; throw new Error('PRICE_QUERY_UNAVAILABLE'); }
    return {
      result: { pending_activation: false, evaluated_at: 1, generation_ref: null,
        items: payload.input.targets.map((target: any) => ({ query_id: target.query_id,
          quote: { origin: 'catalog', quote_digest: 'sha256:fixture', unknown_reasons: [] },
          edit_context: { target_locator: target.target_locator, expected_source_revision: source.revision,
            expected_binding_revision: source.models[0].revision, expected_override_revision: 6 } })) },
      display_rates: payload.input.targets.map(() => rates),
    };
  }
  return mockProductInvoke(command, payload, 'ready');
});

createRoot(document.getElementById('root')!).render(
  <PresentationRoot language={language} theme={theme} textScale={1}>
    <div className="app-window"><header className="titlebar" /><aside className="sidebar" /><main className="main"><div>
      <ModelManagementPage language={language} active trustedAuthority refreshVersion={0}
        onOperation={() => undefined} onRecoveryRefresh={() => undefined} onChanged={() => undefined} />
    </div></main></div>
  </PresentationRoot>,
);
