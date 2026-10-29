import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { modelConnectionBackend } from '../model-connections/backend';
import { clientIdempotencyKey } from '../model-connections/state';
import { saveFailureDefinitelyPreAdmission } from './model';
import type { ComputeSaveResult, ComputeSavePreview, ModelConnectionCheckView, OperationReference } from '../model-connections/types';
import type { ManagedSource, ManagementChange, ManagementSnapshot } from '../models/types';
import { apiLinkItems, isCodexSubscription, synchronizeSubscriptionApi, type ApiSyncResult } from './api-sync';

export function useSubscriptionApi({ language, mutable, onOperation, onRefresh, onUncertain }: {
  language: 'zh' | 'en'; mutable: boolean;
  onOperation(operation: OperationReference): void;
  onRefresh(): Promise<void>;
  onUncertain(): void;
}) {
  const alive = useRef(true);
  const running = useRef(new Set<string>());
  const checks = useRef(new Set<string>());
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  const [results, setResults] = useState<Record<string, ApiSyncResult>>({});
  const backend = modelConnectionBackend(language);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      for (const checkId of checks.current) void invoke('cancel_model_connection_check', { checkId }).catch(() => {});
    };
  }, []);
  const read = () => invoke<ManagementSnapshot>('compute_management_snapshot');

  async function saveLink(sourceId: string, apiSourceId: string): Promise<'saved' | 'uncertain'> {
    if (!mutable || !alive.current) throw new Error('TRUSTED_AUTHORITY_UNAVAILABLE');
    const snapshot = await read();
    const source = snapshot.sources.find(item => item.source_id === sourceId);
    if (!source || !isCodexSubscription(source)) throw new Error('SOURCE_CHANGED');
    const change: ManagementChange = {
      schema: 'hiroute.compute-management-change/v2', subject: { kind: 'saved_source', source_id: source.source_id },
      expected_revisions: snapshot.revisions, selected_model_refs: source.models.map(model => model.model_ref),
      intent: source.state === 'disabled' ? 'save_disabled' : 'save_ready', key_edits: [],
    };
    const preview = await invoke<ComputeSavePreview>('preview_compute_save_v3', {
      change, defaultPreferences: {}, apiLink: apiSourceId ? { intent: 'associate', api_source_id: apiSourceId } : { intent: 'remove' },
    });
    if (!alive.current) return 'uncertain';
    let accepted: { operation: OperationReference };
    try {
      accepted = await backend.applyComputeSave({ spec: preview.spec, accept_digest: preview.accept_digest, expected_revisions: preview.expected_revisions, idempotency_key: clientIdempotencyKey('subscription-api-link') });
    } catch (error) {
      if (saveFailureDefinitelyPreAdmission(error)) throw error;
      onUncertain(); return 'uncertain';
    }
    onOperation(accepted.operation);
    let result: ComputeSaveResult;
    try { result = await backend.getComputeSaveResult(accepted.operation); }
    catch { onUncertain(); return 'uncertain'; }
    if (result.disposition === 'pending') { onUncertain(); return 'uncertain'; }
    if (result.disposition !== 'saved' || result.source_id !== sourceId) throw new Error('API_LINK_SAVE_FAILED');
    if (alive.current) await onRefresh();
    return 'saved';
  }

  async function synchronize(sourceId: string): Promise<ApiSyncResult | undefined> {
    if (!mutable || !alive.current || running.current.has(sourceId)) return;
    running.current.add(sourceId); setBusy(new Set(running.current));
    let link: { source_id: string; api_source_id: string } | undefined;
    let source: ManagedSource | undefined;
    try {
      const snapshot = await read();
      link = snapshot.subscription_api_links?.find(item => item.source_id === sourceId);
      source = snapshot.sources.find(item => item.source_id === sourceId);
      if (!link || !source) return;
      const result = await synchronizeSubscriptionApi(link, {
        read,
        check: async request => {
          checks.current.add(request.check_id);
          try { return await invoke<ModelConnectionCheckView>('check_saved_model_connection', { request }); }
          finally { checks.current.delete(request.check_id); }
        },
        preview: change => invoke<ComputeSavePreview>('preview_compute_save_v3', { change, defaultPreferences: {} }),
        apply: request => backend.applyComputeSave(request),
        result: operation => backend.getComputeSaveResult(operation),
        findSave: async idempotencyKey => {
          const value = await invoke<ComputeSaveResult>('get_compute_save_result', { operation: null, idempotencyKey });
          return value.reason === 'OPERATION_NOT_FOUND' ? null : value;
        },
        renewFailed: async (saved, apiSourceId) => {
          if (await saveLink(saved.source_id, apiSourceId) !== 'saved') throw new Error('API_LINK_SAVE_UNCERTAIN');
        },
        onOperation, onUncertain, current: () => alive.current,
        checkId: () => clientIdempotencyKey('subscription-api-check'),
      });
      if (alive.current) setResults(previous => ({ ...previous, [sourceId]: result }));
      return result;
    } catch {
      if (link && source) {
        const result: ApiSyncResult = { ...link, source_revision: source.revision, items: apiLinkItems(source), reason: 'check_failed' };
        if (alive.current) setResults(previous => ({ ...previous, [sourceId]: result }));
        return result;
      }
    } finally {
      running.current.delete(sourceId);
      if (alive.current) { setBusy(new Set(running.current)); void onRefresh(); }
    }
  }

  async function associate(sourceId: string, apiSourceId: string) {
    if (running.current.has(sourceId)) return 'uncertain' as const;
    running.current.add(sourceId); setBusy(new Set(running.current));
    let saved: 'saved' | 'uncertain';
    try { saved = await saveLink(sourceId, apiSourceId); }
    finally { running.current.delete(sourceId); if (alive.current) setBusy(new Set(running.current)); }
    if (saved === 'saved' && apiSourceId) await synchronize(sourceId);
    if (saved === 'saved' && !apiSourceId && alive.current) setResults(previous => { const next = { ...previous }; delete next[sourceId]; return next; });
    return saved;
  }
  return { synchronize, associate, busy, results };
}
