import { InheritedSourceNetwork } from './InheritedSourceNetwork';
import { SecretTransportWarning } from '../../host/ConsoleUi';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Disclosure, UiIcon } from '../../ui';
import { confirmDiscard, requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import type { DecisionServiceV2, DecisionServiceView, DecisionWireProtocol } from './contracts';
import {
  applySourceDefault, chooseMode, chooseProvider, chooseSource, comparable, currentBundle, displayName, editBundle, endpointHost,
  independentPresets, knownSources, lineageChanged, nativeIssues, nativeService, presetName, retainedIndependentAuth, sameBundle,
  sameEndpointOrigin, sourceChoiceLabels, sourceFor, sourceName, sourceStateLabel, suggestedBundle, resetToSuggested,
  type DecisionMode, type NativeDraft, type NativeIssueKey, type SourceRead,
} from './form-state';
import { DecisionIcon } from './DecisionIcon';

export type NativeSaveOutcome = 'committed' | 'conflict' | 'failed';
const protocolName = (protocol: DecisionWireProtocol, language: 'zh' | 'en') => protocol === 'openai_decisions'
  ? (language === 'zh' ? 'OpenAI 决策接口' : 'OpenAI decisions') : 'System One';

export function DecisionModelForm({ initial, saved, view, revision, sources, language, mutable, onRetrySources, onManageSource, onSave, onCancel, onNetworkDirty }: {
  initial: NativeDraft; saved: boolean; view: DecisionServiceView | null; revision: number; sources: SourceRead;
  language: 'zh' | 'en'; mutable: boolean; onRetrySources(): void | Promise<void>; onManageSource?(sourceId: string): void;
  onNetworkDirty?(dirty: boolean): void;
  onSave(service: DecisionServiceV2, secret: string | null, test: boolean, sourceIds: string[]): Promise<NativeSaveOutcome>;
  onCancel(): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [draft, setDraft] = useState(initial);
  const [baseline] = useState(() => comparable(initial, null, language));
  const [replacing, setReplacing] = useState(false), [revealed, setRevealed] = useState(false);
  const [hasSecret, setHasSecret] = useState(false), [busy, setBusy] = useState(false);
  const [networkDirty, setNetworkDirty] = useState(false);
  const networkChanged = useCallback((dirty: boolean) => { setNetworkDirty(dirty); onNetworkDirty?.(dirty); }, [onNetworkDirty]);
  const [issues, setIssues] = useState<Partial<Record<NativeIssueKey, string>>>({});
  const [credentialNotice, setCredentialNotice] = useState('');
  const [urlOpen, setUrlOpen] = useState(false), [protocolOpen, setProtocolOpen] = useState(false);
  const secret = useRef<HTMLInputElement>(null), form = useRef<HTMLFormElement>(null);
  const source = sourceFor(draft, sources, view);
  const bundle = currentBundle(draft, source);
  const listed = knownSources(sources);
  const retained = retainedIndependentAuth(draft, source);
  const dirty = !saved || comparable(draft, source, language) !== baseline || hasSecret;
  useDiscardGuard('decisions', dirty || busy || networkDirty, language, async () => {
    if (busy || networkDirty || !await confirmDiscard(language)) return false;
    if (secret.current) secret.current.value = '';
    onCancel();
    return true;
  });
  // A successful first read may only supply the default mode; explicit or saved intent wins.
  useEffect(() => { if (sources.status === 'ready') setDraft(current => applySourceDefault(current, sources.sources)); }, [sources]);

  function clearSecret(notice?: string) {
    const pending = !!secret.current?.value;
    if (secret.current) secret.current.value = '';
    setHasSecret(false); setRevealed(false); setReplacing(false);
    if (pending && notice) setCredentialNotice(notice);
  }
  function clearChangedOrigin(next: NativeDraft) {
    const nextBundle = currentBundle(next, source);
    if (draft.mode === 'independent' && next.mode === 'independent' && bundle && nextBundle
      && !sameEndpointOrigin(bundle.endpoint, nextBundle.endpoint) && (retained || secret.current?.value)) {
      clearSecret();
      setCredentialNotice(t('地址的来源已变化，请重新填写 API Key。', 'The address origin changed. Enter the API key again.'));
    }
  }
  function update(next: NativeDraft) { clearChangedOrigin(next); setDraft(next); setIssues({}); setUrlOpen(false); setProtocolOpen(false); }
  function switchContext(next: NativeDraft) {
    clearSecret(t('配置方式、供应商或 API 接入已变化，未保存的 Key 已清除。', 'The setup, provider or API connection changed. The unsaved key was cleared.'));
    update(next);
  }
  function setMode(mode: DecisionMode) { if (mode !== draft.mode || !draft.modeChosen) switchContext(chooseMode(draft, mode, source)); }
  function setEndpoint(endpoint: string) {
    if (!bundle) return;
    const next = editBundle(draft, source, { endpoint });
    clearChangedOrigin(next);
    setDraft(next); setIssues({});
  }
  async function submit(test: boolean) {
    if (busy || !mutable || networkDirty) return;
    const found = nativeIssues(draft, source, !!secret.current?.value.trim(), replacing, language);
    const first = Object.keys(found)[0] as NativeIssueKey | undefined;
    if (first) {
      setIssues(found);
      if (first === 'endpoint') setUrlOpen(true);
      if (first === 'wire_protocol') setProtocolOpen(true);
      requestAnimationFrame(() => {
        const target = form.current?.querySelector<HTMLElement>(`[data-native-field="${first}"]`);
        let disclosure = target?.closest('details');
        while (disclosure) { disclosure.open = true; disclosure = disclosure.parentElement?.closest('details'); }
        target?.focus({ preventScroll: true });
        target?.closest('.field')?.scrollIntoView({ block: 'center', behavior: 'instant' });
      });
      return;
    }
    const service = nativeService(draft, source, revision + 1, replacing, language);
    const independentSecret = draft.mode === 'independent' && (replacing || !retained) ? secret.current?.value ?? null : null;
    setBusy(true);
    try {
      // Only the newly selected source is registered; a previously used source stays retained untouched.
      const outcome = await onSave(service, independentSecret || null, test, draft.mode === 'shared' && draft.sourceId ? [draft.sourceId] : []);
      // Conflicts and uncertain outcomes keep non-secret edits; a Key is never resubmitted implicitly.
      if (outcome !== 'committed') clearSecret(t('未保存的 Key 已清除，请核对后重新填写。', 'The unsaved key was cleared. Review and enter it again.'));
    } finally { setBusy(false); }
  }

  const fieldError = (name: NativeIssueKey) => issues[name] && <span className="oc-inline-error" role="alert">{issues[name]}</span>;
  const suggestion = draft.mode ? suggestedBundle(draft, source) : null;
  const sourceLabels = sourceChoiceLabels(listed, language);
  const rebind = lineageChanged(draft, source);
  const showUrlInput = !!bundle && (urlOpen || !bundle.endpoint || !!issues.endpoint);
  const showProtocolInput = !!bundle && (protocolOpen || !bundle.wire_protocol);
  const host = bundle ? endpointHost(bundle.endpoint) : null;
  const preset = draft.mode === 'independent' ? draft.provider : null;
  const title = saved ? t('编辑决策模型', 'Edit decision model') : t('添加决策模型', 'Add decision model');

  const sharedPanel = <>
    {sources.status === 'loading' && !listed.length && <div className="decision-source-loading" role="status"><UiIcon name="clock" /><span>{t('正在读取 API 接入…', 'Reading API connections…')}</span></div>}
    {sources.status === 'error' && <div className="callout bad decision-source-error" role="alert"><p>{t('暂时无法读取 API 接入的状态。', 'API connection state could not be read.')}</p><button className="btn" type="button" onClick={onRetrySources}><UiIcon name="refresh" />{t('重试', 'Retry')}</button></div>}
    {(sources.status === 'ready' || listed.length > 0 || draft.sourceId) && <label className="field"><span className="field-label">{t('API 接入', 'API connection')}</span>
      <select className="input" data-native-field="source" disabled={networkDirty} aria-invalid={!!issues.source} value={rebind ? '' : draft.sourceId ?? ''} onChange={event => {
        const id = event.target.value || null;
        const next = listed.find(item => item.source_id === id) ?? null;
        switchContext(chooseSource(draft, next, id));
      }}>
        <option value="">{t('请选择 API 接入', 'Choose an API connection')}</option>
        {draft.sourceId && source && !listed.some(item => item.source_id === draft.sourceId) && <option value={draft.sourceId}>{`${sourceName(source, language)} — ${sourceStateLabel(source.state, language)}`}</option>}
        {listed.map(item => <option key={item.source_id} value={item.source_id}>{sourceLabels.get(item.source_id)}</option>)}
      </select>{fieldError('source')}
      {sources.status === 'ready' && !sources.sources.length && <span className="field-help">{t('还没有可复用的 API 接入。可以改为独立配置。', 'No API connection is available to reuse yet. You can configure independently instead.')}</span>}
    </label>}
    {source && !rebind && <div className="decision-shared-credential" data-state={source.state}>
      <UiIcon name="lock" /><div><strong>{t('使用 API 接入的凭据', 'Credentials from API connection')}</strong>
        <span className="field-help">{[sourceName(source, language), source.provider_label, source.endpoint_host].filter(Boolean).join(' · ')} · {sourceStateLabel(source.state, language)}</span>
        <span className="field-help">{t('“本地可用”表示已启用且有已配置的 Key，不代表上游已授权。', '“Ready locally” means enabled with a configured key; it does not confirm upstream access.')}</span></div>
      {onManageSource && <button className="btn" type="button" disabled={networkDirty} onClick={() => onManageSource(source.source_id)}>{t('管理 API 接入', 'Manage API connection')}</button>}
    </div>}
  </>;

  const independentPanel = preset && <>
    <div className="field"><span className="field-label">{t('供应商', 'Provider')}</span><div className="decision-providers" role="group" aria-label={t('供应商', 'Provider')}>
      {['bailian-token-plan', 'openai', 'openrouter', 'typesafe'].map(id => <button className="decision-provider" type="button" key={id} aria-pressed={id.startsWith('bailian') ? preset.startsWith('bailian') : preset === id} onClick={() => { if (!(id.startsWith('bailian') && preset.startsWith('bailian')) && id !== preset) switchContext(chooseProvider(draft, id)); }}><DecisionIcon provider={id} language={language} /><span>{id.startsWith('bailian') ? t('百炼', 'Bailian') : presetName(id, language)}</span></button>)}
    </div><button className="btn btn-quiet decision-compatible" type="button" aria-pressed={preset === 'compatible'} onClick={() => { if (preset !== 'compatible') switchContext(chooseProvider(draft, 'compatible')); }}>{t('其他兼容接入', 'Other compatible connection')}<UiIcon name="chevronRight" /></button></div>
    {preset.startsWith('bailian') && <div className="field"><span className="field-label">{t('接入方式', 'Access')}</span><div className="segmented" role="group" aria-label={t('百炼接入方式', 'Bailian access')}>{independentPresets.filter(p => p.id.startsWith('bailian')).map(item => <button type="button" key={item.id} className={'segment' + (preset === item.id ? ' active' : '')} aria-pressed={preset === item.id} onClick={() => { if (preset !== item.id) switchContext(chooseProvider(draft, item.id)); }}>{item.id === 'bailian-token-plan' ? 'Token Plan' : t('业务空间', 'Workspace')}</button>)}</div></div>}
    <div className="field"><label className="field-label" htmlFor="decision-credential">API Key</label>
      {retained && !replacing && <div className="decision-credential-saved"><UiIcon name="lock" /><span>{t('已配置', 'Configured')}</span><button className="btn btn-quiet" type="button" onClick={() => { setReplacing(true); requestAnimationFrame(() => secret.current?.focus()); }}>{t('更换', 'Replace')}</button></div>}
      <SecretTransportWarning language={language} />
      <div className="decision-secret-input" hidden={!!retained && !replacing}><input ref={secret} id="decision-credential" data-native-field="credential" className="input" type={revealed ? 'text' : 'password'} autoComplete="new-password" maxLength={32768} aria-invalid={!!issues.credential} placeholder={t('输入 API Key', 'Enter an API key')} onChange={event => { setHasSecret(!!event.target.value); setCredentialNotice(''); setIssues({}); }} /><button className="btn btn-quiet" type="button" aria-pressed={revealed} onClick={() => setRevealed(!revealed)}>{revealed ? t('隐藏', 'Hide') : t('显示', 'Show')}</button></div>
      {replacing && retained && <button className="btn btn-quiet" type="button" onClick={() => clearSecret()}>{t('保留已保存的 Key', 'Keep saved key')}</button>}
      {fieldError('credential')}
    </div>
  </>;

  return <form ref={form} noValidate className="decision-connection-form" onSubmit={event => { event.preventDefault(); void submit(true); }}>
    <fieldset className="detail-fieldset" disabled={!mutable || busy}>
      <header className="editor-section decision-heading"><DecisionIcon provider={preset ?? undefined} language={language} /><div><h2>{title}</h2><p>{t('HiRoute 提供决策逻辑，只需连接模型。', 'HiRoute supplies the decision logic. Connect a model to get started.')}</p></div></header>
      <section className="editor-section decision-form-fields">
        <div className="field"><span className="field-label">{t('凭据', 'Credentials')}</span>
          <div className="decision-modes" role="group" aria-label={t('凭据来源', 'Credential source')} data-native-field="mode" tabIndex={-1}>
            <button type="button" className="decision-mode" disabled={networkDirty} aria-pressed={draft.mode === 'shared'} onClick={() => setMode('shared')}><strong>{t('使用已有 API 接入', 'Use existing API connection')}</strong><span>{t('复用接入中的 Key，无需再次填写。', 'Reuse the keys in a connection without entering them again.')}</span></button>
            <button type="button" className="decision-mode" disabled={networkDirty} aria-pressed={draft.mode === 'independent'} onClick={() => setMode('independent')}><strong>{t('独立配置', 'Configure independently')}</strong><span>{t('为决策模型单独填写 Key。', 'Enter a key for this decision model only.')}</span></button>
          </div>{fieldError('mode')}
          {!draft.mode && sources.status === 'loading' && <div className="decision-source-loading" role="status"><UiIcon name="clock" /><span>{t('正在读取 API 接入…', 'Reading API connections…')}</span><button className="btn btn-quiet" type="button" onClick={() => setMode('independent')}>{t('改为独立配置', 'Configure independently instead')}</button></div>}
          {!draft.mode && sources.status === 'error' && <div className="callout bad decision-source-error" role="alert"><p>{t('暂时无法读取 API 接入。', 'API connections could not be read.')}</p><div className="field-actions"><button className="btn" type="button" onClick={onRetrySources}><UiIcon name="refresh" />{t('重试', 'Retry')}</button><button className="btn" type="button" onClick={() => setMode('independent')}>{t('独立配置', 'Configure independently')}</button></div></div>}
        </div>
        {draft.mode === 'shared' && sharedPanel}
        {draft.mode === 'independent' && independentPanel}
        {credentialNotice && <p className="field-help decision-credential-notice" role="status">{credentialNotice}</p>}
        {bundle && <>
          <label className="field"><span className="field-label">{t('模型 ID', 'Model ID')}</span><input className="input" data-native-field="model" maxLength={256} aria-invalid={!!issues.model} value={bundle.model} onChange={event => { setDraft(editBundle(draft, source, { model: event.target.value })); setIssues({}); }} />{fieldError('model')}</label>
          <div className="field decision-address"><span className="field-label">{t('完整地址', 'Complete URL')}</span>
            {showUrlInput ? <input className="input" data-native-field="endpoint" type="url" maxLength={2048} aria-invalid={!!issues.endpoint} aria-label={t('完整地址', 'Complete URL')} value={bundle.endpoint} placeholder="https://…" onChange={event => setEndpoint(event.target.value)} />
              : <div className="decision-address-value"><span className="decision-endpoint">{bundle.endpoint}</span><button className="btn btn-quiet" type="button" onClick={() => { setUrlOpen(true); requestAnimationFrame(() => form.current?.querySelector<HTMLInputElement>('[data-native-field="endpoint"]')?.focus()); }}>{t('更改', 'Change')}</button></div>}
            {fieldError('endpoint')}
            {showProtocolInput ? <label className="decision-protocol-choice"><span className="field-label">{t('协议', 'Protocol')}</span><select className="input" data-native-field="wire_protocol" aria-invalid={!!issues.wire_protocol} value={bundle.wire_protocol ?? ''} onChange={event => { setDraft(editBundle(draft, source, { wire_protocol: (event.target.value || null) as DecisionWireProtocol | null })); setIssues({}); }}>
              {!bundle.wire_protocol && <option value="">{t('请选择协议', 'Choose a protocol')}</option>}<option value="openai_decisions">{protocolName('openai_decisions', language)}</option><option value="system_one">System One</option></select></label>
              : <p className="field-help decision-protocol-summary">{t('协议：', 'Protocol: ')}{protocolName(bundle.wire_protocol!, language)} <button className="btn btn-quiet" type="button" onClick={() => setProtocolOpen(true)}>{t('更改', 'Change')}</button></p>}
            {fieldError('wire_protocol')}
            {draft.mode === 'shared' && source?.endpoint_host && host && host !== source.endpoint_host && <p className="field-help decision-host-note" role="note">{t(`这个地址的主机（${host}）与 API 接入（${source.endpoint_host}）不同。调用时会向这个地址发送接入中的 Key。`, `This address’s host (${host}) differs from the API connection (${source.endpoint_host}). Calls send the connection’s key to this address.`)}</p>}
          </div>
          {draft.mode === 'shared' && source && !rebind && <InheritedSourceNetwork key={source.source_id} sourceId={source.source_id} target={{ kind: 'draft', endpoint_url: bundle.endpoint }} refreshKey={source.revision ?? 0} language={language} mutable={mutable && !busy && source.state !== 'missing'} onDirty={networkChanged} onSaved={onRetrySources} />}
          {suggestion && !sameBundle(bundle, suggestion) && (suggestion.model || suggestion.endpoint) && <button className="btn btn-quiet decision-suggested" type="button" onClick={() => update(resetToSuggested(draft, source))}><UiIcon name="refresh" />{t('使用建议设置', 'Use suggested settings')}</button>}
          <label className="field"><span className="field-label">{t('名称', 'Name')}</span><input className="input" data-native-field="name" maxLength={128} aria-invalid={!!issues.name} value={displayName(draft, source, language)} onChange={event => { setDraft({ ...draft, name: event.target.value, nameEdited: true }); setIssues({}); }} />{fieldError('name')}<span className="field-help">{t('在路由中用这个名称选择决策模型。', 'Use this name to select the decision model in routes.')}</span></label>
          <Disclosure label={t(`高级设置 · 超时 ${draft.timeout_ms / 1000} 秒`, `Advanced settings · ${draft.timeout_ms / 1000}s timeout`)} language={language}><label className="field"><span className="field-label">{t('决策超时（秒）', 'Decision timeout (seconds)')}</span><input className="input" data-native-field="timeout" type="number" min={0.001} max={3600} step={0.001} aria-invalid={!!issues.timeout} value={draft.timeout_ms / 1000} onChange={event => { setDraft({ ...draft, timeout_ms: Math.round(Number(event.target.value) * 1000) }); setIssues({}); }} />{fieldError('timeout')}</label></Disclosure>
          <p className="field-help decision-publish-note">{draft.mode === 'shared'
            ? t('API 接入中的 Key 或网络设置变更会影响之后的调用，无需重新发布；更改模型、地址或协议会生成新配置，需在路由中选用并发布。', 'Key or network changes in the API connection affect later calls without republishing. Changing the model, URL or protocol creates a new configuration that must be selected and published in the route.')
            : t('更改模型、地址、协议或 Key 会生成新配置，需在路由中选用并发布。', 'Changing the model, URL, protocol or key creates a new configuration that must be selected and published in the route.')}</p>
        </>}
      </section>
      <footer className="decision-form-actions">{networkDirty && <p className="field-help" role="status">{t('请先保存或放弃网络修改，再继续或离开；保存结果未确认时，请刷新原操作。', 'Save or discard the network changes before continuing or leaving. If a save is unconfirmed, refresh its original operation.')}</p>}<p className="field-help">{t('测试发送固定示例，可能消耗配额；不读取真实会话，也不能证明所有 Key 可用或路由质量。', 'Testing sends a fixed example and may use quota. It does not read real conversations and does not prove every key works or routing quality.')}</p><div className="field-actions"><button type="submit" className="btn btn-primary" disabled={!dirty || networkDirty}>{busy ? t('正在保存…', 'Saving…') : t('保存并测试', 'Save and test')}</button><button className="btn" type="button" disabled={!dirty || networkDirty} onClick={() => void submit(false)}>{t('仅保存', 'Save only')}</button><button className="btn btn-quiet" type="button" disabled={busy || networkDirty} onClick={() => void requestEditorReplacement('decisions').then(accepted => { if (accepted) onCancel(); })}>{t('取消', 'Cancel')}</button></div></footer>
    </fieldset>
  </form>;
}
