import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { PlanEditor, type Draft, type Editor, type Plan, type PlanEditorHandle } from '../../plan-editor';
import { resolvePersistedEditor, persistenceTargetWasSuperseded, type PersistenceIdentity, type PersistedEditor } from '../../plan-editor-persistence';
import type { DesktopOperation, DesktopSnapshot } from '../../product/home-projections';
import type { OpenDecisionConnection } from '../decision-services/presentation';

export type TrialReviewHandle = { currentEditor(): Editor | undefined };

export const TrialReview = forwardRef<TrialReviewHandle, {
  editor: Editor; language: 'zh' | 'en'; mutable: boolean; snapshot: DesktopSnapshot | null; operation: DesktopOperation | null;
  previewAgentId: string;
  onPendingChange(pending: boolean): void; onRepresentedOperations(ids: string[]): void; onOperation(operation: DesktopOperation | null): void; onRefresh(): Promise<void>; onPublished(plan: Plan): void; onClose(): void; onBack(editor: Editor): void; onOpenServices: OpenDecisionConnection;
}>(function TrialReview({ editor, previewAgentId, language, mutable, snapshot, operation, onOperation, onRefresh, onPublished, onClose, onBack, onOpenServices, onPendingChange, onRepresentedOperations }, ref) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const handle = useRef<PlanEditorHandle>(null);
  const [draft, setDraft] = useState<Draft>();
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [phase, setPhase] = useState<'idle' | 'saving' | 'publishing' | 'confirming' | 'error'>('idle');
  const [ownedIds, setOwnedIds] = useState<string[]>([]);
  const [pending, setPending] = useState<{ action: 'save_draft' | 'publish'; identity: PersistenceIdentity; operationId: string | null; priorOperations: string[] } | null>(null);
  useImperativeHandle(ref, () => ({ currentEditor: () => busy || pending || draft ? undefined : handle.current?.currentEditor() }));
  useEffect(() => { onPendingChange(!!pending); return () => onPendingChange(false); }, [!!pending, onPendingChange]);
  useEffect(() => { onRepresentedOperations(['saving', 'publishing', 'confirming'].includes(phase) ? ownedIds : []); }, [phase, ownedIds, onRepresentedOperations]);
  useEffect(() => () => onRepresentedOperations([]), [onRepresentedOperations]);
  const autoPublish = useRef(false);
  const settled = useRef(new Set<string>());
  function complete(value: PersistedEditor<Plan, Draft>, action: 'save_draft' | 'publish', identity: PersistenceIdentity) {
    const key = `${action}/${identity.draftId}/${identity.targetDraftRevision ?? identity.targetHeadRevision}`;
    if (settled.current.has(key)) return;
    settled.current.add(key); setPending(null); setError('');
    if (action === 'publish' && value.plan) { autoPublish.current = false; setPhase('idle'); onPublished(value.plan); }
    else if (value.draft) { setPhase(autoPublish.current ? 'publishing' : 'idle'); setDraft(value.draft); }
  }
  useEffect(() => {
    if (!draft || !autoPublish.current || !ready || busy || !mutable) return;
    autoPublish.current = false;
    // The saved draft identity/revision is now mounted in the ordinary editor.
    void handle.current?.publish().then(ok => { if (!ok) setPhase(current => current === 'publishing' ? 'idle' : current); });
  }, [draft, ready, busy, mutable]);
  useEffect(() => {
    if (!pending || !snapshot) return;
    const hint = snapshot.pending?.plan_id === (pending.identity.planId ?? pending.identity.draftId) ? snapshot.pending.operation_id : null;
    const id = pending.operationId ?? (hint && !pending.priorOperations.includes(hint) ? hint : null);
    if (!id || operation?.operation_id !== id) return;
    if (!pending.operationId) { setOwnedIds(ids => ids.includes(id) ? ids : [...ids, id]); setPending({ ...pending, operationId: id }); return; }
    const result = resolvePersistedEditor(pending.action, snapshot.catalog, pending.identity, { operationId: id, operation });
    if (result) { complete(result, pending.action, pending.identity); return; }
    if (operation.state === 'succeeded' && persistenceTargetWasSuperseded(pending.action, snapshot.catalog, pending.identity)) {
      autoPublish.current = false;
      setPending(null); setPhase('error');
      setError(t('配置随后被再次修改，请到智能路由页面核对当前版本。', 'The configuration was updated again. Inspect the current revision in Routing.'));
    } else if (['rolled_back', 'failed', 'cancelled', 'compensated', 'needs_attention'].includes(operation.state)) {
      autoPublish.current = false; setPending(null); setPhase('error');
      setError(t('此步未生效，已保存的草稿和接入仍保留。可核对后继续。', 'This step did not apply. Saved drafts and connections are retained; review and continue.'));
    }
  }, [pending, snapshot, operation]);
  async function refresh() {
    await onRefresh();
    // observe_operation reconciles the host's same idempotency key. It does not
    // submit a second Plan mutation or accept an unrelated operation identity.
    const current = await invoke<DesktopOperation | null>('observe_operation');
    if (current) onOperation(current);
  }
  return <section className="trial-review">
    <fieldset disabled={!mutable || !!pending}><PlanEditor ref={handle} draft={draft} trialOnly trialBusy={busy || ['saving', 'publishing', 'confirming'].includes(phase)} previewAgentId={previewAgentId} creating initialContent={draft ? undefined : editor} language={language} onClose={onClose} onDone={onRefresh} onBusyChange={setBusy} onPublicationReady={setReady} onOpenServices={onOpenServices} onOperation={onOperation}
      onPersistenceStarted={(action, identity) => { setPhase(action === 'save_draft' ? 'saving' : 'publishing'); setPending({ action, identity, operationId: null, priorOperations: [operation?.operation_id, snapshot?.pending?.operation_id].filter((id): id is string => !!id) }); }}
      onPersistenceCancelled={() => { autoPublish.current = false; setPending(null); setPhase('idle'); }}
      onPersisted={(value, action, identity, submitted) => {
        if (submitted?.operation_id) setOwnedIds(ids => ids.includes(submitted.operation_id) ? ids : [...ids, submitted.operation_id]);
        if (value) complete(value, action, identity);
        else if (!settled.current.has(`${action}/${identity.draftId}/${identity.targetDraftRevision ?? identity.targetHeadRevision}`)) { setPhase('confirming'); setPending(current => ({ action, identity, operationId: submitted?.operation_id ?? current?.operationId ?? null, priorOperations: current?.priorOperations ?? [] })); }
      }} /></fieldset>
    <div className="actions trial-review-actions"><button className="btn" type="button" disabled={busy || !!pending || !!draft} onClick={() => { const current = handle.current?.currentEditor(); if (current) onBack(current); }}>{t('返回设置', 'Back to setup')}</button><button className="btn btn-primary" disabled={!mutable || !ready || busy || !!pending || phase === 'publishing'} onClick={() => {
      setError(''); setOwnedIds([]); autoPublish.current = !draft;
      void (draft ? handle.current?.publish() : handle.current?.saveDraft());
    }}>{['saving', 'publishing', 'confirming'].includes(phase) ? t('正在保存并发布…', 'Saving and publishing…') : draft ? t('发布并继续', 'Publish and continue') : t('保存并发布路由', 'Save and publish route')}</button></div>
    <div className="trial-publication-status" aria-live="polite">{error ? <p className="oc-inline-error" role="alert">{error}</p> : phase === 'saving' ? <span role="status">{t('正在保存草稿。', 'Saving the draft.')}</span> : phase === 'publishing' ? <span role="status">{t('正在发布路由。', 'Publishing the route.')}</span> : phase === 'confirming' ? <span role="status">{pending?.action === 'save_draft' ? t('正在确认草稿保存结果。', 'Confirming the draft save.') : t('正在确认发布结果。', 'Confirming the publication result.')}</span> : null}{pending && !busy && <button className="btn btn-quiet" onClick={() => void refresh().catch(() => setError(t('暂时无法核实，请稍后刷新。', 'Unable to verify yet. Refresh shortly.')))}>{t('刷新结果', 'Refresh result')}</button>}</div>
  </section>;
});
