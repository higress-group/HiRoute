import { budgetAssignment, type BudgetDraft, type KeyBudgetEdit } from '../key-budgets/types';
import { randomId } from '../../random-id.ts';
import type { KeyEdit, ManagedSource, PreparedKeyInput, ProtectedKeyInput } from './types';

export type DraftKey = {
  draftId: string;
  keyId?: string;
  expectedGeneration: number;
  fingerprintHint: string;
  enabled: boolean;
  removed: boolean;
  replacement: string;
  budget?: BudgetDraft;
};

export type ModelDraft = {
  selectedModels: string[];
  enabled: boolean;
  keys: DraftKey[];
  addedSecret: string;
};

export function createDraft(source: ManagedSource): ModelDraft {
  return {
    selectedModels: source.models.map(model => model.model_ref),
    enabled: source.state !== 'disabled',
    keys: source.keys.map(key => ({
      draftId: key.key_id,
      keyId: key.key_id,
      expectedGeneration: key.generation,
      fingerprintHint: key.fingerprint_hint,
      enabled: key.enabled,
      removed: false,
      replacement: '',
    })),
    addedSecret: '',
  };
}

export function addDraftKey(draft: ModelDraft): ModelDraft {
  const value = draft.addedSecret;
  if (!value.trim()) return draft;
  return {
    ...draft,
    addedSecret: '',
    keys: [...draft.keys, {
      draftId: randomId(),
      expectedGeneration: 0,
      fingerprintHint: '待保存',
      enabled: true,
      removed: false,
      replacement: value,
    }],
  };
}

export function moveDraftKey(draft: ModelDraft, draftId: string, offset: -1 | 1): ModelDraft {
  const keys = [...draft.keys];
  const from = keys.findIndex(key => key.draftId === draftId);
  const to = from + offset;
  if (from < 0 || to < 0 || to >= keys.length) return draft;
  [keys[from], keys[to]] = [keys[to], keys[from]];
  return { ...draft, keys };
}

/** Only the currently entered value; saved material is masked by the service. */
export function maskEnteredKey(value: string): string {
  const chars = [...value];
  if (chars.length <= 2 || /[\u0000-\u001f\u007f-\u009f]/.test(value)) return '••••••••';
  if (chars.length <= 12) return `••••••••${chars.slice(-2).join('')}`;
  return `${chars.slice(0, 4).join('')}••••••••${chars.slice(-4).join('')}`;
}

export function protectedInputs(draft: ModelDraft): ProtectedKeyInput[] {
  return draft.keys
    .filter(key => !key.removed && key.replacement.length > 0)
    .map(key => ({
      draft_id: key.draftId,
      key_id: key.keyId,
      expected_generation: key.expectedGeneration,
      value: key.replacement,
    }));
}

export function clearSecrets(draft: ModelDraft): ModelDraft {
  return {
    ...draft,
    addedSecret: '',
    keys: draft.keys.map(key => ({ ...key, replacement: '' })),
  };
}

/** Submit the entire final membership and order in the same atomic save. */
export async function buildCredentialEdits(
  source: ManagedSource,
  draft: ModelDraft,
  prepare: (sourceId: string, input: ProtectedKeyInput) => Promise<PreparedKeyInput>,
): Promise<KeyEdit[]> {
  const edits: KeyEdit[] = [];
  const orderedIds: string[] = [];
  for (const key of draft.keys) {
    if (key.removed) {
      if (key.keyId) edits.push({ action: 'remove', key_id: key.keyId, expected_generation: key.expectedGeneration });
      continue;
    }
    let keyId = key.keyId;
    if (key.replacement) {
      const prepared = await prepare(source.source_id, {
        draft_id: key.draftId, key_id: key.keyId,
        expected_generation: key.expectedGeneration, value: key.replacement,
      });
      if (!prepared.key_id) throw new Error('CREDENTIAL_INPUT_INVALID');
      keyId ??= prepared.key_id;
      edits.push(key.keyId
        ? { action: 'replace', key_id: key.keyId, expected_generation: key.expectedGeneration, input_candidate: prepared.input_candidate }
        : { action: 'add', input_candidate: prepared.input_candidate });
    }
    if (!keyId) throw new Error('CREDENTIAL_INPUT_INVALID');
    orderedIds.push(keyId);
    const original = source.keys.find(item => item.key_id === key.keyId);
    if (original ? original.enabled !== key.enabled : !key.enabled) {
      edits.push({ action: 'set_enabled', key_id: keyId,
        expected_generation: original ? key.expectedGeneration : 1, enabled: key.enabled });
    }
  }
  if (edits.length || orderedIds.some((id, index) => id !== source.keys[index]?.key_id)) {
    edits.push({ action: 'set_order', key_ids: orderedIds });
  }
  return edits;
}

export function validateDraftBudgets(draft: ModelDraft) {
  draft.keys.filter(key => !key.removed && key.budget).forEach(key => budgetAssignment(key.budget!));
}
export function buildKeyBudgetEdits(draft: ModelDraft, edits: KeyEdit[]): KeyBudgetEdit[] {
  const added = edits.filter((edit): edit is Extract<KeyEdit, { action: 'add' }> => edit.action === 'add');
  let next = 0;
  return draft.keys.filter(key => !key.removed).flatMap(key => {
    const input = !key.keyId ? added[next++]?.input_candidate : undefined;
    if (!key.budget) return [];
    if (!key.keyId && !input) throw new Error('KEY_BUDGET_INVALID');
    return [{ key: key.keyId ? { kind: 'saved' as const, key_id: key.keyId } : { kind: 'input' as const, candidate_ref: input!.candidate_ref }, assignment: budgetAssignment(key.budget) }];
  });
}
