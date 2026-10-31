import type { Editor, Options, Plan } from '../../plan-editor';
import type { ManagedModel, ManagementSnapshot } from './types';

export type ModelRouteRequest = { modelId: string; bindingIds: string[]; plan?: Plan };

export function modelRouteHasAdditions(request?: ModelRouteRequest) {
  return !!request?.plan && request.bindingIds.some(id =>
    !request.plan!.desired.strategy.candidates?.some(item => item.binding_id === id));
}

export function matchingModelRoutes(modelId: string, plans: Plan[]) {
  return plans.filter(plan => plan.head.status !== 'deleted'
    && (plan.model_alias === modelId || plan.desired.display_name === modelId));
}

export function matchingModelSources(snapshot: ManagementSnapshot, model: ManagedModel) {
  const seen = new Set([model.binding_id]);
  return snapshot.sources.flatMap(source => source.models.flatMap(member => {
    if (member.upstream_model_id !== model.upstream_model_id || seen.has(member.binding_id)
      || source.state !== 'ready' || member.presentation?.availability !== 'available') return [];
    seen.add(member.binding_id);
    return [{ source, model: member }];
  }));
}

export function modelRouteAuthoring(request: ModelRouteRequest) {
  const previous = request.plan?.desired.strategy.candidates ?? [];
  const added = request.bindingIds.filter(id => !previous.some(item => item.binding_id === id));
  const candidates = [...previous, ...[...new Set(added)].map(binding_id => ({ binding_id }))];
  return request.plan
    ? { plan_id: request.plan.agent_plan_id, candidates }
    : { mode: 'fixed_model', display_name: request.modelId, candidates };
}

export function modelRouteEditor(options: Options, request: ModelRouteRequest): Editor {
  if (!options.authoring) throw { code: 'SCHEMA_INCOMPATIBLE' };
  if (request.plan && (request.plan.desired.mode !== 'fixed_model'
    || options.authoring.expected_head_revision !== request.plan.head.head_revision)) {
    throw { code: 'CHANGE_PREVIEW_STALE' };
  }
  const editor = structuredClone(options.authoring.editor);
  if (!request.plan) {
    editor.custom_alias = request.modelId;
    editor.follow_request_reasoning = true;
  }
  // Budget-only models have no shared automatic reasoning choice. Seed a visible,
  // valid minimum in this explicit creation flow; Save still admits the exact value.
  const previous = new Set(request.plan?.desired.strategy.candidates?.map(item => item.binding_id));
  for (const selection of editor.candidates) {
    if (previous.has(selection.binding_id) || selection.reasoning) continue;
    const native = options.candidates.find(item => item.binding_id === selection.binding_id)?.reasoning;
    if (native?.kind === 'budget') selection.reasoning = { kind: 'budget', tokens: native.minimum_tokens };
  }
  return editor;
}

