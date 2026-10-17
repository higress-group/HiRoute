import '../../desktop/src/occami/styles.css';
import './status.css';
declare global { interface Window { __TAURI_INTERNALS__: { invoke<T>(command: string): Promise<T> } } }
type Report = { phase: 'starting' | 'waiting' | 'ready' | 'failed'; code: string | null; logs_path: string };
let language = navigator.language.startsWith('zh') ? 'zh' : 'en';
let dark = matchMedia('(prefers-color-scheme: dark)').matches;
let report: Report = { phase: 'starting', code: null, logs_path: '' };
const element = (id: string) => document.getElementById(id)!;
function render() {
  const zh = language === 'zh'; const text = (cn: string, en: string) => zh ? cn : en;
  document.documentElement.lang = zh ? 'zh-CN' : 'en';
  element('root').dataset.theme = dark ? 'dark' : 'light';
  element('phase').textContent = report.phase === 'failed' ? text('无法启动本机服务', 'Could not start the local service') : text('正在连接 HiRoute 服务', 'Connecting to HiRoute');
  element('explanation').textContent = report.phase === 'failed' ? text('请保留现有数据，在下方日志目录查看详情后重试。', 'Keep your existing data. Check the log directory below, then try again.') : report.phase === 'waiting' ? text('服务正在准备控制台与 Gateway…', 'Preparing the console and Gateway…') : text('正在检查并启动后台服务…', 'Checking and starting the background service…');
  element('failure').textContent = report.code ?? '';
  element('logs-label').textContent = text('本机日志目录', 'Local log directory');
  (element('logs') as HTMLInputElement).value = report.logs_path;
  element('retry').hidden = report.phase !== 'failed'; element('retry').textContent = text('重试', 'Try again');
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
