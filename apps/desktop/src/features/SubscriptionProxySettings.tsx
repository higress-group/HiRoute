import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { safeDiagnosticCode } from '../error-code';

type Mode = 'inherit' | 'direct' | 'manual';
type Policy = { mode: Mode; url?: string; no_proxy?: string };
type View = { config: { revision: string; policy: Policy }; applied: boolean };

export function SubscriptionProxySettings({ active, language }: { active: boolean; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [view, setView] = useState<View | null>(null);
  const [mode, setMode] = useState<Mode>('inherit');
  const [url, setUrl] = useState('');
  const [bypass, setBypass] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!active) return;
    let current = true;
    setBusy(true);
    void invoke<View>('subscription_proxy_status').then(value => {
      if (!current) return;
      setView(value); setMode(value.config.policy.mode); setUrl(value.config.policy.url ?? ''); setBypass(value.config.policy.no_proxy ?? ''); setError('');
    }).catch(reason => { if (current) setError(safeDiagnosticCode(reason, 'SUBSCRIPTION_PROXY_UNAVAILABLE')); })
      .finally(() => { if (current) setBusy(false); });
    return () => { current = false; };
  }, [active]);
  async function save() {
    setBusy(true); setError('');
    try {
      const policy: Policy = mode === 'manual' ? { mode, url: url.trim(), no_proxy: bypass.trim() } : { mode };
      await invoke('subscription_proxy_apply', { policy });
    } catch (reason) {
      setError(safeDiagnosticCode(reason, 'SUBSCRIPTION_PROXY_UNAVAILABLE')); setBusy(false);
    }
  }
  return <section className="settings-group" id="subscription-proxy-settings">
    <h2>{text('网络', 'Network')}</h2>
    <div className="settings-list"><div className="settings-row">
      <div className="settings-copy"><strong>{text('订阅连接代理', 'Subscription connection proxy')}</strong>
        <span>{text('作用于 HiRoute 接入的全部订阅及订阅检查。', 'Applies to all subscriptions connected through HiRoute and their connection checks.')}</span>
        <span>{view ? text(`保存的模式：${{ inherit: '继承启动环境', direct: '直接连接', manual: '手动代理' }[view.config.policy.mode]} · ${view.applied ? '配置已应用' : '等待应用或服务尚未就绪'}`, `Saved mode: ${view.config.policy.mode} · ${view.applied ? 'Configuration applied' : 'Pending or service not ready'}`) : text('正在读取设置…', 'Loading settings…')}</span>
        <span>{text('配置已应用不代表订阅可用；请在“模型 → 添加模型”中检查订阅，再发起一次真实请求。', 'An applied configuration does not confirm subscription availability. Check the subscription in Models → Add model, then send a real request.')}</span>
      </div>
      <div className="subscription-proxy-control">
        <label className="field-label">{text('连接方式', 'Connection mode')}
          <select className="select" aria-label={text('订阅代理模式', 'Subscription proxy mode')} value={mode} disabled={busy} onChange={event => setMode(event.target.value as Mode)}>
            <option value="inherit">{text('继承启动环境', 'Inherit launch environment')}</option>
            <option value="direct">{text('直接连接', 'Direct connection')}</option>
            <option value="manual">{text('手动代理', 'Manual proxy')}</option>
          </select>
        </label>
        {mode === 'manual' && <>
          <label className="field-label">{text('HTTP / HTTPS 代理地址', 'HTTP / HTTPS proxy URL')}<input className="input" aria-label={text('订阅代理地址', 'Subscription proxy URL')} placeholder="http://127.0.0.1:1187" value={url} disabled={busy} onChange={event => setUrl(event.target.value)} /></label>
          <label className="field-label">{text('绕过代理的地址（可选）', 'Proxy bypass list (optional)')}<input className="input" aria-label={text('订阅代理绕过列表', 'Subscription proxy bypass list')} placeholder="example.com,192.168.0.0/16" value={bypass} disabled={busy} onChange={event => setBypass(event.target.value)} /></label>
          <span className="field-help">{text('多个地址用逗号分隔；本机地址自动绕过。暂不支持在代理地址中填写用户名或密码。', 'Separate entries with commas; local addresses always bypass the proxy. Usernames and passwords in proxy URLs are not supported.')}</span>
        </>}
        {mode === 'inherit' && <span className="field-help">{text('继承 HiRoute 启动时的 HTTP(S)_PROXY 与 NO_PROXY，不会运行 startProxy 或读取 shell 配置。', 'Uses HTTP(S)_PROXY and NO_PROXY from HiRoute’s launch environment; shell startup functions are not executed.')}</span>}
        <span className="field-help">{text('保存后会重启 HiRoute，正在执行的请求和任务可能中断。', 'Saving restarts HiRoute and may interrupt active requests and tasks.')}</span>
        <button className="btn" type="button" disabled={busy || (mode === 'manual' && !url.trim())} onClick={() => void save()}>{busy ? text('处理中…', 'Working…') : text('保存并重启 HiRoute', 'Save and restart HiRoute')}</button>
      </div>
    </div>{error && <div className="settings-inline-error" role="status">{text(`代理设置未完成（${error}）。请检查地址格式和服务状态。`, `Proxy setup failed (${error}). Check the URL and service status.`)}</div>}</div>
  </section>;
}
