import type {
  CandidateModelView,
  ComputeCandidateRef,
  ComputeManagementChange,
  ComputeSaveResult,
  ModelConnectionDraft,
  ModelConnectionDraftInput,
  ModelConnectionCheckView,
  RevisionSet,
} from './types';
import type { ManagedSource } from '../models/types';

export function savedSourceConnectionFields(source: ManagedSource) {
  const baseUrl = (target: ManagedSource['target']) => {
    const host = target.authority.includes(':') && !target.authority.startsWith('[')
      ? `[${target.authority}]`
      : target.authority;
    return `${target.scheme}://${host}:${target.port}`;
  };
  return {
    display_template_id: source.display_template_id ?? null,
    base_url: baseUrl(source.target),
    request_path_override: source.target.request_path,
    inventory_path_override: source.inventory_path ?? null,
    protocol: source.target.upstream_protocol as ModelConnectionDraft['protocol'],
    protocol_profile_id: source.target.protocol_profile_id,
    protocol_profile_revision: source.target.protocol_profile_revision,
    authentication: source.authentication,
    additional_endpoints: (source.additional_native_endpoints ?? []).map(endpoint => ({
      base_url: baseUrl(endpoint.target),
      base_kind: 'api_root' as const,
      request_path_override: endpoint.target.request_path,
      inventory_path_override: endpoint.recheck?.inventory_path ?? null,
      protocol: endpoint.target.upstream_protocol as ModelConnectionDraft['protocol'],
      protocol_profile_id: endpoint.target.protocol_profile_id,
      protocol_profile_revision: endpoint.target.protocol_profile_revision,
      authentication: endpoint.authentication,
    })),
  };
}

export function buildCheckDraft(draft: ModelConnectionDraft): ModelConnectionDraftInput {
  if (draft.provenance.kind !== 'user_configured') {
    throw new Error('MODEL_CONNECTION_PROVENANCE_UNAVAILABLE');
  }
  const {
    entry_kind: _entryKind,
    provenance,
    qualification: _qualification,
    models,
    ...wireDraft
  } = draft;
  return {
    ...wireDraft,
    configuration_revision: provenance.configuration_revision,
    models: models.filter(model => model.upstream_model_id.trim()).map(({ client_id: _clientId, ...model }) => ({ ...model, display_name: model.display_name.trim() || model.upstream_model_id })),
  };
}

export type CheckExpectation = {
  candidateRef: string | null;
  editRevision: number;
  checkId: string;
};

export function checkMatches(result: ModelConnectionCheckView, expected: CheckExpectation): boolean {
  const candidate = result.candidate;
  const actualRef = candidate.candidate.candidate_ref;
  return (expected.candidateRef === null || expected.candidateRef === actualRef)
    && candidate.correlation.candidate_ref === actualRef
    && candidate.correlation.edit_revision === expected.editRevision
    && candidate.correlation.check_id === expected.checkId
    && candidate.correlation.input_digest === result.input_digest;
}

export function saveEligibility(result: ModelConnectionCheckView | null, enable: boolean): {
  allowed: boolean;
  reason: 'check_required' | 'approval_required' | 'credential_required' | 'explicit_failure' | 'model_required' | null;
} {
  if (!result) return { allowed: false, reason: 'check_required' };
  if (result.candidate.fact_state === 'pending_approval') return { allowed: false, reason: 'approval_required' };
  if (enable && result.candidate.fact_state === 'pending_credential') return { allowed: false, reason: 'credential_required' };
  if (enable && (result.authentication === 'rejected' || result.inference === 'failed')) {
    return { allowed: false, reason: 'explicit_failure' };
  }
  if (enable && !result.candidate.models.some(model => model.selectable)) {
    return { allowed: false, reason: 'model_required' };
  }
  return { allowed: true, reason: null };
}

export function modelSaveCompleted(result: ComputeSaveResult): boolean {
  return result.disposition === 'saved';
}

export function modelCanBeSelected(
  result: ModelConnectionCheckView,
  model: CandidateModelView,
  enable: boolean,
): boolean {
  return model.selectable || (
    !enable
    && result.candidate.producer === 'native'
    && model.membership === 'user_declared'
    && model.reason === 'model_connections.connection_check_required'
  );
}

export function defaultSelectedModelRefs(result: ModelConnectionCheckView): string[] {
  return result.candidate.models
    .filter(model => modelCanBeSelected(result, model, false))
    .map(model => model.model_ref);
}

export function selectedModelRefsForSave(
  result: ModelConnectionCheckView,
  selected: ReadonlySet<string>,
  enable: boolean,
): string[] {
  return result.candidate.models
    .filter(model => selected.has(model.model_ref) && modelCanBeSelected(result, model, enable))
    .map(model => model.model_ref);
}

export function buildSaveChange(input: {
  result: ModelConnectionCheckView;
  expectedRevisions: RevisionSet;
  selectedModelRefs: string[];
  enable: boolean;
  protectedInput: ComputeCandidateRef | null;
}): ComputeManagementChange {
  return {
    schema: 'hiroute.compute-management-change/v2',
    subject: { kind: 'candidate', candidate: input.result.candidate.candidate },
    expected_revisions: input.expectedRevisions,
    selected_model_refs: input.selectedModelRefs,
    intent: input.enable ? 'save_ready' : 'save_disabled',
    key_edits: input.protectedInput
      ? [{ action: 'add', input_candidate: input.protectedInput.candidate_ref === input.result.candidate.candidate.candidate_ref
          ? input.result.candidate.candidate : input.protectedInput }]
      : [],
    ...(input.result.candidate.validation ? { validation: input.result.candidate.validation } : {}),
  };
}

export function clientOperationId(prefix: string): string {
  return `${prefix}/${globalThis.crypto.randomUUID()}`;
}

export function clientIdempotencyKey(prefix: string): string {
  const safePrefix = prefix.replace(/[^A-Za-z0-9._-]/g, '-').slice(0, 31) || 'desktop';
  const entropy = Array.from(
    globalThis.crypto.getRandomValues(new Uint8Array(32)),
    byte => byte.toString(16).padStart(2, '0'),
  ).join('');
  return `${safePrefix}:${entropy}`.slice(0, 64);
}
