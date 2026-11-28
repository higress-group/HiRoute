import type { FactValue, ModelCapabilities, ModelDeclaration, ModelMetadataRecord } from './types';

function metadataBooleanFact(
  value: ModelMetadataRecord['capability_hints']['tool'],
  eligible: boolean,
): FactValue<boolean> {
  if (!eligible) return { value: null, basis: 'unknown' };
  if (value === 'supported') return { value: true, basis: 'user_declared' };
  if (value === 'unsupported') return { value: false, basis: 'user_declared' };
  return { value: null, basis: 'unknown' };
}

function metadataNumberFact(
  value: ModelMetadataRecord['context_tokens'],
  eligible: boolean,
): FactValue<number> {
  return eligible && value.state === 'known' && value.value !== null
    && Number.isSafeInteger(value.value) && value.value > 0
    ? { value: value.value, basis: 'user_declared' }
    : { value: null, basis: 'unknown' };
}

/**
 * Metadata hints become editable suggestions, never declared facts. An explicit field edit or
 * Apply suggestions is required before any of these values are submitted.
 */
export function metadataCapabilitySuggestions(record: ModelMetadataRecord): ModelCapabilities {
  // A record whose execution outcome is unsupported or not applicable (for example an
  // images-API model or an internal reviewer agent) must never suggest a text model shape.
  const executable = record.execution_fit.state === 'native_text_representable';
  const eligible = (scenario: string) => executable && record.usable_for.includes(scenario);
  const reasoningEligible = eligible('reasoning-capability-prefill');
  const nativeReasoning = reasoningEligible && record.capability_hints.reasoning === 'unsupported'
    ? { kind: 'fixed' as const, profile: 'non-thinking' }
    : null;
  return {
    // The maintained metadata policy has no tool/streaming prefill scenario. Preserve these as
    // unknown even when a source-scoped record happens to contain a hint.
    tool: metadataBooleanFact(record.capability_hints.tool, false),
    vision: metadataBooleanFact(record.capability_hints.vision, eligible('vision-capability-prefill')),
    streaming: metadataBooleanFact(record.capability_hints.streaming, false),
    context_tokens: metadataNumberFact(record.context_tokens, eligible('context-limit-prefill')),
    max_output_tokens: metadataNumberFact(record.max_output_tokens, eligible('output-limit-prefill')),
    native_reasoning: {
      value: nativeReasoning,
      basis: nativeReasoning ? 'user_declared' : 'unknown',
    },
  };
}

export function metadataModelPrefill(
  record: ModelMetadataRecord,
  clientId: string,
): ModelDeclaration {
  return {
    client_id: clientId,
    upstream_model_id: record.upstream_model_id,
    display_name: record.display_name,
    catalog_configuration_id: null,
    membership: 'user_declared',
    capabilities: {
      tool: { value: null, basis: 'unknown' },
      vision: { value: null, basis: 'unknown' },
      streaming: { value: null, basis: 'unknown' },
      context_tokens: { value: null, basis: 'unknown' },
      max_output_tokens: { value: null, basis: 'unknown' },
      native_reasoning: { value: null, basis: 'unknown' },
    },
    suggested_capabilities: metadataCapabilitySuggestions(record),
  };
}

export function metadataCostHint(record: ModelMetadataRecord, zh: boolean): string {
  if (!record.usable_for.includes('cost-hint-display') || record.cost_hints.length === 0) return '';
  const rendered = JSON.stringify(record.cost_hints[0]);
  return zh ? `来源中的成本提示（非价格）：${rendered}` : `Source cost hint (not a price): ${rendered}`;
}

export function metadataReasoningHint(record: ModelMetadataRecord, zh: boolean): string {
  const efforts = record.reasoning_rendering_hints.supported_reasoning_efforts;
  if (!record.usable_for.includes('reasoning-rendering-hint') || efforts.length === 0) return '';
  return zh
    ? `来源中的思考档位提示（不是可执行参数映射）：${efforts.join(', ')}`
    : `Source reasoning-level hint (not an executable parameter mapping): ${efforts.join(', ')}`;
}
