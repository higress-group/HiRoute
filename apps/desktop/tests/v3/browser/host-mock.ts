import { mockIPC as nativeMockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { setHostTransport } from '../../../src/host/index.ts';
import { browserCapabilities } from '../../../src/host/types.ts';
export { mockWindows };
export const consoleFixture = () => new URLSearchParams(location.search).get('transport') === 'console';
export function mockIPC(callback: Parameters<typeof nativeMockIPC>[0], options?: Parameters<typeof nativeMockIPC>[1]): void {
  if (!consoleFixture()) { setHostTransport(undefined); nativeMockIPC(callback, options); return; }
  setHostTransport({
    capabilities: () => ({ ...browserCapabilities, diagnostic_directory: '/example/service/diagnostics', recovery_directory: '/example/service' }),
    call: async <T,>(command: string, args: Record<string, unknown> = {}) => await callback(command, args) as T,
  });
  Object.assign(window, { fixtureTransport: 'console' });
}
