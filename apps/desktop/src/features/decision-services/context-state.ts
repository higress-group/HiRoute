import type { DecisionContextPolicy, DecisionWireProtocol } from './contracts.ts';

/** Strings retain invalid edits and parked custom values while Automatic is selected. */
export type DecisionContextDraft = {
  mode: 'automatic' | 'custom'; window: string; question: string; typesafe_long_context: 'automatic' | 'disabled';
};
type Binding = { model: string; endpoint: string; wire_protocol: DecisionWireProtocol | null; context?: DecisionContextDraft };
export function contextDraft(bundle: Binding): DecisionContextDraft {
  return bundle.context ?? { mode: 'automatic', window: '', question: '', typesafe_long_context: 'automatic' };
}
export function contextFromPolicy(policy: DecisionContextPolicy): DecisionContextDraft {
  return { mode: policy.mode, window: policy.window_tokens?.toString() ?? '', question: policy.per_question_tokens?.toString() ?? '', typesafe_long_context: policy.typesafe_long_context };
}
function positive(value: string): number | null {
  const number = Number(value);
  return /^[0-9]+$/.test(value) && Number.isSafeInteger(number) && number > 0 && number <= 4_294_967_295 ? number : null;
}
export function contextErrors(bundle: Binding, language: 'zh' | 'en'): { context_window?: string; context_question?: string } {
  const draft = contextDraft(bundle), errors: { context_window?: string; context_question?: string } = {};
  if (draft.mode === 'automatic') return errors;
  const window = positive(draft.window), question = draft.question ? positive(draft.question) : null;
  if (window === null) errors.context_window = language === 'zh' ? '请输入 1 到 4,294,967,295 之间的整数。' : 'Enter an integer from 1 to 4,294,967,295.';
  if (draft.question && question === null) errors.context_question = language === 'zh' ? '问题上限需为 1 到 4,294,967,295 之间的整数。' : 'Use an integer from 1 to 4,294,967,295 for the question limit.';
  else if (question !== null && window !== null && question > window) errors.context_question = language === 'zh'
    ? `问题上限 ${question.toLocaleString()} 超过总窗口 ${window.toLocaleString()}。请降低问题上限或提高总窗口。`
    : `The question limit (${question.toLocaleString()}) exceeds the request window (${window.toLocaleString()}). Lower the question limit or raise the request window.`;
  return errors;
}
export function contextPolicy(bundle: Binding): DecisionContextPolicy | null {
  if (Object.keys(contextErrors(bundle, 'en')).length) return null;
  const draft = contextDraft(bundle);
  return draft.mode === 'automatic' ? { mode: 'automatic', typesafe_long_context: draft.typesafe_long_context }
    : { mode: 'custom', window_tokens: positive(draft.window)!, ...(draft.question ? { per_question_tokens: positive(draft.question)! } : {}), typesafe_long_context: draft.typesafe_long_context };
}
export function contextSignature(bundle: Binding): string {
  return JSON.stringify({ endpoint: bundle.endpoint.trim(), model: bundle.model.trim(), wire_protocol: bundle.wire_protocol, policy: contextPolicy(bundle) });
}
export function typesafeDestination(bundle: Binding): boolean {
  try {
    const url = new URL(bundle.endpoint.trim());
    return bundle.wire_protocol === 'system_one' && url.protocol === 'https:' && url.hostname === 'api.typesafe.ai'
      && (!url.port || url.port === '443') && url.pathname === '/v1/systemone' && !url.username && !url.password
      && !url.href.includes('?') && !url.href.includes('#');
  } catch { return false; }
}
