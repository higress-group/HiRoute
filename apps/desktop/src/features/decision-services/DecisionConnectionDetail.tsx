import { InheritedSourceNetwork } from './InheritedSourceNetwork';
import { useState } from 'react';
import { networkFailureText, networkRepairable, type NativeNetworkRoute } from '../model-connections/native-network';
import { Disclosure, UiIcon } from '../../ui';
import type { Draft, Plan } from '../../plan-editor';
import type { DecisionServiceView } from './contracts';
import { decisionFailureHelp, decisionReferences, providerLabel } from './presentation';
import { recordProtocol, recordProvider, recordSharedSource, repairActions, sourceName, sourceStateLabel, type RepairAction, type TestPresentation, type TestRecord } from './form-state';
import { DecisionIcon } from './DecisionIcon';

export function DecisionConnectionDetail({ view, language, mutable, busy, state, result, savedBeforeTest, plans, drafts, onEdit, onTest, onRefresh, onDelete, onProtocol, onOpenRoute, onUse, onManageSource, network, onNetworkSaved, onNetworkDirty, networkDirty }: {
  networkDirty: boolean; network: NativeNetworkRoute | null; onNetworkSaved(): void | Promise<void>; onNetworkDirty(dirty: boolean): void;
  view: DecisionServiceView; language: 'zh' | 'en'; mutable: boolean; busy: boolean; state: TestPresentation; result?: TestRecord; savedBeforeTest: boolean;
  plans: Plan[]; drafts: Draft[]; onEdit(): void; onTest(): void; onRefresh(): void; onDelete(): void; onProtocol(): void;
  onOpenRoute?(plan?: Plan, draft?: Draft): void; onUse?(): void; onManageSource?(sourceId: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [networkReveal, setNetworkReveal] = useState(0);
  const service = view.service, connection = service.connection;
  const references = decisionReferences(service.id, plans, drafts);
  const sharedId = recordSharedSource(service), source = sharedId ? view.source ?? view.related_sources.find(item => item.source_id === sharedId) ?? null : null;
  const protocol = recordProtocol(service);
  const native = connection.kind !== 'custom';
  const authFact = connection.kind === 'custom' ? connection.auth_header ? t('已配置', 'Configured') : t('无需认证', 'No authentication')
    : sharedId ? `${t('使用 API 接入的凭据', 'Credentials from API connection')} · ${source ? `${sourceName(source, language)} · ${sourceStateLabel(source.state, language)}` : t('状态暂不可读', 'State unavailable')}`
      : t('独立 Key · 已配置', 'Independent key · Configured');
  const sourceRepair: RepairAction[] = !source ? [] : source.state === 'disabled' || source.state === 'missing_credentials' ? ['manage_source'] : source.state === 'missing' ? ['choose_source'] : source.state === 'unavailable' ? ['refresh'] : [];
  const action = (item: RepairAction) => {
    switch (item) {
      case 'manage_source': return sharedId && onManageSource ? <button key={item} className="btn" disabled={busy || networkDirty} onClick={() => onManageSource(sharedId)}>{t('管理 API 接入', 'Manage API connection')}</button> : null;
      case 'choose_source': return <button key={item} className="btn" disabled={!mutable || busy || networkDirty} onClick={onEdit}>{t('选择其他接入', 'Choose another connection')}</button>;
      case 'edit': return <button key={item} className="btn" disabled={!mutable || busy || networkDirty} onClick={onEdit}>{native ? t('编辑决策模型', 'Edit decision model') : t('编辑扩展', 'Edit extension')}</button>;
      case 'retry': return <button key={item} className="btn" disabled={!mutable || busy || networkDirty} onClick={onTest}><UiIcon name="refresh" />{t('重试', 'Retry')}</button>;
      case 'refresh': return <button key={item} className="btn" disabled={busy || networkDirty} onClick={onRefresh}><UiIcon name="refresh" />{t('刷新', 'Refresh')}</button>;
    }
  };
  const failedActions = state === 'failed' ? result?.network_failure && networkRepairable(result.network_failure) ? ['retry' as const] : repairActions(result?.code, !!sharedId) : [];
  const heading = state === 'testing' ? t('正在测试…', 'Testing…')
    : state === 'passed' ? t('连接测试通过', 'Connection test passed')
      : state === 'stale' ? t('上次测试通过 — 凭据、网络或配置已变化，尚未重新测试', 'Previous test passed — credentials, network or configuration changed; not retested')
        : state === 'unverified' ? t('上次测试通过 — 未能读取当前凭据或网络状态，无法确认仍然有效', 'Previous test passed — current credential or network state could not be read, so it is not confirmed')
        : state === 'failed' ? savedBeforeTest ? t('配置已保存，测试失败', 'Configuration saved; test failed') : t('连接测试失败', 'Connection test failed')
          : t('本次打开尚未测试', 'Not tested during this visit');
  return <>
    <header className="editor-section decision-heading"><DecisionIcon provider={recordProvider(service)} language={language} /><div><h2>{service.name}</h2><p>{providerLabel(connection, language)}</p></div><button className="btn" disabled={!mutable || busy || networkDirty} onClick={onEdit}>{t('编辑', 'Edit')}</button></header>
    <section className="editor-section"><dl className="decision-facts">
      <div><dt>{t('类型', 'Type')}</dt><dd>{native ? t('决策模型 · HiRoute 内置逻辑', 'Decision model · HiRoute logic') : t('自定义扩展 · 自行实现决策', 'Custom extension · your decision logic')}</dd></div>
      {(connection.kind === 'system_one' || connection.kind === 'native') && <div><dt>{t('模型', 'Model')}</dt><dd>{connection.model}</dd></div>}
      <div><dt>{t('接入点', 'Endpoint')}</dt><dd className="decision-endpoint">{connection.endpoint}</dd></div>
      {protocol && <div><dt>{t('协议', 'Protocol')}</dt><dd>{protocol === 'openai_decisions' ? t('OpenAI 决策接口', 'OpenAI decisions') : 'System One'}</dd></div>}
      <div><dt>{t('认证', 'Authentication')}</dt><dd>{authFact}</dd></div></dl>
      {sourceRepair.length > 0 && <div className="callout decision-source-callout" role="note"><p>{source?.state === 'missing' ? t('所选 API 接入已不存在。', 'The selected API connection no longer exists.') : source?.state === 'unavailable' ? t('暂时无法读取 API 接入的状态；这不代表接入已不存在。', 'The API connection state could not be read. This does not mean it is missing.') : t('所选 API 接入目前没有可用的 Key 或已停用。', 'The selected API connection is disabled or has no usable key.')}</p><div className="field-actions">{sourceRepair.map(action)}</div></div>}
      {sharedId && <InheritedSourceNetwork sourceId={sharedId} target={{ kind: 'decision', id: service.id, revision: service.revision }} route={network} language={language} mutable={mutable && !busy} reveal={networkReveal} onDirty={onNetworkDirty} onSaved={onNetworkSaved} />}
      {connection.kind === 'custom' && <div className="decision-protocol-intro"><p>{t('扩展直接选择类别和模型组，可选评分仅用于观察；HiRoute 校验结果并执行候选选择、可用性接力与回退。', 'The extension selects the category and model group; optional scores are observation only. HiRoute validates the result and handles candidate selection, availability relay and fallback.')}</p><button className="btn" onClick={onProtocol}>{t('查看接入协议', 'View protocol')}</button></div>}
    </section>
    <section className="editor-section"><div className={'decision-test-result' + (state === 'passed' ? ' passed' : state === 'failed' ? ' failed' : state === 'stale' || state === 'unverified' ? ' stale' : '')} role="status" data-test-state={state}><UiIcon name={state === 'testing' ? 'clock' : state === 'passed' ? 'check' : state === 'failed' || state === 'stale' || state === 'unverified' ? 'warning' : 'plug'} /><div><strong>{heading}</strong>
      {result && state === 'passed' && <p>{t(`配置版本 ${service.revision} · ${result.duration ?? '—'} ms · ${new Date(result.at).toLocaleTimeString()}`, `Configuration ${service.revision} · ${result.duration ?? '—'} ms · ${new Date(result.at).toLocaleTimeString()}`)}</p>}
      {result && state === 'failed' && <p>{result.network_failure ? networkFailureText(result.network_failure, language === 'zh') : decisionFailureHelp(result.code ?? '', language)}</p>}
      {sharedId && state === 'failed' && networkRepairable(result?.network_failure) && <button className="btn" type="button" disabled={busy} onClick={() => setNetworkReveal(value => value + 1)}>{t('检查网络设置', 'Review network settings')}</button>}
      {(result?.code || state !== 'untested') && <Disclosure label={t('诊断详情', 'Diagnostic details')} language={language}><dl className="decision-facts decision-diagnostics">{result?.code && <div><dt>{t('代码', 'Code')}</dt><dd><code>{result.code}</code></dd></div>}<div><dt>{t('配置摘要', 'Configuration digest')}</dt><dd><code>{view.configuration_digest}</code></dd></div>{result?.tested_configuration_digest && <div><dt>{t('测试的配置', 'Tested configuration')}</dt><dd><code>{result.tested_configuration_digest}</code></dd></div>}</dl></Disclosure>}
    </div></div>
      <div className="decision-test-actions"><button className="btn" disabled={!mutable || busy || networkDirty} onClick={onTest}><UiIcon name="refresh" />{state === 'untested' ? t('测试连接', 'Test connection') : t('重新测试', 'Test again')}</button>{failedActions.filter(item => item !== 'retry').map(action)}{state === 'unverified' && action('refresh')}{onUse && <button className="btn btn-primary" disabled={busy || networkDirty} onClick={onUse}>{t('用于当前路由', 'Use in current route')}</button>}</div>
      <p className="field-help">{t('测试发送固定示例，可能消耗配额；不读取真实会话。结果仅对应此配置版本，不能证明所有 Key 可用，也不代表实际路由质量。', 'Testing sends a fixed example and may use quota; no real conversation is read. Results apply to this version only and prove neither that every key works nor routing quality.')}</p></section>
    <section className="editor-section"><h3>{t('用于这些路由', 'Used by these routes')}</h3>{references.length ? <div className="decision-references">{references.map(reference => <button className="decision-reference" key={reference.key} onClick={() => onOpenRoute?.(reference.plan, reference.draft)} disabled={!onOpenRoute || busy || networkDirty}><UiIcon name="route" /><span>{reference.name || t('未命名草稿', 'Untitled draft')}<small>{reference.draft ? t('草稿', 'Draft') : t('已发布', 'Published')} · {t(`配置版本 ${reference.revision}`, `Configuration ${reference.revision}`)}</small></span><UiIcon name="chevronRight" /></button>)}</div> : <p className="field-help">{t('当前路由和草稿尚未使用此配置。', 'No current route or draft uses this configuration.')}</p>}<p className="field-help">{sharedId ? t('API 接入中的 Key 或网络设置变更会影响之后的调用，无需重新发布；更改模型、地址或协议后，需在路由中选用新版本并发布。', 'Key or network changes in the API connection affect later calls without republishing. After changing the model, URL or protocol, select the new version in a route and publish it.') : t('更改配置后，需在路由中选用新版本并发布。此处不包含保留的历史版本引用。', 'After editing, select the new version in a route and publish it. Retained historical references are not listed here.')}</p></section>
    <section className="editor-section"><Disclosure label={t('详情与更多操作', 'Details and more actions')} language={language}><dl className="decision-facts"><div><dt>{t('配置版本', 'Configuration version')}</dt><dd>{service.revision}</dd></div><div><dt>{t('决策超时', 'Decision timeout')}</dt><dd>{connection.timeout_ms / 1000} {t('秒', 'seconds')}</dd></div></dl><button className="btn btn-danger" disabled={!mutable || busy || networkDirty} onClick={onDelete}><UiIcon name="trash" />{native ? t('删除决策模型', 'Delete decision model') : t('删除扩展', 'Delete extension')}</button></Disclosure></section>
  </>;
}
