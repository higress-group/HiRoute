import { useEffect, useState } from 'react';
import { hostCapabilities, invoke } from '../host/index.ts';
import { safeDiagnosticCode } from '../error-code';
import type { Language } from '../ui/preferences';
import { Dialog } from '../ui/Dialog';
export function ShellHostSettings({ active, language }: { active: boolean; language: Language }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const host = hostCapabilities();
  const windows = host.platform === 'windows';
  const login = windows && host.login_start === true;
  const browser = host.open_in_browser === true || (windows && host.open_in_browser === undefined);
  const restart = host.restart_service === true || (windows && host.restart_service === undefined);
  const remove = host.platform === 'linux' && host.remove_runtime === true;
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [confirmation, setConfirmation] = useState<'restart_service' | 'remove_runtime' | null>(null);
  const removing = confirmation === 'remove_runtime';
  const confirmTitle = removing
    ? text('移除后台服务？', 'Remove the background service?')
    : text('重启后台服务？', 'Restart the background service?');
  const confirmLabel = removing
    ? busy ? text('正在移除…', 'Removing…') : text('确认移除', 'Remove service')
    : busy ? text('正在重启…', 'Restarting…') : text('确认重启', 'Restart service');
  useEffect(() => {
    if (!active || !login) return;
    let current = true;
    void invoke<{ enabled: boolean }>('login_start_get').then(value => { if (current) setEnabled(value.enabled); })
      .catch(cause => { if (current) setError(safeDiagnosticCode(cause, 'SHELL_LOGIN_UNAVAILABLE')); });
    return () => { current = false; };
  }, [active, login]);
  async function action(command: string, args = {}) {
    if (busy) return;
    setBusy(true); setError('');
    try {
      const value = await invoke<{ enabled: boolean } | null>(command, args);
      if (command === 'login_start_set' && value) setEnabled(value.enabled);
    } catch (cause) { setError(safeDiagnosticCode(cause, 'SHELL_HOST_UNAVAILABLE')); }
    finally { setBusy(false); setConfirmation(null); }
  }
  if (!login && !browser && !restart && !remove) return null;
  return <section className="settings-group"><h2>{windows ? text('Windows 桌面应用', 'Windows Desktop') : text('本机桌面应用', 'Desktop app')}</h2><div className="settings-list">
    {login && <div className="settings-row"><div className="settings-copy"><strong>{text('登录时启动', 'Start at login')}</strong><span>{text('同时管理桌面窗口和当前用户的后台服务', 'Manage the Desktop window and current-user background service together')}</span></div>
      <button className="btn" type="button" aria-pressed={enabled === true} disabled={busy || enabled === null} onClick={() => void action('login_start_set', { enabled: !enabled })}>{enabled === null ? text('读取中…', 'Loading…') : enabled ? text('已开启', 'On') : text('已关闭', 'Off')}</button>
    </div>}
    {browser && <div className="settings-row"><div className="settings-copy"><strong>{text('在浏览器中打开', 'Open in browser')}</strong><span>{text('创建一次性本机入口；浏览器会话需要重新进入', 'Create a one-time local entry; the browser session requires re-entry')}</span></div><button className="btn" type="button" disabled={busy} onClick={() => void action('open_in_browser')}>{text('打开', 'Open')}</button></div>}
    {restart && <div className="settings-row"><div className="settings-copy"><strong>{text('重新启动后台服务', 'Restart background service')}</strong><span>{text('桌面窗口可以保留；现有调用可能暂时中断', 'Keep the Desktop window open; current calls may be interrupted')}</span></div><button className="btn" type="button" disabled={busy} onClick={() => setConfirmation('restart_service')}>{text('重启服务…', 'Restart service…')}</button></div>}
    {remove && <div className="settings-row"><div className="settings-copy"><strong>{text('移除后台服务', 'Remove background service')}</strong><span>{text('停止服务并移除当前用户的 CLI，保留已保存的数据', 'Stop the service and remove this account’s CLI. Saved data is kept.')}</span></div><button className="btn" type="button" disabled={busy} onClick={() => setConfirmation('remove_runtime')}>{text('移除服务…', 'Remove service…')}</button></div>}
    <Dialog open={confirmation !== null} title={confirmTitle} closeLabel={text('取消', 'Cancel')} closeDisabled={busy} onClose={() => setConfirmation(null)}
      footer={<>
        <button className="btn" type="button" disabled={busy} onClick={() => setConfirmation(null)}>{text('取消', 'Cancel')}</button>
        <button className={removing ? 'btn btn-danger' : 'btn btn-primary'} type="button" disabled={busy}
          onClick={() => { if (confirmation) void action(confirmation); }}>{confirmLabel}</button>
      </>}>
      <p>{removing
        ? text('进行中的调用会停止。请先在 Agent 中恢复由 HiRoute 管理的设置。重新安装服务后可继续使用已保存的路由和历史记录。', 'Current requests will stop. Restore any HiRoute-managed settings in Agents first. Your saved routes and history will be available after reinstalling the service.')
        : text('现有调用可能暂时中断。请先等待当前调用和 Worker 任务完成，再确认重启。', 'Current calls may be interrupted. Wait for current calls and Worker tasks to finish before restarting.')}</p>
    </Dialog>
    {error && <div className="settings-inline-error" role="status">{error === 'RUNTIME_AGENT_RESTORE_REQUIRED'
      ? text('请在 Agent 中检查并恢复由 HiRoute 管理的设置，然后重试。', 'Review and restore HiRoute-managed settings in Agents, then try again.')
      : error === 'RUNTIME_REMOVE_BLOCKED'
        ? text('无法验证运行时或完成移除。请检查当前用户的服务状态后重试。', 'Could not verify or remove the runtime. Check the current user’s service and try again.')
        : error}</div>}
  </div></section>;
}
