import { storageRecoveryText, startupFailureText } from '../../desktop/src/storage-recovery';
import { bindShellPresentation, currentShellPresentation, listenShellPresentation, updateShellPresentation, type ShellPresentation } from '../../desktop/src/ui/shell-presentation';
import '../../desktop/src/occami/styles.css';
import './status.css';

declare global {
  interface Window {
    __TAURI_INTERNALS__: { invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> };
  }
}

type Report = {
  phase: 'starting' | 'waiting' | 'ready' | 'failed';
  code: string | null;
  logs_path: string;
  storage_recovery?: unknown;
};

const linux = '__HIROUTE_LINUX_SHELL__' in window;
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
let language = navigator.language.startsWith('zh') ? 'zh' : 'en';
let dark = systemTheme.matches;
let report: Report = { phase: 'starting', code: null, logs_path: '' };
let active = false;
let error = '';
const element = (id: string) => document.getElementById(id)!;
const button = (id: string) => element(id) as HTMLButtonElement;
const input = (id: string) => element(id) as HTMLInputElement;
const choice = element('linux-theme') as HTMLSelectElement;
const invoke = <T,>(command: string, args?: Record<string, unknown>) => window.__TAURI_INTERNALS__.invoke<T>(command, args);

function render() {
  const preference = currentShellPresentation();
  const zh = linux && preference.language !== 'system' ? preference.language === 'zh' : language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const failed = report.phase === 'failed';
  const listener = failed && report.code === 'CONSOLE_LISTENER_UNAVAILABLE';
  const repair = failed && report.code === 'SHELL_INSTALL_RECOVERY_REQUIRED' && !report.storage_recovery;
  const setup = linux && failed && report.code === 'INSTALLATION_REQUIRED';
  const browser = linux && failed && (report.code === 'CONSOLE_NATIVE_RELEASE_UNAVAILABLE' || report.code === 'CONSOLE_NATIVE_ASSETS_UNAVAILABLE');
  const foreign = linux && failed && ['SERVICE_OWNER_UNVERIFIED', 'SERVICE_RELEASE_UNVERIFIED'].includes(report.code ?? '');
  const platform = linux ? 'linux' : 'windows';
  const recovery = startupFailureText(report.code, report.storage_recovery, zh ? 'zh' : 'en', platform);
  document.documentElement.lang = zh ? 'zh-CN' : 'en';
  element('root').dataset.theme = (linux ? preference.theme === 'system' ? systemTheme.matches : preference.theme === 'dark' : dark) ? 'dark' : 'light';
  element('root').classList.toggle('linux-startup', linux);
  element('phase').textContent = setup ? text('设置 HiRoute', 'Set up HiRoute')
    : browser ? text('可在浏览器中继续', 'Continue in your browser')
    : repair ? text('安装尚未完成', 'Installation is incomplete')
    : foreign ? text('无法识别现有服务', 'Could not verify the existing service')
    : listener ? text('本机控制台尚未就绪', 'The local console is unavailable')
    : failed ? text('无法启动本机服务', 'Could not start the local service')
    : text('正在连接 HiRoute 服务', 'Connecting to HiRoute');
  element('explanation').textContent = setup
    ? text('为当前用户安装后台服务和 CLI，并启用仅本机访问的控制台。安装使用软件包内的文件。', 'Install the background service and CLI for this account and enable a loopback console. Setup uses files already in the package.')
    : browser ? text('当前服务可以继续处理请求。要使用原生窗口，桌面包、后台服务和控制台需要来自同一构建。', 'Your background service can keep handling requests. The native window requires the Desktop package, service and console from the same build.')
    : foreign ? text('已安装的文件或服务归属无法校验。请检查现有安装及服务定义后再试。', 'The installed files or service ownership could not be verified. Check the existing installation and service definition, then try again.')
    : listener ? text('后台服务已就绪，但本机控制台未监听。控制台可能已被关闭，或指定端口被占用。', 'The background service is ready, but the local console is not listening. It may be disabled, or its configured port may be in use.')
    : repair && linux ? text('恢复会检查已有安装记录，完成或回退中断的文件替换。请保留现有程序、数据和安装记录。', 'Recovery checks the existing installation record and completes or rolls back the interrupted file replacement. Keep the existing programs, data and installation record.')
    : repair ? text('安装尚未完成。请保留全部数据、完整程序和安装记录，重新运行刚才相同版本及构建的完整安装程序，以继续修复当前版本。不要改装更旧版本；缺少原安装程序时请寻求支持。', 'Installation is incomplete. Keep all data, complete programs and installation journals. Re-run the same complete installer, with the same version and build, to finish repairing the current version. Do not install an older version; obtain support if the original installer is unavailable.')
    : failed ? recovery.reason + ' ' + recovery.instructions
    : report.phase === 'waiting' ? text('服务正在准备控制台与 Gateway…', 'Preparing the console and Gateway…')
    : text('正在检查并启动后台服务…', 'Checking and starting the background service…');
  element('recovery-material').hidden = !failed || listener || !recovery.backupBatch;
  element('batch-label').textContent = text('已校验的原批次：', 'Verified original batch: ');
  element('batch').textContent = recovery.backupBatch ?? '';
  element('source-label').textContent = text('恢复目录：', 'Recovery directory: ');
  element('source').textContent = recovery.backupDirectory ?? '';
  element('guide-location').textContent = recovery.locationHint ?? '';
  element('console-recovery').hidden = !listener;
  element('recovery-explanation').textContent = linux
    ? text('如需启用本机控制台，请在终端运行以下命令，使用自动选择的本机端口。', 'To enable the local console, run the following commands in your terminal. HiRoute will select a loopback port.')
    : text('如需恢复本机界面，请在 PowerShell 中运行以下命令，使用自动选择的本机端口：', 'To restore the local interface, run these commands in PowerShell to use an automatically selected local port:');
  element('recovery-warning').textContent = text('重启服务可能中断正在进行的调用和任务。完成后点击“重新检查”。', 'Restarting the service may interrupt active calls and tasks. Then select Recheck.');
  element('failure').textContent = failed && (!linux || !setup && !browser) ? report.code ?? '' : '';
  element('action-error').hidden = !error;
  element('action-error').textContent = error;
  element('logs-label').textContent = text('本机日志目录', 'Local log directory');
  input('logs').value = report.logs_path;
  element('logs-label').hidden = input('logs').hidden = linux && (!failed || setup || browser || !report.logs_path);
  element('login-choice').hidden = !setup;
  element('login-label').textContent = text('登录时启动后台服务', 'Start the background service at login');
  button('retry').hidden = !failed || browser || (repair && !linux) || (!!report.storage_recovery && !storageRecoveryText(report.storage_recovery, zh ? 'zh' : 'en', platform).canRetry);
  button('retry').textContent = setup ? text('安装后台服务', 'Install background service')
    : repair ? text('恢复安装', 'Recover installation')
    : listener ? text('重新检查', 'Recheck') : text('重试', 'Try again');
  button('browser').hidden = !browser;
  button('browser').textContent = text('在浏览器中打开', 'Open in browser');
  button('retry').disabled = button('browser').disabled = active;
  button('language').textContent = zh ? 'English' : '中文';
  button('theme').hidden = linux;
  button('theme').textContent = dark ? text('浅色', 'Light') : text('深色', 'Dark');
  element('theme-choice').hidden = !linux;
  element('theme-label').textContent = text('外观', 'Appearance');
  choice.options[0].textContent = text('跟随系统', 'Follow system');
  choice.options[1].textContent = text('浅色', 'Light');
  choice.options[2].textContent = text('深色', 'Dark');
  choice.value = preference.theme;
}

async function action(command: string, args?: Record<string, unknown>) {
  if (active) return;
  active = true;
  error = '';
  render();
  try { await invoke(command, args); }
  catch (cause) {
    error = typeof cause === 'string' && /^[A-Z_]+$/.test(cause) ? cause : 'SHELL_ACTION_UNAVAILABLE';
  } finally {
    active = false;
    render();
  }
}

button('language').onclick = () => {
  const current = document.documentElement.lang.startsWith('zh');
  if (linux) updateShellPresentation({ language: current ? 'en' : 'zh' });
  else { language = current ? 'en' : 'zh'; render(); }
};
button('theme').onclick = () => { dark = !dark; render(); };
choice.onchange = () => updateShellPresentation({ theme: choice.value as ShellPresentation['theme'] });
input('logs').onfocus = () => input('logs').select();
button('retry').onclick = () => {
  const code = report.code;
  if (linux && code === 'INSTALLATION_REQUIRED') void action('shell_install_runtime', { loginStart: input('login-start').checked });
  else if (linux && code === 'SHELL_INSTALL_RECOVERY_REQUIRED') void action('shell_recover_runtime');
  else void action('shell_retry_startup');
};
button('browser').onclick = () => void action('shell_open_legacy_browser');
if (linux) {
  systemTheme.addEventListener('change', render);
  listenShellPresentation(render);
  window.addEventListener('hiroute-shell-presentation-unavailable', () => { error = 'SHELL_PRESENTATION_UNAVAILABLE'; render(); });
  void invoke<ShellPresentation>('shell_presentation_get')
    .then(value => bindShellPresentation(value, next => invoke('shell_presentation_set', { value: next })))
    .catch(() => bindShellPresentation(undefined, next => invoke('shell_presentation_set', { value: next })));
}

async function poll() {
  try { report = await invoke<Report>('shell_status'); }
  catch { report = { ...report, phase: 'failed', code: 'SHELL_STATUS_UNAVAILABLE' }; }
  render();
  window.setTimeout(() => void poll(), 750);
}
render();
void poll();
