import { type PropsWithChildren, useEffect, useRef, useState } from 'react';
import { HiRouteMark, PresentationRoot, UiIcon, usePresentationPreferences } from '../ui';
import { safeDiagnosticCode } from '../error-code';
import { hostKind, initializeShellHost, invoke } from './index.ts';
import { clearConsoleSession, CONSOLE_EXPIRED_EVENT, CONSOLE_UNAVAILABLE_EVENT, exchangeConsoleCredential, readConsoleSession, sessionBearer } from './console.ts';
type Phase = 'loading' | 'login' | 'expired' | 'unavailable' | 'assets' | 'ready';
export function ConsoleRoot({ children }: PropsWithChildren) {
  if (hostKind() === 'tauri') return children;
  return <ConsoleSessionRoot>{children}</ConsoleSessionRoot>;
}
function ConsoleSessionRoot({ children }: PropsWithChildren) {
  const preferences = usePresentationPreferences();
  const zh = preferences.language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const [phase, setPhase] = useState<Phase>('loading');
  const [error, setError] = useState('');
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [hadSession, setHadSession] = useState(false);
  const started = useRef(false);
  const shell = hostKind() === 'shell-console';
  const reconnecting = useRef(false);
  async function reconnectShell() {
    if (reconnecting.current) return;
    reconnecting.current = true; setHadSession(false); setPhase('loading'); setToken('');
    try { await invoke('reconnect'); }
    catch (cause) { setError(safeDiagnosticCode(cause, 'SHELL_RECONNECT_FAILED')); setPhase('unavailable'); }
    finally { reconnecting.current = false; }
  }
  const plaintext = location.protocol === 'http:' && !['127.0.0.1', 'localhost', '[::1]', '::1'].includes(location.hostname);
  const accept = (identity: Awaited<ReturnType<typeof readConsoleSession>>) => { setHadSession(true); setPhase(identity.assets_available ? 'ready' : 'assets'); setError(''); };
  async function retry() {
    if (shell && !sessionBearer()) { await reconnectShell(); return; }
    setBusy(true); setError('');
    try { await initializeShellHost(); if (!sessionBearer()) { setPhase('login'); return; } accept(await readConsoleSession()); }
    catch (cause) { const code = safeDiagnosticCode(cause, 'CONSOLE_UNAVAILABLE'); setError(code); setPhase(code.includes('AUTH') || code.includes('CREDENTIAL') ? 'expired' : 'unavailable'); }
    finally { setBusy(false); }
  }
  useEffect(() => {
    const expired = () => { if (shell) { void reconnectShell(); return; } setHadSession(false); setPhase('expired'); setToken(''); };
    const unavailable = () => setPhase(current => current === 'ready' ? 'unavailable' : current);
    window.addEventListener(CONSOLE_EXPIRED_EVENT, expired);
    window.addEventListener(CONSOLE_UNAVAILABLE_EVENT, unavailable);
    if (!started.current) {
      started.current = true;
      const values = new URLSearchParams(location.search).getAll('code');
      // Clear the launch credential before any await, rendering, or secondary request.
      if (location.pathname === '/launch') history.replaceState(null, '', '/');
      if (values.length) {
        setBusy(true);
        void initializeShellHost().then(() => {
          if (values.length !== 1 || !/^[a-f0-9]{64}$/.test(values[0])) throw { code: 'CONSOLE_CREDENTIAL_INVALID' };
          return exchangeConsoleCredential('launch', values[0]);
        }).then(accept).catch(cause => { clearConsoleSession(); if (shell) { void reconnectShell(); return; } setError(safeDiagnosticCode(cause, 'CONSOLE_CREDENTIAL_INVALID')); setPhase('login'); }).finally(() => setBusy(false));
      } else void retry();
    }
    return () => { window.removeEventListener(CONSOLE_EXPIRED_EVENT, expired); window.removeEventListener(CONSOLE_UNAVAILABLE_EVENT, unavailable); };
  }, []);
  async function login(event: React.FormEvent) {
    event.preventDefault(); if (busy) return;
    const input = token; setToken(''); setBusy(true); setError('');
    try { accept(await exchangeConsoleCredential('token', input)); }
    catch (cause) { setError(safeDiagnosticCode(cause, 'CONSOLE_CREDENTIAL_INVALID')); }
    finally { setBusy(false); }
  }
  const loginPage = !shell && (phase === 'login' || phase === 'expired');
  return <>
    {hadSession && <div className="console-product" hidden={phase !== 'ready'} inert={phase !== 'ready'}>{children}</div>}
    {phase !== 'ready' && <PresentationRoot language={preferences.language} theme={preferences.resolvedTheme} textScale={1}>
      <main className="console-entry"><section className="console-entry-card">
        <div className="console-entry-brand"><HiRouteMark /><strong>HiRoute</strong></div>
        <h1>{phase === 'loading' ? text('正在连接控制台', 'Connecting to your console')
          : phase === 'expired' ? text('控制台会话已结束', 'Your console session has ended')
            : phase === 'assets' ? text('控制台文件不可用', 'Console files are unavailable')
              : phase === 'unavailable' ? text('暂时无法连接服务', 'The service is unavailable')
                : text('登录控制台', 'Sign in to the console')}</h1>
        {loginPage && <>
          <p>{text('在服务所在设备运行 hiroute console open 获取一次性登录链接，或输入由 hiroute console token create 创建的 access token。', 'On the service host, run hiroute console open for a single-use sign-in link, or enter an access token created with hiroute console token create.')}</p>
          {plaintext && <div className="callout warn" role="note"><UiIcon name="warning" /><span>{text('当前连接未加密，access token 将以明文通过网络发送。仅在信任的网络中继续。', 'This connection is unencrypted. Your access token will travel over the network as plaintext. Continue only on a network you trust.')}</span></div>}
          <form className="console-login" autoComplete="off" onSubmit={event => void login(event)}>
            <label className="field"><span className="field-label">Access token</span><input className="input" type="password" name="access-token" autoComplete="off" spellCheck={false} required maxLength={64} value={token} disabled={busy} onChange={event => setToken(event.currentTarget.value)} autoFocus /></label>
            <button className="btn btn-primary" type="submit" disabled={busy || !token.trim()}>{busy ? text('正在登录…', 'Signing in…') : text('登录', 'Sign in')}</button>
          </form>
        </>}
        {phase === 'unavailable' && <><p>{text('请确认服务正在运行，并检查当前控制台地址。现有数据与操作不会由此页面清除。', 'Check that the service is running and that this console address is current. Your saved data and operations are preserved.')}</p><button className="btn btn-primary" disabled={busy} onClick={() => void retry()}>{busy ? text('正在连接…', 'Connecting…') : text('重试连接', 'Try again')}</button></>}
        {phase === 'assets' && <p>{text('请在服务所在设备重新安装完整的 HiRoute 安装包。', 'Reinstall the complete HiRoute package on the service host.')} <code>CONSOLE_ASSETS_UNAVAILABLE</code></p>}
        {phase === 'loading' && <p role="status">{text('正在读取实际服务状态…', 'Reading the service status…')}</p>}
        {error && <p className="console-entry-error" role="alert">{text('未能完成连接', 'Could not complete the connection')} <code>{error}</code></p>}
        {error === 'CONSOLE_SESSION_LIMIT' && <p>{text('请退出其他设备上的控制台会话，或等待会话过期后重试。', 'Sign out of a console session on another device, or wait for a session to expire and try again.')}</p>}
        {error === 'CONSOLE_RATE_LIMITED' && <p>{text('登录失败次数过多，请等待一分钟后重试。远程登录失败不会阻止本机启动链接。', 'Too many failed sign-in attempts. Wait one minute and try again. Remote sign-in failures do not block local launch links.')}</p>}
        <div className="console-entry-preferences"><button className="btn btn-quiet" onClick={() => preferences.setLanguage(zh ? 'en' : 'zh')}>{zh ? 'English' : '中文'}</button><button className="btn btn-quiet" onClick={() => preferences.setTheme(preferences.resolvedTheme === 'dark' ? 'light' : 'dark')}>{preferences.resolvedTheme === 'dark' ? text('浅色', 'Light') : text('深色', 'Dark')}</button></div>
      </section></main>
    </PresentationRoot>}
  </>;
}
