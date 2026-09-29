import type { ComputeConnectionOption, ModelConnectionDraft, ModelDeclaration, ModelMetadataCatalog, ProviderMetadataRecord, UpstreamProtocol } from './types';

type RegisteredEndpoint = NonNullable<ComputeConnectionOption['endpoints']>[number];

/** Follow another endpoint only while the draft still uses the template defaults. */
export function templateEndpointForProtocol(
  draft: Pick<ModelConnectionDraft, 'display_template_id' | 'base_url' | 'request_path_override' | 'inventory_path_override' | 'protocol' | 'authentication'>,
  option: ComputeConnectionOption | null,
  protocol: UpstreamProtocol,
): RegisteredEndpoint | null {
  if (!option || draft.display_template_id !== option.connection_option_id) return null;
  const endpoints = option.endpoints ?? [];
  const current = endpoints.find(endpoint => endpoint.protocol === draft.protocol
    && endpoint.base_url === draft.base_url
    && endpoint.request_path === draft.request_path_override
    && (endpoint.inventory_path ?? null) === draft.inventory_path_override
    && JSON.stringify(endpoint.authentication_semantics) === JSON.stringify(draft.authentication));
  if (!current) return null;
  return [...endpoints].filter(endpoint => endpoint.protocol === protocol)
    .sort((a, b) => a.stable_preference - b.stable_preference)[0] ?? null;
}

export type RegisteredModelCandidate = {
  upstream_model_id: string;
  display_name: string;
  source: 'built_in' | 'product_metadata';
  capability_prefill?: ModelDeclaration['capabilities'];
};

type CanonicalModel = ModelMetadataCatalog['canonical_models'][number];
type BindingOverrides = ModelMetadataCatalog['endpoint_bindings'][number]['capability_overrides'];

/** Product facts take precedence over publisher facts; the editable fallback fills only gaps. */
function catalogCapabilities(model: CanonicalModel, overrides: BindingOverrides = {}): ModelDeclaration['capabilities'] {
  const basis = 'user_declared' as const;
  const capabilities: ModelDeclaration['capabilities'] = {
    tool: { value: true, basis },
    vision: { value: false, basis },
    streaming: { value: true, basis },
    context_tokens: { value: 200_000, basis },
    max_output_tokens: { value: 32_768, basis },
    native_reasoning: { value: { kind: 'toggle', parameter: 'enable_thinking' }, basis },
  };
  const numberFact = (value: number | null | undefined, fallback: number) =>
    Number.isSafeInteger(value) && (value ?? 0) > 0 ? value! : fallback;
  const booleanFact = (value: string | undefined, fallback: boolean) =>
    value === 'supported' ? true : value === 'unsupported' ? false : fallback;
  capabilities.context_tokens = { value: numberFact(overrides.context_tokens ?? model.context_tokens.value, 200_000), basis: 'user_declared' };
  capabilities.max_output_tokens = { value: Math.min(
    numberFact(overrides.max_output_tokens ?? model.max_output_tokens.value, 32_768),
    capabilities.context_tokens.value!,
  ), basis: 'user_declared' };
  capabilities.tool = { value: overrides.tool ?? booleanFact(model.capabilities.tool, true), basis: 'user_declared' };
  capabilities.vision = { value: overrides.vision ?? booleanFact(model.modalities.image_input, false), basis: 'user_declared' };
  capabilities.streaming = { value: overrides.streaming ?? booleanFact(model.capabilities.streaming, true), basis: 'user_declared' };
  if (['discrete-or-budget', 'toggle-plus-discrete'].includes(model.reasoning.kind)
    && model.reasoning.profiles.length) {
    capabilities.native_reasoning = { value: {
      kind: 'discrete', parameter: 'reasoning_effort', profiles: model.reasoning.profiles,
    }, basis: 'user_declared' };
  } else if (model.reasoning.kind === 'discrete-fixed-on' || model.reasoning.kind === 'fixed-on') {
    capabilities.native_reasoning = { value: { kind: 'fixed', profile: 'provider-default' }, basis: 'user_declared' };
  } else if (model.reasoning.kind === 'unsupported') {
    capabilities.native_reasoning = { value: { kind: 'fixed', profile: 'non-thinking' }, basis: 'user_declared' };
  }
  return capabilities;
}

/** Product priority belongs to the connection picker, not the generated registry. */
export function sortRegisteredOptions(options: ComputeConnectionOption[]): ComputeConnectionOption[] {
  const rank = (id: string): number => id === 'bailian.token-plan.cn.v1' ? 0
    : id === 'bailian.coding-plan.cn.v1' ? 1
      : id === 'bailian.payg.cn.v1' ? 2
        : id.startsWith('bailian.') ? 3 : 4;
  return [...options].sort((left, right) => rank(left.connection_option_id) - rank(right.connection_option_id));
}

export function sortMetadataProviders(providers: ProviderMetadataRecord[]): ProviderMetadataRecord[] {
  const rank = (key: string): number => key === 'hermes-agent/alibaba-token-plan-cn' ? 0
    : key === 'hermes-agent/alibaba-coding-plan-cn' ? 1
      : key === 'hermes-agent/alibaba-cn' ? 2
        : key.startsWith('hermes-agent/alibaba-') ? 3 : 4;
  return [...providers].sort((left, right) => rank(left.provider_record_key) - rank(right.provider_record_key));
}

/** Offer every built-in product even when no provider record carries prefill evidence. */
export function customApiPrefillOptions(options: ComputeConnectionOption[], providers: ProviderMetadataRecord[]) {
  return [
    ...sortRegisteredOptions(options).map(option => ({
      kind: 'template' as const, key: `template:${option.connection_option_id}`, option,
    })),
    ...sortMetadataProviders(providers).map(provider => ({
      kind: 'provider' as const, key: `provider:${provider.provider_record_key}`, provider,
    })),
  ];
}

const metadataProtocol: Record<UpstreamProtocol, string> = {
  chat_completions: 'openai-chat',
  messages: 'anthropic-messages',
  responses: 'openai-responses',
};

function target(baseUrl: string, requestPath: string): string {
  return `${baseUrl.replace(/\/+$/, '')}/${requestPath.replace(/^\/+/, '')}`;
}

/** Product evidence stays separate from the registered executable capability table. */
export function registeredModelCandidates(
  option: ComputeConnectionOption,
  catalog: ModelMetadataCatalog | null | undefined,
): RegisteredModelCandidate[] {
  const candidates = new Map<string, RegisteredModelCandidate>();
  for (const [upstream_model_id, display_name] of Object.entries(option.known_models ?? {})) {
    candidates.set(upstream_model_id, { upstream_model_id, display_name, source: 'built_in' });
  }

  const primary = [...(option.endpoints ?? [])].sort((a, b) => a.stable_preference - b.stable_preference)[0];
  if (!primary || !catalog) return [...candidates.values()].sort(compareCandidates);

  // The preferred registered endpoint identifies the product, including when two products
  // share a secondary Messages URL (for example Zhipu Coding Plan and General API).
  const primaryTarget = target(primary.base_url, primary.request_path);
  const interfaceKeys = new Set<string>();
  const productKeys = new Set<string>();
  for (const product of catalog.access_products) {
    for (const item of product.interfaces) {
      if (item.base_url && item.request_path
        && item.protocol === metadataProtocol[primary.protocol]
        && target(item.base_url, item.request_path) === primaryTarget) {
        productKeys.add(product.product_key);
        interfaceKeys.add(item.interface_key);
      }
    }
  }
  const models = new Map(catalog.canonical_models.map(model => [model.model_key, model]));
  const names = new Map(catalog.canonical_models.map(model => [model.model_key, model.display_name]));
  const retiredIds = new Set(catalog.endpoint_bindings
    .filter(binding => productKeys.has(binding.product_key) && binding.lifecycle === 'deprecated')
    .map(binding => binding.upstream_model_id));
  for (const binding of catalog.endpoint_bindings) {
    if (!productKeys.has(binding.product_key)
      || !binding.interface_candidates.some(key => interfaceKeys.has(key))
      || binding.lifecycle === 'deprecated') continue;
    const id = binding.upstream_model_id;
    const model = models.get(binding.model_key);
    if (!candidates.has(id)) candidates.set(id, {
      upstream_model_id: id,
      display_name: names.get(binding.model_key) ?? id,
      source: 'product_metadata',
      ...(model ? { capability_prefill: catalogCapabilities(model, binding.capability_overrides) } : {}),
    });
    else if (model && !candidates.get(id)?.capability_prefill) {
      candidates.set(id, { ...candidates.get(id)!, capability_prefill: catalogCapabilities(model, binding.capability_overrides) });
    }
  }
  for (const product of catalog.access_products) {
    if (!productKeys.has(product.product_key)) continue;
    for (const id of product.documented_upstream_model_ids) {
      if (retiredIds.has(id)) continue;
      if (!candidates.has(id)) candidates.set(id, {
        upstream_model_id: id,
        display_name: id,
        source: 'product_metadata',
      });
    }
  }
  for (const candidate of candidates.values()) {
    if (candidate.capability_prefill) continue;
    const model = catalog.canonical_models.find(value => value.upstream_ids.some(id => id.toLowerCase() === candidate.upstream_model_id.toLowerCase()));
    if (model) candidate.capability_prefill = catalogCapabilities(model);
  }
  return [...candidates.values()].sort(compareCandidates);
}

/** Reuse a registered product's exact endpoint scope when a custom API prefill selects it. */
export function metadataProviderOption(
  provider: ProviderMetadataRecord,
  options: ComputeConnectionOption[],
): ComputeConnectionOption | null {
  return options.find(option => option.endpoints?.some(endpoint =>
    provider.base_url_candidates.some(base => target(endpoint.base_url, endpoint.request_path)
      .startsWith(`${base.replace(/\/+$/, '')}/`))
    && provider.protocol_candidates.includes(endpoint.protocol))) ?? null;
}

export function metadataProviderCandidates(
  provider: ProviderMetadataRecord,
  catalog: ModelMetadataCatalog,
  options: ComputeConnectionOption[],
): RegisteredModelCandidate[] {
  const option = metadataProviderOption(provider, options);
  if (option) return registeredModelCandidates(option, catalog);
  return catalog.model_records
    .filter(model => model.provider_record_key === provider.provider_record_key
      && model.usable_for.includes('custom-api-model-prefill')
      && model.execution_fit.state === 'native_text_representable'
      && model.lifecycle !== 'deprecated' && model.lifecycle !== 'retired')
    .map(model => ({ upstream_model_id: model.upstream_model_id,
      display_name: model.display_name, source: 'product_metadata' as const }))
    .sort(compareCandidates);
}

function compareCandidates(left: RegisteredModelCandidate, right: RegisteredModelCandidate): number {
  if (left.source !== right.source) return left.source === 'built_in' ? -1 : 1;
  return left.upstream_model_id.localeCompare(right.upstream_model_id);
}
