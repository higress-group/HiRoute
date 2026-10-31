import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { confirmDiscard } from '../../ui/discard-guard';
import { saveFailureDefinitelyPreAdmission } from '../subscriptions/model';
import { clientIdempotencyKey } from './state';
import { safeConnectionErrorCode } from './copy';
import { NativeNetworkImpact, NativeNetworkSettings } from './NativeNetworkSettings';
import { manualProxyError, networkDraft, networkEdit, networkPolicyEqual, reconcileNetworkDraft, type NativeNetworkDraft, type NativeSourceNetwork } from './native-network';
import type { ComputeSavePreview, ComputeSaveResult, OperationReference } from './types';

/** An in-place source editor. Saving is durable; cancelling a parent consumer does not undo it. */
export function NativeSourceNetworkEditor({ sourceId, language, mutable, onSaved, onDirty, reveal = 0 }: {
  sourceId: string; language: 'zh' | 'en'; mutable: boolean; reveal?: number;
  onSaved?(view: NativeSourceNetwork): void | Promise<void>; onDirty?(dirty: boolean): void;
}) {
  const zh = language === 'zh';
  const [view, setView] = useState<NativeSourceNetwork | null>(null), [draft, setDraft] = useState<NativeNetworkDraft>(networkDraft);
  const [attempt, setAttempt] = useState(0), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [preview, setPreview] = useState<{ preview: ComputeSavePreview; before: NativeSourceNetwork; after: NativeSourceNetwork['policy'] } | null>(null);
  const [pending, setPending] = useState(false), [saved, setSaved] = useState(false);
  const [consumerRefreshFailed, setConsumerRefreshFailed] = useState(false);
  const [returnReveal, setReturnReveal] = useState(0);
  const live = useRef(true), working = useRef(false);
  const dirtyCallback = useRef(onDirty); dirtyCallback.current = onDirty;
  const submission = useRef<{ key: string; digest: string; operation?: OperationReference } | null>(null);
  const changed = Boolean(view && !networkPolicyEqual(view.policy, networkEdit(draft).policy));
  function changeDraft(value: NativeNetworkDraft) {
    setDraft(value); setSaved(false); setError('');
    // Notify callers before a pending test can return and replace this editor.
    dirtyCallback.current?.(Boolean(view && !networkPolicyEqual(view.policy, networkEdit(value).policy)));
  }
  useEffect(() => { dirtyCallback.current?.(changed || pending || busy); }, [changed, pending, busy]);
  useEffect(() => () => dirtyCallback.current?.(false), []);
  useEffect(() => {
    live.current = true; let current = true;
    working.current = true; setLoading(true);
    if (view?.source_id !== sourceId) { setView(null); setConsumerRefreshFailed(false); }
    setError(''); setPreview(null); setSaved(false);
    void invoke<NativeSourceNetwork>('native_source_network', { input: { source_id: sourceId } }).then(value => {
      if (current) { setDraft(draft => reconcileNetworkDraft(view, value, draft)); setView(value); }
    }).catch(() => { if (current) setError(zh ? '网络设置暂时无法读取，输入已保留，请重试。' : 'Network settings could not be read. Your input is retained. Retry.'); })
      .finally(() => { if (current) { working.current = false; setLoading(false); } });
    return () => { current = false; live.current = false; };
  }, [sourceId, attempt]);
  async function prepare() {
    if (!view || working.current || !mutable || pending || manualProxyError(draft)) return;
    working.current = true; setBusy(true); setError('');
    try {
      const value = await invoke<NonNullable<typeof preview>>('preview_native_source_network', { input: { source_id: sourceId, expected_source_revision: view.source_revision, network_edit: networkEdit(draft) } });
      if (live.current) setPreview(value);
    } catch { if (live.current) setError(zh ? '无法生成保存预览。请重新读取最新设置，当前输入会保留，再预览后保存。' : 'Could not preview this save. Reload the latest settings to keep your input, then preview again.'); }
    finally { working.current = false; if (live.current) setBusy(false); }
  }
  async function finish(operation: OperationReference) {
    const result = await invoke<ComputeSaveResult>('get_compute_save_result', { operation });
    if (!live.current) return;
    if (result.disposition === 'pending') { setPending(true); return; }
    if (result.disposition !== 'saved') {
      submission.current = null; setPending(false); setPreview(null);
      setError(zh ? '网络设置未保存，输入已保留。请重新预览后重试。' : 'Network settings were not saved. Your input is retained. Preview again before retrying.');
      return;
    }
    const current = await invoke<NativeSourceNetwork>('native_source_network', { input: { source_id: sourceId } });
    if (!live.current) return;
    // A failed readback still belongs to this Operation; refresh must not create a new save.
    submission.current = null; setPending(false);
    setView(current); setDraft(networkDraft(current.policy, current.network_revision)); setPreview(null); setSaved(true);
    await refreshConsumer(current);
  }
  async function refreshConsumer(current: NativeSourceNetwork) {
    try { await onSaved?.(current); if (live.current) setConsumerRefreshFailed(false); }
    catch { if (live.current) setConsumerRefreshFailed(true); }
  }
  async function retryConsumerRefresh() {
    if (!view || working.current || pending) return;
    working.current = true; setBusy(true); setError('');
    try { await refreshConsumer(view); }
    finally { working.current = false; if (live.current) setBusy(false); }
  }
  async function apply() {
    if (!preview || working.current || !mutable || submission.current) return;
    working.current = true; setBusy(true); setError('');
    const key = clientIdempotencyKey('source-network');
    submission.current = { key, digest: preview.preview.accept_digest };
    try {
      const accepted = await invoke<{ operation: OperationReference }>('apply_native_source_network', { language, request: { spec: preview.preview.spec, accept_digest: preview.preview.accept_digest, expected_revisions: preview.preview.expected_revisions, idempotency_key: key } });
      submission.current.operation = accepted.operation;
      if (live.current) await finish(accepted.operation);
    } catch (cause) {
      if (!submission.current?.operation && (saveFailureDefinitelyPreAdmission(cause) || saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(cause, true)))) { submission.current = null; if (live.current) setError(zh ? '尚未提交。输入已保留，请重试或重新读取最新设置。' : 'The change was not submitted. Your input is retained. Retry or reload the latest settings.'); }
      else if (live.current) { setPending(Boolean(submission.current)); setError(zh ? '保存结果尚未确认，请刷新原操作结果。' : 'The save is not confirmed. Refresh the original operation result.'); }
    } finally { working.current = false; if (live.current) setBusy(false); }
  }
  async function refreshPending() {
    if (!submission.current || working.current) return;
    working.current = true; setBusy(true); setError('');
    try {
      const record = submission.current;
      if (!record.operation) {
        type Hint = { operation_kind: string; idempotency_key: string; accepted_digest: string; operation_id: string | null; latest_edit_not_applied?: boolean };
        const matches = (hint: Hint | null) => hint?.operation_kind === 'ApplyComputeSave' && hint.idempotency_key === record.key && hint.accepted_digest === record.digest && !hint.latest_edit_not_applied;
        const before = await invoke<{ pending: Hint | null }>('desktop_snapshot');
        if (!matches(before.pending)) throw Error('SAVE_UNCONFIRMED');
        const operation = await invoke<(OperationReference & { accepted_digest: string }) | null>('observe_operation');
        const after = await invoke<{ pending: Hint | null }>('desktop_snapshot');
        if (!operation || operation.accepted_digest !== record.digest || !matches(after.pending) || (before.pending?.operation_id && before.pending.operation_id !== operation.operation_id) || (after.pending?.operation_id && after.pending.operation_id !== operation.operation_id)) throw Error('SAVE_UNCONFIRMED');
        record.operation = operation;
      }
      await finish(record.operation);
    } catch { if (live.current) setError(zh ? '结果尚未核实。刷新只检查原操作，不会重复提交。' : 'The result is unconfirmed. Refresh checks the original operation without resubmitting.'); }
    finally { working.current = false; if (live.current) setBusy(false); }
  }
  function reload() {
    if (working.current || pending || submission.current) return;
    working.current = true; setAttempt(value => value + 1);
  }
  return <section className="native-source-network" aria-label={zh ? '接入网络设置' : 'Connection network settings'}>
    {loading && <p role="status">{zh ? '正在读取网络设置…' : 'Reading network settings…'}</p>}
    {view && (preview ? <><NativeNetworkImpact before={preview.before.policy} after={preview.after} models={view.models} decisions={view.decisions} language={language} /><div className="native-network-actions"><button className="btn" type="button" disabled={busy || pending} onClick={() => { setPreview(null); setReturnReveal(value => value + 1); }}>{zh ? '返回修改' : 'Back'}</button><button className="btn btn-primary" type="button" disabled={busy || pending || !mutable} onClick={() => void apply()}>{busy ? zh ? '正在保存…' : 'Saving…' : zh ? '确认并保存' : 'Confirm and save'}</button></div></> : <>
      <NativeNetworkSettings reveal={reveal + returnReveal} value={draft} onChange={changeDraft} language={language} disabled={!mutable || busy || pending || loading} target={{ kind: 'saved', source_id: sourceId, expected_source_revision: view.source_revision, protocol: null }} />
      <p className="field-help" role="status">{view.application_state === 'applied' ? zh ? `已保存的网络配置已应用 · 版本 ${view.applied_revision}，连通性尚需检测。` : `Saved network configuration applied · revision ${view.applied_revision}. Connectivity requires a check.` : view.application_state === 'pending' ? zh ? '正在应用配置，请等待原操作完成。' : 'Configuration is being applied. Wait for the original operation.' : zh ? '配置暂不可用，请检查设置。' : 'Configuration is unavailable. Review the settings.'}</p>
      {changed && <div className="native-network-actions"><button className="btn" type="button" disabled={busy || pending || loading} onClick={() => { void confirmDiscard(language).then(allowed => { if (allowed) setDraft(networkDraft(view.policy, view.network_revision)); }); }}>{zh ? '放弃修改' : 'Discard changes'}</button><button className="btn btn-primary" type="button" disabled={busy || pending || loading || !mutable || !!manualProxyError(draft)} onClick={() => void prepare()}>{zh ? '保存网络设置' : 'Save network settings'}</button></div>}
    </>)}
    {saved && <p role="status" className="field-help">{zh ? '网络设置已保存，请重新检测。关闭当前流程不会撤销此修改。' : 'Network settings were saved. Check again. Closing this flow does not undo the change.'}</p>}
    {error && <p role="alert" className="callout warn">{error}</p>}
    {consumerRefreshFailed && !changed && <p role="alert" className="callout warn">{zh ? '已保存的网络配置尚未同步到当前流程。请刷新当前流程后再检测。' : 'Saved network settings have not been refreshed in this flow. Refresh this flow before checking again.'}</p>}
    {pending ? <button className="btn" type="button" disabled={busy} onClick={() => void refreshPending()}>{zh ? '刷新原操作结果' : 'Refresh original operation'}</button> : consumerRefreshFailed && !changed ? <button className="btn" type="button" disabled={busy || loading} onClick={() => void retryConsumerRefresh()}>{zh ? '刷新当前流程' : 'Refresh this flow'}</button> : error ? <button className="btn" type="button" disabled={busy || loading} onClick={reload}>{changed ? zh ? '重新读取并保留输入' : 'Reload and keep input' : zh ? '重新读取' : 'Reload'}</button> : null}
  </section>;
}
