import { safeDiagnosticCode } from '../../error-code.ts';

/** Retry only pure reads whose immutable catalog capture was superseded. Never retry a mutation. */
export async function readCurrentCatalog<T>(read: () => Promise<T>): Promise<T> {
  for (let attempt = 0; ; attempt += 1) {
    try { return await read(); }
    catch (error) {
      const code = safeDiagnosticCode(error, 'UNKNOWN');
      if (attempt >= 2 || !['CHANGE_PREVIEW_STALE', 'application.error.change_preview_stale'].includes(code)) throw error;
      await new Promise(resolve => setTimeout(resolve, 40 * (attempt + 1)));
    }
  }
}
