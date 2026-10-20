import { storageRecoveryText, startupFailureText } from '../../desktop/src/storage-recovery';
import '../../desktop/src/occami/styles.css';
import './status.css';
declare global { interface Window { __TAURI_INTERNALS__: { invoke<T>(command: string): Promise<T> } } }
type Report = { phase: 'starting' | 'waiting' | 'ready' | 'failed'; code: string | null; logs_path: string; storage_recovery?: unknown };
let language = navigator.language.startsWith('zh') ? 'zh' : 'en';
let dark = matchMedia('(prefers-color-scheme: dark)').matches;
let report: Report = { phase: 'starting', code: null, logs_path: '' };
const element = (id: string) => document.getElementById(id)!;
function render() {
  const zh = language === 'zh'; const text = (cn: string, en: string) => zh ? cn : en;
  const listenerUnavailable = report.phase === 'failed' && report.code === 'CONSOLE_LISTENER_UNAVAILABLE';
  const repairInstaller = report.phase === 'failed' && report.code === 'SHELL_INSTALL_RECOVERY_REQUIRED' && !report.storage_recovery;
  const recovery = startupFailureText(report.code, report.storage_recovery, language, 'windows');
  document.documentElement.lang = zh ? 'zh-CN' : 'en';
  element('root').dataset.theme = dark ? 'dark' : 'light';
  element('phase').textContent = repairInstaller ? text('安装尚未完成', 'Installation is incomplete') : listenerUnavailable ? text('本机控制台尚未就绪', 'The local console is unavailable') : report.phase === 'failed' ? text('无法启动本机服务', 'Could not start the local service') : text('正在连接 HiRoute 服务', 'Connecting to HiRoute');
  element('explanation').textContent = listenerUnavailable ? text('后台服务已就绪，但本机控制台未监听。控制台可能已被关闭，或指定端口被占用。', 'The background service is ready, but the local console is not listening. It may be disabled, or its configured port may be in use.') : report.phase === 'failed' ? repairInstaller ? text('安装尚未完成。请保留全部数据、完整程序和安装记录，重新运行刚才相同版本及构建的完整安装程序，以继续修复当前版本。不要改装更旧版本；缺少原安装程序时请寻求支持。', 'Installation is incomplete. Keep all data, complete programs and installation journals. Re-run the same complete installer, with the same version and build, to finish repairing the current version. Do not install an older version; obtain support if the original installer is unavailable.') : recovery.reason + ' ' + recovery.instructions : report.phase === 'waiting' ? text('服务正在准备控制台与 Gateway…', 'Preparing the console and Gateway…') : text('正在检查并启动后台服务…', 'Checking and starting the background service…');
  element('recovery-material').hidden = report.phase !== 'failed' || listenerUnavailable || !recovery.backupBatch;
  element('batch-label').textContent = text('已校验的原批次：', 'Verified original batch: ');
  element('batch').textContent = recovery.backupBatch ?? '';
  element('source-label').textContent = text('恢复目录：', 'Recovery directory: ');
  element('source').textContent = recovery.backupDirectory ?? '';
  element('guide-location').textContent = recovery.locationHint ?? '';
  element('console-recovery').hidden = !listenerUnavailable;
  element('recovery-explanation').textContent = text('如需恢复本机界面，请在 PowerShell 中运行以下命令，使用自动选择的本机端口：', 'To restore the local interface, run these commands in PowerShell to use an automatically selected local port:');
  element('recovery-warning').textContent = text('重启服务可能中断正在进行的调用和任务。完成后点击“重新检查”；此按钮只检查状态，不会修改监听设置。', 'Restarting the service may interrupt active calls and tasks. Then select Recheck; the button only checks status and does not change listener settings.');
  element('failure').textContent = report.code ?? '';
  element('logs-label').textContent = text('本机日志目录', 'Local log directory');
  (element('logs') as HTMLInputElement).value = report.logs_path;
  element('retry').hidden = report.phase !== 'failed' || repairInstaller || (!!report.storage_recovery && !storageRecoveryText(report.storage_recovery, language, 'windows').canRetry); element('retry').textContent = listenerUnavailable ? text('重新检查', 'Recheck') : text('重试', 'Try again');
  element('language').textContent = zh ? 'English' : '中文'; element('theme').textContent = dark ? text('浅色', 'Light') : text('深色', 'Dark');
}
element('language').onclick = () => { language = language === 'zh' ? 'en' : 'zh'; render(); };
element('theme').onclick = () => { dark = !dark; render(); };
element('logs').onfocus = () => (element('logs') as HTMLInputElement).select();
element('retry').onclick = () => { (element('retry') as HTMLButtonElement).disabled = true; void window.__TAURI_INTERNALS__.invoke('shell_retry_startup').finally(() => { (element('retry') as HTMLButtonElement).disabled = false; }); };
async function poll() {
  try { report = await window.__TAURI_INTERNALS__.invoke<Report>('shell_status'); }
  catch { report = { ...report, phase: 'failed', code: 'SHELL_STATUS_UNAVAILABLE' }; }
  render(); window.setTimeout(() => void poll(), 750);
}
render(); void poll();
