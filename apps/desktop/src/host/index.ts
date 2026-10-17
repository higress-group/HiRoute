import { browserCapabilities, type HostCapabilities, type HostKind, type HostTransport } from './types.ts';
import { consoleTransport, consoleCapabilities } from './console.ts';
import type { WebConfirmationRequest } from '../features/web-confirmation.ts';
import type { ThemePreference, ResolvedTheme } from '../ui/preferences';
export type { HostCapabilities, HostKind, HostTransport } from './types.ts';
export { plaintextConsole } from './console.ts';
let override: HostTransport | undefined;
let shellCapabilities: HostCapabilities | undefined;
export function hostKind(): HostKind {
  if (override) return override.capabilities().host;
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return 'console';
  return '__HIROUTE_SHELL__' in window ? 'shell-console' : 'tauri';
}
export const isTauriHost = () => hostKind() !== 'console';
export function hostCapabilities(): HostCapabilities {
  if (override) return override.capabilities();
  if (hostKind() === 'tauri') return { host: 'tauri', platform: 'macos', updates: true, cli_entry: true, quit: true, titlebar: true, login_start: true, directory_reveal: 'native' };
  return hostKind() === 'shell-console' ? shellCapabilities ?? { ...browserCapabilities, host: 'shell-console', platform: 'windows' } : consoleCapabilities();
}
/** Explicit seam for the browser harness. It does not change server authority. */
export function setHostTransport(transport: HostTransport | undefined): void { override = transport; }
export async function initializeShellHost(): Promise<void> {
  if (override) return;
  if (hostKind() === 'shell-console') shellCapabilities = await (await import('@tauri-apps/api/core')).invoke<HostCapabilities>('shell_capabilities');
}
function externalUrl(value: unknown): string {
  if (typeof value !== 'string' || !value || value.length > 2048 || /[\s\u0000-\u001f\u007f]/u.test(value)) throw { kind: 'native', code: 'EXTERNAL_URL_INVALID' };
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol) || !url.host || url.username || url.password) throw { kind: 'native', code: 'EXTERNAL_URL_SCHEME_DENIED' };
  return value;
}
async function browserCall(command: string, args: Record<string, unknown>): Promise<unknown> {
  if (command === 'open_external_url') {
    const anchor = document.createElement('a'); anchor.href = externalUrl(args.url); anchor.target = '_blank'; anchor.rel = 'noopener noreferrer'; anchor.click(); return;
  }
  if (command === 'save_classifier_openapi') {
    const source = (await import('../../../../decision-extensions/api/decision.openapi.json?raw')).default;
    const url = URL.createObjectURL(new Blob([source], { type: 'application/json' }));
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = 'hiroute-decision.openapi.json'; anchor.rel = 'noopener noreferrer'; anchor.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 0); return 'saved';
  }
  if (command === 'open_diagnostic_directory' || command === 'open_startup_recovery_directory') {
    const caps = hostCapabilities(); const path = command === 'open_diagnostic_directory' ? caps.diagnostic_directory : caps.recovery_directory;
    if (!path) throw { kind: 'native', code: 'APP_DATA_UNAVAILABLE' };
    window.dispatchEvent(new CustomEvent('hiroute-host-path', { detail: path })); return;
  }
  return consoleTransport.call(command, args);
}
export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (override) {
    if (override.capabilities().host === 'console' && ['open_external_url', 'save_classifier_openapi', 'open_diagnostic_directory', 'open_startup_recovery_directory'].includes(command)) return browserCall(command, args) as Promise<T>;
    return override.call<T>(command, args);
  }
  if (hostKind() === 'tauri') return (await import('./tauri.ts')).tauriTransport.call<T>(command, args);
  if (hostKind() === 'shell-console' && (command.startsWith('update_') || command.startsWith('login_start_') || ['open_external_url', 'save_classifier_openapi', 'open_in_browser', 'restart_service', 'reconnect'].includes(command))) return (await import('@tauri-apps/api/core')).invoke<T>(`shell_${command}`, args);
  return browserCall(command, args) as Promise<T>;
}
export async function listenForHostConfirmations(onRequest: (request: WebConfirmationRequest) => void): Promise<() => void> { return hostKind() === 'tauri' ? (await import('./tauri.ts')).listenNativeConfirmation(onRequest) : () => {}; }
export async function syncHostTheme(preference: ThemePreference, resolved: ResolvedTheme): Promise<'skipped' | 'synced'> { if (hostKind() !== 'tauri') return 'skipped'; await (await import('./tauri.ts')).syncNativeTheme(preference, resolved); return 'synced'; }
export async function nativeClipboard(value: string): Promise<boolean> { if (hostKind() !== 'tauri') return false; return (await import('./tauri.ts')).writeNativeClipboard(value); }
