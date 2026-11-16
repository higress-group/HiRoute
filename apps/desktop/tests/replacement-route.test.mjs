import assert from 'node:assert/strict';
import test from 'node:test';
import { replacementSlots, replaceRouteModel } from '../src/features/trial/replacement-route.ts';

const old = { binding_id: 'binding/old', reasoning: { kind: 'toggle', enabled: true } };
const next = {
  binding_id: 'binding/new', display_name: 'New model', routable: true, billing_class: 'paid', ingress_protocols: ['responses'],
  reasoning: { kind: 'discrete', profiles: ['low', 'high'], default_profile: 'high' },
};
const options = {
  candidates: [next],
  unavailable_candidates: [{ binding_id: old.binding_id, display_name: 'Unavailable model' }],
  authoring: { candidate_defaults: [{ binding_id: next.binding_id, economy: { kind: 'profile', profile: 'low' }, primary: { kind: 'profile', profile: 'high' } }] },
};

function route(mode) {
  const primary = { binding_id: 'binding/primary' };
  return {
    display_name: 'Route', mode, candidates: [old], follow_request_reasoning: false, delegation_enabled: false,
    free: { candidates: [old], primary: [primary], primary_fallback: false },
    smart: { economy: [old], primary: [primary] },
    branch_routing: { branches: [
      { id: 'first', name: 'One', candidates: [old, primary], primary_candidates: [primary] },
      { id: 'second', name: 'Two', candidates: [primary, old], primary_candidates: [] },
    ] },
  };
}

test('replacement changes one position even when a binding repeats across branches', () => {
  const input = route('custom_branches');
  const slots = replacementSlots(input, options, next, 'en');
  const slot = slots.find(value => value.id === 'branch/second/regular/1');
  assert.equal(slot.oldName, 'Unavailable model');
  assert.equal(slot.label, 'Two · Regular models · Position 2');
  const expected = structuredClone(input);
  expected.branch_routing.branches[1].candidates[1] = { binding_id: next.binding_id, reasoning: { kind: 'profile', profile: 'low' } };
  assert.deepEqual(replaceRouteModel(input, slot, next, options), expected);
  assert.deepEqual(input, route('custom_branches'));
});

test('replacements use only active groups and preserve verified-free and protocol constraints', () => {
  const free = route('free_first');
  assert.deepEqual(replacementSlots(free, options, next, 'en').map(slot => [slot.id, !!slot.reason]), [['free/0', true]]);
  free.free.primary_fallback = true;
  assert.deepEqual(replacementSlots(free, options, next, 'en').map(slot => [slot.id, !!slot.reason]), [['free/0', true], ['fallback/0', false]]);
  free.work = { protocol: 'messages' };
  free.delegation_enabled = true;
  assert.ok(replacementSlots(free, options, next, 'en').every(slot => slot.reason));
  const fixed = route('fixed_model');
  assert.deepEqual(replacementSlots(fixed, options, next, 'en').map(slot => [slot.id, !!slot.reason]), [['fixed/0', false]]);
  fixed.candidates.unshift({ binding_id: next.binding_id });
  assert.ok(replacementSlots(fixed, options, next, 'en').every(slot => slot.reason));
});

test('a route following request reasoning uses the replacement binding default', () => {
  const input = route('smart_saving');
  input.follow_request_reasoning = true;
  const slot = replacementSlots(input, options, next, 'en')[0];
  assert.deepEqual(replaceRouteModel(input, slot, next, options).smart.economy, [{ binding_id: next.binding_id, reasoning: { kind: 'profile', profile: 'high' } }]);
});
