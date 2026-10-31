import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { Models } from '../../../src/features/models/Models';
import { PresentationRoot } from '../../../src/ui';
import { readyManagement, idleModelFeedStatus } from './product-fixtures';
import '../../../src/occami/styles.css';
const params = new URLSearchParams(location.search), language = params.get('lang') === 'en' ? 'en' : 'zh';
const source = structuredClone(readyManagement.sources[1]);
source.connection_identity = { ...source.connection_identity!, access_kind: 'api', connection_option_id: 'openai.platform.global.v1', product_label: 'OpenAI API' };
source.keys = [{ key_id: 'key/budget', generation: 2, fingerprint_hint: 'budget', enabled: true, model_statuses: [] }];
const snapshot = { ...structuredClone(readyManagement), sources: [source] };
const budget = { definition: { budget_id: 'budget/monthly', revision: 3, display_name: 'Monthly API credits with a long recognizable name', policy: { currency: 'USD', amount_micros: 100_000_000, renewal: { period: 'monthly', mode: 'reset', anchor_local: '2026-11-30T09:00:00', timezone: 'America/Los_Angeles', balance_cap_micros: null } } }, balance_micros: 75_000_000, available_micros: 70_000_000, reserved_micros: 5_000_000, spent_micros: 25_000_000, lifetime_spent_micros: 125_000_000, next_renewal_at_ms: Date.UTC(2026, 10, 30, 17), key_count: 2 };
const trace: any[] = [];
mockIPC((command, args: any) => {
  if (command === 'compute_key_budgets') { if (params.has('unavailable')) throw new Error('unavailable'); return { source_id: args.input.source_id, bindings: { 'key/budget': 'budget/monthly' }, budgets: [budget], next_cursor: null }; }
  if (command === 'model_feed_status') return structuredClone(idleModelFeedStatus);
  throw new Error(`Unexpected optional read ${command}`);
});
Object.assign(window, { budgetTrace: trace });
createRoot(document.getElementById('root')!).render(<PresentationRoot language={language} theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={1}>
  <Models language={language} snapshot={snapshot} mutable onRefresh={async () => {}} onPrepareKeyInput={async () => { throw new Error('No key edits expected'); }}
    onReadKeyMasks={async () => ({ source_id: source.source_id, source_revision: source.revision, keys: [{ key_id: 'key/budget', generation: 2, masked_key: 'sk-••••••••test' }] })}
    onPreview={async (change, budgets) => { trace.push({ change, budgets }); return { spec: {} as any, accept_digest: '', expected_revisions: snapshot.revisions, changes: [] }; }}
    onApply={async () => params.has('pending') ? await new Promise<'saved'>(resolve => { Object.assign(window, { finishBudgetSave: () => resolve('saved') }); }) : 'saved'} onDiscardProtectedInputs={() => {}} />
</PresentationRoot>);
