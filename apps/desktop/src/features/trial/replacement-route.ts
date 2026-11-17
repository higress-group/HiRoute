import type { Editor, Options, Selection } from '../../plan-editor';

type Candidate = Options['candidates'][number];
type SelectionGroup = {
  id: string;
  name: string;
  selections: Selection[];
  primary?: boolean;
  free?: boolean;
};

function selectionGroups(editor: Editor, language: 'zh' | 'en'): SelectionGroup[] {
  const zh = language === 'zh';
  const primary = zh ? '主力模型' : 'Primary models';
  const regular = zh ? '常规模型' : 'Regular models';
  switch (editor.mode) {
    case 'fixed_model':
      return [{ id: 'fixed', name: zh ? '固定候选' : 'Fixed candidates', selections: editor.candidates, primary: true }];
    case 'smart_saving':
      return [
        { id: 'economy', name: regular, selections: editor.smart.economy },
        { id: 'primary', name: primary, selections: editor.smart.primary, primary: true },
      ];
    case 'free_first':
      return [
        { id: 'free', name: zh ? '免费候选' : 'Free candidates', selections: editor.free.candidates, free: true },
        ...(editor.free.primary_fallback ? [{ id: 'fallback', name: primary, selections: editor.free.primary, primary: true }] : []),
      ];
    case 'custom_branches':
      return editor.branch_routing?.branches.flatMap(branch => [
        { id: `branch/${branch.id}/regular`, name: `${branch.name} · ${regular}`, selections: branch.candidates },
        { id: `branch/${branch.id}/primary`, name: `${branch.name} · ${primary}`, selections: branch.primary_candidates, primary: true },
      ]) ?? [];
  }
}

export function replacementSlots(editor: Editor, options: Options, candidate: Candidate, language: 'zh' | 'en') {
  const zh = language === 'zh';
  const names = new Map([
    ...(options.unavailable_candidates ?? []).map(value => [value.binding_id, value.display_name] as const),
    ...options.candidates.map(value => [value.binding_id, value.display_name] as const),
  ]);
  return selectionGroups(editor, language).flatMap(group => group.selections.map((selection, index) => {
    const label = `${group.name} · ${zh ? `第 ${index + 1} 位` : `Position ${index + 1}`}`;
    const reason = group.selections.some(value => value.binding_id === candidate.binding_id)
      ? (zh ? '此组已包含所选接入' : 'This group already uses the selected connection')
      : group.free && candidate.billing_class !== 'free'
        ? (zh ? '此位置仅限免费模型' : 'This position requires a free model')
        : editor.delegation_enabled && editor.work && !candidate.ingress_protocols.includes(editor.work.protocol)
          ? (zh ? '所选接入不支持执行 Agent 的协议' : 'This connection does not support the execution agent’s protocol')
          : '';
    return {
      id: `${group.id}/${index}`, groupId: group.id, index, label, reason, primary: group.primary,
      oldName: names.get(selection.binding_id) ?? selection.binding_id,
    };
  }));
}

export type ReplacementSlot = ReturnType<typeof replacementSlots>[number];

export function replaceRouteModel(editor: Editor, slot: ReplacementSlot, candidate: Candidate, options: Options): Editor {
  const next = structuredClone(editor);
  const group = selectionGroups(next, 'en').find(value => value.id === slot.groupId);
  if (!group?.selections[slot.index] || slot.reason) throw Error('CHANGE_PREVIEW_STALE');
  // Use the new binding's defaults instead of carrying reasoning across models.
  const defaults = options.authoring?.candidate_defaults.find(value => value.binding_id === candidate.binding_id);
  const native = candidate.reasoning;
  const reasoning = editor.follow_request_reasoning && native.kind === 'discrete' && native.default_profile && native.profiles.includes(native.default_profile)
    ? { kind: 'profile' as const, profile: native.default_profile }
    : (slot.primary ? defaults?.primary : defaults?.economy) ?? undefined;
  group.selections[slot.index] = { binding_id: candidate.binding_id, ...(reasoning ? { reasoning: structuredClone(reasoning) } : {}) };
  return next;
}
