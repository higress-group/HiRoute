import type { DecisionContextPreviewInput, ResolvedDecisionContext } from '../../../src/features/decision-services/contracts';
import contract from '../../decision-services-v3-contract.json';

/** Controlled IPC evidence only. Native acceptance exercises the production resolver. */
export function fixtureDecisionContext(input: DecisionContextPreviewInput): ResolvedDecisionContext {
  const fallback = contract.preview_decision_context.result as ResolvedDecisionContext;
  const direct = input.endpoint === 'https://api.typesafe.ai/v1/systemone' && input.wire_protocol === 'system_one';
  const enabled = direct && input.policy.typesafe_long_context === 'automatic';
  const knownTypeSafe = direct && ['jev-1.13.0', 'jev-1.14.0', 'jev-latest'].includes(input.model);
  const routerAlias = input.endpoint === 'https://openrouter.ai/api/alpha/decisions' && input.wire_protocol === 'system_one' && input.model === '~typesafe/jev-latest';
  const openai = input.endpoint === 'https://api.openai.com/v1/decisions' && input.wire_protocol === 'openai_decisions' && input.model === 'gpt-6-luna';
  const known = knownTypeSafe || openai || routerAlias;
  const maintained = openai ? 1050000 : routerAlias ? 262144 : knownTypeSafe ? enabled ? 262144 : 64000 : 32000;
  const request = input.policy.window_tokens ?? maintained;
  const providerQuestion = knownTypeSafe ? enabled ? 262144 : 32000 : null;
  const question = input.policy.per_question_tokens ?? Math.min(request, providerQuestion ?? request);
  const requestSource = input.policy.mode === 'custom' ? 'user' : known ? 'metadata' : 'fallback';
  const resolved: ResolvedDecisionContext = { ...fallback, request_window_tokens: request, per_question_window_tokens: question,
    request_source: requestSource,
    per_question_source: input.policy.per_question_tokens ? 'user' : providerQuestion !== null ? 'metadata' : requestSource,
    typesafe_long_context_header: enabled,
    metadata: known ? { binding_key: 'fixture/decision', interface_key: 'fixture/interface', constraint_key: enabled ? 'fixture/long' : 'fixture/default', basis: openai ? 'maintainer_policy' : 'inferred', evidence_refs: ['fixture/public-source'], note: 'Synthetic metadata for browser presentation.' } : null };
  // Synthetic identity varies with every input. This is not a runtime snapshot digest oracle.
  let hash = 0x811c9dc5;
  for (const char of JSON.stringify(input)) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0;
  resolved.context_digest = 'sha256:' + hash.toString(16).padStart(8, '0').repeat(8);
  return resolved;
}

export const fixtureDecisionRoute = (destination: string) => ({ mode: 'direct' as const, route: 'direct' as const, reason: 'policy_direct' as const, destination, state_revision: 'sha256:' + '9'.repeat(64), failure: null });
