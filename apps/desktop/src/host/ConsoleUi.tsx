import { useEffect, useState } from 'react';
import { Dialog, UiIcon } from '../ui';
import { hostKind, plaintextConsole } from './index.ts';
import { logoutConsole } from './console.ts';
export function SecretTransportWarning({ language }: { language: 'zh' | 'en' }) {
  if (!plaintextConsole() || hostKind() === 'tauri') return null;
  return <p className="console-secret-warning" role="note">{language === 'zh'
    ? '此连接未加密。提交的密钥或授权内容会以明文通过网络传输；仅在信任的网络中继续。'
    : 'This connection is unencrypted. Submitted keys or authorization details cross the network as plaintext. Continue only on a network you trust.'}</p>;
}
export function HostNotices({ language }: { language: 'zh' | 'en' }) {
  const [path, setPath] = useState<string | null>(null);
  const zh = language === 'zh';
  useEffect(() => {
    const showPath = (event: Event) => { const value = (event as CustomEvent<unknown>).detail; if (typeof value === 'string' && value.length < 32768) setPath(value); };
    window.addEventListener('hiroute-host-path', showPath);
    return () => window.removeEventListener('hiroute-host-path', showPath);
  }, []);
  if (hostKind() === 'tauri') return null;
  return <>
    <div className={`console-transport-bar ${plaintextConsole() ? 'plaintext' : ''}`} role="status">
      <UiIcon name={plaintextConsole() ? 'warning' : 'info'} />
      <div><strong>{plaintextConsole() ? (zh ? '未加密的远程连接' : 'Unencrypted remote connection') : hostKind() === 'shell-console' ? (zh ? 'Windows 本机控制台' : 'Windows local console') : (zh ? '浏览器控制台会话' : 'Browser console session')}</strong>{plaintextConsole() && <span>{zh ? '会话、密钥和操作内容会以明文传输。' : 'Session credentials, keys and actions travel as plaintext.'}</span>}</div>
      {hostKind() === 'console' && <button className="btn btn-quiet" type="button" onClick={() => void logoutConsole()}>{zh ? '退出控制台' : 'Sign out'}</button>}
    </div>
    <Dialog open={path !== null} title={zh ? '本机目录路径' : 'Directory on the service host'} closeLabel={zh ? '关闭' : 'Close'} onClose={() => setPath(null)}>
      <p>{zh ? '请在运行 HiRoute 服务的设备上打开此目录。' : 'Open this directory on the device running the HiRoute service.'}</p>
      <input className="input console-path" readOnly value={path ?? ''} onFocus={event => event.currentTarget.select()} aria-label={zh ? '目录路径' : 'Directory path'} />
    </Dialog>
  </>;
}
