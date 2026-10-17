import { useEffect, useState } from 'react';
import { invoke } from '../host/index.ts';
import { safeDiagnosticCode } from '../error-code';
import type { Language } from '../ui/preferences';
export function ShellHostSettings({ active, language }: { active: boolean; language: Language }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [confirmRestart, setConfirmRestart] = useState(false);
  useEffect(() => {
    if (!active) return;
    let current = true;
    void invoke<{ enabled: boolean }>('login_start_get').then(value => { if (current) setEnabled(value.enabled); })
      .catch(cause => { if (current) setError(safeDiagnosticCode(cause, 'SHELL_LOGIN_UNAVAILABLE')); });
    return () => { current = false; };
  }, [active]);
  async function action(command: string, args = {}) {
    setBusy(true); setError('');
    try {
      const value = await invoke<{ enabled: boolean } | null>(command, args);
      if (command === 'login_start_set' && value) setEnabled(value.enabled);
    } catch (cause) { setError(safeDiagnosticCode(cause, 'SHELL_HOST_UNAVAILABLE')); }
    finally { setBusy(false); setConfirmRestart(false); }
  }
  return <section className="settings-group"><h2>{text('Windows 桌面应用', 'Windows Desktop')}</h2><div className="settings-list">
    <div className="settings-row"><div className="settings-copy"><strong>{text('登录时启动', 'Start at login')}</strong><span>{text('同时管理桌面窗口和当前用户的后台服务', 'Manage the Desktop window and current-user background service together')}</span></div>
      <button className="btn" type="button" aria-pressed={enabled === true} disabled={busy || enabled === null} onClick={() => void action('login_start_set', { enabled: !enabled })}>{enabled === null ? text('读取中…', 'Loading…') : enabled ? text('已开启', 'On') : text('已关闭', 'Off')}</button>
    </div>
    <div className="settings-row"><div className="settings-copy"><strong>{text('在浏览器中打开', 'Open in browser')}</strong><span>{text('创建一次性本机入口；浏览器会话需要重新进入', 'Create a one-time local entry; the browser session requires re-entry')}</span></div><button className="btn" type="button" disabled={busy} onClick={() => void action('open_in_browser')}>{text('打开', 'Open')}</button></div>
    <div className="settings-row"><div className="settings-copy"><strong>{text('重新启动后台服务', 'Restart background service')}</strong><span>{text('桌面窗口可以保留；现有调用可能暂时中断', 'Keep the Desktop window open; current calls may be interrupted')}</span></div><button className="btn" type="button" disabled={busy} onClick={() => setConfirmRestart(true)}>{text('重启服务…', 'Restart service…')}</button></div>
    {confirmRestart && <div className="gateway-risk"><p>{text('确认重启后台服务？请先等待当前调用和 Worker 任务完成。', 'Restart the background service? Wait for current calls and Worker tasks first.')}</p><button className="btn" type="button" disabled={busy} onClick={() => void action('restart_service')}>{text('确认重启', 'Restart service')}</button><button className="btn" type="button" disabled={busy} onClick={() => setConfirmRestart(false)}>{text('取消', 'Cancel')}</button></div>}
    {error && <div className="settings-inline-error" role="status">{error}</div>}
  </div></section>;
}
