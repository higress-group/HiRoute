import { useRef, useState } from 'react';
import { Disclosure, UiIcon } from '../../ui';
import { confirmDiscard, requestEditorReplacement, useDiscardGuard } from '../../ui/discard-guard';
import { providers, type DecisionConnection, type DecisionService } from './types';
import { initialAuthMode, protectedInput, providerLabel, sameEndpointOrigin, type AuthMode } from './presentation';
import { DecisionIcon } from './DecisionIcon';

export function DecisionConnectionForm({ initial, saved, language, mutable, onSave, onCancel, onProtocol }: {
  initial: DecisionService; saved: boolean; language: 'zh' | 'en'; mutable: boolean;
  onSave(service: DecisionService, secret: string | null, test: boolean): Promise<void>;
  onCancel(): void; onProtocol(): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [draft, setDraft] = useState(() => structuredClone(initial));
  const [authMode, setAuthMode] = useState<AuthMode>(() => initialAuthMode(initial.connection));
  const [replacing, setReplacing] = useState(false), [revealed, setRevealed] = useState(false);
  const [hasSecret, setHasSecret] = useState(false), [busy, setBusy] = useState(false);
  const [issues, setIssues] = useState<Record<string, string>>({});
  const [credentialNotice, setCredentialNotice] = useState('');
  const secret = useRef<HTMLInputElement>(null), form = useRef<HTMLFormElement>(null);
  const connection = draft.connection;
  const dirty = !saved || JSON.stringify(draft) !== JSON.stringify(initial) || hasSecret;
  useDiscardGuard('decisions', dirty || busy, language, async () => {
    if (busy || !await confirmDiscard(language)) return false;
    if (secret.current) secret.current.value = '';
    onCancel();
    return true;
  });
  function patch(value: Partial<DecisionService>) { setDraft(current => ({ ...current, ...value })); setIssues({}); }
  function clearCredential(next: DecisionConnection): DecisionConnection {
    if (secret.current) secret.current.value = '';
    setHasSecret(false); setRevealed(false); setReplacing(false);
    if (connection.auth_header?.value_secret_ref || hasSecret) setCredentialNotice(t('接入方式或地址已变化，请重新填写认证。', 'The provider or endpoint changed. Enter credentials again.'));
    return next.auth_header ? { ...next, auth_header: { ...next.auth_header, value_secret_ref: '' } } : next;
  }
  function selectProvider(id: string) {
    if (connection.kind !== 'system_one' || id === connection.provider) return;
    const provider = providers.find(p => p.id === id)!;
    const next = clearCredential({ ...connection, provider: id, model: provider.model, endpoint: provider.endpoint });
    patch({ connection: next, name: draft.name === providerLabel(connection, language) ? providerLabel(next, language) : draft.name });
  }
  function setEndpoint(endpoint: string) {
    const next = { ...connection, endpoint };
    patch({ connection: sameEndpointOrigin(connection.endpoint, endpoint) ? next : clearCredential(next) });
  }
  function setAuth(mode: AuthMode) {
    if (connection.kind !== 'custom') return;
    setAuthMode(mode);
    patch({ connection: clearCredential({ ...connection, auth_header: mode === 'none' ? null : { name: 'Authorization', value_secret_ref: '' } }) });
  }
  async function submit(test: boolean) {
    if (busy || !mutable) return;
    const errors: Record<string, string> = {};
    for (const element of Array.from(form.current?.elements ?? [])) {
      if (element instanceof HTMLInputElement && !element.validity.valid) errors[element.name] = element.validationMessage;
    }
    if (!draft.name.trim()) errors.name = t('请填写名称。', 'Enter a name.');
    if (connection.kind === 'system_one' && !connection.model.trim()) errors.model = t('请填写模型名称。', 'Enter a model name.');
    try {
      const url = new URL(connection.endpoint);
      if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.hash) throw new Error();
    } catch { errors.endpoint = t('请填写完整 HTTP 或 HTTPS 地址，不含用户名、密码或片段。', 'Enter a complete HTTP or HTTPS URL without user info or a fragment.'); }
    if (connection.auth_header && (replacing || !connection.auth_header.value_secret_ref) && !secret.current?.value.trim()) errors.credential = t('请填写认证凭据。', 'Enter a credential.');
    if (Object.keys(errors).length) {
      setIssues(errors);
      const target = form.current?.elements.namedItem(Object.keys(errors)[0]) as HTMLInputElement | null;
      let disclosure = target?.closest('details');
      while (disclosure) { disclosure.open = true; disclosure = disclosure.parentElement?.closest('details'); }
      requestAnimationFrame(() => {
        target?.focus({ preventScroll: true });
        target?.closest('.field')?.scrollIntoView({ block: 'center', behavior: 'instant' });
      });
      return;
    }
    setBusy(true);
    try { await onSave({ ...draft, name: draft.name.trim() }, protectedInput(connection, authMode, secret.current?.value ?? ''), test); }
    finally { setBusy(false); }
  }
  const fieldError = (name: string) => issues[name] && <span className="oc-inline-error" role="alert">{issues[name]}</span>;
  const endpointField = <label className="field"><span className="field-label">{t('完整接入点', 'Complete endpoint')}</span><input className="input" name="endpoint" type="url" required maxLength={2048} aria-invalid={!!issues.endpoint} value={connection.endpoint} placeholder="https://…" onChange={event => setEndpoint(event.target.value)} />{fieldError('endpoint')}<span className="field-help">{connection.kind === 'custom' ? t('HiRoute 向此地址发送 POST 请求，不追加路径。', 'HiRoute sends POST requests to this URL without adding a path.') : connection.provider === 'bailian-workspace' ? t('填写所在地域与业务空间的完整 System One 接入点。', 'Enter the complete System One endpoint for your region and workspace.') : t('需支持决策接口，普通对话接口不可用于此处。', 'Use a decision API endpoint, not a chat endpoint.')}</span></label>;
  const modelField = connection.kind === 'system_one' && <label className="field"><span className="field-label">{t('模型名称', 'Model name')}</span><input className="input" name="model" required maxLength={256} aria-invalid={!!issues.model} value={connection.model} onChange={event => patch({ connection: { ...connection, model: event.target.value } })} />{fieldError('model')}</label>;
  const title = saved ? t('编辑连接', 'Edit connection') : connection.kind === 'custom' ? t('接入自定义扩展', 'Connect custom extension') : t('添加决策模型', 'Add decision model');
  return <form ref={form} noValidate className="decision-connection-form" onSubmit={event => { event.preventDefault(); void submit(true); }}>
    <fieldset className="detail-fieldset" disabled={!mutable || busy}>
      <header className="editor-section decision-heading"><DecisionIcon provider={connection.kind === 'system_one' ? connection.provider : undefined} language={language} /><div><h2>{title}</h2><p>{connection.kind === 'custom' ? t('由你的服务实现分支判断和胜任评分。', 'Your service implements branch selection and competence scoring.') : t('HiRoute 提供决策逻辑，只需连接模型。', 'HiRoute supplies the decision logic. Connect a model to get started.')}</p></div></header>
      <section className="editor-section decision-form-fields">
        {connection.kind === 'system_one' && <>
          <div className="field"><span className="field-label">{t('供应商', 'Provider')}</span><div className="decision-providers" role="group" aria-label={t('供应商', 'Provider')}>
            {['bailian-token-plan', 'openrouter', 'typesafe'].map(id => <button className="decision-provider" type="button" key={id} aria-pressed={id.startsWith('bailian') ? connection.provider.startsWith('bailian') : connection.provider === id} onClick={() => selectProvider(id)}><DecisionIcon provider={id} language={language} /><span>{id.startsWith('bailian') ? t('百炼', 'Bailian') : id === 'typesafe' ? 'TypeSafe' : 'OpenRouter'}</span></button>)}
          </div><button className="btn btn-quiet decision-compatible" type="button" aria-pressed={connection.provider === 'compatible'} onClick={() => selectProvider('compatible')}>{t('其他兼容接入', 'Other compatible connection')}<UiIcon name="chevronRight" /></button></div>
          {connection.provider.startsWith('bailian') && <div className="field"><span className="field-label">{t('接入方式', 'Access')}</span><div className="segmented" role="group" aria-label={t('百炼接入方式', 'Bailian access')}>{providers.filter(p => p.id.startsWith('bailian')).map(provider => <button type="button" key={provider.id} className={'segment' + (connection.provider === provider.id ? ' active' : '')} aria-pressed={connection.provider === provider.id} onClick={() => selectProvider(provider.id)}>{provider.id === 'bailian-token-plan' ? 'Token Plan' : t('业务空间', 'Workspace')}</button>)}</div></div>}
        </>}
        {connection.kind === 'custom' && <><div className="decision-protocol-intro"><p>{t('HiRoute 发送允许的分支和可见执行历史；扩展返回分支及可选评分。', 'HiRoute sends allowed branches and visible execution history; the extension returns a branch and optional score.')}</p><button type="button" className="btn" onClick={onProtocol}>{t('查看接入协议', 'View protocol')}<UiIcon name="chevronRight" /></button></div>{endpointField}<label className="field"><span className="field-label">{t('认证方式', 'Authentication')}</span><select className="input" value={authMode} onChange={event => setAuth(event.target.value as AuthMode)}><option value="none">{t('无需认证', 'No authentication')}</option><option value="bearer">Bearer Token</option><option value="header">{t('自定义请求头', 'Custom header')}</option></select></label>
          {authMode === 'header' && <label className="field"><span className="field-label">{t('请求头名称', 'Header name')}</span><input className="input" name="header" required pattern="[!#$%&'*+.^_`|~0-9A-Za-z-]+" value={connection.auth_header?.name ?? ''} onChange={event => patch({ connection: clearCredential({ ...connection, auth_header: { name: event.target.value, value_secret_ref: '' } }) })} />{fieldError('header')}</label>}
        </>}
        {credentialNotice && <p className="field-help" role="status">{credentialNotice}</p>}
        {connection.auth_header && <div className="field"><label className="field-label" htmlFor="decision-credential">{connection.kind === 'system_one' ? 'API Key' : authMode === 'bearer' ? 'Bearer Token' : t('请求头的完整值', 'Full header value')}</label>
          {!!connection.auth_header.value_secret_ref && !replacing && <div className="decision-credential-saved"><UiIcon name="lock" /><span>{t('已配置', 'Configured')}</span><button className="btn btn-quiet" type="button" onClick={() => { setReplacing(true); requestAnimationFrame(() => secret.current?.focus()); }}>{t('更换', 'Replace')}</button></div>}
          <div className="decision-secret-input" hidden={!!connection.auth_header.value_secret_ref && !replacing}><input ref={secret} id="decision-credential" name="credential" className="input" type={revealed ? 'text' : 'password'} autoComplete="new-password" required={!connection.auth_header.value_secret_ref || replacing} maxLength={32768} aria-invalid={!!issues.credential} placeholder={authMode === 'header' ? t('例如 Bearer your-token', 'For example Bearer your-token') : t('输入 Key 或 Token', 'Enter a key or token')} onChange={event => { setHasSecret(!!event.target.value); setCredentialNotice(''); setIssues({}); }} /><button className="btn btn-quiet" type="button" aria-pressed={revealed} onClick={() => setRevealed(!revealed)}>{revealed ? t('隐藏', 'Hide') : t('显示', 'Show')}</button></div>
          {replacing && !!connection.auth_header.value_secret_ref && <button className="btn btn-quiet" type="button" onClick={() => { if (secret.current) secret.current.value = ''; setReplacing(false); setHasSecret(false); setRevealed(false); }}>{t('保留已保存的认证', 'Keep saved credential')}</button>}
          {fieldError('credential')}{authMode === 'bearer' && connection.kind === 'custom' && <span className="field-help">{t('只需填写 Token，HiRoute 自动添加 Bearer 前缀。', 'Enter the token only. HiRoute adds the Bearer prefix.')}</span>}
        </div>}
        {connection.kind === 'system_one' && (connection.provider === 'compatible' ? <>{modelField}{endpointField}</> : <>
          <div className="decision-default"><span className="field-label">{t('决策模型', 'Decision model')}</span><strong>{connection.model}</strong><span className="field-help">{t('用于任务分支判断与胜任评分', 'For branch selection and competence scoring')}</span></div>
          {connection.provider === 'bailian-workspace' && endpointField}
          <Disclosure label={t('查看或修改模型与接入点', 'View or change model and endpoint')} language={language}>{modelField}{connection.provider !== 'bailian-workspace' && endpointField}</Disclosure>
        </>)}
        <label className="field"><span className="field-label">{t('连接名称', 'Connection name')}</span><input className="input" name="name" required maxLength={128} aria-invalid={!!issues.name} value={draft.name} onChange={event => patch({ name: event.target.value })} />{fieldError('name')}<span className="field-help">{t('在路由中用这个名称选择连接。', 'Use this name to select the connection in routes.')}</span></label>
        <Disclosure label={t(`高级设置 · 超时 ${connection.timeout_ms / 1000} 秒`, `Advanced settings · ${connection.timeout_ms / 1000}s timeout`)} language={language}><label className="field"><span className="field-label">{t('决策超时（秒）', 'Decision timeout (seconds)')}</span><input className="input" name="timeout" type="number" min={0.001} max={3600} step={0.001} required aria-invalid={!!issues.timeout} value={connection.timeout_ms / 1000} onChange={event => patch({ connection: { ...connection, timeout_ms: Math.round(Number(event.target.value) * 1000) } })} />{fieldError('timeout')}</label></Disclosure>
      </section>
      <footer className="decision-form-actions"><p className="field-help">{t('测试发送固定示例，可能消耗供应商配额；不读取真实会话。', 'Testing sends a fixed example and may consume provider quota; it does not read real conversations.')}</p><div className="field-actions"><button type="submit" className="btn btn-primary" disabled={!dirty}>{busy ? t('正在保存…', 'Saving…') : t('保存并测试', 'Save and test')}</button><button className="btn" type="button" disabled={!dirty} onClick={() => void submit(false)}>{t('仅保存', 'Save only')}</button><button className="btn btn-quiet" type="button" onClick={() => void requestEditorReplacement('decisions').then(accepted => { if (accepted) onCancel(); })}>{t('取消', 'Cancel')}</button></div></footer>
    </fieldset>
  </form>;
}
