export type BudgetPeriod = 'daily' | 'weekly' | 'monthly' | 'yearly';
export type BudgetPolicy = {
  currency: string; amount_micros: number;
  renewal: null | { period: BudgetPeriod; mode: 'reset' | 'accumulate'; anchor_local: string; timezone: string; balance_cap_micros: number | null };
};
export type BudgetDefinition = { budget_id: string; revision: number; display_name: string; policy: BudgetPolicy };
export type BudgetView = { definition: BudgetDefinition; balance_micros: number; available_micros: number; reserved_micros: number; spent_micros: number; lifetime_spent_micros: number; next_renewal_at_ms: number | null; key_count: number };
export type KeyBudgetResult = { source_id: string | null; bindings: Record<string, string>; budgets: BudgetView[]; next_cursor: string | null };
export type BudgetAssignment = { intent: 'unlimited' } | { intent: 'share'; budget_id: string; expected_revision: number } | {
  intent: 'configure'; budget_id: string | null; expected_revision: number; display_name: string; policy: BudgetPolicy; initial_remaining_micros: number | null;
};
export type KeyBudgetEdit = { key: { kind: 'saved'; key_id: string } | { kind: 'input'; candidate_ref: string }; assignment: BudgetAssignment };
export type BudgetDraft = {
  enabled: boolean; mode: 'once' | 'periodic' | 'shared'; amount: string; currency: string; period: BudgetPeriod;
  renewalMode: 'reset' | 'accumulate'; anchor: string; timezone: string; cap: string; initial: string;
  existing?: BudgetDefinition; sharedId: string; sharedRevision: number; name: string;
};
const million = 1_000_000;
export function budgetDraft(view?: BudgetView): BudgetDraft {
  const value = view?.definition, renewal = value?.policy.renewal;
  const date = new Date(); date.setMonth(date.getMonth() + 1); date.setHours(0, 0, 0, 0);
  const local = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}T00:00`;
  return { enabled: Boolean(value), mode: renewal ? 'periodic' : value ? 'once' : 'periodic', amount: value ? String(value.policy.amount_micros / million) : '', currency: value?.policy.currency ?? 'USD',
    period: renewal?.period ?? 'monthly', renewalMode: renewal?.mode ?? 'reset', anchor: renewal?.anchor_local.slice(0, 16) ?? local,
    timezone: renewal?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone, cap: renewal?.balance_cap_micros ? String(renewal.balance_cap_micros / million) : '', initial: '', existing: value,
    sharedId: '', sharedRevision: 0, name: value?.display_name ?? 'API allowance' };
}
export function moneyMicros(value: string): number {
  if (!/^\d+(?:\.\d{1,6})?$/.test(value.trim())) throw new Error('KEY_BUDGET_INVALID');
  const [whole, fraction = ''] = value.trim().split('.');
  const amount = Number(whole) * million + Number(fraction.padEnd(6, '0'));
  if (!Number.isSafeInteger(amount) || amount < 0 || amount > 1_000_000_000_000) throw new Error('KEY_BUDGET_INVALID');
  return amount;
}
export function budgetAssignment(draft: BudgetDraft): BudgetAssignment {
  if (!draft.enabled) return { intent: 'unlimited' };
  if (draft.mode === 'shared') {
    if (!draft.sharedId || !draft.sharedRevision) throw new Error('KEY_BUDGET_INVALID');
    return { intent: 'share', budget_id: draft.sharedId, expected_revision: draft.sharedRevision };
  }
  const amount = moneyMicros(draft.amount), cap = draft.mode === 'periodic' && draft.renewalMode === 'accumulate' ? moneyMicros(draft.cap) : null;
  if (!amount || !/^[A-Z]{3}$/.test(draft.currency)) throw new Error('KEY_BUDGET_INVALID');
  if (draft.mode === 'periodic' && (!/^\d{4}-\d\d-\d\dT\d\d:\d\d$/.test(draft.anchor) || !draft.timezone.trim() || (cap !== null && cap < amount))) throw new Error('KEY_BUDGET_INVALID');
  try { if (draft.mode === 'periodic') Intl.DateTimeFormat('en', { timeZone: draft.timezone }).format(); } catch { throw new Error('KEY_BUDGET_INVALID'); }
  const initial = !draft.existing && draft.initial.trim() ? moneyMicros(draft.initial) : null;
  if (initial !== null && initial > amount) throw new Error('KEY_BUDGET_INVALID');
  return { intent: 'configure', budget_id: draft.existing?.budget_id ?? null, expected_revision: draft.existing?.revision ?? 0, display_name: draft.name.trim() || 'API allowance',
    policy: { currency: draft.currency, amount_micros: amount, renewal: draft.mode === 'once' ? null : { period: draft.period, mode: draft.renewalMode, anchor_local: `${draft.anchor}:00`, timezone: draft.timezone.trim(), balance_cap_micros: cap } }, initial_remaining_micros: initial };
}
export function budgetMoney(micros: number, currency: string, language: 'zh' | 'en') {
  return new Intl.NumberFormat(language === 'zh' ? 'zh-CN' : 'en-US', { style: 'currency', currency, maximumFractionDigits: 4 }).format(micros / million);
}
