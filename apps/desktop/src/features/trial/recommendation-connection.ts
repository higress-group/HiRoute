import { invoke } from '../../host/index.ts';
import type { AvailableModel } from '../model-feed/types';
import { registeredCatalogModelId } from '../model-connections/registered-metadata';
import type { ComputeConnectionOptions } from '../model-connections/types';
import type { ManagementSnapshot } from '../models/types';
import { readTrialSetup } from './read-setup';
import type { TrialSetupSnapshot } from './types';

export function referenceModelKey(model: AvailableModel): string | undefined {
  const prefix = 'model.catalog.';
  if (['descriptive_only', 'needs_connection', 'executable'].includes(model.availability.kind)
    && model.model_configuration_id.startsWith(prefix)) {
    return model.model_configuration_id.slice(prefix.length) || undefined;
  }
}

export type RecommendedConnection = {
  bindingId: string;
  sourceId: string;
  sourceName: string;
  revision: number;
  modelRef?: string;
};

export async function readRecommendationRequestModelIds(modelKey: string) {
  const options = await invoke<ComputeConnectionOptions>('compute_connection_options', { includeRequestModelIds: true });
  const catalog = options.metadata_catalog;
  const requestModelIds = new Map<string, string>();
  if (catalog) {
    for (const option of options.options) {
      const id = registeredCatalogModelId(option, catalog, modelKey);
      if (id) requestModelIds.set(option.connection_option_id, id);
    }
  }
  return requestModelIds;
}

export async function readRecommendationConnections(model: AvailableModel, requestModelIds?: ReadonlyMap<string, string>) {
  const [setup, identities] = await Promise.all([
    readTrialSetup(),
    requestModelIds ? invoke<ManagementSnapshot>('compute_management_snapshot') : Promise.resolve(undefined),
  ]);
  return { setup, ...recommendationConnections(setup, model, requestModelIds, identities) };
}

export function recommendationConnections(setup: Pick<TrialSetupSnapshot, 'options' | 'management'>, model: AvailableModel, requestModelIds?: ReadonlyMap<string, string>, identities?: ManagementSnapshot) {
  const ready: RecommendedConnection[] = [];
  const reusable: { sourceId: string; sourceName: string; requestId?: string }[] = [];
  const candidates = new Map(setup.options.candidates.filter(candidate => candidate.routable).map(candidate => [candidate.binding_id, candidate]));
  const identitiesBySource = new Map(identities?.sources.map(source => [source.source_id, source]));
  for (const source of setup.management.sources) {
    if (source.state !== 'ready') continue;
    const saved = identitiesBySource.get(source.source_id);
    const product = saved?.connection_identity?.connection_option_id;
    const requestId = product && saved?.revision === source.revision ? requestModelIds?.get(product) : undefined;
    if (requestModelIds && !requestId) continue;
    if (source.actions.includes('append_models') && (requestId || ['api', 'subscription'].includes(source.access_kind))) {
      reusable.push({ sourceId: source.source_id, sourceName: source.display_name, requestId });
    }
    for (const member of source.models) {
      const candidate = candidates.get(member.binding_id);
      if (member.availability !== 'available' || !candidate) continue;
      if (requestId ? member.upstream_model_id !== requestId
        : member.catalog_configuration_id !== model.model_configuration_id || !member.catalog_admitted
          || candidate.model_configuration_id !== model.model_configuration_id) continue;
      ready.push({ bindingId: member.binding_id, sourceId: source.source_id, sourceName: source.display_name,
        revision: member.revision, modelRef: member.model_ref });
    }
  }
  // The routing owner has already admitted these retained API bindings. Exact
  // catalog identity remains valid alongside source-local product matching.
  for (const connection of setup.management.retained_api_connections) {
    if (candidates.get(connection.binding_id)?.model_configuration_id !== model.model_configuration_id
      || connection.model_configuration_id !== model.model_configuration_id
      || ready.some(item => item.bindingId === connection.binding_id)) continue;
    ready.push({ bindingId: connection.binding_id, sourceId: connection.source_id, sourceName: connection.display_name,
      revision: connection.binding_revision });
  }
  return { ready, reusable };
}

export function sameConnection(left: RecommendedConnection, right: RecommendedConnection): boolean {
  return left.bindingId === right.bindingId && left.sourceId === right.sourceId
    && left.modelRef === right.modelRef && left.revision === right.revision;
}

export type RecommendationConnections = Awaited<ReturnType<typeof readRecommendationConnections>>;
