import assert from 'node:assert/strict';
import test from 'node:test';

import {
  metadataCostHint,
  metadataModelPrefill,
  metadataReasoningHint,
} from '../src/features/model-connections/metadata-prefill.ts';

function record(overrides = {}) {
  return {
    model_record_key: 'fixture/model',
    provider_record_key: 'fixture',
    provider_id: 'fixture',
    upstream_model_id: 'fixture-model',
    display_name: 'Fixture Model',
    context_tokens: { state: 'known', value: 128_000 },
    max_output_tokens: { state: 'known', value: 8_192 },
    input_modalities: ['text', 'image'],
    capability_hints: {
      reasoning: 'supported',
      streaming: 'supported',
      tool: 'supported',
      vision: 'supported',
    },
    reasoning_rendering_hints: {
      reasoning_effort_maps: [{ high: 'high' }],
      supported_reasoning_efforts: ['high'],
      thinking_level_maps: [],
    },
    cost_hints: [{ input: '1.25', tiers: [{ threshold: '1000000', rate: '0.75' }] }],
    lifecycle: 'active',
    replacement_upstream_ids: [],
    normalized_model_matches: [],
    metadata_completeness: {
      capabilities: 'complete', cost: 'complete', identity: 'complete', lifecycle: 'complete',
      limits: 'complete', modalities: 'complete', reasoning_rendering: 'complete',
    },
    usable_for: [],
    execution_fit: { state: 'native_text_representable' },
    cost_hint_state: 'recorded',
    ...overrides,
  };
}

test('metadata prefill suggests only fields authorized by explicit usage scenarios', () => {
  const projected = metadataModelPrefill(record({
    usable_for: ['context-limit-prefill', 'vision-capability-prefill'],
  }), 'model/test');

  for (const key of ['tool', 'vision', 'streaming', 'context_tokens', 'max_output_tokens', 'native_reasoning']) {
    assert.deepEqual(projected.capabilities[key], { value: null, basis: 'unknown' }, key);
  }
  assert.deepEqual(projected.suggested_capabilities.context_tokens, { value: 128_000, basis: 'user_declared' });
  assert.deepEqual(projected.suggested_capabilities.vision, { value: true, basis: 'user_declared' });
  assert.deepEqual(projected.suggested_capabilities.max_output_tokens, { value: null, basis: 'unknown' });
  assert.deepEqual(projected.suggested_capabilities.tool, { value: null, basis: 'unknown' });
  assert.deepEqual(projected.suggested_capabilities.streaming, { value: null, basis: 'unknown' });
  assert.deepEqual(projected.suggested_capabilities.native_reasoning, { value: null, basis: 'unknown' });
});

test('unknown and conditional facts remain unknown while explicit unsupported reasoning is safe', () => {
  const unknown = metadataModelPrefill(record({
    context_tokens: { state: 'unknown', value: null },
    capability_hints: {
      reasoning: 'conditional', streaming: 'conditional', tool: 'conditional', vision: 'conditional',
    },
    usable_for: [
      'context-limit-prefill', 'output-limit-prefill', 'reasoning-capability-prefill',
      'vision-capability-prefill',
    ],
  }), 'model/test');
  assert.equal(unknown.capabilities.context_tokens.value, null);
  assert.equal(unknown.capabilities.vision.value, null);
  assert.equal(unknown.capabilities.native_reasoning.value, null);
  assert.equal(unknown.suggested_capabilities.context_tokens.value, null);
  assert.equal(unknown.suggested_capabilities.vision.value, null);
  assert.equal(unknown.suggested_capabilities.native_reasoning.value, null);

  const conflict = metadataModelPrefill(record({
    context_tokens: { state: 'conflict', value: null, candidates: [8_192, 16_384] },
    usable_for: ['context-limit-prefill'],
  }), 'model/test');
  assert.deepEqual(conflict.suggested_capabilities.context_tokens, { value: null, basis: 'unknown' });

  const unsafeInteger = metadataModelPrefill(record({
    context_tokens: { state: 'known', value: Number.MAX_SAFE_INTEGER + 1 },
    usable_for: ['context-limit-prefill'],
  }), 'model/test');
  assert.deepEqual(unsafeInteger.suggested_capabilities.context_tokens, { value: null, basis: 'unknown' });

  const unsupported = metadataModelPrefill(record({
    capability_hints: {
      reasoning: 'unsupported', streaming: 'unknown', tool: 'unknown', vision: 'unknown',
    },
    usable_for: ['reasoning-capability-prefill'],
  }), 'model/test');
  assert.deepEqual(unsupported.suggested_capabilities.native_reasoning, {
    value: { kind: 'fixed', profile: 'non-thinking' },
    basis: 'user_declared',
  });
  assert.deepEqual(unsupported.capabilities.native_reasoning, { value: null, basis: 'unknown' });
});

test('cost and reasoning hints are display-only and scenario gated', () => {
  const hidden = record();
  assert.equal(metadataCostHint(hidden, false), '');
  assert.equal(metadataReasoningHint(hidden, false), '');

  const visible = record({ usable_for: ['cost-hint-display', 'reasoning-rendering-hint'] });
  assert.match(metadataCostHint(visible, false), /^Source cost hint \(not a price\):/);
  assert.match(metadataReasoningHint(visible, false), /not an executable parameter mapping/);
});

test('records without a native text execution outcome never prefill a text model', () => {
  for (const state of ['unsupported', 'not_applicable']) {
    const projected = metadataModelPrefill(record({
      execution_fit: { state, reason: 'not a native text model' },
      usable_for: [
        'context-limit-prefill', 'output-limit-prefill', 'reasoning-capability-prefill',
        'vision-capability-prefill',
      ],
    }), 'model/test');
    assert.deepEqual(projected.suggested_capabilities.context_tokens, { value: null, basis: 'unknown' });
    assert.deepEqual(projected.suggested_capabilities.max_output_tokens, { value: null, basis: 'unknown' });
    assert.deepEqual(projected.suggested_capabilities.vision, { value: null, basis: 'unknown' });
    assert.deepEqual(projected.suggested_capabilities.native_reasoning, { value: null, basis: 'unknown' });
  }
});
