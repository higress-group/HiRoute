import type { ConsoleIdentity, HostCapabilities, HostTransport } from './types.ts';
import { browserCapabilities } from './types.ts';
const SESSION_KEY = 'hiroute.console.session';
const CONTEXT_KEY = 'hiroute.console.context';
const API = '/api/console/v1';
export const CONSOLE_EXPIRED_EVENT = 'hiroute-console-expired';
export const CONSOLE_UNAVAILABLE_EVENT = 'hiroute-console-unavailable';
let context: string | undefined;
let capabilities: HostCapabilities = browserCapabilities;
let transport: ConsoleIdentity['transport'] = 'loopback';
const failure = (code: string) => ({ kind: 'native', code });
function storage(): Storage { try { return window.sessionStorage; } catch { throw failure('CONSOLE_STORAGE_UNAVAILABLE'); } }
export function sessionBearer(): string | null { try { return storage().getItem(SESSION_KEY); } catch { return null; } }
export function clearConsoleSession(): void { try { storage().removeItem(SESSION_KEY); } catch { /* Read-only/denied storage has no usable session. */ } }
function contextId(): string {
  if (context) return context;
  // Reload reuses this tab's context. A new document/tab gets its own random id,
  // even when the browser initially clones the opener's sessionStorage.
  const navigation = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined;
  const retained = navigation?.type === 'reload' ? storage().getItem(CONTEXT_KEY) : null;
  context = retained && /^[a-f0-9]{32}$/.test(retained) ? retained
    : Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
  storage().setItem(CONTEXT_KEY, context);
  return context;
}
async function request<T>(path: string, options: RequestInit = {}, authenticated = true): Promise<T> {
  const headers = new Headers(options.headers);
  headers.set('Content-Type', 'application/json');
  if (authenticated) {
    const bearer = sessionBearer();
    if (!bearer) throw failure('CONSOLE_AUTH_REQUIRED');
    headers.set('Authorization', `Bearer ${bearer}`);
    headers.set('X-HiRoute-Context', contextId());
  }
  let response: Response;
  try {
    response = await fetch(`${API}${path}`, { ...options, headers, credentials: 'omit', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(90_000) });
  } catch {
    window.dispatchEvent(new Event(CONSOLE_UNAVAILABLE_EVENT));
    throw failure('CONSOLE_UNAVAILABLE');
  }
  let value: unknown;
  try { value = await response.json(); } catch { throw failure('RESPONSE_DATA_INVALID'); }
  if (!response.ok) {
    if (response.status === 401 && authenticated) { clearConsoleSession(); window.dispatchEvent(new Event(CONSOLE_EXPIRED_EVENT)); }
    throw value;
  }
  return value as T;
}
export async function exchangeConsoleCredential(kind: 'launch' | 'token', value: string): Promise<ConsoleIdentity> {
  const exchange = await request<{ bearer: string }>(`/session/${kind}`, { method: 'POST', body: JSON.stringify(kind === 'launch' ? { code: value } : { token: value }) }, false);
  if (!/^[a-f0-9]{64}$/.test(exchange.bearer)) throw failure('RESPONSE_DATA_INVALID');
  storage().setItem(SESSION_KEY, exchange.bearer);
  return readConsoleSession();
}
export async function readConsoleSession(): Promise<ConsoleIdentity> {
  const identity = await request<ConsoleIdentity>('/session');
  if (!identity.session?.id || !['loopback', 'network_plaintext'].includes(identity.transport) || typeof identity.assets_available !== 'boolean') throw failure('RESPONSE_DATA_INVALID');
  transport = identity.transport;
  capabilities = { ...browserCapabilities, platform: typeof identity.capabilities?.platform === 'string' ? identity.capabilities.platform : 'browser', diagnostic_directory: identity.capabilities?.diagnostic_directory, recovery_directory: identity.capabilities?.recovery_directory };
  return identity;
}
export async function logoutConsole(): Promise<void> { try { await request('/session', { method: 'DELETE' }); } finally { clearConsoleSession(); window.dispatchEvent(new Event(CONSOLE_EXPIRED_EVENT)); } }
export const plaintextConsole = () => transport === 'network_plaintext';
export const consoleCapabilities = () => capabilities;
export const consoleTransport: HostTransport = {
  capabilities: consoleCapabilities,
  async call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    if (command === 'startup_status') {
      // The console starts after daemon admission. Confirm the live session
      // instead of turning an earlier request error into a startup failure.
      await readConsoleSession();
      return { state: 'ready', recovery_available: false } as T;
    }
    return request<T>(`/command/${encodeURIComponent(command)}`, { method: 'POST', body: JSON.stringify(args) });
  },
};
