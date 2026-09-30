import assert from 'node:assert/strict';
import test from 'node:test';

import {
  canSave,
  checkStatusText,
  closeAction,
  isCurrentResult,
  saveFailureDefinitelyPreAdmission,
  saveIntent,
  selectionRows,
  statusText,
  validatedSubscriptionCandidate,
} from '../src/features/subscriptions/model.ts';

const operationA = {
  operation_id: 'operation-a',
  state: 'succeeded',
  sequence: 8,
  cancellable: false,
};
const operationB = {
  operation_id: 'operation-b',
  state: 'accepted',
  sequence: 1,
  cancellable: true,
};
const validation = {
  approval_operation: operationA,
  validation_ref: 'validation/cpa/one',
  validation_revision: '9',
};

function candidate(overrides = {}) {
  return {
    candidate: { candidate_ref: 'candidate/cpa/codex/one', candidate_revision: 2 },
    correlation: {
      candidate_ref: 'candidate/cpa/codex/one',
      edit_revision: 3,
      check_id: 'check/subscription/3',
      input_digest: 'sha256:checked',
    },
    producer: 'cpa',
    provenance: 'connector_owned',
    display_name: 'Codex subscription',
    models: [{
      model_ref: 'model/gpt',
      upstream_model_id: 'gpt',
      display_name: 'GPT',
      membership: 'catalog',
      selectable: true,
    }],
    input_state: 'provided',
    fact_state: 'complete',
    validation,
    issues: [],
    ...overrides,
  };
}

test('candidate revision and edit correlation remain independent', () => {
  const value = candidate();
  assert.equal(isCurrentResult(value, 3, 'check/subscription/3'), true);
  assert.equal(isCurrentResult(value, value.candidate.candidate_revision, 'check/subscription/3'), false);
});

test('pending approval cannot save and enable maps only to save_ready', () => {
  const pending = candidate({ fact_state: 'pending_approval', validation: undefined, models: [] });
  assert.equal(canSave(pending, true, new Set()), false);
  assert.equal(canSave(pending, false, new Set()), false);
  assert.equal(saveIntent(true), 'save_ready');
  assert.equal(saveIntent(false), 'save_disabled');
});

test('checked catalog model is selectable while InventoryOnly is not treated as login failure', () => {
  const checked = candidate();
  assert.equal(canSave(checked, true, new Set(['model/gpt'])), true);
  const inventoryOnly = candidate({
    models: [{
      model_ref: 'model/unknown', upstream_model_id: 'unknown', display_name: 'unknown',
      membership: 'observed', selectable: false, reason: 'inventory_only',
    }],
  });
  assert.match(statusText(inventoryOnly, 'en'), /catalog data/i);
  assert.doesNotMatch(statusText(inventoryOnly, 'en'), /sign in/i);
});

test('close follows A ownership until B exists, then only observes B', () => {
  const checking = { candidate: candidate().candidate, approval_operation: operationA, status: 'checking' };
  assert.deepEqual(closeAction(checking), { kind: 'cancel_a', operation_id: 'operation-a' });
  const verified = {
    candidate: candidate().candidate,
    approval_operation: operationA,
    status: 'verified',
    validation,
    checked_candidate: candidate(),
  };
  assert.deepEqual(closeAction(verified), { kind: 'release_a', validation_ref: validation.validation_ref });
  assert.deepEqual(closeAction({ ...verified, status: 'source_changed', checked_candidate: undefined }),
    { kind: 'release_a', validation_ref: validation.validation_ref });
  assert.deepEqual(closeAction(verified, operationB.operation_id), { kind: 'observe_b', operation_id: operationB.operation_id });
  assert.deepEqual(closeAction({ ...verified, save_operation: operationB }), { kind: 'observe_b', operation_id: operationB.operation_id });
});

test('backend failure classes remain distinct in user-facing status', () => {
  const base = { candidate: candidate().candidate, approval_operation: operationA };
  assert.match(checkStatusText({ ...base, status: 'source_changed' }, 'en'), /changed/i);
  assert.match(checkStatusText({ ...base, status: 'needs_auth' }, 'en'), /sign in/i);
  assert.match(checkStatusText({ ...base, status: 'unavailable' }, 'en'), /unavailable/i);
  assert.match(checkStatusText({ ...base, status: 'retained', save_operation: operationB }, 'en'), /retained/i);
});

test('only definite pre-admission save failures return A ownership to close cleanup', () => {
  assert.equal(saveFailureDefinitelyPreAdmission('MODEL_SAVE_CANCELLED'), true);
  assert.equal(saveFailureDefinitelyPreAdmission('REVISION_CONFLICT'), true);
  assert.equal(saveFailureDefinitelyPreAdmission('CLIENT_DEADLINE'), false);
  assert.equal(saveFailureDefinitelyPreAdmission('RESPONSE_UNKNOWN'), false);
});

test('failed, released and retained checks cannot reuse a scanned validation for another save', () => {
  const scanned = candidate();
  for (const status of ['checking', 'source_changed', 'needs_auth', 'unavailable', 'failed', 'released', 'retained']) {
    assert.equal(validatedSubscriptionCandidate(scanned, {
      candidate: scanned.candidate, approval_operation: operationA, status,
    }), null, status);
  }
  assert.equal(validatedSubscriptionCandidate(scanned), scanned);
  assert.equal(validatedSubscriptionCandidate(candidate({ validation: undefined })), null);
});

test('a newer scan invalidates old check selection without discarding inventory', () => {
  const checked = candidate();
  const check = { candidate: checked.candidate, approval_operation: operationA, status: 'verified', checked_candidate: checked };
  assert.equal(validatedSubscriptionCandidate(checked, check), checked);
  const newer = candidate({ candidate: { ...checked.candidate, candidate_revision: 3 } });
  assert.equal(validatedSubscriptionCandidate(newer, check), null);
  const other = candidate({ candidate: { candidate_ref: 'candidate/other', candidate_revision: 1 } });
  assert.equal(validatedSubscriptionCandidate(other, check), null);
  assert.equal(checked.models.length, 1);
});

test('inventory-only rows stay visible but cannot be selected or saved', () => {
  const checked = candidate();
  checked.models.push({ model_ref: 'model/unknown', upstream_model_id: 'unknown', display_name: 'unknown', membership: 'observed', selectable: false, reason: 'inventory_only' });
  assert.equal(validatedSubscriptionCandidate(checked).models.length, 2);
  assert.equal(canSave(checked, true, new Set(['model/gpt'])), true);
  assert.equal(canSave(checked, true, new Set(['model/gpt', 'model/unknown'])), false);
  assert.equal(canSave(checked, true, new Set()), false);
});

test('an existing checked subscription permits explicit addition and removal, not unverified additions', () => {
  const checked = candidate({ existing_source_id: 'source/subscription' });
  checked.models.push({
    model_ref: 'model/new', upstream_model_id: 'new', display_name: 'New',
    membership: 'catalog', selectable: true,
  });
  assert.equal(canSave(checked, true, new Set(['model/gpt', 'model/new'])), true);
  assert.equal(canSave(checked, true, new Set(['model/new'])), true);
  checked.models[1].selectable = false;
  assert.equal(canSave(checked, true, new Set(['model/gpt', 'model/new'])), false);
  assert.equal(canSave(checked, true, new Set(['model/missing'])), false);
});

test('a saved model missing from the checked directory stays visible until explicitly removed', () => {
  const checked = candidate({ existing_source_id: 'source/subscription' });
  const selected = new Set(['model/gpt', 'model/retired']);
  const saved = [{ model_ref: 'model/retired', display_name: 'Saved retired model' }];
  assert.equal(canSave(checked, true, selected), false);
  assert.deepEqual(selectionRows(checked, selected, saved).at(-1), {
    model_ref: 'model/retired', display_name: 'Saved retired model', selectable: false, missing: true,
  });
  selected.delete('model/retired');
  assert.equal(selectionRows(checked, selected, saved).length, 1);
  assert.equal(canSave(checked, true, selected), true);
});

test('a disappeared unsaved selection is also removable without granting eligibility', () => {
  const checked = candidate();
  const selected = new Set(['model/gpt', 'model/no-longer-listed']);
  const missing = selectionRows(checked, selected).at(-1);
  assert.equal(missing.model_ref, 'model/no-longer-listed');
  assert.equal(missing.selectable, false);
  assert.equal(missing.missing, true);
  assert.equal(canSave(checked, true, selected), false);
  selected.delete(missing.model_ref);
  assert.equal(canSave(checked, true, selected), true);
});
