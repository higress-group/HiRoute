import { ProviderIcon } from '../../ui/ProviderIcon';
import { connectionName } from '../../ui/provider-identity';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Plan } from '../../plan-editor';
import { Dialog, UiIcon } from '../../ui';
import { confirmDiscard, useDiscardGuard } from '../../ui/discard-guard';
import type { PriceDisplay } from '../model-reference/types';
import { clientOperationId } from '../model-connections/state';
import { addDraftKey, clearSecrets, createDraft, moveDraftKey, type ModelDraft } from './model-draft';
import { subscriptionAttentionCopy } from './subscription-copy';
import type {
  CandidateRef,
  KeyEdit,
  ManagedModel,
  ManagedSource,
  ManagementChange,
  ManagementSnapshot,
  ProtectedKeyInput,
  SavePreview,
} from './types';

type Props = {
  language: 'zh' | 'en';
  snapshot: ManagementSnapshot;
  initialSourceId?: string | null;
  busy?: boolean;
  onRefresh(): Promise<void>;
  onPrepareKeyInput(sourceId: string, input: ProtectedKeyInput): Promise<CandidateRef>;
  onPreview(change: ManagementChange): Promise<SavePreview>;
  onApply(preview: SavePreview): Promise<'saved' | 'uncertain'>;
  onDiscardProtectedInputs(): void;
  onCredentialsSaved?(): void;
  plans?: Plan[];
  onOpenPlan?(planId: string): void;
  onCreatePlan?(bindingId: string): void;
  onEditPrice?(source: ManagedSource, model: ManagedModel): void;
  onReauthorize?(sourceId: string): void;
  onRecheck?(source: ManagedSource, checkId: string, editRevision: number): Promise<'saved' | 'uncertain'>;
  onCancelRecheck?(checkId: string): Promise<void>;
  onReconnect?(source: ManagedSource): void;
  onSourceStateSaved?(enabled: boolean): void;
  mutable: boolean;
};

type Filter = 'all' | 'subscription' | 'api' | 'free';

function sourceAccess(source: ManagedSource): Exclude<Filter, 'all' | 'free'> | 'unknown' {
  if (source.connection_identity?.access_kind === 'subscription') return 'subscription';
  if (source.connection_identity?.access_kind === 'api') return 'api';
  return 'unknown';
}

function modelMatchesFilter(source: ManagedSource, model: ManagedModel | undefined, filter: Filter) {
  if (filter === 'all') return true;
  if (filter === 'free') return model?.presentation?.billing_class === 'free';
  return sourceAccess(source) === filter;
}

function modelAvailability(_source: ManagedSource, model: ManagedModel | undefined) {
  if (model?.presentation?.availability) return model.presentation.availability;
  // Source health is not a per-binding readiness proof. Older or partial
  // snapshots keep their saved rows visible, but fail closed until the model
  // presentation is available.
  return 'unknown' as const;
}

function safeErrorCode(error: unknown): string {
  const pending: unknown[] = [error];
  const visited = new Set<object>();
  while (pending.length && visited.size < 8) {
    const current = pending.shift();
    if (!current || typeof current !== 'object' || visited.has(current)) continue;
    visited.add(current);
    const value = current as Record<string, unknown>;
    if (typeof value.code === 'string' && /^[A-Za-z0-9_.\/-]{1,80}$/.test(value.code)) return value.code;
    pending.push(value.error, value.envelope, value.failure);
  }
  return 'COMPUTE_MANAGEMENT_SAVE_FAILED';
}

export function Models(props: Props) {
  const zh = props.language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(props.initialSourceId ?? props.snapshot.sources[0]?.source_id ?? null);
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [draft, setDraft] = useState<ModelDraft | null>(null);
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [errorCode, setErrorCode] = useState('');
  const [credentialEditing, setCredentialEditing] = useState(false);
  const [rechecking, setRechecking] = useState<{ sourceId: string; checkId: string } | null>(null);
  const [recheckError, setRecheckError] = useState('');
  const [recheckNotice, setRecheckNotice] = useState('');
  const [stateChangeError, setStateChangeError] = useState('');
  const activeRow = useRef<HTMLButtonElement | null>(null);
  const activeRecheck = useRef<string | null>(null);
  const recheckRevision = useRef(0);

  const needle = query.trim().toLocaleLowerCase();
  const entries = useMemo(() => props.snapshot.sources.flatMap<{ source: ManagedSource; model: ManagedModel | undefined }>(source => {
    const models = source.models.length ? source.models : [undefined];
    return models
      .filter(model => modelMatchesFilter(source, model, filter))
      .filter(model => !needle || `${model?.display_name ?? ''} ${source.display_name} ${source.connection_identity?.product_label ?? ''}`.toLocaleLowerCase().includes(needle))
      .map(model => ({ source, model }));
  }), [filter, needle, props.snapshot.sources]);

  const selectedEntry = entries.find(item => item.source.source_id === selectedSourceId
    && (!selectedModelId || item.model?.model_ref === selectedModelId));
  const activeEntry = selectedEntry ?? entries[0];
  const source = activeEntry?.source;
  const model = activeEntry?.model;
  const access = source ? sourceAccess(source) : 'unknown';
  const sourceLabel = source ? connectionLabel(source, props.language) : '';
  const connectorManaged = source?.provenance === 'connector_owned';
  const canManageCredentials = Boolean(props.mutable && source && !connectorManaged && access !== 'subscription'
    && source.authentication.kind !== 'none'
    && source.actions.some(action => action === 'edit' || action === 'add_key' || action === 'enable' || action === 'disable'));
  const canReauthorize = Boolean(props.mutable && source?.actions.includes('reauthorize') && props.onReauthorize);
  const usedPlans = useMemo(() => !model ? [] : (props.plans ?? []).filter(plan => {
    if (plan.head.status !== 'enabled') return false;
    const strategy = plan.desired.strategy;
    return [...(strategy.candidates ?? []), ...(strategy.economy ?? []), ...(strategy.primary ?? [])]
      .some(selection => selection.binding_id === model.binding_id);
  }), [model, props.plans]);

  const pristineDraft = source ? createDraft(source) : null;
  const dirty = Boolean(draft && pristineDraft && JSON.stringify(draft) !== JSON.stringify(pristineDraft));
  useDiscardGuard('models', dirty, props.language, confirmCredentialReplacement);

  useEffect(() => {
    if (!props.initialSourceId || !props.snapshot.sources.some(item => item.source_id === props.initialSourceId)) return;
    setFilter('all');
    setQuery('');
    setSelectedSourceId(props.initialSourceId);
  }, [props.initialSourceId, props.snapshot.sources]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented || event.key !== 'Escape' || draft || !detailOpen || !window.matchMedia('(max-width: 800px)').matches) return;
      event.preventDefault();
      returnToList();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [detailOpen, draft]);

  useEffect(() => {
    if (!recheckNotice) return;
    const timer = window.setTimeout(() => setRecheckNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [recheckNotice]);

  useEffect(() => () => {
    const checkId = activeRecheck.current;
    activeRecheck.current = null;
    if (checkId) void props.onCancelRecheck?.(checkId).catch(() => undefined);
  }, []);

  function returnToList() {
    setDetailOpen(false);
    requestAnimationFrame(() => activeRow.current?.focus({ preventScroll: true }));
  }

  function discardCredentials() {
    if (draft) setDraft(clearSecrets(draft));
    setDraft(null);
    setErrorCode('');
    setCredentialEditing(false);
    props.onDiscardProtectedInputs();
  }

  async function confirmCredentialReplacement() {
    if (dirty && !(await confirmDiscard(props.language))) return false;
    discardCredentials();
    return true;
  }

  function cancelCredentials() {
    discardCredentials();
  }

  async function saveCredentials() {
    if (!props.mutable || !source || !draft || submittingRef.current) return;
    if (credentialEditing) {
      setErrorCode('CREDENTIAL_INPUT_PENDING');
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setErrorCode('');
    try {
      const edits: KeyEdit[] = [];
      for (const key of draft.keys) {
        if (key.removed && key.keyId) {
          edits.push({ action: 'remove', key_id: key.keyId, expected_generation: key.expectedGeneration });
          continue;
        }
        if (key.removed) continue;
        if (key.replacement) {
          const candidate = await props.onPrepareKeyInput(source.source_id, {
            draft_id: key.draftId,
            key_id: key.keyId,
            expected_generation: key.expectedGeneration,
            value: key.replacement,
          });
          edits.push(key.keyId
            ? { action: 'replace', key_id: key.keyId, expected_generation: key.expectedGeneration, input_candidate: candidate }
            : { action: 'add', input_candidate: candidate });
        }
        const original = source.keys.find(item => item.key_id === key.keyId);
        if (key.keyId && original && original.enabled !== key.enabled) {
          edits.push({ action: 'set_enabled', key_id: key.keyId, expected_generation: key.expectedGeneration, enabled: key.enabled });
        }
      }
      const retainedIds = draft.keys.filter(key => !key.removed && key.keyId).map(key => key.keyId!);
      const originalIds = source.keys.map(key => key.key_id);
      if (retainedIds.length === originalIds.length && retainedIds.some((id, index) => id !== originalIds[index])) {
        edits.push({ action: 'set_order', key_ids: retainedIds });
      }
      const change: ManagementChange = {
        schema: 'hiroute.compute-management-change/v2',
        subject: { kind: 'saved_source', source_id: source.source_id },
        expected_revisions: props.snapshot.revisions,
        selected_model_refs: draft.selectedModels,
        intent: draft.enabled ? 'save_ready' : 'save_disabled',
        key_edits: edits,
      };
      const preview = await props.onPreview(change);
      const outcome = await props.onApply(preview);
      setDraft(clearSecrets(draft));
      setDraft(null);
      setCredentialEditing(false);
      props.onDiscardProtectedInputs();
      await props.onRefresh();
      if (outcome !== 'uncertain') props.onCredentialsSaved?.();
    } catch (cause) {
      props.onDiscardProtectedInputs();
      setErrorCode(safeErrorCode(cause));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  function selectEntry(nextSource: ManagedSource, nextModel: ManagedModel | undefined) {
    setRecheckError('');
    setRecheckNotice('');
    setStateChangeError('');
    setSelectedSourceId(nextSource.source_id);
    setSelectedModelId(nextModel?.model_ref ?? null);
    setDetailOpen(true);
  }

  async function recheckSource() {
    if (!props.mutable || !source || !props.onRecheck || activeRecheck.current) return;
    const checkId = clientOperationId('saved-model-check');
    const editRevision = ++recheckRevision.current;
    activeRecheck.current = checkId;
    setRechecking({ sourceId: source.source_id, checkId });
    setRecheckError('');
    setRecheckNotice('');
    try {
      const outcome = await props.onRecheck(source, checkId, editRevision);
      if (activeRecheck.current !== checkId) return;
      if (outcome !== 'uncertain') setRecheckNotice(text('接入检查已更新。', 'Connection check updated.'));
    } catch (cause) {
      if (activeRecheck.current !== checkId) return;
      const code = safeErrorCode(cause).toLocaleUpperCase();
      setRecheckError(code.includes('RECHECK_CONTEXT_UNAVAILABLE')
        ? source.authentication.kind === 'none'
          ? text('缺少可重检的接入信息。请使用“重新接入”创建新来源。', 'The saved connection cannot be rechecked. Use Reconnect to create a new source.')
          : text('无法安全复用原凭据检查。请通过“管理凭据”重新保存。', 'The saved credential cannot be safely reused. Save it again through Manage credentials.')
        : code.includes('AUTH') || code.includes('CREDENTIAL') || code.includes('ACTION_REQUIRED')
          ? text('当前凭据无法完成检查。请通过“管理凭据”更新后重试。', 'The saved credential could not complete the check. Update it through Manage credentials and try again.')
          : code.includes('CONFLICT') || code.includes('SOURCE_MISMATCH')
            ? text('接入状态已经变化。请刷新后再试。', 'The connection changed. Refresh and try again.')
            : text('暂时无法检查这个接入。请确认本机服务和网络后重试。', 'This connection could not be checked. Confirm the local service and network, then try again.'));
    } finally {
      if (activeRecheck.current === checkId) {
        activeRecheck.current = null;
        setRechecking(null);
      }
    }
  }

  async function changeSourceState(enabled: boolean) {
    if (!props.mutable || !source || source.provenance === 'connector_owned' || submittingRef.current) return;
    if (!source.actions.includes(enabled ? 'enable' : 'disable')) return;
    submittingRef.current = true;
    setSubmitting(true);
    setStateChangeError('');
    try {
      const change: ManagementChange = {
        schema: 'hiroute.compute-management-change/v2',
        subject: { kind: 'saved_source', source_id: source.source_id },
        expected_revisions: props.snapshot.revisions,
        selected_model_refs: source.models.map(model => model.model_ref),
        intent: enabled ? 'save_ready' : 'save_disabled',
        key_edits: [],
      };
      const preview = await props.onPreview(change);
      const outcome = await props.onApply(preview);
      await props.onRefresh();
      if (outcome !== 'uncertain') props.onSourceStateSaved?.(enabled);
    } catch (cause) {
      setStateChangeError(safeErrorCode(cause));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  function cancelRecheck() {
    const checkId = activeRecheck.current;
    if (!checkId) return;
    activeRecheck.current = null;
    setRechecking(null);
    setRecheckError('');
    setRecheckNotice(text('已取消检查。', 'Check cancelled.'));
    void props.onCancelRecheck?.(checkId).catch(() => undefined);
  }

  const status = source ? modelAvailability(source, model) : 'unknown';
  const billing = model?.presentation?.billing_class ?? 'unknown';
  const priceContexts = model?.presentation?.price_contexts ?? [];
  const canEditPrice = access === 'api'
    && props.mutable
    && !['free', 'subscription'].includes(billing)
    && priceContexts.length > 0
    && Boolean(props.onEditPrice);
  const canRecheck = Boolean(props.mutable && source?.actions.includes('recheck') && access !== 'subscription' && props.onRecheck);
  const sourceRechecking = Boolean(source && rechecking?.sourceId === source.source_id);
  const reason = model?.presentation?.reason_code ?? null;
  const subscriptionAttention = access === 'subscription'
    ? subscriptionAttentionCopy(reason, props.language)
    : null;

  return <section className="models-feature" aria-label={text('我的模型', 'My models')}>
    <div className={`split-view oc-models${detailOpen ? ' oc-model-detail-open' : ''}`}>
      <aside className="master-pane" aria-label={text('模型列表', 'Model list')}>
        <div className="master-toolbar">
          <label className="search-box">
            <span className="sr-only">{text('搜索模型或来源', 'Search models or sources')}</span>
            <UiIcon name="search" />
            <input className="input" value={query} onChange={event => { setQuery(event.target.value); setDetailOpen(false); }} placeholder={text('搜索模型或来源', 'Search models or sources')} />
          </label>
          <div className="source-tabs" aria-label={text('来源筛选', 'Source filter')}>
            {([
              ['all', text('全部', 'All')],
              ['subscription', text('订阅', 'Subscriptions')],
              ['api', 'API'],
              ['free', text('免费', 'Free')],
            ] as const).map(([value, label]) => <button className={`source-tab${filter === value ? ' active' : ''}`} key={value} type="button" aria-pressed={filter === value} onClick={() => { setFilter(value); setDetailOpen(false); }}>{label}</button>)}
          </div>
        </div>
        <nav className="master-list native-list" aria-label={text('模型', 'Models')}>
          {entries.map(item => {
            const active = source?.source_id === item.source.source_id && model?.model_ref === item.model?.model_ref;
            const itemStatus = modelAvailability(item.source, item.model);
            return <button ref={active ? activeRow : undefined} className={`list-row${active ? ' active' : ''}`} key={item.model?.binding_id ?? item.source.source_id} aria-current={active ? 'page' : undefined} onClick={() => selectEntry(item.source, item.model)}>
              <ProviderIcon optionId={item.source.display_template_id ?? item.source.connection_identity?.connection_option_id} language={props.language} />
              <span className="row-main"><span className="row-title">{item.model?.display_name || item.source.display_name}</span><span className="row-meta">{connectionLabel(item.source, props.language)}</span></span>
              <StatusBadge value={itemStatus} reason={item.model?.presentation?.reason_code} subscription={sourceAccess(item.source) === 'subscription'} compact language={props.language} />
            </button>;
          })}
          {!entries.length && <div className="empty-state"><div><div className="empty-icon"><UiIcon name="search" /></div><h3>{text('没有匹配的模型', 'No matching models')}</h3><p>{text('试试其他关键词，或清除筛选。', 'Try another keyword or clear filters.')}</p><button className="btn" type="button" onClick={() => { setQuery(''); setFilter('all'); }}>{text('清除筛选', 'Clear filters')}</button></div></div>}
        </nav>
      </aside>

      <section className="detail-pane">
        <div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={returnToList}><UiIcon name="arrowLeft" />{text('返回模型列表', 'Back to models')}</button></div>
        {source ? <div className="detail-inner">
          <div className="detail-hero">
            <div className="detail-identity"><ProviderIcon optionId={source.display_template_id ?? source.connection_identity?.connection_option_id} language={props.language} /><div><h2>{model?.display_name ?? source.display_name}</h2><p>{sourceLabel}</p></div></div>
            <StatusBadge value={status} reason={reason} subscription={access === 'subscription'} language={props.language} />
          </div>

          {status === 'available' && <p className="field-help">{text('接入就绪仅表示本地路由条件满足，不代表上游推理或工具调用已验证。', 'Connection ready means local routing conditions are met; upstream inference and tool calls are not verified by this status.')}</p>}

          {!props.mutable && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('模型可以查看，暂时不能修改', 'Models are viewable but cannot be changed')}</strong><p>{text('本机服务尚未就绪，连接恢复后即可修改接入。', 'The local service is not ready. Reconnect before saving.')}</p></div></div>}
          {props.snapshot.runtime_state === 'partial' && <div className="callout warn" role="status"><UiIcon name="warning" /><div><strong>{text('部分状态暂无法确认', 'Some status is temporarily unavailable')}</strong><p>{text('已保存的模型仍会显示；未知状态不会按可用处理。', 'Saved models remain visible; unknown status is not treated as available.')}</p></div></div>}
          {status !== 'available' && <div className="callout warn"><UiIcon name="warning" /><div><strong>{subscriptionAttention?.title ?? (status === 'needs_credentials' ? text('添加 API Key 后即可使用', 'Add an API key to use this model') : text('这个接入需要处理', 'This connection needs attention'))}</strong><p>{sourceRechecking ? text('正在使用已保存的接入信息重新检查。', 'Checking again with the saved connection details.') : subscriptionAttention?.detail ?? text('其他已连接模型不受影响。', 'Other connected models are unaffected.')}</p></div>{sourceRechecking
            ? <button className="btn" type="button" onClick={cancelRecheck}>{text('取消检查', 'Cancel check')}</button>
            : canReauthorize
              ? <button className="btn" type="button" onClick={() => props.onReauthorize?.(source.source_id)}>{access === 'subscription' ? text('重新检查订阅', 'Check subscription again') : text('重新检查接入', 'Check connection again')}</button>
              : status !== 'needs_credentials' && canRecheck
                ? <button className="btn" type="button" onClick={() => void recheckSource()}>{text('检查接入', 'Check connection')}</button>
                : canManageCredentials && <button className="btn" type="button" onClick={() => setDraft(createDraft(source))}>{text('管理凭据', 'Manage credentials')}</button>}</div>}
          {recheckError && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{recheckError}</span><button className="btn" type="button" disabled={!canRecheck} onClick={() => void recheckSource()}>{text('重试', 'Retry')}</button></div>}
          {recheckNotice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{recheckNotice}</span></div></div>}

          <section className="detail-section">
            <div className="detail-section-head"><h3>{text('接入来源', 'Connection')}</h3>{canManageCredentials && <button className="btn btn-quiet" type="button" onClick={() => setDraft(createDraft(source))}>{text('管理凭据', 'Manage credentials')}</button>}</div>
            <p className="muted">{access === 'subscription'
              ? text('使用本机订阅连接，无需重复填写 API Key。', 'Uses the local subscription; no additional API key is needed.')
              : connectorManaged
                ? text('凭据由本机连接器管理，无需在 HiRoute 中填写 API Key。', 'Credentials are managed by the local connector; no API key is entered in HiRoute.')
              : source.authentication.kind === 'none'
                ? text('此接入不需要用户提供 API Key。', 'No user-provided API key is required.')
                : sourceLabel}</p>
            {source.provenance === 'user_configured' && props.onReconnect && <div className="actions"><button className="btn" type="button" disabled={!props.mutable || submitting} onClick={() => props.onReconnect?.(source)}>{text('重新接入', 'Reconnect')}</button><span className="field-help">{text('地址、协议或能力需要修改时新建接入；旧接入和已有路由不会自动变更。', 'To change endpoint, protocol, or capabilities, create a new connection. The old connection and existing routes stay unchanged.')}</span></div>}
            {source.provenance !== 'connector_owned' && <div className="actions"><button className="btn" type="button" disabled={!props.mutable || submitting || !source.actions.includes(source.state === 'disabled' ? 'enable' : 'disable')} onClick={() => void changeSourceState(source.state === 'disabled')}>{submitting ? text('正在保存…', 'Saving…') : source.state === 'disabled' ? text('启用接入', 'Enable connection') : text('停用接入', 'Disable connection')}</button></div>}
            {stateChangeError && <div className="callout bad" role="alert" data-error-code={stateChangeError}><UiIcon name="warning" /><span>{text('接入状态未修改；请刷新后重试。', 'The connection state was not changed. Refresh and try again.')}</span></div>}
          </section>

          {model && <section className="detail-section">
            <div className="detail-section-head"><h3>{text('费用', 'Cost')}</h3>{canEditPrice && <button className="btn btn-quiet" type="button" onClick={() => props.onEditPrice?.(source, model)}>{text('调整价格', 'Edit pricing')}</button>}</div>
            <ModelPriceSummary language={props.language} model={model} billing={billing} contexts={priceContexts} />
          </section>}

          {model && <section className="detail-section">
            <div className="detail-section-head"><h3>{text('用于这些路由', 'Used in these routes')}</h3></div>
            {usedPlans.length ? usedPlans.map(plan => <button className="list-row v3-linked" type="button" key={plan.agent_plan_id} onClick={() => props.onOpenPlan?.(plan.agent_plan_id)} disabled={!props.onOpenPlan}><UiIcon name="route" /><span className="row-main">{plan.desired.display_name}</span><UiIcon name="chevronRight" /></button>) : <><p className="muted">{text('还没有加入已发布的路由。', 'Not used by a published route yet.')}</p>{props.onCreatePlan && model && <button className="btn" type="button" onClick={() => props.onCreatePlan?.(model.binding_id)}><UiIcon name="plus" />{text('创建智能路由', 'Create routing')}</button>}</>}
          </section>}

        </div> : <div className="empty-state"><div><div className="empty-icon"><UiIcon name="models" /></div><h3>{text('选择一个模型', 'Select a model')}</h3></div></div>}
      </section>
    </div>

    <Dialog
      open={Boolean(draft && source)}
      title={text('管理接入凭据', 'Manage connection credentials')}
      description={sourceLabel}
      closeLabel={text('关闭凭据设置', 'Close credential settings')}
      closeDisabled={submitting}
      onClose={() => { if (!submitting) cancelCredentials(); }}
      footer={<><button className="btn" type="button" disabled={submitting} onClick={cancelCredentials}>{text('取消', 'Cancel')}</button><span className="mc-footer-spacer" /><button className="btn btn-primary" type="button" disabled={submitting || !props.mutable || !draft?.selectedModels.length} onClick={() => void saveCredentials()}>{submitting ? text('正在保存…', 'Saving…') : text('保存凭据', 'Save credentials')}</button></>}
    >
      {source && draft && <CredentialEditor language={props.language} source={source} model={model} draft={draft} errorCode={errorCode} onChange={setDraft} onEditingChange={setCredentialEditing} />}
    </Dialog>
  </section>;
}

function connectionLabel(source: ManagedSource, language: 'zh' | 'en') {
  const product = connectionName(source.connection_identity?.connection_option_id, language, source.connection_identity?.product_label ?? '');
  const access = sourceAccess(source);
  if (product) {
    if (access === 'subscription') return `${product} · OAuth`;
    if (access === 'api') return `${product} · API`;
    return product;
  }
  return source.display_name || (language === 'zh' ? '已保存接入' : 'Saved connection');
}

function StatusBadge({ value, reason, subscription = false, compact = false, language }: { value: ReturnType<typeof modelAvailability>; reason?: string | null; subscription?: boolean; compact?: boolean; language: 'zh' | 'en' }) {
  const zh = language === 'zh';
  const subscriptionLabel = subscription ? ({
    subscription_updating: zh ? '更新中' : 'Updating',
    authentication_required: zh ? '需要登录' : 'Sign-in required',
    model_not_allowed: zh ? '当前不可用' : 'Not allowed',
    runtime_unavailable: zh ? '服务不可用' : 'Service unavailable',
  } as Record<string, string>)[reason ?? ''] : undefined;
  const [tone, label] = ({
    available: ['good', zh ? '接入就绪' : 'Connection ready'],
    cooling_down: ['warn', zh ? '暂时不可用' : 'Cooling'],
    disabled: ['bad', zh ? '已停用' : 'Disabled'],
    needs_credentials: ['warn', subscription ? zh ? '需要登录' : 'Sign-in required' : zh ? '需要 API Key' : 'API key required'],
    unavailable: ['bad', zh ? '无法使用' : 'Unavailable'],
    unknown: ['warn', zh ? '状态待确认' : 'Status unknown'],
  } as const)[value];
  return <span className={`badge ${tone}${compact ? ' no-dot' : ''}`}>{subscriptionLabel ?? label}</span>;
}

function ModelPriceSummary({ language, model, billing, contexts }: {
  language: 'zh' | 'en';
  model: ManagedModel;
  billing: 'free' | 'subscription' | 'paid' | 'unknown';
  contexts: NonNullable<ManagedModel['presentation']>['price_contexts'];
}) {
  const zh = language === 'zh';
  // The compact model detail describes estimated usage cost. Prefer that
  // exact server-advertised context when more than one valuation is exposed.
  const context = contexts.find(item => item.valuation_kind === 'usage_estimate') ?? contexts[0];
  const [display, setDisplay] = useState<PriceDisplay | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [revision, setRevision] = useState(0);
  const target = context ? `${model.binding_id}/${context.currency}/${context.valuation_kind}` : '';
  useEffect(() => {
    if (!context || billing === 'subscription' || billing === 'free') {
      setDisplay(null);
      setLoading(false);
      setFailed(false);
      return;
    }
    let active = true;
    setLoading(true);
    setFailed(false);
    void invoke<PriceDisplay>('effective_price_query', { input: { targets: [{
      query_id: 'model-detail',
      target_locator: { kind: 'binding', binding_id: model.binding_id },
      currency: context.currency,
      valuation_kind: context.valuation_kind,
    }] } }).then(value => {
      if (active) setDisplay(value);
    }).catch(() => {
      if (active) setFailed(true);
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [target, billing, revision]);
  if (billing === 'subscription') return <div className="v3-price"><strong>{zh ? '订阅权益' : 'Subscription'}</strong><span>{zh ? '使用已有订阅权益，实际费用以供应商账单为准' : 'Uses your existing subscription; actual cost follows the provider bill'}</span></div>;
  if (billing === 'free') return <div className="v3-price"><strong>{zh ? '免费' : 'Free'}</strong><span>{zh ? '免费资格来自当前接入，不由手工费率决定' : 'Free eligibility comes from the connection, not a manual rate'}</span></div>;
  if (!context) return <div className="v3-price"><strong>{zh ? '未计价' : 'Unpriced'}</strong><span>{zh ? '没有可核实的目录价格或人工费率' : 'No verified catalog or manual rate is available'}</span></div>;
  if (loading) return <div className="v3-price"><strong>{zh ? '正在读取价格…' : 'Reading pricing…'}</strong><span>{context.currency} · {context.valuation_kind === 'usage_estimate' ? (zh ? '用量估算' : 'Usage estimate') : (zh ? 'API 等价值' : 'API equivalent')}</span></div>;
  const rates = display?.display_rates[0];
  const trim = (value: string | null | undefined) => value ? value.replace(/(?:\.0+|(?:(\.\d*?)0+))$/, '$1') : '—';
  if (!rates || failed) return <div className="v3-price"><strong>{zh ? '未计价' : 'Unpriced'}</strong><span>{failed ? (zh ? '当前价格暂时无法读取' : 'Pricing is temporarily unavailable') : (zh ? '当前口径没有可核实费率' : 'No verified rates are available for this context')}</span>{failed && <button className="btn" type="button" onClick={() => setRevision(current => current + 1)}>{zh ? '重试' : 'Retry'}</button>}</div>;
  const origin = display?.result.items[0]?.quote.origin;
  const currency = context.currency === 'CNY' ? '¥' : context.currency === 'USD' ? '$' : `${context.currency} `;
  return <div className="v3-price"><strong>{currency}{trim(rates[0])} / {currency}{trim(rates[1])}</strong><span>{zh ? '输入 / 输出 · 每 100 万 tokens' : 'Input / output · per million tokens'} · {context.currency} · {origin === 'manual' ? (zh ? '人工费率' : 'Manual rates') : (zh ? '目录或参考费率' : 'Catalog or reference rates')}</span></div>;
}

function CredentialEditor({ language, source, model, draft, errorCode, onChange, onEditingChange }: {
  language: 'zh' | 'en';
  source: ManagedSource;
  model: ManagedModel | undefined;
  draft: ModelDraft;
  errorCode: string;
  onChange(value: ModelDraft): void;
  onEditingChange(value: boolean): void;
}) {
  const zh = language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const [editing, setEditing] = useState<{ kind: 'add' } | { kind: 'replace'; draftId: string } | null>(null);
  const [secretError, setSecretError] = useState('');
  const visibleKeys = draft.keys.filter(key => !key.removed);
  const affected = source.models.map(item => item.display_name).join('、');

  function updateKey(draftId: string, update: (key: ModelDraft['keys'][number]) => ModelDraft['keys'][number]) {
    onChange({ ...draft, keys: draft.keys.map(key => key.draftId === draftId ? update(key) : key) });
  }

  function stopEditing() {
    if (editing?.kind === 'add') onChange({ ...draft, addedSecret: '' });
    if (editing?.kind === 'replace') updateKey(editing.draftId, key => ({ ...key, replacement: '' }));
    setEditing(null);
    setSecretError('');
    onEditingChange(false);
  }

  function acceptSecret() {
    const secret = editing?.kind === 'add'
      ? draft.addedSecret
      : editing?.kind === 'replace'
        ? draft.keys.find(key => key.draftId === editing.draftId)?.replacement ?? ''
        : '';
    if (!secret.trim()) {
      setSecretError(text('请填写 API Key。', 'Enter an API key.'));
      return;
    }
    if (editing?.kind === 'add') onChange(addDraftKey(draft));
    setEditing(null);
    setSecretError('');
    onEditingChange(false);
  }

  function removeKey(draftId: string) {
    const keys = draft.keys.map(key => key.draftId === draftId ? { ...key, removed: true, replacement: '' } : key);
    onChange({ ...draft, keys: [...keys.filter(key => !key.removed), ...keys.filter(key => key.removed)] });
    if (editing?.kind === 'replace' && editing.draftId === draftId) {
      setEditing(null);
      setSecretError('');
      onEditingChange(false);
    }
  }

  return <div>
    <p className="v3-select-intro">{text('同一接入的模型共用这组凭据，按下列顺序尝试。', 'Models on this connection share these credentials, tried in order.')}</p>
    <p className="oc-meta">{text('影响模型：', 'Models: ')}{affected || text('暂无模型', 'None')}</p>
    {visibleKeys.map((key, index) => {
      const availability = summarizeAvailability(source.keys.find(item => item.key_id === key.keyId)?.model_statuses.filter(item => !model || item.binding_id === model.binding_id).map(item => item.availability) ?? [], language);
      return <div className="oc-key-row" key={key.draftId} onKeyDown={event => {
        if (event.altKey && event.key === 'ArrowUp') { event.preventDefault(); onChange(moveDraftKey(draft, key.draftId, -1)); }
        if (event.altKey && event.key === 'ArrowDown') { event.preventDefault(); onChange(moveDraftKey(draft, key.draftId, 1)); }
      }}>
        <span className="candidate-index">{index + 1}</span>
        <div className="row-main"><strong>{text(`Key ${index + 1}`, `Key ${index + 1}`)}</strong><p className="oc-meta">{key.keyId ? availability : text('新增，尚未保存', 'New, not saved')}</p></div>
        <div className="oc-key-actions">
          <button className="icon-btn" type="button" aria-label={text('上移', 'Move up')} disabled={index === 0} onClick={() => onChange(moveDraftKey(draft, key.draftId, -1))}><UiIcon name="arrowUp" /></button>
          <button className="icon-btn" type="button" aria-label={text('下移', 'Move down')} disabled={index === visibleKeys.length - 1} onClick={() => onChange(moveDraftKey(draft, key.draftId, 1))}><UiIcon name="arrowDown" /></button>
          <button className="icon-btn" type="button" aria-label={text('替换 Key', 'Replace key')} onClick={() => { setEditing({ kind: 'replace', draftId: key.draftId }); setSecretError(''); onEditingChange(true); updateKey(key.draftId, item => ({ ...item, replacement: '' })); }}><UiIcon name="refresh" /></button>
          <button className="icon-btn" type="button" aria-label={text('移除', 'Remove')} onClick={() => removeKey(key.draftId)}><UiIcon name="trash" /></button>
        </div>
      </div>;
    })}
    {!visibleKeys.length && <p className="oc-meta">{text('还没有凭据，添加后可用于这个接入。', 'No credentials yet. Add one for this connection.')}</p>}
    {editing ? <div className="field">
      <label className="field-label" htmlFor="credential-secret">{editing.kind === 'replace' ? text('替换 API Key', 'Replace API key') : 'API Key'}</label>
      <input id="credential-secret" data-autofocus className="input" type="password" autoComplete="new-password" aria-invalid={Boolean(secretError)} aria-describedby={secretError ? 'credential-secret-error' : undefined} value={editing.kind === 'add' ? draft.addedSecret : draft.keys.find(key => key.draftId === editing.draftId)?.replacement ?? ''} onChange={event => { setSecretError(''); if (editing.kind === 'add') onChange({ ...draft, addedSecret: event.target.value }); else updateKey(editing.draftId, key => ({ ...key, replacement: event.target.value })); }} />
      {secretError && <p id="credential-secret-error" className="oc-inline-error" role="alert">{secretError}</p>}
      <div className="task-actions"><button className="btn" type="button" onClick={stopEditing}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" type="button" onClick={acceptSecret}>{text('加入待保存修改', 'Add to changes')}</button></div>
    </div> : source.actions.includes('add_key') && <button className="btn" type="button" onClick={() => { setEditing({ kind: 'add' }); setSecretError(''); onEditingChange(true); }}><UiIcon name="plus" />{text('添加 API Key', 'Add API key')}</button>}
    {errorCode && <div className="callout bad" role="alert" data-error-code={errorCode}><UiIcon name="warning" /><span>{errorCode === 'CREDENTIAL_INPUT_PENDING'
      ? text('请先加入或取消正在输入的凭据。', 'Add or cancel the credential currently being entered first.')
      : text('凭据未保存，修改已保留。请读取当前状态后重试。', 'Credentials were not saved. Your changes are retained; refresh the current state and try again.')}</span></div>}
  </div>;
}

function summarizeAvailability(values: string[], language: 'zh' | 'en') {
  const zh = language === 'zh';
  if (values.includes('available')) return zh ? '可用' : 'Available';
  if (values.includes('cooling_down')) return zh ? '暂时冷却；可尝试其他可用凭据' : 'Cooling; other available credentials can be tried';
  if (values.includes('unavailable')) return zh ? '不可用' : 'Unavailable';
  if (values.includes('disabled')) return zh ? '已停用' : 'Disabled';
  return zh ? '状态待确认' : 'Status unknown';
}
