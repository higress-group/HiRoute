import { invoke, hostCapabilities } from '../host/index.ts';
import { useEffect, useState } from 'react';
import { safeDiagnosticCode } from '../error-code';
import type { Language } from '../ui/preferences';

type UpdateView = {
  current_version: string;
  phase: 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'verifying' | 'ready' | 'waiting' | 'installing' | 'failed';
  available: { version: string; notes: { zh: string; en: string }; size: number } | null;
  downloaded_bytes: number;
  active_calls: number;
  active_tasks: number;
  error: string | null;
  can_install: boolean;
  package_ready: boolean;
  recovery: 'service_available' | 'stop_pending' | 'restart_required' | null;
};
const busyPhases = ['checking', 'downloading', 'verifying', 'waiting', 'installing'];
export function UpdatesSettings({ active, language, onServiceChanged }: { active: boolean; language: Language; onServiceChanged: () => Promise<void> }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [view, setView] = useState<UpdateView | null>(null);
  const [error, setError] = useState('');
  const [actionPending, setActionPending] = useState(false);
  const busy = view !== null && busyPhases.includes(view.phase);
  const recovery = view?.recovery;
  useEffect(() => {
    if (recovery) void onServiceChanged();
  }, [recovery, onServiceChanged]);
  useEffect(() => {
    if (!active && !busy) return;
    let current = true;
    const read = () => void invoke<UpdateView>('update_status').then(value => {
      if (current) setView(value);
    }).catch(failure => { if (current) setError(safeDiagnosticCode(failure, 'UPGRADE_STATUS_UNAVAILABLE')); });
    read();
    const timer = window.setInterval(read, 1000);
    return () => { current = false; window.clearInterval(timer); };
  }, [active, busy]);
  const act = async (command: 'update_check' | 'update_download' | 'update_install' | 'update_cancel') => {
    setActionPending(true); setError('');
    try { setView(await invoke<UpdateView>(command)); }
    catch (failure) { setError(safeDiagnosticCode(failure, 'UPGRADE_UNAVAILABLE')); }
    finally { setActionPending(false); }
  };
  const status = !view ? text('正在读取版本信息…', 'Reading version information…')
    : view.phase === 'checking' ? text('正在检查官网发布版本…', 'Checking official releases…')
    : view.phase === 'current' ? text('已是最新稳定版本', 'The latest stable version is installed')
    : view.phase === 'available' ? text(`可升级到 ${view.available?.version}`, `Version ${view.available?.version} is available`)
    : view.phase === 'downloading' ? text(`正在下载 ${view.available?.version} · ${Math.min(100, Math.floor(view.downloaded_bytes / (view.available?.size || 1) * 100))}%`, `Downloading ${view.available?.version} · ${Math.min(100, Math.floor(view.downloaded_bytes / (view.available?.size || 1) * 100))}%`)
    : view.phase === 'verifying' ? text('正在校验安装包…', 'Verifying the installation package…')
    : view.phase === 'ready' ? text(`安装包已就绪：${view.available?.version}`, `Package ready: ${view.available?.version}`)
    : view.phase === 'waiting' ? text(`等待 ${view.active_calls} 个调用、${view.active_tasks} 个任务结束；可取消升级继续使用。`, `Waiting for ${view.active_calls} call(s) and ${view.active_tasks} task(s); cancel to continue using HiRoute.`)
    : view.phase === 'installing' ? text('正在停止服务并安装，完成后会重新打开 HiRoute。', 'Stopping the service and installing. HiRoute will reopen when complete.')
    : view.phase === 'failed' ? text('升级未完成，请查看错误及启动页中的恢复说明。', 'The upgrade did not complete. See the error and startup recovery instructions.')
    : text('从官网获取稳定版本', 'Stable releases from the official website');
  const code = view?.error || error;
  const canCancel = view && ['downloading', 'verifying', 'waiting'].includes(view.phase);
  const restartRequired = recovery === 'stop_pending' || recovery === 'restart_required';
  return <section className="settings-group"><h2>{text('版本升级', 'Updates')}</h2><div className="settings-list">
    <div className="settings-row"><div className="settings-copy"><strong>HiRoute {view?.current_version ?? ''}</strong><span role="status">{status}</span></div>
      <div className="worker-settings-control">
        {!busy && <button className="btn" type="button" disabled={actionPending} onClick={() => void act('update_check')}>{text('检查更新', 'Check for updates')}</button>}
        {view && !busy && view.available && !view.package_ready && <button className="btn" type="button" disabled={actionPending || !view.can_install} onClick={() => void act('update_download')}>{text('下载更新', 'Download update')}</button>}
        {view?.package_ready && !busy && <button className="btn" type="button" disabled={actionPending || restartRequired} onClick={() => void act('update_install')}>{text('安装并重启', 'Install and restart')}</button>}
        {canCancel && <button className="btn" type="button" onClick={() => void act('update_cancel')}>{text('取消升级', 'Cancel update')}</button>}
      </div>
    </div>
    {view?.available && !busy && <div className="gateway-risk">{view.available.notes[language]}</div>}
    {view?.phase === 'ready' && <div className="gateway-risk">{hostCapabilities().host === 'shell-console' ? text('安装前会等待现有调用和任务结束，可以取消等待。安装期间 Agent 路由会暂时不可用；失败时会尝试恢复旧版本，启动页会显示需要处理的恢复问题。', 'Installation waits for current calls and tasks; you can cancel the wait. Agent routing pauses during installation. A failed update attempts to restore the previous version; the startup page shows any recovery that needs attention.') : text('安装前会等待现有调用和任务结束。安装与首次启动期间，Agent 路由会暂时不可用；需要时可按备份目录中的说明恢复旧 App 和旧数据。', 'Installation waits for current calls and tasks. Agent routing is temporarily unavailable during installation and first startup. If needed, follow the backup instructions to restore the previous App and data.')}</div>}
    {recovery === 'service_available' && <div className="gateway-risk">{text('升级未完成，旧版本服务已恢复，可继续使用或重试安装。', 'The upgrade did not complete. The previous service is available; continue using HiRoute or retry installation.')}</div>}
    {recovery === 'stop_pending' && <div className="settings-inline-error" role="status">{text('服务仍在停止，尚未替换应用。请等待服务退出后，完全退出并重新打开 HiRoute。', 'The service is still stopping and the App has not been replaced. Wait for the service to exit, then fully quit and reopen HiRoute.')}</div>}
    {recovery === 'restart_required' && <div className="settings-inline-error" role="status">{text('尚未替换应用，旧版本服务未能恢复。请完全退出并重新打开 HiRoute，并查看启动页的恢复说明。', 'The App has not been replaced, but the previous service could not be restored. Fully quit and reopen HiRoute and follow the startup recovery instructions.')}</div>}
    {code && <div className="settings-inline-error" role="status">{text(`升级不可用（${code}）。可重试检查，或从官网下载安装包并完全退出 HiRoute 后覆盖安装。`, `Update unavailable (${code}). Check again, or download from the official website and fully quit HiRoute before replacing the App.`)}</div>}
    {view && !view.can_install && <div className="gateway-risk">{text('应用内安装需要使用完整的 HiRoute App。', 'In-app installation requires a complete HiRoute App.')}</div>}
  </div></section>;
}
