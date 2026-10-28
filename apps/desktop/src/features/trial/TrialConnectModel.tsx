import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { ModelConnectionForm } from '../model-connections/ModelConnectionForm';
import { modelConnectionBackend } from '../model-connections/backend';
import { newModelConnectionDraft } from '../model-connections/new-draft';
import { checkMatches } from '../model-connections/state';
import { safeConnectionErrorCode } from '../model-connections/copy';
import { saveFailureDefinitelyPreAdmission } from '../subscriptions/model';
import type { ComputeCandidateRef, ComputeSaveResult, ModelConnectionBackend, ModelConnectionCheckView, OperationReference } from '../model-connections/types';
import type { TrialCatalogSnapshot, TrialManagementSnapshot } from './types';
import type { AvailableModel } from '../model-feed/types';
type ResolvedBinding = { bindingId: string; modelId: string; name: string; source: string; linked: boolean };
export type TrialConnectionState = 'editing' | 'saving' | 'uncertain' | 'saved';

export function TrialConnectModel({ model, language, management, mutable, onResolve, onReceipt, onSaved, onSelectionIntent, onBack, onOperation, onStateChange, onReuseSource }: {
  model: AvailableModel; language: 'zh' | 'en'; management: TrialManagementSnapshot; mutable: boolean;
  onResolve(receipt: ComputeSaveResult): Promise<TrialCatalogSnapshot | undefined>;
  onReceipt(): void;
  onSaved(result: ComputeSaveResult, selection: ResolvedBinding, intent?: number): boolean; onSelectionIntent(): number; onBack(recoverable: boolean): void; onOperation(operation: OperationReference): void;
  onStateChange(state: TrialConnectionState): void;
  onReuseSource?(sourceId: string): Promise<void>;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [draft] = useState(() => newModelConnectionDraft('known'));
  const [saved, setSaved] = useState<ComputeSaveResult | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const [resolved, setResolved] = useState<ResolvedBinding[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [reusing, setReusing] = useState(false);
  const busyRef = useRef(false), live = useRef(true), uncertainRef = useRef(false), checkGeneration = useRef(0);
  const accepted = useRef<OperationReference | null>(null);
  const submitted = useRef<{ idempotency_key: string; accept_digest: string; candidateKey: string; selectedRefs: Set<string> } | null>(null);
  const previewed = useRef<{ acceptDigest: string; candidateKey: string; selectedRefs: Set<string> } | null>(null);
  const unknown = useRef<{ idempotency_key: string; candidate: ComputeCandidateRef } | null>(null);
  const checked = useRef(new Map<string, Set<string>>());
  const currentLanguage = useRef(language);
  currentLanguage.current = language;
  useEffect(() => () => { live.current = false; }, []);
  const backend = useMemo<ModelConnectionBackend>(() => {
    const owner = modelConnectionBackend(language);
    const remember = (result: ModelConnectionCheckView) => {
      const ref = result.candidate.candidate;
      // A new check replaces earlier observations, even if it reuses a ref.
      checked.current.clear();
      checked.current.set(`${ref.candidate_ref}:${ref.candidate_revision}`, new Set(result.candidate.models.filter(m => m.selectable && (result.observed_model_ids?.includes(m.upstream_model_id) || (result.inference === 'verified' && result.inference_model_id === m.upstream_model_id))).map(m => m.model_ref)));
      return result;
    };
    return {
      ...owner,
      listConnectionOptions: async () => {
        const options = await owner.listConnectionOptions();
        return { ...options, options: options.options.filter(option => option.model_configuration_ids.includes(model.model_configuration_id)) };
      },
      checkRegisteredModelConnection: async request => {
        const generation = ++checkGeneration.current; checked.current.clear(); previewed.current = null;
        const result = await owner.checkRegisteredModelConnection(request);
        if (live.current && generation === checkGeneration.current && checkMatches(result, {candidateRef:request.candidate_ref ?? null, editRevision:request.edit_revision, checkId:request.check_id})) remember(result);
        return result;
      },
      checkModelConnection: async request => {
        const generation = ++checkGeneration.current; checked.current.clear(); previewed.current = null;
        const result = await owner.checkModelConnection(request);
        if (live.current && generation === checkGeneration.current && checkMatches(result, {candidateRef:request.draft.candidate_ref ?? null, editRevision:request.draft.edit_revision, checkId:request.draft.check_id})) remember(result);
        return result;
      },
      cancelModelConnectionCheck: async checkId => { ++checkGeneration.current; checked.current.clear(); previewed.current = null; return owner.cancelModelConnectionCheck(checkId); },
      previewComputeSave: async change => {
        const result = await owner.previewComputeSave(change);
        const reference = change.subject.candidate;
        const key = `${reference.candidate_ref}:${reference.candidate_revision}`;
        // Preserve the exact checked subset attached to this Preview, rather
        // than assuming every model later present on the source was saved here.
        previewed.current = result.candidate?.candidate_ref === reference.candidate_ref && result.candidate.candidate_revision === reference.candidate_revision && checked.current.has(key)
          ? { acceptDigest: result.accept_digest, candidateKey: key, selectedRefs: new Set(change.selected_model_refs) } : null;
        return result;
      },
      applyComputeSave: async request => {
        const preview = previewed.current;
        submitted.current = { idempotency_key: request.idempotency_key, accept_digest: request.accept_digest,
          candidateKey: preview?.acceptDigest === request.accept_digest ? preview.candidateKey : '',
          selectedRefs: preview?.acceptDigest === request.accept_digest ? preview.selectedRefs : new Set() };
        onStateChange('saving');
        try { return await modelConnectionBackend(currentLanguage.current).applyComputeSave(request); }
        catch (error) {
          if (saveFailureDefinitelyPreAdmission(error) || saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(error, true))) onStateChange('editing');
          throw error;
        }
      },
    };
  }, [model.model_configuration_id, onStateChange]);

  function resolutionScope(receipt: ComputeSaveResult): ComputeSaveResult {
    const reference = receipt.candidate;
    const selected = reference && submitted.current?.candidateKey === `${reference.candidate_ref}:${reference.candidate_revision}` ? submitted.current.selectedRefs : new Set<string>();
    return { ...receipt, bindings: receipt.bindings.filter(binding => selected.has(binding.model_ref)) };
  }

  async function confirm(result?: ComputeSaveResult) {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setError(''); setResolved([]);
    try {
      if (!result && !saved && !accepted.current && unknown.current && submitted.current) {
        // Observe only the native session's original submitted operation. The
        // pending hint can be replaced by a later action, so check its exact key
        // and digest on both sides of the observation before adopting its ID.
        type Hint = { idempotency_key: string; accepted_digest: string; operation_kind: string; operation_id: string | null; latest_edit_not_applied?: boolean };
        type Observed = OperationReference & { accepted_digest: string };
        const identity = unknown.current, submission = submitted.current;
        const before = await invoke<{pending: Hint | null}>('desktop_snapshot');
        if (!live.current) return;
        const matches = (hint: Hint | null) => !!hint && hint.operation_kind === 'ApplyComputeSave' && hint.idempotency_key === identity.idempotency_key && hint.idempotency_key === submission.idempotency_key && hint.accepted_digest === submission.accept_digest && !hint.latest_edit_not_applied;
        if (matches(before.pending)) {
          const operation = await invoke<Observed | null>('observe_operation');
          const after = await invoke<{pending: Hint | null}>('desktop_snapshot');
          if (!live.current) return;
          if (operation && operation.accepted_digest === submission.accept_digest && matches(after.pending) && (!before.pending?.operation_id || before.pending.operation_id === operation.operation_id) && (!after.pending?.operation_id || after.pending.operation_id === operation.operation_id)) {
            accepted.current = { operation_id: operation.operation_id, state: operation.state, sequence: operation.sequence, cancellable: operation.cancellable };
            onOperation(accepted.current);
          }
        }
      }
      const receipt = result ?? saved ?? (accepted.current ? await backend.getComputeSaveResult(accepted.current) : null);
      if (!live.current) return;
      if (!receipt || receipt.disposition !== 'saved') {
        if (live.current) { setUncertain(true); setError(text('保存结果仍未确认，请稍后刷新。', 'The save result is still unconfirmed. Refresh shortly.')); }
        return;
      }
      const reference = receipt.candidate;
      if (unknown.current && (!reference || reference.candidate_ref !== unknown.current.candidate.candidate_ref || reference.candidate_revision !== unknown.current.candidate.candidate_revision)) {
        if (live.current) { setUncertain(true); setError(text('保存结果与原始接入不匹配，请返回核对。', 'The save result does not match the original connection. Return to review it.')); }
        return;
      }
      if (live.current) { setSaved(receipt); setUncertain(false); onStateChange('saved'); onReceipt(); }
      const current = await onResolve(resolutionScope(receipt));
      if (!live.current) return;
      if (!current) throw Error('READ_FAILED');
      const observed = reference && checked.current.get(`${reference.candidate_ref}:${reference.candidate_revision}`);
      const selectedRefs = reference && submitted.current?.candidateKey === `${reference.candidate_ref}:${reference.candidate_revision}` ? submitted.current.selectedRefs : undefined;
      const source = current.management.sources.find(s => s.source_id === receipt.source_id);
      const matches = receipt.bindings.flatMap(binding => {
        const member = source?.models.find(m => m.binding_id === binding.binding_id && m.model_ref === binding.model_ref && m.revision === binding.revision);
        const candidate = current.options.candidates.find(c => c.binding_id === binding.binding_id && c.routable);
        if (!source || source.state !== 'ready' || !member || !selectedRefs?.has(binding.model_ref) || member.availability !== 'available' || !candidate) return [];
        const linked = member.catalog_configuration_id === model.model_configuration_id && candidate.model_configuration_id === model.model_configuration_id && member.catalog_admitted && !!observed?.has(binding.model_ref);
        // An unlinked declaration is a distinct identity, never a guessed catalog match.
        if (!linked && member.catalog_configuration_id) return [];
        return [{ bindingId: binding.binding_id, modelId: candidate.model_configuration_id, name: member.display_name, source: source.display_name, linked }];
      });
      if (!reference || !observed || !selectedRefs || !source || (receipt.saved_revision !== undefined && source.revision < receipt.saved_revision) || !matches.length) {
        setError(text('接入已保存，但当前绑定无法确认可用。请刷新结果或返回检查已保存接入。', 'The connection is saved, but its current binding could not be confirmed. Refresh the result or review the saved connection.'));
        return;
      }
      if (receipt.bindings.length === 1 && matches.length === 1 && matches[0].linked && onSaved(receipt, matches[0])) return;
      setResolved(matches);
    } catch {
      if (live.current) setError(text('暂时无法核实保存结果。已完成的保存会保留，刷新不会重复提交。', 'The save result could not be verified. Completed saves are retained; refresh does not resubmit.'));
    } finally { busyRef.current = false; if (live.current) setBusy(false); }
  }
  async function useSaved(selection: ResolvedBinding) {
    if (!saved || busyRef.current) return;
    const intent = onSelectionIntent();
    busyRef.current = true; setBusy(true); setError('');
    try {
      const current = await onResolve(resolutionScope(saved));
      if (!live.current) return;
      const receiptBinding = saved.bindings.find(b => b.binding_id === selection.bindingId);
      const source = current?.management.sources.find(s => s.source_id === saved.source_id);
      const member = source?.models.find(m => m.binding_id === receiptBinding?.binding_id && m.model_ref === receiptBinding.model_ref && m.revision === receiptBinding.revision);
      const candidate = current?.options.candidates.find(c => c.binding_id === selection.bindingId && c.model_configuration_id === selection.modelId && c.routable);
      const reference = saved.candidate;
      const observation = reference && checked.current.get(`${reference.candidate_ref}:${reference.candidate_revision}`);
      const selected = reference && submitted.current?.candidateKey === `${reference.candidate_ref}:${reference.candidate_revision}` && !!submitted.current.selectedRefs.has(receiptBinding?.model_ref ?? '');
      const linked = !!member && member.catalog_configuration_id === model.model_configuration_id && candidate?.model_configuration_id === model.model_configuration_id && member.catalog_admitted && !!observation?.has(member.model_ref);
      if (!source || source.state !== 'ready' || !member || member.availability !== 'available' || !candidate || !observation || !selected || (selection.linked ? !linked : !!member.catalog_configuration_id)) {
        setResolved([]); setError(text('接入已保存，但所选模型已变化或不可用。请刷新结果。', 'The connection is saved, but this model changed or is unavailable. Refresh the result.')); return;
      }
      onSaved(saved, { ...selection, name: member.display_name, source: source.display_name }, intent);
    } catch { if (live.current) setError(text('接入已保存，但无法重新读取所选模型。请重试。', 'The connection is saved, but the selected model could not be reread. Retry.')); }
    finally { busyRef.current = false; if (live.current) setBusy(false); }
  }
  if (saved || uncertain) return <section className="trial-recovery" aria-live="polite"><h3>{saved ? text('接入已保存', 'Connection saved') : text('正在核实接入结果', 'Verifying connection result')}</h3><p>{error || (busy ? saved ? text('接入已保存，正在更新模型可用状态…', 'Connection saved; updating model availability…') : text('正在核实原始保存操作…', 'Checking the original save operation…') : saved ? text('请选择此接入中保存的模型。', 'Choose the model saved in this connection.') : text('保存结果尚未确认，请刷新以核对原始操作。', 'The save result is unconfirmed. Refresh to check the original operation.'))}</p>{resolved.map(selection => <div className="trial-saved-choice" key={selection.bindingId}><span><strong>{selection.name}</strong> · {selection.source}{!selection.linked && <small>{text('使用此接入中的模型，目录版本未核实', 'Using the model from this connection; catalog version unconfirmed')}</small>}</span><button type="button" className="btn" disabled={busy} onClick={() => void useSaved(selection)}>{resolved.length === 1 ? text('使用此已保存模型', 'Use this saved model') : text('选择此模型', 'Choose this model')}</button></div>)}<div className="actions"><button type="button" className="btn" disabled={busy} onClick={() => void confirm()}>{text('刷新结果', 'Refresh result')}</button><button type="button" className="btn" disabled={busy} onClick={() => onBack(true)}>{text('返回试用', 'Back to trial')}</button></div></section>;
  const reusable = management.sources.filter(source => source.actions.includes('append_models') && ['api', 'subscription'].includes(source.access_kind));
  return <section className="trial-inline-connection"><h3>{text('接入', 'Connect')} {model.display_name}</h3>{error && <p role="alert">{error}</p>}<ModelConnectionForm language={language} mutable={mutable} allowCustom singleProductShortcut initialDraft={draft} existingNames={management.sources.map(s => s.display_name)} expectedRevisions={management.revisions} backend={backend}
    connectionChoice={onReuseSource && <label className="field"><span className="field-label">{text('使用的接入点', 'Connection to use')}</span><select className="input" data-trial-source-choice value="" disabled={reusing} onChange={event => {
      const sourceId = event.currentTarget.value;
      if (!sourceId || reusing) return;
      setReusing(true);
      void onReuseSource(sourceId).catch(() => { if (live.current) setError(text('暂时无法打开此接入，请重试。', 'This connection could not be opened. Retry.')); }).finally(() => { if (live.current) setReusing(false); });
    }}><option value="">{text('新建接入', 'Create a new connection')}</option>{reusable.map((source, index) => <option key={source.source_id} value={source.source_id}>{source.display_name} · {source.access_kind === 'subscription' ? text('订阅', 'Subscription') : 'API'}{reusable.filter(other => other.display_name === source.display_name).length > 1 ? ` · ${index + 1}` : ''}{source.state === 'disabled' ? text('（已停用）', ' (disabled)') : ''}</option>)}</select><span className="field-help">{text('已有接入可复用已保存的端点和凭据，无需再填 API Key。订阅由原来的订阅接入流程处理。', 'Reuse saved endpoints and credentials without entering the API key again. Subscriptions keep their existing connection flow.')}</span></label>}
    onBack={() => onBack(false)} onCancel={() => { if (!uncertainRef.current) onBack(false); }}
    onSaveAccepted={operation => { accepted.current = operation; onOperation(operation); }}
    onSaveResult={result => { if (result.disposition === 'saved') { setSaved(result); onStateChange('saved'); void confirm(result); } else onStateChange('editing'); }}
    onSaveUncertain={identity => { unknown.current = identity; uncertainRef.current = true; setUncertain(true); onStateChange('uncertain'); }} restoreFocus={() => {}} />
  </section>;
}
