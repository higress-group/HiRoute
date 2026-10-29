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
if (params.has('two')) {
  const other = structuredClone(source);
  other.source_id = 'source/fixture/other';
  other.display_name = 'Other fixture source';
  other.models[0].model_ref = 'model-ref/fixture-other';
  other.models[0].binding_id = 'binding/fixture/other';
  other.models[0].display_name = 'Other fixture model';
  if (params.get('two') === 'same') {
    source.models.push(other.models[0]);
    source.ready_model_count = 2;
  } else management.sources.push(other);
}
const rateCases: Record<string, [string | null, string | null, string | null, string | null]> = {
  unknown: [null, null, null, null],
  input: ['0.8', null, null, null],
  output: [null, '2', null, null],
  zero_input: ['0', '2', null, null],
  zero_output: ['0.8', '0', null, null],
  known_cache: ['0.8', '2', '0.05', '0.01'],
};
const rates = rateCases[params.get('rates') ?? 'unknown'];
const primaryBinding = source.models[0].binding_id;
const quotes: Record<string, { rates: (string | null)[]; origin: string; revision: number }> = {
  [primaryBinding]: { rates: [...rates], origin: 'catalog', revision: 6 },
  'binding/fixture/other': { rates: ['9', '10', null, null], origin: 'catalog', revision: 6 },
};
const fixture = { requests: [] as { command: string; payload: any }[], failNext: params.has('fail'),
  primaryBinding, quotes, get rates() { return quotes[primaryBinding].rates; },
  get revision() { return quotes[primaryBinding].revision; }, outcome: 'success',
  releaseSave: null as null | (() => void) };
Object.assign(window, { modelPriceFixture: fixture });
mockIPC((command, payload: any) => {
  fixture.requests.push({ command, payload });
  if (command === 'compute_management_snapshot') return structuredClone(management);
  if (command === 'effective_price_query') {
    if (fixture.failNext) { fixture.failNext = false; throw new Error('PRICE_QUERY_UNAVAILABLE'); }
    return {
      result: { pending_activation: false, evaluated_at: 1, generation_ref: null,
        items: payload.input.targets.map((target: any) => ({ query_id: target.query_id,
          quote: { origin: params.has('stateful') ? quotes[target.target_locator.binding_id].origin : 'catalog', quote_digest: 'sha256:fixture', unknown_reasons: [] },
          edit_context: { target_locator: target.target_locator, expected_source_revision: source.revision,
            expected_binding_revision: source.models[0].revision, expected_override_revision: quotes[target.target_locator.binding_id].revision } })) },
      display_rates: payload.input.targets.map((target: any) => quotes[target.target_locator.binding_id].rates),
    };
  }
  if (command === 'preview_price_change' && params.has('stateful')) {
    if (fixture.outcome === 'failure') throw { code: 'REVISION_CONFLICT' };
    if (fixture.outcome === 'cancel') return { state: 'cancelled_before_apply', operation: null };
    if (fixture.outcome === 'pending' || fixture.outcome === 'failed_operation') return {
      state: fixture.outcome, operation: { operation_id: 'operation/fixture', state: fixture.outcome === 'pending' ? 'accepted' : 'failed', sequence: 1, cancellable: false },
    };
    const complete = () => {
      const action = payload.input.action;
      const quote = quotes[payload.input.target_locator.binding_id];
      quote.rates = action.kind === 'follow_catalog' ? [null, null, null, null]
        : [action.input_uncached, action.output, action.cache_read, action.cache_write];
      quote.origin = action.kind === 'follow_catalog' ? 'catalog' : 'manual';
      quote.revision += 1;
      return { state: 'applied', operation: { operation_id: 'operation/fixture', state: 'succeeded', sequence: 2, cancellable: false } };
    };
    if (fixture.outcome === 'hold') return new Promise(resolve => { fixture.releaseSave = () => { fixture.releaseSave = null; resolve(complete()); }; });
    return complete();
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
