import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog, Disclosure, ProductPage, UiIcon } from '../../ui';
import { requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import { ClassifierProtocolDialog } from '../ClassifierProtocolDialog';
import { randomId } from '../../random-id.ts';
import type { DecisionService } from './types';
import type {
  DecisionConnectionSources, DecisionServiceApplyResult, DecisionServiceChange, DecisionServices, DecisionServiceTestResult,
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
  draftFromRecord, expectedSourceRevisions, isNativeRecord, knownSources, newNativeDraft, recordModel, recordProvider, sourceName,
  testKey, testPresentation, testRecord, type NativeDraft, type SourceRead, type TestRecord,
} from './form-state';
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
  // False after a failed readback: earlier passes cannot be confirmed against current credential state.
  const [factsCurrent, setFactsCurrent] = useState(true);
  const [savedBeforeTest, setSavedBeforeTest] = useState<string | null>(null);
  const [unverifiedSave, setUnverifiedSave] = useState<{ id: string; revision: number } | null>(null);
  const [query, setQuery] = useState(''), [showList, setShowList] = useState(false);
  const [protocol, setProtocol] = useState(false), [menu, setMenu] = useState(false), [deleting, setDeleting] = useState(false);
  const sourceGeneration = useRef(0), testSequence = useRef<Record<string, number>>({});
  const selected = views.find(view => view.service.id === selectedId);
  const editingView = editing && !editing.isNew ? views.find(view => view.service.id === editing.key) ?? null : null;
  useDiscardGuard('decisions', busy, language, async () => false);

  function accept(next: DecisionServiceView[]) {
    setViews(next); setFactsCurrent(true);
    setUnverifiedSave(pending => pending && !next.some(view => view.service.id === pending.id && view.service.revision >= pending.revision) ? pending : null);
    return next;
  }
  async function refresh() {
    try { return accept((await invoke<DecisionServices>('decision_services_v2', {})).services); }
    catch (cause) { setFactsCurrent(false); throw cause; }
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
  useEffect(() => {
    if (!active) return;
    let current = true;
    setLoading(true);
    void invoke<DecisionServices>('decision_services_v2', {}).then(value => {
      if (!current) return;
      accept(value.services);
      setSelectedId(id => value.services.some(view => view.service.id === id) ? id : value.services[0]?.service.id ?? null);
    }).catch(() => { if (current) setError({ message: t('暂时无法读取决策模型，请刷新重试。', 'Unable to read decision models. Refresh and try again.') }); })
      .finally(() => { if (current) setLoading(false); });
    void loadSources();
    return () => { current = false; };
  }, [active]);
  // Returning from API connection management refreshes facts only. Bundles, mode and source stay as authored.
  // A failed read leaves earlier passes unverified; nothing is retested automatically.
  useEffect(() => {
    if (!sourceRefreshKey || !active) return;
    void loadSources();
    void refresh().catch(() => {});
  }, [sourceRefreshKey]);
  function startNew(kind: DecisionEntry) {
    const id = 'decision-' + randomId();
    setEditing(kind === 'custom' ? { key: id, isNew: true, expected: 0, kind: 'custom', record: { ...newDecisionConnection('custom', language), id } }
      : { key: id, isNew: true, expected: 0, kind: 'native', draft: newNativeDraft(id, kind === 'compatible' ? 'compatible' : 'system_one') });
    setShowList(false); setError(null); setNotice(''); setMenu(false);
    if (kind !== 'custom' && sources.status !== 'ready') void loadSources();
  }
  useEffect(() => { if (intent) startNew(intent.kind); }, [intent?.key]);
  async function add(kind: DecisionEntry) {
    if (!await requestEditorReplacement('decisions')) return;
    startNew(kind);
  }
  async function choose(view: DecisionServiceView) {
    if (!await requestEditorReplacement('decisions')) return;
    setEditing(null); setSelectedId(view.service.id); setShowList(false); setError(null); setNotice('');
  }
  function edit(view: DecisionServiceView) {
    const service = structuredClone(view.service);
    setEditing(isNativeRecord(service) ? { key: service.id, isNew: false, expected: service.revision, kind: 'native', draft: draftFromRecord(service) }
      : { key: service.id, isNew: false, expected: service.revision, kind: 'custom', record: service });
    setNotice(''); setError(null);
  }
  async function test(view: DecisionServiceView): Promise<boolean> {
    const key = testKey(view.service), sequence = (testSequence.current[key] ?? 0) + 1;
    testSequence.current[key] = sequence;
    setTesting(key);
    let next: TestRecord;
    try {
      const value = await invoke<DecisionServiceTestResult>('test_decision_service_v2', { input: { id: view.service.id, revision: view.service.revision, expected_configuration_digest: view.configuration_digest } });
      next = testRecord(value, Date.now());
    } catch (cause) { next = { outcome: 'failed', code: planErrorCode(cause), at: Date.now() }; }
    // Credentials may rotate during the call; the pass counts only against state read back afterwards.
    const latest = await refresh().then(all => all.find(item => testKey(item.service) === key), () => undefined);
    // A later test or edit supersedes this result.
    if (testSequence.current[key] !== sequence) return false;
    setTesting(current => current === key ? null : current);
    setTests(current => ({ ...current, [key]: next }));
    return !!latest && testPresentation(next, latest, false) === 'passed';
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
      const saved = latest.find(item => item.service.id === service.id && item.service.revision === expected + 1);
      if (!saved) throw { code: 'DECISION_SERVICE_REFRESH_REQUIRED' };
      onChanged?.();
      setNotice(t('配置已保存。路由选用后需发布才会生效。', 'Configuration saved. Publish a route to use this version.'));
      setSavedBeforeTest(runTest ? testKey(saved.service) : null);
      setBusy(false);
      if (!runTest || await test(saved)) onUse?.(saved.service);
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
      const remaining = await refresh(); setSelectedId(remaining[0]?.service.id ?? null); onChanged?.();
      setNotice(t('已删除。', 'Deleted.'));
    } catch (cause) { setError({ message: t('删除未完成。当前路由、草稿或保留的历史版本仍引用时不能删除；下方列出当前可见引用。', 'Deletion did not complete. Items referenced by current routes, drafts or retained historical versions cannot be deleted. Current visible references are listed below.'), code: planErrorCode(cause) }); }
    finally { setBusy(false); setDeleting(false); }
  }
  const sourceLabel = (view: DecisionServiceView) => view.source ? sourceName(view.source, language) : '';
  const visible = views.filter(view => `${view.service.name} ${providerLabel(view.service.connection, language)} ${recordModel(view.service) ?? ''} ${sourceLabel(view)}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const rowState = (view: DecisionServiceView) => testPresentation(tests[testKey(view.service)], view, testing === testKey(view.service), factsCurrent);
  const rowText = { untested: t('本次尚未测试', 'Not tested this visit'), testing: t('正在测试…', 'Testing…'), passed: t('本次测试通过', 'Test passed this visit'), failed: t('本次测试失败', 'Test failed this visit'), stale: t('上次通过 · 已变化未重测', 'Passed earlier · changed, not retested'), unverified: t('上次通过 · 当前状态未能读取', 'Passed earlier · current state unreadable') };
  const selectedShared = !!selected && selected.service.connection.kind === 'native' && selected.service.connection.authentication.kind === 'shared_source';
  return <ProductPage title={t('模型', 'Models')} subtitle={t('管理已连接的模型与接入', 'Manage connected models and access')} tabs={tabs} flush className="routing-page decision-models-page" actions={
    <div className="decision-add-menu" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setMenu(false); }} onKeyDown={event => { if (event.key === 'Escape') setMenu(false); }}>
      <button className="btn btn-primary" disabled={!mutable || busy} onClick={() => void add('system_one')}><UiIcon name="plus" />{t('添加决策模型', 'Add decision model')}</button><button className="btn" disabled={!mutable || busy} aria-label={t('更多添加方式', 'More connection types')} aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(!menu)}><UiIcon name="chevron" /></button>
      {menu && <div className="decision-menu" role="menu"><button role="menuitem" onClick={() => void add('custom')}>{t('接入自定义扩展', 'Connect custom extension')}</button><button role="menuitem" onClick={() => void add('compatible')}>{t('其他兼容接入', 'Other compatible connection')}</button></div>}
    </div>
  }>
    {onReturn && <div className="decision-return"><button className="btn btn-quiet" disabled={busy} onClick={onReturn}><UiIcon name="arrowLeft" />{returnLabel ?? t('返回路由', 'Back to route')}</button><span>{returnLabel ? t('草稿已保留，保存后可继续配置。', 'Your draft is retained. Save to continue.') : t('路由草稿已保留，保存后可继续配置。', 'Your route draft is retained. Save to continue.')}</span></div>}
    {error && <div className="callout bad decision-notice" role="alert"><div><p>{error.message}</p>{error.code && <Disclosure label={t('诊断详情', 'Diagnostic details')} language={language}><code>{error.code}</code></Disclosure>}</div><button className="btn" disabled={busy} onClick={() => void Promise.all([refresh(), loadSources()]).then(() => setError(null)).catch(() => {})}>{t('刷新', 'Refresh')}</button></div>}
    {notice && <div className="decision-notice field-help" role="status">{notice}</div>}
    <div className={'split-view' + (showList || (!editing && !selected) ? ' show-list' : '')}>
      <aside className="master-pane"><div className="decision-search"><UiIcon name="search" /><input className="input" aria-label={t('搜索决策模型与扩展', 'Search decision models and extensions')} placeholder={t('搜索名称、供应商或模型', 'Search name, provider or model')} value={query} onChange={event => setQuery(event.target.value)} /></div><nav className="master-list native-list" aria-label={t('决策模型与扩展', 'Decision models and extensions')}>
        {visible.map(view => { const service = view.service, state = rowState(view); return <button className={'list-row' + (!editing && selected?.service.id === service.id ? ' active' : '')} key={service.id} disabled={busy} aria-current={!editing && selected?.service.id === service.id ? 'page' : undefined} onClick={() => void choose(view)}><DecisionIcon provider={recordProvider(service)} language={language} /><span className="row-main"><span className="row-title">{service.name}</span><span className="row-meta">{providerLabel(service.connection, language)}{view.source ? ` · ${sourceLabel(view)}` : ''}</span>{recordModel(service) && <span className="row-meta">{recordModel(service)}</span>}<span className={'decision-test-state' + (state === 'passed' ? ' passed' : state === 'failed' ? ' failed' : state === 'stale' || state === 'unverified' ? ' stale' : '')}>{rowText[state]}</span></span><UiIcon name="chevronRight" /></button>; })}
        {!visible.length && <div className="empty-state"><p>{loading ? t('正在读取…', 'Loading…') : query ? t('没有匹配的连接', 'No matching connections') : t('添加决策模型，为路由判断任务分支。', 'Add a decision model to select task branches.')}</p></div>}
      </nav></aside>
      <section className="detail-pane"><div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => setShowList(true)}><UiIcon name="arrowLeft" />{t('返回列表', 'Back to list')}</button></div>
        {editing ? editing.kind === 'native'
          ? <DecisionModelForm key={editing.key} initial={editing.draft} saved={!editing.isNew} view={editingView} revision={editing.isNew ? 0 : editing.expected} sources={sources} language={language} mutable={mutable && !busy} onRetrySources={() => void loadSources()} onManageSource={onManageSource} onSave={save} onCancel={() => { setEditing(null); setError(null); }} />
          : <DecisionConnectionForm key={editing.key} initial={editing.record} saved={!editing.isNew} language={language} mutable={mutable && !busy} onSave={(service, secret, runTest) => save(service, secret, runTest, [])} onCancel={() => { setEditing(null); setError(null); }} onProtocol={() => setProtocol(true)} />
          : selected ? <DecisionConnectionDetail view={selected} language={language} mutable={mutable} busy={busy || selected.service.id === unverifiedSave?.id} state={rowState(selected)} result={tests[testKey(selected.service)]} savedBeforeTest={savedBeforeTest === testKey(selected.service)} plans={plans} drafts={drafts} onEdit={() => edit(selected)} onTest={() => void testSelected()} onRefresh={() => void Promise.all([refresh(), loadSources()]).catch(() => {})} onDelete={() => setDeleting(true)} onProtocol={() => setProtocol(true)} onOpenRoute={onOpenRoute} onUse={onUse ? () => onUse(selected.service) : undefined} onManageSource={onManageSource} />
          : <div className="empty-state"><p>{t('选择连接查看详情，或添加决策模型。', 'Select a connection or add a decision model.')}</p></div>}
      </section>
    </div>
    <ClassifierProtocolDialog open={protocol} language={language} onClose={() => setProtocol(false)} />
    <Dialog open={deleting} title={t('删除这个配置？', 'Delete this configuration?')} description={selected?.service.name} closeLabel={t('关闭', 'Close')} onClose={() => { if (!busy) setDeleting(false); }} footer={<><button className="btn" disabled={busy} onClick={() => setDeleting(false)}>{t('取消', 'Cancel')}</button><button className="btn btn-danger" disabled={busy} onClick={() => void remove()}>{t('删除', 'Delete')}</button></>}><p>{t('被路由、草稿或保留的历史版本引用的配置无法删除。', 'A configuration referenced by routes, drafts or retained historical versions cannot be deleted.')}</p>{selectedShared && <p>{t('API 接入及其 Key 会保留。', 'The API connection and its keys are retained.')}</p>}</Dialog>
  </ProductPage>;
}
