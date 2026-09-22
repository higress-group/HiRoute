import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PlanEditor, type Draft, type Plan, type PlanEditorHandle } from '../plan-editor';
import { persistenceTargetWasSuperseded, resolvePersistedEditor, type PersistenceIdentity } from '../plan-editor-persistence';
import { requestEditorReplacement } from '../ui/discard-guard';
import { ProductPage, UiIcon } from '../ui';
import type { DesktopOperation, DesktopSnapshot } from './home-projections';
import type { AgentSnapshot } from '../agents';
import { routingEditorEntries } from '../routing-editor-entries';

export type RoutingEditorIntent = { key: string; plan?: Plan; draft?: Draft; staleDraft?: boolean; initialBindingId?: string };

export function RoutingPage({ language, active = true, snapshot, agentSnapshot, operation, loading, busy, initialEditor = null, notice, onRefresh, onOperation, onOpenAgent, onOpenSession }: {
  language: 'zh' | 'en'; snapshot: DesktopSnapshot | null; agentSnapshot?: AgentSnapshot | null; loading: boolean; busy: boolean;
  active?: boolean;
  operation?: DesktopOperation | null;
  initialEditor?: RoutingEditorIntent | null; notice?: string; onRefresh(): Promise<void>;
  onOperation(operation: DesktopOperation | null): void; onOpenAgent?(agentId: string): void; onOpenSession?(sessionId: string, requestId: string): void;
}) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [editor, setEditor] = useState<RoutingEditorIntent | null>(initialEditor);
  const [dirty, setDirty] = useState(false);
  const [showList, setShowList] = useState(false);
  const [localBusy, setLocalBusy] = useState(false);
  const [error, setError] = useState('');
  const [persistedNotice, setPersistedNotice] = useState('');
  const [editorBusy, setEditorBusy] = useState(false);
  const [pendingPersistence, setPendingPersistence] = useState<{ action: 'save_draft' | 'publish'; identity: PersistenceIdentity; operationId: string | null } | null>(null);
  const initialized = useRef(!!initialEditor);
  const trigger = useRef<HTMLElement | null>(null);
  const returnEditor = useRef<RoutingEditorIntent | null>(null);
  const planEditor = useRef<PlanEditorHandle>(null);
  const plans = snapshot?.catalog.plans ?? [];
  const drafts = snapshot?.catalog.drafts ?? [];
  const mutable = !!snapshot?.trusted_authority && snapshot.service.mutation_available;
  const modes = { fixed_model: text('固定模型', 'Fixed model'), smart_saving: text('智能省钱', 'Smart saving'), free_first: text('免费优先', 'Free first') };
  const entries: RoutingEditorIntent[] = routingEditorEntries(plans, drafts);
  const creating = Boolean(editor && !editor.plan && !editor.draft);
  useEffect(() => {
    if (!pendingPersistence || !snapshot) return;
    const pendingScope = pendingPersistence.identity.planId ?? pendingPersistence.identity.draftId;
    const recoveredOperationId = snapshot.pending?.plan_id === pendingScope
      ? snapshot.pending.operation_id
      : null;
    if (!pendingPersistence.operationId && recoveredOperationId
      && operation?.operation_id === recoveredOperationId) {
      setPendingPersistence({ ...pendingPersistence, operationId: recoveredOperationId });
      return;
    }
    const persisted = resolvePersistedEditor(pendingPersistence.action, snapshot.catalog, pendingPersistence.identity, {
      operationId: pendingPersistence.operationId,
      operation,
    });
    if (!persisted && operation?.operation_id === pendingPersistence.operationId
      && operation.state === 'succeeded'
      && persistenceTargetWasSuperseded(pendingPersistence.action, snapshot.catalog, pendingPersistence.identity)) {
      setPendingPersistence(null);
      setError(text('本次更改已完成，但路由随后又被更新。当前输入仍保留，请读取最新状态后决定是否重试。', 'This change completed, but the route was updated again afterward. Your input is retained; review the latest state before retrying.'));
      return;
    }
    if (!persisted) return;
    setEditor(persisted);
    returnEditor.current = null;
    setDirty(false);
    setPendingPersistence(null);
    setPersistedNotice(pendingPersistence.action === 'save_draft'
      ? text('草稿已保存，可以继续编辑或发布。', 'Draft saved. Continue editing or publish.')
      : text('更改已发布，新请求将使用当前路由。', 'Changes published. New requests will use this routing.'));
  }, [operation, pendingPersistence, snapshot]);
  useEffect(() => {
    if (!pendingPersistence?.operationId || operation?.operation_id !== pendingPersistence.operationId) return;
    if (!['rolled_back', 'needs_attention'].includes(operation.state)) return;
    setPendingPersistence(null);
    setError(operation.state === 'rolled_back'
      ? text('更改未生效，当前输入已保留。请读取最新状态后重试。', 'The change was not applied. Your input is retained; refresh the latest state and try again.')
      : text('保存未完成。当前输入已保留，请检查路由实际状态后重试。', 'The save did not complete. Your input is retained; check the current route before retrying.'));
  }, [operation, pendingPersistence]);
  useEffect(() => {
    if (!persistedNotice) return;
    const timer = window.setTimeout(() => setPersistedNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [persistedNotice]);
  useEffect(() => {
    if (!initialized.current && snapshot && !snapshot.catalog_error && entries.length) {
      initialized.current = true;
      setEditor(entries[0]);
    }
  }, [snapshot]);
  async function choose(next: RoutingEditorIntent, source: HTMLElement) {
    if (editor?.key !== next.key && dirty && !(await requestEditorReplacement('routing'))) return;
    trigger.current = source;
    if (editor?.key !== next.key) { setDirty(false); setPendingPersistence(null); setPersistedNotice(''); setEditor(next); }
    returnEditor.current = null;
    setShowList(false);
  }
  async function restore(planId: string) {
    setLocalBusy(true); setError('');
    try {
      const result = await invoke<{ operation: DesktopOperation | null }>('preview_restore_name', { input: { plan_id: planId, language } });
      onOperation(result.operation); await onRefresh();
    } catch { setError(text('未能恢复名称，请刷新状态后重试。', 'Could not restore the name. Refresh and retry.')); }
    finally { setLocalBusy(false); }
  }
  async function newRoute(source: HTMLElement) {
    if (dirty && !(await requestEditorReplacement('routing'))) return;
    returnEditor.current = editor;
    trigger.current = source;
    setDirty(false);
    setPendingPersistence(null);
    setPersistedNotice('');
    setEditor({ key: crypto.randomUUID() });
    setShowList(false);
  }
  const agentName = (agentId: string) => agentId === 'agent_codex_default' ? 'Codex' : agentId === 'agent_claude_default' ? 'Claude Code' : text('本机 Agent', 'Local Agent');
  const agentBrand = (agentId: string) => agentId.toLowerCase().includes('codex') ? 'codex' as const : agentId.toLowerCase().includes('claude') ? 'claude-code' as const : 'agent' as const;
  const usedBy = editor?.plan ? (agentSnapshot?.agents ?? []).flatMap(agent => {
    const model = agent.settings?.current_selection;
    const isDefault = model?.mode === 'codex_default'
      && model.default_selection.kind === 'plan'
      && model.default_selection.plan_id === editor.plan!.agent_plan_id;
    const isAllowed = model?.mode === 'codex_default'
      ? model.allowed_plan_ids.includes(editor.plan!.agent_plan_id)
      : model?.mode === 'claude_launcher'
        ? Object.values(model.preset_mappings).some(selection => selection.kind === 'plan' && selection.plan_id === editor.plan!.agent_plan_id)
        : false;
    return isDefault || isAllowed ? [{ id: agent.agent_id, name: agentName(agent.agent_id), brand: agentBrand(agent.agent_id), isDefault }] : [];
  }) : [];
  const pageLocked = busy || localBusy || editorBusy || !mutable;
  const pageActions = creating ? <>
    <button className="btn" type="button" disabled={pageLocked} onClick={() => void planEditor.current?.saveDraft()}>{text('保存草稿', 'Save draft')}</button>
    <button className="btn" type="button" disabled={editorBusy} onClick={() => planEditor.current?.cancel()}>{text('取消', 'Cancel')}</button>
    <button className="btn btn-primary" type="button" disabled={pageLocked} onClick={() => void planEditor.current?.publish()}>{text('启用', 'Enable')}</button>
  </> : entries.length ? <button className="btn btn-primary" disabled={busy || localBusy || !mutable} onClick={event => void newRoute(event.currentTarget)}><UiIcon name="plus" />{text('新建智能路由', 'New smart routing')}</button> : undefined;
  const closeEditor = () => {
    const next = creating ? returnEditor.current : null;
    returnEditor.current = null;
    setEditor(next);
    setDirty(false);
    setEditorBusy(false);
    setPendingPersistence(null);
    setPersistedNotice('');
    requestAnimationFrame(() => trigger.current?.isConnected && trigger.current.focus());
  };
  const editorView = editor ? <fieldset className="detail-fieldset" disabled={!mutable || busy || localBusy || editorBusy}>
    {editor.staleDraft && <div className="callout warn" role="status"><UiIcon name="warning" /><div><strong>{text('这是基于旧版本的草稿', 'This draft is based on an older version')}</strong><p>{text('当前生效路由已单独显示在列表中。保留此草稿供查看；如需继续，请先读取当前配置再重新编辑。', 'The active route is listed separately. This draft is retained for inspection; reload the current route before editing further.')}</p></div></div>}
    <PlanEditor ref={planEditor} key={`${editor.key}:${editor.draft?.revision ?? editor.plan?.head.head_revision ?? 'new'}`} creating={creating} plan={editor.plan} draft={editor.draft} initialBindingId={editor.initialBindingId} language={language} active={active} usedBy={usedBy} onOpenAgent={onOpenAgent} onOpenSession={onOpenSession} onDirty={next => { setDirty(next); if (next) setPersistedNotice(''); }} onEdit={() => setPendingPersistence(null)} onBusyChange={setEditorBusy} onOperation={onOperation} onPersisted={(next, action, identity, submitted) => { setPendingPersistence(next ? null : { action, identity, operationId: submitted?.operation_id ?? null }); if (next) { returnEditor.current = null; setEditor(next); setDirty(false); setPersistedNotice(action === 'save_draft' ? text('草稿已保存，可以继续编辑或发布。', 'Draft saved. Continue editing or publish.') : text('更改已发布，新请求将使用当前路由。', 'Changes published. New requests will use this routing.')); } }} onDone={onRefresh} onClose={closeEditor} />
  </fieldset> : null;
  return <ProductPage
    title={creating ? text('新建智能路由', 'New smart routing') : text('智能路由', 'Smart routing')}
    subtitle={creating ? text('启用前不会对 Agent 生效。', 'This route does not affect Agents until enabled.') : text('为 Agent 创建可复用的智能路由配置。', 'Create reusable smart routing for Agents.')}
    flush={!creating}
    className="routing-page"
    actions={pageActions}
  >
    {(notice || persistedNotice) && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{notice || persistedNotice}</span></div></div>}
    {error && <div role="alert" className="callout bad"><UiIcon name="warning" /><span>{error}</span></div>}
    {loading && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取路由…', 'Reading routes…')}</p></div></div>}
    {!!snapshot?.catalog_error && <div className="callout warn" role="alert"><UiIcon name="warning" /><span>{text('暂时无法刷新路由，当前编辑仍会保留。', 'Unable to refresh routes. Your edits remain available.')}</span><button className="btn" type="button" onClick={() => void onRefresh()}>{text('重试', 'Retry')}</button></div>}
    {!mutable && snapshot && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('路由可以查看，暂时不能修改', 'Routes are viewable but cannot be changed')}</strong><p>{text('本机服务尚未就绪，请刷新状态后保存。', 'The local service is not ready. Refresh its status before saving.')}</p></div></div>}
    {creating ? <div className="new-plan-workspace">{editorView}</div> : !loading && !entries.length && !editor ? <div className="empty-state routing-empty"><div><span className="empty-icon"><UiIcon name="route" /></span><h3>{text('还没有智能路由', 'No smart routing yet')}</h3><p>{text('创建一份配置，决定 Agent 怎样使用你的模型。', 'Create a plan that determines how Agents use your models.')}</p><button className="btn btn-primary" disabled={busy || localBusy || !mutable} onClick={event => void newRoute(event.currentTarget)}><UiIcon name="plus" />{text('新建智能路由', 'Create smart routing')}</button></div></div> : <div className={`split-view${showList || !editor ? ' show-list' : ''}`}>
      <aside className="master-pane"><nav className="master-list native-list" aria-label={text('路由列表', 'Routing list')}>
        {entries.map(item => {
          const mode = item.draft?.editor.mode ?? item.plan?.desired.mode;
          const active = editor?.key === item.key;
          return <button className={`list-row${active ? ' active' : ''}`} key={item.key} aria-current={active ? 'page' : undefined} onClick={event => void choose(item, event.currentTarget)}>
            <span className="route-avatar" data-mode={mode} aria-hidden="true">{mode === 'smart_saving' ? 'S' : mode === 'free_first' ? 'F' : 'M'}</span>
            <span className="row-main"><span className="row-title">{item.draft?.editor.display_name || item.plan?.desired.display_name || text('未命名草稿', 'Untitled draft')}{item.staleDraft ? ` · ${text('过期草稿', 'Outdated draft')}` : ''}</span><span className="row-meta">{modes[mode!]} · {item.draft?.editor.purpose ?? item.plan?.desired.purpose}</span></span>
            <UiIcon name="chevronRight" />
          </button>;
        })}
      </nav></aside>
      <section className="detail-pane"><div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => setShowList(true)}><UiIcon name="arrowLeft" />{text('返回路由列表', 'Back to routes')}</button></div>
        {editor ? <>{editorView}
          {editor.plan && snapshot?.restore_names.some(item => item.plan_id === editor.plan!.agent_plan_id) && <div className="editor-section route-usage"><button className="btn" disabled={busy || localBusy || !mutable} onClick={() => void restore(editor.plan!.agent_plan_id)}>{text('恢复先前名称', 'Restore previous name')}</button></div>}
        </> : <div className="empty-state"><div><h3>{text('选择一份智能路由', 'Select a route')}</h3><p>{text('从列表继续编辑，或创建新路由。', 'Continue from the list, or create a new route.')}</p></div></div>}
      </section>
    </div>}
  </ProductPage>;
}
