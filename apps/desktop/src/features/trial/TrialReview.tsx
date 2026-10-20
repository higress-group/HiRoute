import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { PlanEditor, type Draft, type Editor, type Plan, type PlanEditorHandle } from '../../plan-editor';
import { resolvePersistedEditor, persistenceTargetWasSuperseded, type PersistenceIdentity, type PersistedEditor } from '../../plan-editor-persistence';
import type { DesktopOperation, DesktopSnapshot } from '../../product/home-projections';
import type { OpenDecisionConnection } from '../decision-services/presentation';

export function TrialReview({ editor, language, mutable, snapshot, operation, onOperation, onRefresh, onPublished, onClose, onOpenServices, onPendingChange }: {
  editor: Editor; language: 'zh' | 'en'; mutable: boolean; snapshot: DesktopSnapshot | null; operation: DesktopOperation | null;
  onPendingChange(pending: boolean): void; onOperation(operation: DesktopOperation | null): void; onRefresh(): Promise<void>; onPublished(plan: Plan): void; onClose(): void; onOpenServices: OpenDecisionConnection;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const handle = useRef<PlanEditorHandle>(null);
  const [draft, setDraft] = useState<Draft>();
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [pending, setPending] = useState<{ action: 'save_draft' | 'publish'; identity: PersistenceIdentity; operationId: string | null; priorOperations: string[] } | null>(null);
  useEffect(() => { onPendingChange(!!pending); return () => onPendingChange(false); }, [!!pending, onPendingChange]);
  const autoPublish = useRef(false);
  const settled = useRef(new Set<string>());
  function complete(value: PersistedEditor<Plan, Draft>, action: 'save_draft' | 'publish', identity: PersistenceIdentity) {
    const key = `${action}/${identity.draftId}/${identity.targetDraftRevision ?? identity.targetHeadRevision}`;
    if (settled.current.has(key)) return;
    settled.current.add(key); setPending(null); setError('');
    if (action === 'publish' && value.plan) { autoPublish.current = false; onPublished(value.plan); }
    else if (value.draft) setDraft(value.draft);
  }
  useEffect(() => {
    if (!draft || !autoPublish.current || !ready || busy) return;
    autoPublish.current = false;
    // The saved draft identity/revision is now mounted in the ordinary editor.
    void handle.current?.publish();
  }, [draft, ready, busy]);
  useEffect(() => {
    if (!pending || !snapshot) return;
    const hint = snapshot.pending?.plan_id === (pending.identity.planId ?? pending.identity.draftId) ? snapshot.pending.operation_id : null;
    const id = pending.operationId ?? (hint && !pending.priorOperations.includes(hint) ? hint : null);
    if (!id || operation?.operation_id !== id) return;
    if (!pending.operationId) { setPending({ ...pending, operationId: id }); return; }
    const result = resolvePersistedEditor(pending.action, snapshot.catalog, pending.identity, { operationId: id, operation });
    if (result) { complete(result, pending.action, pending.identity); return; }
    if (operation.state === 'succeeded' && persistenceTargetWasSuperseded(pending.action, snapshot.catalog, pending.identity)) {
      autoPublish.current = false;
      setError(t('配置随后被再次修改，请到智能路由页面核对当前版本。', 'The configuration was updated again. Inspect the current revision in Routing.'));
    } else if (['rolled_back', 'failed', 'cancelled', 'compensated'].includes(operation.state)) {
      autoPublish.current = false; setPending(null);
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
    {error && <p className="callout warn" role="alert">{error}</p>}
    {pending && <div className="callout warn" role="status"><p>{t('正在核实此步结果。已完成的接入和草稿会保留；刷新不会重复提交。', 'Verifying this step. Completed connections and drafts are retained; refresh does not resubmit.')}</p><button className="btn" disabled={busy} onClick={() => void refresh().catch(() => setError(t('暂时无法核实，请稍后刷新。', 'Unable to verify yet. Refresh shortly.')))}>{t('刷新结果', 'Refresh result')}</button></div>}
    {draft && <p role="status">{t('草稿已保存。发布完成后再映射 Agent。', 'Draft saved. Map the Agent after publication completes.')}</p>}
    <fieldset disabled={!mutable || !!pending}><PlanEditor ref={handle} draft={draft} trialOnly creating initialContent={draft ? undefined : editor} language={language} onClose={onClose} onDone={onRefresh} onBusyChange={setBusy} onPublicationReady={setReady} onOpenServices={onOpenServices} onOperation={onOperation}
      onPersistenceStarted={(action, identity) => setPending({ action, identity, operationId: null, priorOperations: [operation?.operation_id, snapshot?.pending?.operation_id].filter((id): id is string => !!id) })}
      onPersistenceCancelled={() => { autoPublish.current = false; setPending(null); }}
      onPersisted={(value, action, identity, submitted) => {
        if (value) complete(value, action, identity);
        else setPending(current => settled.current.has(`${action}/${identity.draftId}/${identity.targetDraftRevision ?? identity.targetHeadRevision}`) ? current : { action, identity, operationId: submitted?.operation_id ?? current?.operationId ?? null, priorOperations: current?.priorOperations ?? [] });
      }} /></fieldset>
    <div className="actions"><button className="btn btn-primary" disabled={!mutable || !ready || busy || !!pending} onClick={() => {
      setError(''); autoPublish.current = !draft;
      void (draft ? handle.current?.publish() : handle.current?.saveDraft());
    }}>{draft ? t('发布并继续', 'Publish and continue') : t('保存并发布路由', 'Save and publish route')}</button></div>
  </section>;
}
