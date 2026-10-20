import { invoke, hostCapabilities } from '../host/index.ts';
import { useEffect, useState } from 'react';
import { safeDiagnosticCode } from '../error-code';
import type { Language } from '../ui/preferences';

type UpdateView = {
  current_version: string;
  phase: 'idle' | 'checking' | 'check_failed' | 'current' | 'available' | 'downloading' | 'verifying' | 'ready' | 'waiting' | 'installing' | 'failed';
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
    : view.phase === 'check_failed' ? view.error === 'UPGRADE_CATALOG_UNAVAILABLE' ? text('Windows 更新目录暂不可用，可稍后重试检查。', 'The Windows update catalog is unavailable. Try checking again later.') : text('检查更新失败，可稍后重试。', 'Could not check for updates. Try again later.')
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
    {view?.phase === 'ready' && <div className="gateway-risk">{text('如果本次升级需要切换存储格式，将保留配置、凭据与路由，并封存旧 HiRoute 历史。当前历史和统计从切换后开始；原生 Agent 的历史不变。旧历史需用完整旧数据组和配套旧程序恢复查看。', 'When this update changes the storage format, it retains configuration, credentials and routing and seals old HiRoute history. Current history and statistics begin at cutover; native Agent history is unchanged. Access old history by recovering the complete old group with its matching program.')}</div>}
    {view?.phase === 'ready' && <div className="gateway-risk">{hostCapabilities().host === 'shell-console' ? text('安装前会等待现有调用和任务结束，可以取消等待。安装期间 Agent 路由会暂时不可用；失败时会尝试恢复旧版本，启动页会显示需要处理的恢复问题。', 'Installation waits for current calls and tasks; you can cancel the wait. Agent routing pauses during installation. A failed update attempts to restore the previous version; the startup page shows any recovery that needs attention.') : text('安装前会等待现有调用和任务结束。安装与首次启动期间，Agent 路由会暂时不可用；需要时可按备份目录中的说明恢复旧 App 和旧数据。', 'Installation waits for current calls and tasks. Agent routing is temporarily unavailable during installation and first startup. If needed, follow the backup instructions to restore the previous App and data.')}</div>}
    {recovery === 'service_available' && <div className="gateway-risk">{text('升级未完成，本机服务可用。请保留数据和升级材料，按对应恢复指引继续处理。', 'The upgrade did not complete. The local service is available. Keep data and upgrade material and follow the applicable recovery instructions.')}</div>}
    {recovery === 'stop_pending' && <div className="settings-inline-error" role="status">{text('服务仍在停止，尚未替换应用。请等待服务退出后，完全退出并重新打开 HiRoute。', 'The service is still stopping and the App has not been replaced. Wait for the service to exit, then fully quit and reopen HiRoute.')}</div>}
    {recovery === 'restart_required' && <div className="settings-inline-error" role="status">{text('服务未能恢复。请保留数据、旧包与升级记录，重新打开 HiRoute 并按启动页的具体指引处理。', 'The service could not be restored. Keep data, old packages and upgrade journals. Reopen HiRoute and follow the specific startup recovery instructions.')}</div>}
    {code === 'UPGRADE_PREFLIGHT_UNAVAILABLE' && <div className="settings-inline-error">{text('此候选无法执行所需升级预检。请保留正常运行的原版和数据，使用已验证的过渡版本或联系支持；不要手动覆盖绕过此拒绝。', 'This candidate cannot perform the required upgrade preflight. Keep the working original installation and data. Use a verified transition release or obtain support; do not bypass this refusal with manual replacement.')}</div>}
    {['UPGRADE_KEY_UNAVAILABLE', 'UPGRADE_RECOVERY_PACKAGE_INVALID', 'UPGRADE_RECOVERY_PACKAGE_MISSING', 'UPGRADE_PREFLIGHT_INVALID', 'UPGRADE_PREFLIGHT_REFUSED'].includes(code ?? '') && <div className="settings-inline-error">{text('升级前提校验未通过。请保留当前程序和全部数据，核对原密钥、配套旧包及本机启动诊断；修复缺失或损坏的材料后再升级。无法取得必要材料时请寻求支持，不要反复重试或直接覆盖安装。', 'Upgrade prerequisites could not be verified. Keep the current program and all data. Check original keys, the matching complete previous package and local startup diagnostics. Correct missing or damaged material before upgrading. Obtain support if essential material is unavailable; repeated retries or manual overwrite cannot reconstruct it.')}</div>}
    {code === 'UPGRADE_INSUFFICIENT_SPACE' && <div className="settings-inline-error">{text('数据所在磁盘缺少恢复与启动空间。请保留封存历史和旧包，释放无关文件后再重试。', 'The storage volume lacks recovery/startup space. Keep sealed history and old packages, free unrelated files, then retry.')}</div>}
    {code && <div className="settings-inline-error" role="status">{view?.phase === 'check_failed' ? text(`检查未完成（${code}）。本次检查未替换程序。请保留当前安装和数据，恢复连接后再检查。`, `The check did not complete (${code}). This check did not replace the program. Keep the installation and data and check again after restoring connectivity.`) : text(`升级未完成（${code}）。请保留原数据和完整旧包，按启动页的具体指引处理。缺少材料、空间或权限的问题解决前，请勿覆盖安装或直接降级。`, `The update did not complete (${code}). Keep the original data and complete previous package and follow the specific startup instructions. Resolve missing material, space or access before replacing the program or downgrading.`)}</div>}
    {view && !view.can_install && <div className="gateway-risk">{text('应用内安装需要使用完整的 HiRoute App。', 'In-app installation requires a complete HiRoute App.')}</div>}
  </div></section>;
}
