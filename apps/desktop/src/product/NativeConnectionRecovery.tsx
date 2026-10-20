import { useEffect, useState } from 'react';
import { invoke } from '../host/index.ts';
import { safeDiagnosticCode } from '../error-code';
import type { NativeConnectionStatus } from '../native-connection';

export function NativeConnectionRecovery({ status, language, onRecovered, onRefresh }: {
  status?: NativeConnectionStatus; language: 'zh' | 'en'; onRecovered: () => Promise<void>; onRefresh: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { setError(''); }, [status]);
  if (!status || status === 'ready') return null;
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  async function recover() {
    if (busy) return;
    setBusy(true); setError('');
    try { await invoke('recover_native_connection'); await onRecovered(); }
    catch (failure) { setError(safeDiagnosticCode(failure, 'AUTHORITY_RECOVERY_UNAVAILABLE')); await onRefresh(); }
    finally { setBusy(false); }
  }
  return <div className="callout bad native-connection-recovery" role="alert" data-connection-state={status}>
    <div><strong>{status === 'not_owned' ? text('本机服务由其他入口管理', 'Local service managed elsewhere') : text('本机可信连接已失效', 'Trusted local connection lost')}</strong>
      <p>{status === 'not_owned' ? text('本机服务正由其他入口管理，当前窗口无法读取受保护记录。请从该入口正常退出服务，再重新打开 HiRoute。', 'The local service is managed by another entry. This window cannot read protected records. Quit the service normally from that entry, then reopen HiRoute.') : text('本机服务仍可响应，但记录读取和需要授权的操作暂时不可用。恢复会在现有工作完成后重新连接，保留已保存的数据。', 'The local service still responds, but records and protected actions are unavailable. Restore reconnects after existing work finishes and keeps saved data.')}</p>
      {busy && <p role="status">{text('正在安全恢复本机连接…', 'Restoring the local connection safely…')}</p>}
      {error && <p data-error-code={error}>{error === 'AUTHORITY_RECOVERY_WORK_ACTIVE'
        ? text('仍有请求或任务运行，已保留当前服务。请等它们完成后再次恢复。', 'Requests or tasks are still running. The service was kept running; restore again after they finish.')
        : error === 'UPGRADE_STOP_PENDING'
          ? text('正在等待原服务退出，请稍后再次恢复。', 'Waiting for the original service to exit. Restore again shortly.')
          : text('恢复未完成。请等待现有工作结束后，正常退出并重新打开 HiRoute。', 'Restore did not complete. After existing work finishes, quit and reopen HiRoute normally.')}</p>}
      {status === 'restart_required' && <p>{text('安全恢复通道也已不可用。请等现有工作结束后，正常退出并重新打开 HiRoute。', 'The safe recovery channel is also unavailable. After existing work finishes, quit and reopen HiRoute normally.')}</p>}
    </div>
    {status === 'recovery_required' && <button className="btn btn-primary" type="button" disabled={busy} onClick={() => void recover()}>{text('恢复本机连接', 'Restore local connection')}</button>}
  </div>;
}
