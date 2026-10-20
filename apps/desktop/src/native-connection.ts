import { safeDiagnosticCode } from './error-code.ts';

export type NativeConnectionStatus = 'ready' | 'recovery_required' | 'restart_required' | 'not_owned';

export function nativeConnectionLost(error: unknown): boolean {
  return ['TRUSTED_AUTHORITY_UNAVAILABLE', 'PROTECTED_CHANNEL_TIMEOUT', 'PROTECTED_CHANNEL_CLOSED', 'PROTECTED_CHANNEL_UNAVAILABLE', 'REGISTRATION_ACK_INVALID', 'REGISTRATION_ACK_MISMATCH'].includes(safeDiagnosticCode(error, ''));
}

export function nativeConnectionReadText(error: unknown, language: 'zh' | 'en'): string | undefined {
  if (!nativeConnectionLost(error)) return;
  return language === 'zh'
    ? '本机可信连接已失效，暂时无法读取记录。请查看页面上方的连接恢复提示。'
    : 'The trusted local connection was lost, so records cannot be read. Follow the connection recovery guidance above.';
}
