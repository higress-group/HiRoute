import { type Editor, type Plan, type Selection } from '../../plan-editor';
import type { Classifier, Judgment } from '../decision-services/types';

export type DivisionSource = { regular: Selection[]; primary: Selection[]; judgment: Judgment; classifier: Classifier; threshold: number };
export function divisionSource(editor: Editor, branchId: string): DivisionSource | null {
  if (editor.mode === 'smart_saving' && branchId === 'smart_saving' && editor.smart.classifier.kind !== 'local_rules' && editor.smart.economy.length && editor.smart.primary.length) {
    return { regular: editor.smart.economy, primary: editor.smart.primary, judgment: editor.smart.judgment, classifier: editor.smart.classifier, threshold: editor.smart.judgment.degree.simple_threshold_millis };
  }
  const routing = editor.branch_routing;
  const branch = routing?.branches.find(b => b.id === branchId);
  if (!branch?.candidates.length || !branch.primary_candidates.length || !routing || routing.classifier.kind === 'local_rules') return null;
  const judgment = branch.judgment ?? routing.judgment;
  return { regular: branch.candidates, primary: branch.primary_candidates, judgment, classifier: routing.classifier, threshold: judgment.degree.simple_threshold_millis };
}

export function divisionDraft(plan: Plan, source: DivisionSource, kind: 'branches' | 'worker', keepPrimary: boolean, descriptions: [string, string], names: [string, string], work: Editor['work'] | undefined, defaults: Editor): Editor {
  const draft = structuredClone(defaults);
  draft.display_name = `${plan.desired.display_name} · ${kind === 'branches' ? names.join(' / ') : 'Worker'}`;
  draft.purpose = descriptions.join(' ');
  draft.mode = 'custom_branches';
  draft.branch_routing = {
    classifier: structuredClone(source.classifier), judgment: structuredClone(source.judgment), default_branch_id: 'implementation',
    branches: [
      { id: 'design', name: names[0], condition: descriptions[0], candidates: structuredClone(kind === 'worker' ? source.regular : source.primary), primary_candidates: [] },
      { id: 'implementation', name: names[1], condition: descriptions[1], candidates: structuredClone(source.regular), primary_candidates: kind === 'branches' && keepPrimary ? structuredClone(source.primary) : [] },
    ],
  };
  draft.delegation_enabled = kind === 'worker';
  if (kind === 'worker' && work) draft.work = structuredClone(work);
  draft.requirements = structuredClone(plan.desired.requirements);
  draft.limits = structuredClone(plan.desired.limits);
  return draft;
}
