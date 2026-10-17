export type HostKind = 'tauri' | 'console' | 'shell-console';
export type HostCapabilities = {
  host: HostKind;
  platform: string;
  updates: boolean;
  cli_entry: boolean;
  quit: boolean;
  titlebar: boolean;
  login_start: boolean;
  directory_reveal: 'native' | 'path';
  diagnostic_directory?: string;
  recovery_directory?: string;
};
export type HostTransport = {
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  capabilities(): HostCapabilities;
};
export type ConsoleIdentity = {
  session: { id: string; credential_kind: string; token_id: string | null; expires: number };
  transport: 'loopback' | 'network_plaintext';
  assets_available: boolean;
  capabilities: HostCapabilities;
};
export const browserCapabilities: HostCapabilities = {
  host: 'console', platform: 'browser', updates: false, cli_entry: false,
  quit: false, titlebar: false, login_start: false, directory_reveal: 'path',
};
