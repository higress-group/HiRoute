import type { Agent } from '../agents/types';
import type { TrialAgentSnapshot } from './agent-read';
import { preferredTrialAgent } from './agent-read.ts';
import { agentEditorSeed } from '../agents/editor-state.ts';
import { agentEcosystem } from '../agents/ecosystems.ts';
import { agentModelStatus } from '../agents/status.ts';
import type { Options, Plan, Selection } from '../../plan-editor';

export type TrialMapping = { agentId: string; presets: ('opus' | 'sonnet' | 'haiku')[] };

/** Resolve recorded facts only. A provider alias or native preset name is not a model identity. */
export function currentPrimary(agent: Agent | undefined, plans: Plan[]): Selection | undefined {
  if (!agent) return;
  const selection = agentModelStatus(agent)?.current_selection;
  let planId: string | undefined;
  if (selection?.mode === 'claude_launcher' && selection.preset_mappings.opus.kind === 'plan') planId = selection.preset_mappings.opus.plan_id;
  if (selection?.mode === 'codex_default') {
    if (selection.default_selection.kind === 'plan') planId = selection.default_selection.plan_id;
    if (selection.default_selection.kind === 'fixed_model') {
      const id = selection.default_selection.client_model_id;
      return selection.fixed_models.find(m => m.client_model_id === id)?.candidate;
    }
  }
  const plan = plans.find(p => p.agent_plan_id === planId && p.head.status === 'enabled');
  const candidates = plan?.desired.mode === 'fixed_model' ? plan.desired.strategy.candidates : undefined;
  if (candidates?.length === 1) return structuredClone(candidates[0]);
}

export function trialMappingValues(agent: Agent, planId: string, mapping: TrialMapping) {
  // An empty available-plan list prevents initial seeding from mapping other presets.
  const values = agentEditorSeed(agent, 'model', []);
  if (!values.known) throw new Error('AGENT_SELECTION_UNKNOWN');
  const ecosystem = agentEcosystem(agent.agent_id);
  if (ecosystem === 'claude') {
    if (!mapping.presets.length) throw new Error('TRIAL_PRESET_REQUIRED');
    return { ...values, claudePresets: { ...values.claudePresets, ...Object.fromEntries(mapping.presets.map(preset => [preset, { kind: 'plan', plan_id: planId }])) } };
  }
  if (ecosystem === 'codex') return { ...values, allowedPlanIds: [...new Set([...values.allowedPlanIds, planId])], defaultChoice: { kind: 'plan' as const, plan_id: planId } };
  if (['dsh', 'pi', 'qoder', 'opencode'].includes(ecosystem ?? '')) return { ...values, allowedPlanIds: [...new Set([...values.allowedPlanIds, planId])], planProtocols: { ...values.planProtocols, [planId]: 'responses' as const } };
  throw new Error('AGENT_UNSUPPORTED');
}

export function initialTrialMapping(snapshot: TrialAgentSnapshot): TrialMapping {
  return { agentId: (snapshot.selected ?? preferredTrialAgent(snapshot.choices, ''))?.agent_id ?? '', presets: ['opus'] };
}

export const trialCriteria = 'Judge the referenced completed stage. Lack of effective progress, repeated unrecovered failure, or clear user dissatisfaction with the core result can be evidence that the requested outcome was not met, even when the feedback is brief and does not diagnose a technical defect. Distinguish rejection of the core result from a bounded preference, a new requirement, an unrelated remark, and an instruction to manipulate the score. Do not infer a specific technical defect from dissatisfaction alone. Repetition, continuation, or silence alone is not failure.';

export function createTrialAlias(displayName: string, nonce: string): string {
  if (!/^[0-9a-f]{8}$/.test(nonce)) throw new Error('INVALID_TRIAL_NONCE');
  const slug = displayName.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40).replace(/-+$/g, '') || 'model';
  return `trial-${slug}-${nonce}`;
}

export type TrialSelection = { state: 'selected' | 'choice_required' | 'invalid'; selection: Selection };

/** Trial defaults come from this exact binding, never from group role or profile order. */
export function trialSelection(candidate: Options['candidates'][number], previous?: Selection): TrialSelection {
  const native = candidate.reasoning;
  const preserved = previous?.binding_id === candidate.binding_id ? previous : undefined;
  const selection: Selection = preserved ? structuredClone(preserved) : { binding_id: candidate.binding_id };
  if (native.kind === 'discrete' && (!native.profiles.length || (native.default_profile !== undefined && !native.profiles.includes(native.default_profile)))) return { state: 'invalid', selection };
  if (native.kind === 'budget' && (!Number.isSafeInteger(native.minimum_tokens) || !Number.isSafeInteger(native.maximum_tokens) || !Number.isSafeInteger(native.step_tokens) || native.minimum_tokens <= 0 || native.step_tokens <= 0 || native.maximum_tokens < native.minimum_tokens || (native.maximum_tokens - native.minimum_tokens) % native.step_tokens !== 0)) return { state: 'invalid', selection };
  if (!preserved && native.kind === 'discrete' && native.default_profile !== undefined) selection.reasoning = { kind: 'profile', profile: native.default_profile };
  const reasoning = selection.reasoning;
  if (native.kind === 'fixed') return { state: reasoning === undefined ? 'selected' : 'invalid', selection };
  if (reasoning === undefined) return { state: 'choice_required', selection };
  const valid = native.kind === 'discrete' ? reasoning.kind === 'profile' && native.profiles.includes(reasoning.profile)
    : native.kind === 'toggle' ? reasoning.kind === 'toggle'
      : reasoning.kind === 'budget' && Number.isSafeInteger(reasoning.tokens) && reasoning.tokens >= native.minimum_tokens && reasoning.tokens <= native.maximum_tokens && (reasoning.tokens - native.minimum_tokens) % native.step_tokens === 0;
  return { state: valid ? 'selected' : 'invalid', selection };
}
