import type { ComputeConnectionApplyRequest, ComputeSaveResult, ComputeSavePreview, ModelConnectionCheckView, OperationReference, SavedModelConnectionCheckRequest } from '../model-connections/types';
import type { ManagedSource, ManagementChange, ManagementSnapshot } from '../models/types';

export type SubscriptionApiLink = { source_id: string; api_source_id: string };
export type ApiSyncItem = { model_id: string; state: 'imported' | 'pending' | 'unsupported' };
export type ApiSyncResult = {
  source_id: string;
  api_source_id: string;
  source_revision: number;
  api_revision?: number;
  items: ApiSyncItem[];
  reason?: 'api_missing' | 'api_disabled' | 'check_failed' | 'source_changed' | 'save_failed' | 'save_pending' | 'cancelled';
  operation?: OperationReference;
};
export type ApiSyncBackend = {
  read(): Promise<ManagementSnapshot>;
  check(request: SavedModelConnectionCheckRequest): Promise<ModelConnectionCheckView>;
  preview(change: ManagementChange): Promise<ComputeSavePreview>;
  apply(request: ComputeConnectionApplyRequest): Promise<{ operation: OperationReference }>;
  result(operation: OperationReference): Promise<ComputeSaveResult>;
  findSave(key: string): Promise<ComputeSaveResult | null>;
  renewFailed(source: ManagedSource, apiSourceId: string): Promise<void>;
  onOperation(operation: OperationReference): void;
  onUncertain(): void;
  current(): boolean;
  checkId(): string;
};

export type LinkedSubscriptionProvider = 'codex' | 'claude';
export function subscriptionProvider(source: ManagedSource): LinkedSubscriptionProvider | null {
  if (source.provenance !== 'connector_owned') return null;
  switch (source.connection_identity?.connection_option_id) {
    case 'codex.subscription.global.v1': return 'codex';
    case 'claude.subscription.global.v1': return 'claude';
    default: return null;
  }
}
export function candidateSubscriptionProvider(candidateRef: string): LinkedSubscriptionProvider | null {
  if (candidateRef.startsWith('candidate/cpa/codex/')) return 'codex';
  if (candidateRef.startsWith('candidate/cpa/claude/')) return 'claude';
  return null;
}
export function associatedApiOption(provider: LinkedSubscriptionProvider): string {
  return provider === 'claude' ? 'anthropic.platform.global.v1' : 'openai.platform.global.v1';
}
export function isLinkedSubscription(source: ManagedSource): boolean {
  return subscriptionProvider(source) !== null;
}
export function apiLinkItems(source: ManagedSource, api?: ManagedSource): ApiSyncItem[] {
  const ids = new Set(api?.models.map(model => model.upstream_model_id));
  return source.models.map(model => ({ model_id: model.upstream_model_id, state: ids.has(model.upstream_model_id) ? 'imported' : 'pending' }));
}

// Encode separators as well, so two distinct source identities cannot share a key.
export function apiSyncKey(source: ManagedSource, api: ManagedSource): string {
  const encode = (value: string) => [...value].map(c => /^[a-zA-Z0-9.-]$/.test(c) ? c : `_${c.codePointAt(0)!.toString(16)}_`).join('');
  const key = `api-sync:${encode(source.source_id)}:${source.revision}:${encode(api.source_id)}:${api.revision}`;
  if (key.length > 200) throw new Error('API_SYNC_IDENTITY_TOO_LONG');
  return key;
}

/** Import only exact checked API identities. Every write still uses normal Preview/Apply. */
export async function synchronizeSubscriptionApi(link: SubscriptionApiLink, backend: ApiSyncBackend, renewed = false): Promise<ApiSyncResult> {
  let snapshot = await backend.read();
  const source = snapshot.sources.find(item => item.source_id === link.source_id);
  let api = snapshot.sources.find(item => item.source_id === link.api_source_id);
  const output: ApiSyncResult = {
    ...link, source_revision: source?.revision ?? 0, api_revision: api?.revision,
    items: source ? apiLinkItems(source, api) : [],
  };
  const pending = (reason: ApiSyncResult['reason']) => ({ ...output, reason });
  const linked = (view: ManagementSnapshot) => view.subscription_api_links?.some(item => item.source_id === link.source_id && item.api_source_id === link.api_source_id);
  if (!source || !isLinkedSubscription(source) || !linked(snapshot)) return pending('source_changed');
  if (!api || api.provenance === 'connector_owned') return pending('api_missing');
  if (!output.items.some(item => item.state === 'pending')) return output;
  if (api.state !== 'ready') return pending('api_disabled');
  if (!backend.current()) return pending('cancelled');
  try {
    const key = apiSyncKey(source, api);
    const existing = await backend.findSave(key);
    if (!backend.current()) return pending('cancelled');
    if (existing) {
      if (existing.source_id && existing.source_id !== api.source_id) return pending('source_changed');
      if (existing.operation) {
        output.operation = existing.operation;
        backend.onOperation(existing.operation);
      }
      if (existing.disposition === 'pending') {
        backend.onUncertain();
        return pending('save_pending');
      }
      if (existing.disposition !== 'saved') {
        // An explicit retry can replace a terminal failed request, never a pending request.
        if (renewed) return pending('save_failed');
        await backend.renewFailed(source, api.source_id);
        return synchronizeSubscriptionApi(link, backend, true);
      }
      snapshot = await backend.read();
      api = snapshot.sources.find(item => item.source_id === link.api_source_id);
      if (!linked(snapshot) || snapshot.sources.find(item => item.source_id === source.source_id)?.revision !== source.revision) return pending('source_changed');
      output.api_revision = api?.revision;
      output.items = apiLinkItems(source, api);
      return output.items.some(item => item.state === 'pending') ? pending('save_failed') : output;
    }
    const checked = await backend.check({ source_id: api.source_id, expected_source_revision: api.revision, edit_revision: 1, check_id: backend.checkId() });
    if (!backend.current()) return pending('cancelled');
    if (!['verified', 'not_required'].includes(checked.authentication)
      || !['available', 'empty', 'partial'].includes(checked.directory)
      || checked.candidate.existing_source_id !== api.source_id
      || checked.candidate.producer !== 'native') return pending('check_failed');
    // A partial directory cannot prove absence. Only an exact, complete read can.
    const complete = checked.directory === 'available' || checked.directory === 'empty';
    const observed = new Set([...checked.observed_model_ids ?? [], ...checked.candidate.models.filter(model => model.membership === 'observed').map(model => model.upstream_model_id)]);
    const eligible = new Map(checked.candidate.models.filter(model => model.selectable && observed.has(model.upstream_model_id)).map(model => [model.upstream_model_id, model.model_ref]));
    output.items = output.items.map(item => item.state === 'imported' || eligible.has(item.model_id) || observed.has(item.model_id) ? item
      : { ...item, state: complete ? 'unsupported' : 'pending' });
    const selected = output.items.filter(item => item.state === 'pending' && eligible.has(item.model_id)).map(item => eligible.get(item.model_id)!);
    if (!selected.length) return output;
    snapshot = await backend.read();
    const currentApi = snapshot.sources.find(item => item.source_id === api!.source_id);
    if (!linked(snapshot) || snapshot.sources.find(item => item.source_id === source.source_id)?.revision !== source.revision
      || !currentApi || currentApi.revision !== api.revision) return pending('source_changed');
    const change: ManagementChange = {
      schema: 'hiroute.compute-management-change/v2', subject: { kind: 'candidate', candidate: checked.candidate.candidate },
      expected_revisions: snapshot.revisions, selected_model_refs: selected, intent: 'save_ready', key_edits: [],
      validation: checked.candidate.validation, edit: { action: 'append_models' },
    };
    const preview = await backend.preview(change);
    if (!backend.current()) return pending('cancelled');
    let accepted: { operation: OperationReference };
    try {
      accepted = await backend.apply({ spec: preview.spec, accept_digest: preview.accept_digest, expected_revisions: preview.expected_revisions, idempotency_key: key });
    } catch {
      backend.onUncertain();
      return pending('save_pending');
    }
    output.operation = accepted.operation;
    backend.onOperation(accepted.operation);
    let saved: ComputeSaveResult;
    try { saved = await backend.result(accepted.operation); }
    catch { backend.onUncertain(); return pending('save_pending'); }
    if (saved.disposition === 'pending') { backend.onUncertain(); return pending('save_pending'); }
    if (saved.disposition !== 'saved' || saved.source_id !== api.source_id) return pending('save_failed');
    const after = await backend.read();
    if (!linked(after) || after.sources.find(item => item.source_id === source.source_id)?.revision !== source.revision) return pending('source_changed');
    const savedApi = after.sources.find(item => item.source_id === api!.source_id);
    output.api_revision = savedApi?.revision;
    const imported = new Set(savedApi?.models.map(item => item.upstream_model_id));
    output.items = output.items.map(item => imported.has(item.model_id) ? { ...item, state: 'imported' } : item);
    if (output.items.some(item => item.state === 'pending')) output.reason = 'save_failed';
    return output;
  } catch {
    return pending('check_failed');
  }
}
