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

export function trialSelection(candidate: Options['candidates'][number]): Selection {
  const native = candidate.reasoning;
  const reasoning: Selection['reasoning'] = native.kind === 'toggle' ? { kind: 'toggle', enabled: true }
    : native.kind === 'discrete' ? { kind: 'profile', profile: native.profiles.at(-1)! }
    : native.kind === 'budget' ? { kind: 'budget', tokens: native.minimum_tokens } : undefined;
  return { binding_id: candidate.binding_id, ...(reasoning && { reasoning }) };
}
