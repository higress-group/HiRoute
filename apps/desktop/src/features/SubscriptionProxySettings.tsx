import { invoke } from '../host/index.ts';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { safeDiagnosticCode } from '../error-code';
import { clearSubscriptionProxyReturn, requestSubscriptionProxyReturn } from './subscription-proxy-return';

type Mode = 'inherit' | 'direct' | 'manual';
type Policy = { mode: Mode; url?: string; no_proxy?: string };
type View = { config: { revision: string; policy: Policy }; applied: boolean };

// Scan owns the dialog and its retained results; this editor owns the draft and
// footer actions so reading/saving cannot leave unrelated scan actions enabled.
export function SubscriptionProxySettings({ editing, language, writable, onCancel, children }: {
  editing: boolean;
  language: 'zh' | 'en';
  writable: boolean;
  onCancel(): void;
  children(parts: { panel: ReactNode; actions: ReactNode; saving: boolean }): ReactNode;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const formId = useId();
  const panel = useRef<HTMLFormElement>(null);
  const inFlight = useRef(false);
  const [view, setView] = useState<View | null>(null);
  const [mode, setMode] = useState<Mode>('inherit');
  const [url, setUrl] = useState('');
  const [bypass, setBypass] = useState('');
  const [reading, setReading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [retry, setRetry] = useState(0);
  const [error, setError] = useState<{ phase: 'read' | 'save'; code: string } | null>(null);

  useEffect(() => {
    if (!editing) return;
    let current = true;
    setView(null); setReading(true); setError(null);
    panel.current?.scrollIntoView({ block: 'nearest' });
    void invoke<View>('subscription_proxy_status').then(value => {
      if (!current) return;
      setView(value); setMode(value.config.policy.mode);
      setUrl(value.config.policy.url ?? ''); setBypass(value.config.policy.no_proxy ?? '');
    }).catch(reason => {
      if (current) setError({ phase: 'read', code: safeDiagnosticCode(reason, 'SUBSCRIPTION_PROXY_UNAVAILABLE') });
    }).finally(() => { if (current) setReading(false); });
    return () => { current = false; };
  }, [editing, retry]);

  useEffect(() => {
    if (editing && view && !reading) panel.current?.querySelector<HTMLSelectElement>('select')?.focus();
  }, [editing, view, reading]);

  async function save() {
    if (!view || reading || !writable || inFlight.current) return;
    inFlight.current = true; setSaving(true); setError(null);
    try {
      const policy: Policy = mode === 'manual' ? { mode, url: url.trim(), no_proxy: bypass.trim() } : { mode };
      requestSubscriptionProxyReturn();
      await invoke('subscription_proxy_apply', { policy });
      // The native command restarts the application; retain the busy state until
      // that happens rather than offering a second save during shutdown.
    } catch (reason) {
      clearSubscriptionProxyReturn();
      setError({ phase: 'save', code: safeDiagnosticCode(reason, 'SUBSCRIPTION_PROXY_UNAVAILABLE') });
      inFlight.current = false; setSaving(false);
    }
  }

  const busy = reading || saving;
  const modeName = (value: Mode) => ({
    inherit: text('自动（使用启动时的代理）', 'Automatic (use launch proxy)'),
    direct: text('直接连接', 'Direct connection'),
    manual: text('手动代理', 'Manual proxy'),
  })[value];
  return children({
    saving,
    actions: <>
      <button className="btn" type="button" disabled={saving} onClick={onCancel}>{text('取消', 'Cancel')}</button>
      <button className="btn btn-primary" type="submit" form={formId} disabled={busy || !view || !writable || (mode === 'manual' && !url.trim())}>{saving ? text('正在保存并重启…', 'Saving and restarting…') : text('保存并重启 HiRoute', 'Save and restart HiRoute')}</button>
    </>,
    panel: editing && <form ref={panel} id={formId} className="subscription-proxy-panel" aria-label={text('订阅连接代理', 'Subscription connection proxy')} onSubmit={event => { event.preventDefault(); void save(); }}>
      <strong>{text('订阅连接代理', 'Subscription connection proxy')}</strong>
      <p className="oc-meta">{text('用于 HiRoute 接入的全部订阅及订阅检查。以后可从这里修改。', 'Applies to all subscriptions connected through HiRoute and their checks. Return here to change it.')}</p>
      {reading && <p role="status">{text('正在读取代理配置…', 'Loading proxy configuration…')}</p>}
      {view && <>
        <p className="oc-meta">{text(`当前保存：${modeName(view.config.policy.mode)} · ${view.applied ? '已应用' : '等待应用或服务就绪'}`, `Currently saved: ${modeName(view.config.policy.mode)} · ${view.applied ? 'Applied' : 'Pending or service not ready'}`)}</p>
        <div className="subscription-proxy-control">
          <label className="field-label">{text('连接方式', 'Connection mode')}
            <select className="select" aria-label={text('订阅代理模式', 'Subscription proxy mode')} value={mode} disabled={busy || !writable} onChange={event => setMode(event.target.value as Mode)}>
              {(['inherit', 'direct', 'manual'] as const).map(value => <option key={value} value={value}>{modeName(value)}</option>)}
            </select>
          </label>
          {mode === 'manual' && <>
            <label className="field-label">{text('HTTP / HTTPS 代理地址', 'HTTP / HTTPS proxy URL')}<input className="input" type="url" required maxLength={2048} aria-label={text('订阅代理地址', 'Subscription proxy URL')} placeholder={text('http://127.0.0.1:端口', 'http://127.0.0.1:port')} value={url} disabled={busy || !writable} onChange={event => setUrl(event.target.value)} /></label>
            <label className="field-label">{text('绕过代理的地址（可选）', 'Proxy bypass list (optional)')}<input className="input" maxLength={4096} aria-label={text('订阅代理绕过列表', 'Subscription proxy bypass list')} placeholder="example.com,198.51.100.0/24" value={bypass} disabled={busy || !writable} onChange={event => setBypass(event.target.value)} /></label>
            <span className="field-help">{text('多个地址用逗号分隔；本机地址自动绕过。代理地址暂不支持用户名或密码。', 'Separate entries with commas; local addresses always bypass the proxy. Proxy URLs do not support usernames or passwords.')}</span>
          </>}
          {mode === 'inherit' && <span className="field-help">{text('使用 HiRoute 启动时已有的代理设置。', 'Uses the proxy settings available when HiRoute starts.')}</span>}
          {mode === 'direct' && <span className="field-help">{text('订阅连接不使用代理。', 'Subscription connections bypass the proxy.')}</span>}
        </div>
      </>}
      {error && <div className="callout bad" role="alert" data-error-code={error.code}>
        <span>{error.phase === 'read'
          ? text(`无法读取代理配置（${error.code}）。请重试后再修改。`, `Could not read the proxy configuration (${error.code}). Retry before editing.`)
          : text(`代理配置未保存（${error.code}）。输入已保留，请检查地址格式后重试。`, `Proxy configuration was not saved (${error.code}). Your input is retained; check the URL and retry.`)}</span>
        {error.phase === 'read' && <button className="btn" type="button" onClick={() => setRetry(value => value + 1)}>{text('重试读取', 'Retry loading')}</button>}
      </div>}
      <p className="field-help">{text('保存后会重启 HiRoute，正在执行的请求和任务可能中断。重启后将回到扫描本机，继续检查订阅。', 'Saving restarts HiRoute and may interrupt active requests and tasks. After restart, return to this device scan to check subscriptions.')}</p>
    </form>,
  });
}
