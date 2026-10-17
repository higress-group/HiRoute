import { invoke, isTauri } from '@tauri-apps/api/core';
import type { HostTransport } from './types.ts';
import type { ThemePreference, ResolvedTheme } from '../ui/preferences';
import { listenForWebConfirmations, type WebConfirmationRequest } from '../features/web-confirmation.ts';
export const tauriTransport: HostTransport = {
  call: (command, args) => invoke(command, args),
  capabilities: () => ({ host: 'tauri', platform: 'macos', updates: true, cli_entry: true, quit: true, titlebar: true, login_start: true, directory_reveal: 'native' }),
};
export async function writeNativeClipboard(text: string): Promise<boolean> { if (!isTauri()) return false; await (await import('@tauri-apps/plugin-clipboard-manager')).writeText(text); return true; }
export async function syncNativeTheme(preference: ThemePreference, resolved: ResolvedTheme): Promise<void> { await (await import('@tauri-apps/api/window')).getCurrentWindow().setTheme(preference === 'system' ? null : resolved); }
export async function listenNativeConfirmation(onRequest: (request: WebConfirmationRequest) => void): Promise<() => void> { return listenForWebConfirmations((await import('@tauri-apps/api/webview')).getCurrentWebview(), onRequest); }
