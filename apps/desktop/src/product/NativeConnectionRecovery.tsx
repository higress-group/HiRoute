import { useEffect, useState } from 'react';
import { invoke } from '../host/index.ts';
import { safeDiagnosticCode } from '../error-code';
import type { NativeConnectionStatus } from '../native-connection';

export function NativeConnectionRecovery({ status, language, serviceResponsive, onRecovered, onRefresh }: {
  status?: NativeConnectionStatus; language: 'zh' | 'en'; serviceResponsive: boolean; onRecovered: () => Promise<void>; onRefresh: () => Promise<void>;
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
      <p>{status === 'not_owned' ? text('本机服务正由其他入口管理，当前窗口无法读取受保护记录。请从该入口正常退出服务，再重新打开 HiRoute。', 'The local service is managed by another entry. This window cannot read protected records. Quit the service normally from that entry, then reopen HiRoute.') : !serviceResponsive ? text('当前无法确认本机服务状态。恢复连接会保留已保存的数据，不会重启服务。', 'The current local service state cannot be confirmed. Restore keeps saved data without restarting the service.') : text('本机服务仍可响应，但记录读取和需要授权的操作暂时不可用。恢复连接不会重启服务，运行中的请求和任务会继续执行。', 'The local service still responds, but records and protected actions are unavailable. Restore keeps the service and running requests and tasks active.')}</p>
      {busy && <p role="status">{text('正在安全恢复本机连接…', 'Restoring the local connection safely…')}</p>}
      {error && <p data-error-code={error}>{error === 'AUTHORITY_RECOVERY_PENDING'
        ? text('尚未收到完整回复。当前服务保持运行，请稍后再次恢复。', 'The complete reply has not arrived. The service is still running; try restoring again shortly.')
        : text('连接恢复未完成。请查看诊断信息或稍后重试。', 'Connection restoration did not complete. Check diagnostics or try again shortly.')}</p>}
      {status === 'restart_required' && <p>{text('连接已断开或无法安全同步。请等现有工作结束后，正常退出并重新打开 HiRoute。', 'The connection is closed or cannot be safely synchronized. After existing work finishes, quit and reopen HiRoute normally.')}</p>}
    </div>
    {status === 'recovery_required' && <button className="btn btn-primary" type="button" disabled={busy} onClick={() => void recover()}>{text('恢复本机连接', 'Restore local connection')}</button>}
  </div>;
}
