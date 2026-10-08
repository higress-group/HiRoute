import { useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Dialog, Disclosure, ProductPage, UiIcon } from '../../ui';
import { requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import { ClassifierProtocolDialog } from '../ClassifierProtocolDialog';
import type { DecisionService } from './types';
import type { Draft, Plan } from '../../plan-editor';
import { planErrorCode } from '../../plan-editor-errors';
import { DecisionConnectionForm } from './DecisionConnectionForm';
import { DecisionConnectionDetail } from './DecisionConnectionDetail';
import { DecisionIcon } from './DecisionIcon';
import { decisionFailureHelp, newDecisionConnection, providerLabel, testKey, type DecisionEntry, type DecisionIntent, type DecisionTest } from './presentation';
import './decision-models.css';

export function DecisionServicesPage({ language, active, mutable, tabs, intent, plans = [], drafts = [], onOpenRoute, onReturn, onUse, onChanged }: {
  language: 'zh' | 'en'; active: boolean; mutable: boolean; tabs?: ReactNode; intent?: DecisionIntent | null;
  plans?: Plan[]; drafts?: Draft[]; onOpenRoute?(plan?: Plan, draft?: Draft): void;
  onReturn?(): void; onUse?(service: DecisionService): void; onChanged?(): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [services, setServices] = useState<DecisionService[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ service: DecisionService; isNew: boolean } | null>(null);
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true);
  const [error, setError] = useState<{ message: string; code?: string } | null>(null), [notice, setNotice] = useState('');
  const [tests, setTests] = useState<Record<string, DecisionTest>>({});
  const [savedBeforeTest, setSavedBeforeTest] = useState<string | null>(null);
  const [unverifiedSave, setUnverifiedSave] = useState<{ id: string; revision: number } | null>(null);
  const [query, setQuery] = useState(''), [showList, setShowList] = useState(false);
  const [protocol, setProtocol] = useState(false), [menu, setMenu] = useState(false), [deleting, setDeleting] = useState(false);
  const selected = services.find(service => service.id === selectedId);
  useDiscardGuard('decisions', busy, language, async () => false);
  async function refresh() {
    const value = await invoke<{ services: DecisionService[] }>('decision_services');
    setServices(value.services);
    setUnverifiedSave(pending => pending && !value.services.some(service => service.id === pending.id && service.revision >= pending.revision) ? pending : null);
    return value.services;
  }
  useEffect(() => {
    if (!active) return;
    let current = true;
    setLoading(true);
    void invoke<{ services: DecisionService[] }>('decision_services').then(value => {
      if (!current) return;
      setServices(value.services);
      setUnverifiedSave(pending => pending && !value.services.some(service => service.id === pending.id && service.revision >= pending.revision) ? pending : null);
      setSelectedId(id => value.services.some(service => service.id === id) ? id : value.services[0]?.id ?? null);
    }).catch(() => { if (current) setError({ message: t('暂时无法读取决策模型，请刷新重试。', 'Unable to read decision models. Refresh and try again.') }); })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [active]);
  useEffect(() => {
    if (!intent) return;
    setEditing({ service: newDecisionConnection(intent.kind, language), isNew: true });
    setShowList(false); setError(null); setNotice(''); setMenu(false);
  }, [intent?.key]);
  async function add(kind: DecisionEntry) {
    if (!await requestEditorReplacement('decisions')) return;
    setEditing({ service: newDecisionConnection(kind, language), isNew: true });
    setShowList(false); setMenu(false); setError(null); setNotice('');
  }
  async function choose(service: DecisionService) {
    if (!await requestEditorReplacement('decisions')) return;
    setEditing(null); setSelectedId(service.id); setShowList(false); setError(null); setNotice('');
  }
  async function test(service: DecisionService): Promise<boolean> {
    let next: DecisionTest;
    try {
      const value = await invoke<{ outcome: string; failure_code?: string; duration_millis?: number }>('test_classifier_decision', { input: { classifier: { kind: 'decision_service', service } } });
      next = { passed: value.outcome === 'passed', code: value.outcome === 'passed' ? undefined : planErrorCode(value.failure_code ?? 'DECISION_TEST_FAILED'), duration: value.duration_millis, at: Date.now() };
    } catch (cause) { next = { passed: false, code: planErrorCode(cause), at: Date.now() }; }
    setTests(current => ({ ...current, [testKey(service)]: next }));
    return next.passed;
  }
  async function save(service: DecisionService, secret: string | null, runTest: boolean) {
    if (!editing || busy) return;
    const expected = editing.isNew ? 0 : editing.service.revision;
    let committed = false;
    setBusy(true); setError(null); setNotice('');
    try {
      const value = await invoke<{ state: string }>('save_decision_service', { input: { change: { id: service.id, expected_revision: expected, service: { ...service, revision: expected + 1 } }, secret } });
      if (value.state !== 'succeeded') throw { code: 'DECISION_SERVICE_' + value.state.toUpperCase() };
      committed = true;
      // An acknowledged write must never leave a resubmittable credential draft
      // when reading back the protected revision fails.
      setEditing(null); setSelectedId(service.id);
      setUnverifiedSave({ id: service.id, revision: expected + 1 });
      const latest = await refresh();
      const saved = latest.find(item => item.id === service.id && item.revision === expected + 1);
      if (!saved) throw { code: 'DECISION_SERVICE_REFRESH_REQUIRED' };
      setEditing(null); setSelectedId(saved.id); onChanged?.();
      setNotice(t('配置已保存。路由选用后需发布才会生效。', 'Configuration saved. Publish a route to use this version.'));
      setSavedBeforeTest(runTest ? testKey(saved) : null);
      if (!runTest || await test(saved)) onUse?.(saved);
    } catch (cause) {
      const code = planErrorCode(cause);
      setError({ message: committed ? t('配置已保存，但未能核实新版本。请刷新核对后再继续，尚未执行测试。', 'Configuration saved, but the new version could not be verified. Refresh before continuing; no test was run.') : t('保存未完成，当前输入已保留。', 'Save did not complete. Your input is retained.') + ' ' + decisionFailureHelp(code, language), code });
    } finally { setBusy(false); }
  }
  async function testSelected() {
    if (!selected || busy) return;
    setBusy(true); setSavedBeforeTest(null); setNotice('');
    try { await test(structuredClone(selected)); } finally { setBusy(false); }
  }
  async function remove() {
    if (!selected || busy) return;
    setBusy(true); setError(null);
    try {
      const value = await invoke<{ state: string }>('save_decision_service', { input: { change: { id: selected.id, expected_revision: selected.revision, service: null }, secret: null } });
      if (value.state !== 'succeeded') throw { code: 'DECISION_SERVICE_' + value.state.toUpperCase() };
      const remaining = await refresh(); setSelectedId(remaining[0]?.id ?? null); onChanged?.();
      setNotice(t('连接已删除。', 'Connection deleted.'));
    } catch (cause) { setError({ message: t('删除未完成。当前路由、草稿或保留的历史版本仍引用此连接时，不能删除；下方列出当前可见引用。', 'Deletion did not complete. Connections referenced by current routes, drafts or retained historical versions cannot be deleted. Current visible references are listed below.'), code: planErrorCode(cause) }); }
    finally { setBusy(false); setDeleting(false); }
  }
  const visible = services.filter(service => `${service.name} ${providerLabel(service.connection, language)} ${service.connection.kind === 'system_one' ? service.connection.model : ''}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  return <ProductPage title={t('模型', 'Models')} subtitle={t('管理已连接的模型与接入', 'Manage connected models and access')} tabs={tabs} flush className="routing-page decision-models-page" actions={
    <div className="decision-add-menu" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setMenu(false); }} onKeyDown={event => { if (event.key === 'Escape') setMenu(false); }}>
      <button className="btn btn-primary" disabled={!mutable || busy} onClick={() => void add('system_one')}><UiIcon name="plus" />{t('添加决策模型', 'Add decision model')}</button><button className="btn" disabled={!mutable || busy} aria-label={t('更多添加方式', 'More connection types')} aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(!menu)}><UiIcon name="chevron" /></button>
      {menu && <div className="decision-menu" role="menu"><button role="menuitem" onClick={() => void add('custom')}>{t('接入自定义扩展', 'Connect custom extension')}</button><button role="menuitem" onClick={() => void add('compatible')}>{t('其他兼容接入', 'Other compatible connection')}</button></div>}
    </div>
  }>
    {onReturn && <div className="decision-return"><button className="btn btn-quiet" disabled={busy} onClick={onReturn}><UiIcon name="arrowLeft" />{t('返回路由', 'Back to route')}</button><span>{t('路由草稿已保留，保存连接后可继续配置。', 'Your route draft is retained. Save a connection to continue.')}</span></div>}
    {error && <div className="callout bad decision-notice" role="alert"><div><p>{error.message}</p>{error.code && <Disclosure label={t('诊断详情', 'Diagnostic details')} language={language}><code>{error.code}</code></Disclosure>}</div><button className="btn" disabled={busy} onClick={() => void refresh().then(() => setError(null)).catch(() => {})}>{t('刷新', 'Refresh')}</button></div>}
    {notice && <div className="decision-notice field-help" role="status">{notice}</div>}
    <div className={'split-view' + (showList || (!editing && !selected) ? ' show-list' : '')}>
      <aside className="master-pane"><div className="decision-search"><UiIcon name="search" /><input className="input" aria-label={t('搜索决策模型与扩展', 'Search decision models and extensions')} placeholder={t('搜索名称、供应商或模型', 'Search name, provider or model')} value={query} onChange={event => setQuery(event.target.value)} /></div><nav className="master-list native-list" aria-label={t('决策模型与扩展', 'Decision models and extensions')}>
        {visible.map(service => <button className={'list-row' + (!editing && selected?.id === service.id ? ' active' : '')} key={service.id} disabled={busy} aria-current={!editing && selected?.id === service.id ? 'page' : undefined} onClick={() => void choose(service)}><DecisionIcon provider={service.connection.kind === 'system_one' ? service.connection.provider : undefined} language={language} /><span className="row-main"><span className="row-title">{service.name}</span><span className="row-meta">{providerLabel(service.connection, language)}</span>{service.connection.kind === 'system_one' && <span className="row-meta">{service.connection.model}</span>}<span className={'decision-test-state' + (tests[testKey(service)] ? tests[testKey(service)].passed ? ' passed' : ' failed' : '')}>{tests[testKey(service)] ? tests[testKey(service)].passed ? t('本次测试通过', 'Test passed this visit') : t('本次测试失败', 'Test failed this visit') : t('本次尚未测试', 'Not tested this visit')}</span></span><UiIcon name="chevronRight" /></button>)}
        {!visible.length && <div className="empty-state"><p>{loading ? t('正在读取…', 'Loading…') : query ? t('没有匹配的连接', 'No matching connections') : t('添加决策模型，为路由判断任务分支。', 'Add a decision model to select task branches.')}</p></div>}
      </nav></aside>
      <section className="detail-pane"><div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => setShowList(true)}><UiIcon name="arrowLeft" />{t('返回列表', 'Back to list')}</button></div>
        {editing ? <DecisionConnectionForm key={editing.service.id + '/' + editing.service.revision} initial={editing.service} saved={!editing.isNew} language={language} mutable={mutable && !busy} onSave={save} onCancel={() => { setEditing(null); setError(null); }} onProtocol={() => setProtocol(true)} /> : selected ? <DecisionConnectionDetail service={selected} language={language} mutable={mutable} busy={busy || selected.id === unverifiedSave?.id} result={tests[testKey(selected)]} savedBeforeTest={savedBeforeTest === testKey(selected)} plans={plans} drafts={drafts} onEdit={() => { setEditing({ service: structuredClone(selected), isNew: false }); setNotice(''); setError(null); }} onTest={() => void testSelected()} onDelete={() => setDeleting(true)} onProtocol={() => setProtocol(true)} onOpenRoute={onOpenRoute} onUse={onUse ? () => onUse(selected) : undefined} /> : <div className="empty-state"><p>{t('选择连接查看详情，或添加决策模型。', 'Select a connection or add a decision model.')}</p></div>}
      </section>
    </div>
    <ClassifierProtocolDialog open={protocol} language={language} onClose={() => setProtocol(false)} />
    <Dialog open={deleting} title={t('删除这个连接？', 'Delete this connection?')} description={selected?.name} closeLabel={t('关闭', 'Close')} onClose={() => { if (!busy) setDeleting(false); }} footer={<><button className="btn" disabled={busy} onClick={() => setDeleting(false)}>{t('取消', 'Cancel')}</button><button className="btn btn-danger" disabled={busy} onClick={() => void remove()}>{t('删除连接', 'Delete connection')}</button></>}><p>{t('被路由、草稿或保留的历史版本引用的连接无法删除。', 'A connection referenced by routes, drafts or retained historical versions cannot be deleted.')}</p></Dialog>
  </ProductPage>;
}
