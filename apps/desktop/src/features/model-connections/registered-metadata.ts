import type { ComputeConnectionOption, ModelConnectionDraft, ModelMetadataCatalog, UpstreamProtocol } from './types';

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
};

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
  const names = new Map(catalog.canonical_models.map(model => [model.model_key, model.display_name]));
  const retiredIds = new Set(catalog.endpoint_bindings
    .filter(binding => productKeys.has(binding.product_key) && binding.lifecycle === 'deprecated')
    .map(binding => binding.upstream_model_id));
  for (const binding of catalog.endpoint_bindings) {
    if (!productKeys.has(binding.product_key)
      || !binding.interface_candidates.some(key => interfaceKeys.has(key))
      || binding.lifecycle === 'deprecated') continue;
    const id = binding.upstream_model_id;
    if (!candidates.has(id)) candidates.set(id, {
      upstream_model_id: id,
      display_name: names.get(binding.model_key) ?? id,
      source: 'product_metadata',
    });
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
  return [...candidates.values()].sort(compareCandidates);
}

function compareCandidates(left: RegisteredModelCandidate, right: RegisteredModelCandidate): number {
  if (left.source !== right.source) return left.source === 'built_in' ? -1 : 1;
  return left.upstream_model_id.localeCompare(right.upstream_model_id);
}
