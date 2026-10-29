import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { randomId } from '../../random-id.ts';
import { Dialog } from '../../ui/Dialog';
import { requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import { connectionErrorMessage, manualModelIdError, safeConnectionErrorCode } from '../model-connections/form-support';
import { checkMatches, clientIdempotencyKey, saveEligibility } from '../model-connections/state';
import type { ComputeConnectionOptions, ComputeSaveResult, ComputeValidationRef, ModelConnectionCheckView, OperationReference } from '../model-connections/types';
import type { AvailableModel } from '../model-feed/types';
import type { ManagedSource, ManagementChange, ManagementSnapshot, SavePreview } from '../models/types';
import { saveFailureDefinitelyPreAdmission, validatedSubscriptionCandidate } from '../subscriptions/model';
import type { SubscriptionCandidate, SubscriptionCheckResult } from '../subscriptions/types';
import { requestModelDefault, savedRequestModel, type ReusedTrialBinding } from './reuse-connection';

type Checked = { candidate: { candidate_ref: string; candidate_revision: number }; existing_source_id?: string; validation?: ComputeValidationRef; models: { model_ref: string; upstream_model_id: string; selectable: boolean }[] };
type Submission = { key: string; digest: string; candidate: Checked['candidate']; modelRef: string; operation?: OperationReference };

/** A selected trial model is an intent, not another general model-management entry. */
export function TrialReuseConnection({ model, sourceId, options, visible, language, mutable, onUse, onOperation, onClose }: {
  model: AvailableModel; sourceId: string; language: 'zh' | 'en'; mutable: boolean;
  options?: ComputeConnectionOptions;
  visible: boolean;
  onUse(binding: ReusedTrialBinding): Promise<boolean>;
  onOperation(operation: OperationReference): void; onClose(recoverable: boolean): void;
}) {
  const zh = language === 'zh', text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [source, setSource] = useState<ManagedSource>();
  const [requestId, setRequestId] = useState('');
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false);
  const [error, setError] = useState(''), [uncertain, setUncertain] = useState(false);
  const [receipt, setReceipt] = useState<ComputeSaveResult>();
  const initialId = useRef(''), live = useRef(true), working = useRef(false), completed = useRef(false);
  const submitted = useRef<Submission | undefined>(undefined);
  const check = useRef<{ apiId?: string; subscription?: SubscriptionCheckResult; handedOff: boolean }>({ handedOff: false });
  useDiscardGuard('models', () => !completed.current && (working.current || (!submitted.current && requestId !== initialId.current)), language, working.current ? async () => false : undefined);

  async function load() {
    setLoading(true); setError('');
    try {
      const management = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (!live.current) return;
      const current = management.sources.find(source => source.source_id === sourceId);
      if (!current) throw Error('SOURCE_CHANGED');
      let id = requestModelDefault(current, model.model_configuration_id);
      if (!id) {
        const defaults = options ?? await invoke<ComputeConnectionOptions>('compute_connection_options', { includeRequestModelIds: true }).catch(() => undefined);
        if (!live.current) return;
        id = requestModelDefault(current, model.model_configuration_id, defaults);
      }
      setSource(current);
      initialId.current = id; setRequestId(id);
    } catch { if (live.current) setError(text('暂时无法读取这个接入，请重试。', 'This connection could not be read. Retry.')); }
    finally { if (live.current) setLoading(false); }
  }
  useEffect(() => {
    live.current = true; void load();
    return () => {
      live.current = false;
      if (check.current.handedOff) return;
      if (check.current.apiId) void invoke('cancel_model_connection_check', { checkId: check.current.apiId }).catch(() => undefined);
      if (check.current.subscription) void invoke('close_subscription_check').catch(() => undefined);
    };
  }, []);

  async function releaseCheck() {
    const previous = check.current;
    check.current = { handedOff: false };
    if (previous.handedOff) return;
    if (previous.apiId) await invoke('cancel_model_connection_check', { checkId: previous.apiId }).catch(() => undefined);
    if (previous.subscription) await invoke('close_subscription_check').catch(() => undefined);
  }

  async function checkSource(current: ManagedSource): Promise<Checked> {
    await releaseCheck();
    if (current.provenance === 'connector_owned') {
      const scan = await invoke<{ candidates: SubscriptionCandidate[] }>('compute_subscriptions', {
        sourceId, expectedSourceRevision: current.revision,
      });
      if (!live.current) throw Error('CHECK_CANCELLED');
      const candidates = scan.candidates.filter(candidate => candidate.existing_source_id === sourceId);
      if (candidates.length !== 1) throw Error('SUBSCRIPTION_NOT_FOUND');
      const candidate = candidates[0];
      let result = await invoke<SubscriptionCheckResult>('check_subscription', { candidate: candidate.candidate, language });
      check.current.subscription = result;
      while (live.current && result.status === 'checking') {
        await new Promise(resolve => window.setTimeout(resolve, 500));
        if (!live.current) throw Error('CHECK_CANCELLED');
        result = await invoke<SubscriptionCheckResult>('get_subscription_check_result', { operation: result.approval_operation });
        check.current.subscription = result;
      }
      const checked = validatedSubscriptionCandidate(candidate, result);
      if (!checked || checked.existing_source_id !== sourceId || checked.fact_state !== 'complete') throw Error(result.reason ?? 'SUBSCRIPTION_CHECK_REQUIRED');
      return checked;
    }
    const checkId = `trial-reuse/${randomId()}`;
    check.current.apiId = checkId;
    const result = await invoke<ModelConnectionCheckView>('check_saved_model_connection', { request: {
      source_id: sourceId, expected_source_revision: current.revision, edit_revision: 1, check_id: checkId,
    } });
    if (!checkMatches(result, { candidateRef: null, editRevision: 1, checkId }) || result.candidate.existing_source_id !== sourceId) throw Error('CHECK_CORRELATION_MISMATCH');
    if (result.reachability === 'transport_failed') throw Error('MODEL_CONNECTION_TRANSPORT_FAILED');
    if (result.authentication === 'rejected') throw Error('MODEL_CONNECTION_AUTHENTICATION_REJECTED');
    if (!saveEligibility(result, true).allowed) throw Error('MODEL_CONNECTION_FAILED');
    return result.candidate;
  }

  async function useMember(current: ManagedSource, binding: ManagedSource['models'][number]) {
    if (current.state !== 'ready' || binding.presentation?.availability !== 'available') throw Error('MODEL_UNAVAILABLE');
    const used = await onUse({ sourceId, bindingId: binding.binding_id, modelRef: binding.model_ref, revision: binding.revision });
    if (!used) throw Error('BINDING_UNRESOLVED');
    completed.current = true;
  }

  async function finish(saved: ComputeSaveResult) {
    if (saved.disposition !== 'saved') {
      if (saved.disposition === 'pending') { setUncertain(true); return; }
      submitted.current = undefined;
      check.current = { handedOff: false };
      setUncertain(false);
      throw Error(saved.reason ?? 'MODEL_SAVE_FAILED');
    }
    const submission = submitted.current!;
    if (saved.source_id !== sourceId || (saved.candidate && (saved.candidate.candidate_ref !== submission.candidate.candidate_ref || saved.candidate.candidate_revision !== submission.candidate.candidate_revision))) throw Error('BINDING_UNRESOLVED');
    setReceipt(saved); setUncertain(false);
    const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
    if (!live.current) return;
    const savedSource = current.sources.find(source => source.source_id === sourceId);
    const binding = saved.bindings.find(binding => binding.model_ref === submission.modelRef);
    const member = savedSource?.models.find(member => member.binding_id === binding?.binding_id && member.model_ref === binding.model_ref && member.revision === binding.revision && member.upstream_model_id === requestId);
    if (!savedSource || !member || (saved.saved_revision !== undefined && savedSource.revision < saved.saved_revision)) throw Error('BINDING_UNRESOLVED');
    await useMember(savedSource, member);
  }

  async function refreshResult() {
    if (working.current || !submitted.current) return;
    working.current = true; setBusy(true); setError('');
    try {
      const submission = submitted.current;
      if (!submission.operation && !receipt) {
        type Hint = { operation_kind: string; idempotency_key: string; accepted_digest: string; operation_id: string | null; latest_edit_not_applied?: boolean };
        const matches = (hint: Hint | null) => !!hint && hint.operation_kind === 'ApplyComputeSave' && hint.idempotency_key === submission.key && hint.accepted_digest === submission.digest && !hint.latest_edit_not_applied;
        const before = await invoke<{ pending: Hint | null }>('desktop_snapshot');
        if (!live.current) return;
        if (!matches(before.pending)) throw Error('SAVE_UNCONFIRMED');
        const operation = await invoke<(OperationReference & { accepted_digest: string }) | null>('observe_operation');
        const after = await invoke<{ pending: Hint | null }>('desktop_snapshot');
        if (!live.current) return;
        if (!operation || operation.accepted_digest !== submission.digest || !matches(after.pending) || (before.pending?.operation_id && before.pending.operation_id !== operation.operation_id) || (after.pending?.operation_id && after.pending.operation_id !== operation.operation_id)) throw Error('SAVE_UNCONFIRMED');
        submission.operation = operation; onOperation(operation);
      }
      const saved = receipt ?? await invoke<ComputeSaveResult>('get_compute_save_result', { operation: submission.operation });
      if (live.current) await finish(saved);
    } catch { if (live.current) setError(text('保存结果尚未核实。刷新只检查原操作，不会重复添加。', 'The save is not yet confirmed. Refresh checks the original operation without adding again.')); }
    finally { working.current = false; if (live.current) setBusy(false); }
  }

  async function useConnection() {
    if (!mutable || working.current || submitted.current || !source) return;
    const idError = manualModelIdError(requestId, new Set([...source.models.map(model => model.upstream_model_id), initialId.current]));
    if (!requestId || idError) { setError(connectionErrorMessage(idError || 'MODEL_ID_REQUIRED', zh)); return; }
    working.current = true; setBusy(true); setError('');
    try {
      const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (!live.current) return;
      const currentSource = current.sources.find(item => item.source_id === sourceId && item.revision === source.revision);
      if (!currentSource || currentSource.state !== 'ready') throw Error('SOURCE_CHANGED');
      const existing = savedRequestModel(currentSource, requestId);
      if (existing) { await useMember(currentSource, existing); return; }
      if (!currentSource.actions.includes('append_models')) throw Error('SOURCE_CHANGED');
      const checked = await checkSource(currentSource);
      if (!live.current) return;
      const models = checked.models.filter(model => model.upstream_model_id === requestId && model.selectable);
      if (models.length !== 1) {
        setError(text('此接入没有提供这个可用的模型 ID。请核对 ID，或到模型页检查接入。', 'This connection did not provide this usable model ID. Check the ID or review the connection in Models.'));
        return;
      }
      const latest = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (!live.current) return;
      const unchanged = latest.sources.find(item => item.source_id === sourceId && item.revision === currentSource.revision && item.state === 'ready');
      if (!unchanged) throw Error('SOURCE_CHANGED');
      const change: ManagementChange = { schema: 'hiroute.compute-management-change/v2', subject: { kind: 'candidate', candidate: checked.candidate }, expected_revisions: latest.revisions, selected_model_refs: [models[0].model_ref], intent: 'save_ready', key_edits: [], edit: { action: 'append_models' }, validation: checked.validation };
      const preview = await invoke<SavePreview>('preview_compute_save', { change });
      if (!live.current) return;
      const submission: Submission = { key: clientIdempotencyKey('trial-reuse'), digest: preview.accept_digest, candidate: checked.candidate, modelRef: models[0].model_ref };
      submitted.current = submission; check.current.handedOff = true;
      let accepted: { operation: OperationReference };
      try {
        accepted = await invoke('apply_compute_save', { request: { spec: preview.spec, accept_digest: preview.accept_digest, expected_revisions: preview.expected_revisions, idempotency_key: submission.key }, language });
      } catch (cause) {
        if (saveFailureDefinitelyPreAdmission(cause) || saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(cause, true))) { submitted.current = undefined; check.current.handedOff = false; throw cause; }
        setUncertain(true); return;
      }
      submission.operation = accepted.operation; onOperation(accepted.operation);
      const saved = await invoke<ComputeSaveResult>('get_compute_save_result', { operation: accepted.operation });
      if (live.current) await finish(saved);
    } catch {
      if (live.current) {
        if (submitted.current && !receipt) setUncertain(true);
        setError(submitted.current ? text('正在核实保存结果；请刷新原操作。', 'Check the original operation to confirm the save.') : text('这个接入或模型暂时不可用。请检查状态后重试，当前 ID 已保留。', 'This connection or model is unavailable. Check its state and retry; the ID is retained.'));
      }
    } finally { working.current = false; if (live.current) setBusy(false); }
  }

  const close = () => void requestEditorReplacement('models').then(allowed => { if (allowed) onClose(!!submitted.current); });
  return <Dialog open={visible} title={text('复用已有接入', 'Reuse an existing connection')} closeLabel={text('返回试用', 'Back to trial')} className="trial-dialog" closeDisabled={busy} onClose={close}><section className="trial-reuse-connection" aria-busy={loading || busy}>
    <h3>{model.display_name}</h3>
    <p className="field-help">{text('保留已选的试用模型，复用这个接入的端点和凭据。', 'Keep your trial model and reuse this connection’s endpoints and credentials.')}</p>
    {loading ? <p role="status">{text('正在读取接入…', 'Reading connection…')}</p> : <>
      {source && <><p><strong>{text('接入点', 'Connection')}: </strong>{source.display_name}</p>
        <label className="field"><span className="field-label">{text('请求模型 ID', 'Request model ID')} <span aria-label={text('必填', 'Required')}>*</span></span><input className="input" data-trial-request-model value={requestId} disabled={busy || !!submitted.current || !mutable} onChange={event => { setRequestId(event.target.value); setError(''); }} aria-invalid={!!error} aria-describedby="trial-reuse-help trial-reuse-error" /></label>
        <p className="field-help" id="trial-reuse-help">{initialId.current ? text('已按这个接入预填，可修改为服务使用的模型 ID。修改 ID 不会自动确认目录版本。', 'Prefilled for this connection. You can edit the service’s model ID; an edit does not verify its catalog version.') : text('无法确认此接入的默认 ID，请填写服务实际使用的模型 ID。', 'No default ID could be confirmed for this connection. Enter the model ID used by the service.')}</p>
        {source.state !== 'ready' && <p role="status">{text('此接入尚未就绪，请先到模型页处理。', 'This connection is not ready. Review it in Models first.')}</p>}
      </>}
      {error && <p className="oc-inline-error" id="trial-reuse-error" role="alert">{error}</p>}
      {(receipt || uncertain) && <p role="status">{receipt ? text('接入已保存，正在核实所选绑定。', 'Connection saved; confirming the selected binding.') : text('保存结果尚未确认，不会重复提交。', 'The save is unconfirmed; it will not be submitted again.')}</p>}
      <div className="actions">
        <button type="button" className="btn" disabled={busy} onClick={close}>{text('返回试用', 'Back to trial')}</button>
        {!source && <button type="button" className="btn" onClick={() => void load()}>{text('重试', 'Retry')}</button>}
        {submitted.current ? <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void refreshResult()}>{text('刷新结果', 'Refresh result')}</button> : source && <button type="button" className="btn btn-primary" disabled={!mutable || busy || source.state !== 'ready'} onClick={() => void useConnection()}>{busy ? text('正在确认…', 'Checking…') : savedRequestModel(source, requestId) ? text('使用此接入', 'Use this connection') : text('检查并添加', 'Check and add')}</button>}
      </div>
    </>}
  </section></Dialog>;
}
