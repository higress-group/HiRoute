import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog, Disclosure, ProductPage, UiIcon } from '../../ui';
import { requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import { ClassifierProtocolDialog } from '../ClassifierProtocolDialog';
import { randomId } from '../../random-id.ts';
import type { DecisionService } from './types';
import type {
  DecisionConnectionSources, DecisionServiceApplyResult, DecisionServiceChange, DecisionServices, DecisionServiceNetworkTestResult,
  DecisionServiceV2, DecisionServiceView,
} from './contracts';
import type { Draft, Plan } from '../../plan-editor';
import { planErrorCode } from '../../plan-editor-errors';
import { DecisionConnectionForm } from './DecisionConnectionForm';
import { DecisionModelForm, type NativeSaveOutcome } from './DecisionModelForm';
import { DecisionConnectionDetail } from './DecisionConnectionDetail';
import { DecisionIcon } from './DecisionIcon';
import { decisionFailureHelp, newDecisionConnection, providerLabel, type DecisionEntry, type DecisionIntent } from './presentation';
import {
  draftFromRecord, expectedSourceRevisions, isNativeRecord, knownSources, newNativeDraft, recordModel, recordProvider, recordSharedSource, sourceName,
  testKey, testPresentation, networkTestRecord, type NativeDraft, type SourceRead, type TestRecord,
} from './form-state';
import type { NativeNetworkRoute } from '../model-connections/native-network';
import './decision-models.css';

type Editing = { key: string; isNew: boolean; expected: number } & ({ kind: 'native'; draft: NativeDraft } | { kind: 'custom'; record: DecisionService });

export function DecisionServicesPage({ language, active, mutable, tabs, intent, plans = [], drafts = [], sourceRefreshKey = 0, onOpenRoute, onReturn, returnLabel, onUse, onChanged, onManageSource }: {
  language: 'zh' | 'en'; active: boolean; mutable: boolean; tabs?: ReactNode; intent?: DecisionIntent | null;
  plans?: Plan[]; drafts?: Draft[]; onOpenRoute?(plan?: Plan, draft?: Draft): void;
  /** Changes after returning from API connection management; refreshes state without touching the draft. */
  sourceRefreshKey?: number;
  onReturn?(): void; returnLabel?: string; onUse?(service: DecisionService): void; onChanged?(): void;
  onManageSource?(sourceId: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [views, setViews] = useState<DecisionServiceView[]>([]);
  const [sources, setSources] = useState<SourceRead>({ status: 'loading' });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true);
  const [error, setError] = useState<{ message: string; code?: string } | null>(null), [notice, setNotice] = useState('');
  const [tests, setTests] = useState<Record<string, TestRecord>>({});
  const [testing, setTesting] = useState<string | null>(null);
  const [networks, setNetworks] = useState<Record<string, NativeNetworkRoute | null>>({});
  const networkSequence = useRef<Record<string, number>>({});
  const networkFacts = useRef<Record<string, NativeNetworkRoute | null>>({});
  const [networkDirty, setNetworkDirty] = useState(false);
  const handledIntent = useRef<string | null>(null);
  // False after a failed readback: earlier passes cannot be confirmed against current credential state.
  const [factsCurrent, setFactsCurrent] = useState(true);
  const [savedBeforeTest, setSavedBeforeTest] = useState<string | null>(null);
  const [unverifiedSave, setUnverifiedSave] = useState<{ id: string; revision: number } | null>(null);
  const [query, setQuery] = useState(''), [showList, setShowList] = useState(false);
  const [protocol, setProtocol] = useState(false), [menu, setMenu] = useState(false), [deleting, setDeleting] = useState(false);
  const sourceGeneration = useRef(0), serviceGeneration = useRef(0), testSequence = useRef<Record<string, number>>({});
  const useGeneration = useRef(0);
  function networkChanged(dirty: boolean) {
    if (dirty) useGeneration.current++;
    setNetworkDirty(dirty);
  }
  const selected = views.find(view => view.service.id === selectedId);
  const editingView = editing && !editing.isNew ? views.find(view => view.service.id === editing.key) ?? null : null;
  useDiscardGuard('decisions', busy || networkDirty, language, async () => false);
  // A saved test may finish after this caller or page has been replaced.
  useLayoutEffect(() => () => { useGeneration.current++; }, [active, intent?.key]);

  function accept(next: DecisionServiceView[]) {
    setViews(next); setFactsCurrent(true);
    setUnverifiedSave(pending => pending && !next.some(view => view.service.id === pending.id && view.service.revision >= pending.revision) ? pending : null);
    return next;
  }
  async function readNetwork(view: DecisionServiceView): Promise<NativeNetworkRoute | null> {
    if (!recordSharedSource(view.service)) return null;
    const key = testKey(view.service), sequence = (networkSequence.current[key] ?? 0) + 1;
    networkSequence.current[key] = sequence;
    networkFacts.current[key] = null;
    setNetworks(current => ({ ...current, [key]: null }));
    const route = await invoke<NativeNetworkRoute>('preview_native_network_route', { input: { target: { kind: 'decision', id: view.service.id, revision: view.service.revision }, policy: null } }).catch(() => null);
    if (networkSequence.current[key] !== sequence) return null;
    networkFacts.current[key] = route;
    setNetworks(current => ({ ...current, [key]: route }));
    return route;
  }
  async function refresh(networkKey?: string): Promise<DecisionServiceView[] | null> {
    const generation = ++serviceGeneration.current;
    setFactsCurrent(false);
    try {
      const value = await invoke<DecisionServices>('decision_services_v2', {});
      // A slower earlier read must not replace state confirmed after a Key change or a test.
      if (generation !== serviceGeneration.current) return null;
      const next = accept(value.services);
      const selected = next.find(view => networkKey ? testKey(view.service) === networkKey : view.service.id === selectedId);
      if (selected) await readNetwork(selected);
      return generation === serviceGeneration.current ? next : null;
    } catch (cause) {
      if (generation !== serviceGeneration.current) return null;
      setFactsCurrent(false); throw cause;
    }
  }
  /** Only the newest read applies; a failed read keeps earlier summaries as unavailable. */
  async function loadSources() {
    const generation = ++sourceGeneration.current;
    setSources(current => ({ status: 'loading', previous: knownSources(current).length ? current.status === 'ready' ? current.sources : current.previous : undefined }));
    try {
      const value = await invoke<DecisionConnectionSources>('decision_connection_sources', {});
      if (generation === sourceGeneration.current) setSources({ status: 'ready', sources: value.sources });
    } catch (cause) {
      if (generation === sourceGeneration.current) setSources(current => ({ status: 'error', code: planErrorCode(cause), previous: current.status === 'loading' ? current.previous : undefined }));
    }
  }
  // Activation and a management return share one read. Keep draft bundles intact.
  useEffect(() => {
    if (!active) return;
    let current = true;
    setLoading(true);
    void refresh().then(next => {
      if (!current || !next) return;
      setSelectedId(id => next.some(view => view.service.id === id) ? id : next[0]?.service.id ?? null);
    }).catch(() => { if (current) setError({ message: t('暂时无法读取决策模型，请刷新重试。', 'Unable to read decision models. Refresh and try again.') }); })
      .finally(() => { if (current) setLoading(false); });
    void loadSources();
    return () => { current = false; serviceGeneration.current++; sourceGeneration.current++; };
  }, [active, sourceRefreshKey]);
  useEffect(() => {
    if (active && selected) void readNetwork(selected);
  }, [active, selected?.service.id, selected?.service.revision]);
  function startNew(kind: DecisionEntry) {
    useGeneration.current++;
    const id = 'decision-' + randomId();
    setEditing(kind === 'custom' ? { key: id, isNew: true, expected: 0, kind: 'custom', record: { ...newDecisionConnection('custom', language), id } }
      : { key: id, isNew: true, expected: 0, kind: 'native', draft: newNativeDraft(id, kind === 'compatible' ? 'compatible' : 'system_one') });
    setShowList(false); setError(null); setNotice(''); setMenu(false);
    if (kind !== 'custom' && sources.status !== 'ready') void loadSources();
  }
  useEffect(() => {
    if (!intent || busy || networkDirty || handledIntent.current === intent.key) return;
    let current = true;
    void requestEditorReplacement('decisions').then(accepted => {
      if (!current) return;
      handledIntent.current = intent.key;
      if (accepted) startNew(intent.kind);
    });
    return () => { current = false; };
  }, [intent?.key, busy, networkDirty]);
  async function add(kind: DecisionEntry) {
    if (!await requestEditorReplacement('decisions')) return;
    startNew(kind);
  }
  async function choose(view: DecisionServiceView) {
    if (!await requestEditorReplacement('decisions')) return;
    useGeneration.current++;
    setEditing(null); setSelectedId(view.service.id); setShowList(false); setError(null); setNotice('');
  }
  function edit(view: DecisionServiceView) {
    useGeneration.current++;
    const service = structuredClone(view.service);
    setEditing(isNativeRecord(service) ? { key: service.id, isNew: false, expected: service.revision, kind: 'native', draft: draftFromRecord(service) }
      : { key: service.id, isNew: false, expected: service.revision, kind: 'custom', record: service });
    setNotice(''); setError(null);
  }
  function cancelEdit() {
    useGeneration.current++;
    setEditing(null); setError(null);
  }
  function manageSource(id: string) {
    useGeneration.current++;
    onManageSource?.(id);
  }
  function returnToCaller() {
    if (!onReturn || busy || networkDirty) return;
    useGeneration.current++;
    // The caller owns navigation/discard approval; do not ask the same guard twice.
    onReturn();
  }
  async function test(view: DecisionServiceView): Promise<boolean> {
    const key = testKey(view.service), sequence = (testSequence.current[key] ?? 0) + 1;
    testSequence.current[key] = sequence;
    setTesting(key);
    let next: TestRecord;
    try {
      const value = await invoke<DecisionServiceNetworkTestResult>('test_decision_service_v3', { input: { id: view.service.id, revision: view.service.revision, expected_configuration_digest: view.configuration_digest } });
      next = networkTestRecord(value, Date.now());
    } catch (cause) { next = { outcome: 'failed', code: planErrorCode(cause), at: Date.now() }; }
    // Credentials may rotate during the call; the pass counts only against state read back afterwards.
    const latest = await refresh(key).then(all => all?.find(item => testKey(item.service) === key), () => undefined);
    const network = networkFacts.current[key] ?? null;
    // Only another test of this saved revision supersedes its diagnostic result.
    if (testSequence.current[key] !== sequence) return false;
    setTesting(current => current === key ? null : current);
    setTests(current => ({ ...current, [key]: next }));
    return !!latest && testPresentation(next, latest, false, true, network) === 'passed';
  }
  async function reconcile(id: string) {
    await loadSources();
    const latest = await refresh().catch(() => null);
    if (!latest || !editing || editing.key !== id) return false;
    const view = latest.find(item => item.service.id === id);
    const expected = view?.service.revision ?? 0;
    const rebased = expected !== (editing.isNew ? 0 : editing.expected);
    setEditing(current => current && current.key === id ? { ...current, expected, isNew: !view } : current);
    return rebased;
  }
  async function save(service: DecisionServiceV2, secret: string | null, runTest: boolean, sourceIds: string[]): Promise<NativeSaveOutcome> {
    if (!editing || busy) return 'failed';
    const useAtStart = useGeneration.current;
    const expected = editing.isNew ? 0 : editing.expected;
    const expected_source_revisions = expectedSourceRevisions(sourceIds, sources, editingView);
    // A selected source without an observed revision cannot be registered safely.
    if (expected_source_revisions.length !== sourceIds.length) {
      setError({ message: t('暂时无法读取所选 API 接入的状态，当前输入已保留。请刷新后再保存。', 'The selected API connection could not be read. Your input is retained. Refresh, then save again.'), code: 'DECISION_SOURCE_UNAVAILABLE' });
      return 'failed';
    }
    const change: DecisionServiceChange = {
      id: service.id, expected_revision: expected, service: { ...service, revision: expected + 1 }, expected_source_revisions,
    };
    let committed = false;
    setBusy(true); setError(null); setNotice('');
    try {
      const value = await invoke<DecisionServiceApplyResult>('save_decision_service_v2', { input: { change, secret } });
      if (value.state !== 'succeeded') throw { code: 'DECISION_SERVICE_' + value.state.toUpperCase() };
      committed = true;
      // An acknowledged write never leaves a resubmittable credential draft, even when readback fails.
      setEditing(null); setSelectedId(service.id);
      setUnverifiedSave({ id: service.id, revision: expected + 1 });
      const latest = await refresh();
      const saved = latest?.find(item => item.service.id === service.id && item.service.revision === expected + 1);
      if (!saved) throw { code: 'DECISION_SERVICE_REFRESH_REQUIRED' };
      onChanged?.();
      setNotice(t('配置已保存。路由选用后需发布才会生效。', 'Configuration saved. Publish a route to use this version.'));
      setSavedBeforeTest(runTest ? testKey(saved.service) : null);
      setBusy(false);
      const passed = !runTest || await test(saved);
      if (passed && useGeneration.current === useAtStart) onUse?.(saved.service);
      return 'committed';
    } catch (cause) {
      const code = planErrorCode(cause);
      if (committed) {
        setError({ message: t('已保存 — 需要刷新。未能核实新版本，尚未执行测试；请刷新后再继续。', 'Saved — refresh required. The new version could not be verified and no test was run. Refresh before continuing.'), code });
        return 'committed';
      }
      // Conflicts and uncertain outcomes refresh state while keeping non-secret edits; nothing is resubmitted.
      const rebased = await reconcile(service.id);
      const conflict = code.endsWith('REVISION_CONFLICT') || rebased;
      setError({ message: conflict
        ? t('配置或 API 接入已被更新。状态已刷新，非密钥修改已保留；请核对后再次保存。', 'The configuration or API connection changed. State was refreshed and non-secret edits were kept; review, then save again.')
        : t('保存未完成，当前输入已保留。', 'Save did not complete. Your input is retained.') + ' ' + decisionFailureHelp(code, language), code });
      return conflict ? 'conflict' : 'failed';
    } finally { setBusy(false); }
  }
  async function testSelected() {
    if (!selected || busy) return;
    setSavedBeforeTest(null); setNotice('');
    await test(selected);
  }
  async function remove() {
    if (!selected || busy) return;
    setBusy(true); setError(null);
    try {
      const change: DecisionServiceChange = { id: selected.service.id, expected_revision: selected.service.revision, service: null,
        expected_source_revisions: expectedSourceRevisions(selected.related_sources.map(source => source.source_id), sources, selected) };
      const value = await invoke<DecisionServiceApplyResult>('save_decision_service_v2', { input: { change, secret: null } });
      if (value.state !== 'succeeded') throw { code: 'DECISION_SERVICE_' + value.state.toUpperCase() };
      const remaining = await refresh(); if (remaining) setSelectedId(remaining[0]?.service.id ?? null); onChanged?.();
      setNotice(t('已删除。', 'Deleted.'));
    } catch (cause) { setError({ message: t('删除未完成。当前路由、草稿或保留的历史版本仍引用时不能删除；下方列出当前可见引用。', 'Deletion did not complete. Items referenced by current routes, drafts or retained historical versions cannot be deleted. Current visible references are listed below.'), code: planErrorCode(cause) }); }
    finally { setBusy(false); setDeleting(false); }
  }
  const sourceLabel = (view: DecisionServiceView) => view.source ? sourceName(view.source, language) : '';
  const visible = views.filter(view => `${view.service.name} ${providerLabel(view.service.connection, language)} ${recordModel(view.service) ?? ''} ${sourceLabel(view)}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const rowState = (view: DecisionServiceView) => testPresentation(tests[testKey(view.service)], view, testing === testKey(view.service), factsCurrent, networks[testKey(view.service)]);
  const rowText = { untested: t('本次尚未测试', 'Not tested this visit'), testing: t('正在测试…', 'Testing…'), passed: t('本次测试通过', 'Test passed this visit'), failed: t('本次测试失败', 'Test failed this visit'), stale: t('上次通过 · 已变化未重测', 'Passed earlier · changed, not retested'), unverified: t('上次通过 · 当前状态未能读取', 'Passed earlier · current state unreadable') };
  const selectedShared = !!selected && selected.service.connection.kind === 'native' && selected.service.connection.authentication.kind === 'shared_source';
  return <ProductPage title={t('模型', 'Models')} subtitle={t('管理已连接的模型与接入', 'Manage connected models and access')} tabs={tabs} flush className="routing-page decision-models-page" actions={
    <div className="decision-add-menu" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setMenu(false); }} onKeyDown={event => { if (event.key === 'Escape') setMenu(false); }}>
      <button className="btn btn-primary" disabled={!mutable || busy || networkDirty} onClick={() => void add('system_one')}><UiIcon name="plus" />{t('添加决策模型', 'Add decision model')}</button><button className="btn" disabled={!mutable || busy || networkDirty} aria-label={t('更多添加方式', 'More connection types')} aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(!menu)}><UiIcon name="chevron" /></button>
      {menu && <div className="decision-menu" role="menu"><button role="menuitem" disabled={networkDirty || busy} onClick={() => void add('custom')}>{t('接入自定义扩展', 'Connect custom extension')}</button><button role="menuitem" disabled={networkDirty || busy} onClick={() => void add('compatible')}>{t('其他兼容接入', 'Other compatible connection')}</button></div>}
    </div>
  }>
    {onReturn && <div className="decision-return"><button className="btn btn-quiet" disabled={busy || networkDirty} onClick={() => void returnToCaller()}><UiIcon name="arrowLeft" />{returnLabel ?? t('返回路由', 'Back to route')}</button><span>{returnLabel ? t('草稿已保留，保存后可继续配置。', 'Your draft is retained. Save to continue.') : t('路由草稿已保留，保存后可继续配置。', 'Your route draft is retained. Save to continue.')}</span></div>}
    {error && <div className="callout bad decision-notice" role="alert"><div><p>{error.message}</p>{error.code && <Disclosure label={t('诊断详情', 'Diagnostic details')} language={language}><code>{error.code}</code></Disclosure>}</div><button className="btn" disabled={busy} onClick={() => void Promise.all([refresh(), loadSources()]).then(() => setError(null)).catch(() => {})}>{t('刷新', 'Refresh')}</button></div>}
    {notice && <div className="decision-notice field-help" role="status">{notice}</div>}
    {networkDirty && <p className="decision-notice field-help" role="status">{t('请先保存或放弃当前网络修改，再切换或离开；保存结果未确认时，请刷新原操作。', 'Save or discard the current network changes before switching or leaving. If a save is unconfirmed, refresh its original operation.')}</p>}
    <div className={'split-view' + (showList || (!editing && !selected) ? ' show-list' : '')}>
      <aside className="master-pane"><div className="decision-search"><UiIcon name="search" /><input className="input" aria-label={t('搜索决策模型与扩展', 'Search decision models and extensions')} placeholder={t('搜索名称、供应商或模型', 'Search name, provider or model')} value={query} onChange={event => setQuery(event.target.value)} /></div><nav className="master-list native-list" aria-label={t('决策模型与扩展', 'Decision models and extensions')}>
        {visible.map(view => { const service = view.service, state = rowState(view); return <button className={'list-row' + (!editing && selected?.service.id === service.id ? ' active' : '')} key={service.id} disabled={busy || networkDirty} aria-current={!editing && selected?.service.id === service.id ? 'page' : undefined} onClick={() => void choose(view)}><DecisionIcon provider={recordProvider(service)} language={language} /><span className="row-main"><span className="row-title">{service.name}</span><span className="row-meta">{providerLabel(service.connection, language)}{view.source ? ` · ${sourceLabel(view)}` : ''}</span>{recordModel(service) && <span className="row-meta">{recordModel(service)}</span>}<span className={'decision-test-state' + (state === 'passed' ? ' passed' : state === 'failed' ? ' failed' : state === 'stale' || state === 'unverified' ? ' stale' : '')}>{rowText[state]}</span></span><UiIcon name="chevronRight" /></button>; })}
        {!visible.length && <div className="empty-state"><p>{loading ? t('正在读取…', 'Loading…') : query ? t('没有匹配的连接', 'No matching connections') : t('添加决策模型，为路由判断任务分支。', 'Add a decision model to select task branches.')}</p></div>}
      </nav></aside>
      <section className="detail-pane"><div className="oc-model-back"><button className="btn btn-quiet" type="button" disabled={networkDirty} onClick={() => { useGeneration.current++; setShowList(true); }}><UiIcon name="arrowLeft" />{t('返回列表', 'Back to list')}</button></div>
        {editing ? editing.kind === 'native'
          ? <DecisionModelForm key={editing.key} initial={editing.draft} saved={!editing.isNew} view={editingView} revision={editing.isNew ? 0 : editing.expected} sources={sources} language={language} mutable={mutable && !busy} onNetworkDirty={networkChanged} onRetrySources={async () => { await Promise.all([loadSources(), refresh()]); }} onManageSource={onManageSource ? manageSource : undefined} onSave={save} onCancel={cancelEdit} />
          : <DecisionConnectionForm key={editing.key} initial={editing.record} saved={!editing.isNew} language={language} mutable={mutable && !busy} onSave={(service, secret, runTest) => save(service, secret, runTest, [])} onCancel={cancelEdit} onProtocol={() => setProtocol(true)} />
          : selected ? <DecisionConnectionDetail key={testKey(selected.service)} view={selected} network={networks[testKey(selected.service)] ?? null} networkDirty={networkDirty} onNetworkDirty={networkChanged} onNetworkSaved={async () => { await Promise.all([refresh(), loadSources()]); }} language={language} mutable={mutable} busy={busy || selected.service.id === unverifiedSave?.id} state={rowState(selected)} result={tests[testKey(selected.service)]} savedBeforeTest={savedBeforeTest === testKey(selected.service)} plans={plans} drafts={drafts} onEdit={() => edit(selected)} onTest={() => void testSelected()} onRefresh={() => void Promise.all([refresh(), loadSources()]).catch(() => {})} onDelete={() => setDeleting(true)} onProtocol={() => setProtocol(true)} onOpenRoute={onOpenRoute} onUse={onUse ? () => { useGeneration.current++; onUse(selected.service); } : undefined} onManageSource={onManageSource ? manageSource : undefined} />
          : <div className="empty-state"><p>{t('选择连接查看详情，或添加决策模型。', 'Select a connection or add a decision model.')}</p></div>}
      </section>
    </div>
    <ClassifierProtocolDialog open={protocol} language={language} onClose={() => setProtocol(false)} />
    <Dialog open={deleting} title={t('删除这个配置？', 'Delete this configuration?')} description={selected?.service.name} closeLabel={t('关闭', 'Close')} onClose={() => { if (!busy) setDeleting(false); }} footer={<><button className="btn" disabled={busy} onClick={() => setDeleting(false)}>{t('取消', 'Cancel')}</button><button className="btn btn-danger" disabled={busy} onClick={() => void remove()}>{t('删除', 'Delete')}</button></>}><p>{t('被路由、草稿或保留的历史版本引用的配置无法删除。', 'A configuration referenced by routes, drafts or retained historical versions cannot be deleted.')}</p>{selectedShared && <p>{t('API 接入及其 Key 会保留。', 'The API connection and its keys are retained.')}</p>}</Dialog>
  </ProductPage>;
}
