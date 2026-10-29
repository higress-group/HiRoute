import type { ComputeConnectionOptions } from '../model-connections/types';
import type { ManagedSource } from '../models/types';

/** These are exact identities, never a display-name or internal-ID heuristic. */
export function requestModelDefault(source: ManagedSource, modelId: string, options?: ComputeConnectionOptions): string {
  const saved = [...new Set(source.models.filter(model => model.catalog_configuration_id === modelId).map(model => model.upstream_model_id))];
  if (saved.length === 1) return saved[0];
  if (saved.length > 1) return '';
  const product = source.connection_identity?.connection_option_id;
  const option = product && options?.options.find(option => option.connection_option_id === product);
  const ids = [...new Set(option && option.request_model_ids?.[modelId] || [])];
  return ids.length === 1 ? ids[0] : '';
}

export function savedRequestModel(source: ManagedSource, requestId: string) {
  const matches = source.models.filter(model => model.upstream_model_id === requestId);
  return matches.length === 1 ? matches[0] : undefined;
}

export type ReusedTrialBinding = {
  sourceId: string; bindingId: string; modelRef: string; revision: number;
};
