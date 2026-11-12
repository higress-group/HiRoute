import { observationReadOnce, observationReadPaused } from './observation-client.ts';
import { observationRecoveryAction } from './observation-failure.ts';

export type ObservationRecoveryStatus = 'retrying' | 'rebuilding' | 'slow';
export type ObservationReader = <T>(view: string, query: object) => Promise<T>;
type Options = {
  signal: AbortSignal;
  onRecovery?: (status: ObservationRecoveryStatus) => void;
  onRebuild?: () => void;
};

const delays = [400, 1000, 2000, 4000, 8000, 15000, 30000];
const slowAfter = 10000;

export function observationRecoveryText(status: ObservationRecoveryStatus, language: 'zh' | 'en'): string {
  if (status === 'slow') return language === 'zh' ? '读取耗时较长，正在自动恢复。' : 'Taking longer than usual. Retrying automatically.';
  if (status === 'rebuilding') return language === 'zh' ? '正在重新读取…' : 'Reloading…';
  return language === 'zh' ? '正在更新…' : 'Updating…';
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { signal.removeEventListener('abort', cancel); resolve(); }, ms);
    function cancel() { clearTimeout(timer); reject(signal.reason); }
    signal.addEventListener('abort', cancel, { once: true });
  });
}

function waitUntilVisible(signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    function cleanup() {
      document.removeEventListener('visibilitychange', visible);
      signal.removeEventListener('abort', cancel);
    }
    function visible() { if (!document.hidden) { cleanup(); resolve(); } }
    function cancel() { cleanup(); reject(signal.reason); }
    document.addEventListener('visibilitychange', visible);
    signal.addEventListener('abort', cancel, { once: true });
  });
}

/**
 * Own one visible read operation. Busy attempts replay just the failed read.
 * A stale cursor unwinds the operation so its owner can discard the old range.
 */
export async function recoverObservation<T>(work: (read: ObservationReader) => Promise<T>, options: Options): Promise<T> {
  const { signal, onRecovery, onRebuild } = options;
  const started = performance.now();
  let failures = 0;
  let rebuilding = false;
  let status: ObservationRecoveryStatus | undefined;
  function report(next: ObservationRecoveryStatus) {
    if (status === next) return;
    status = next;
    onRecovery?.(next);
  }
  async function backoff(hint?: number) {
    const base = delays[Math.min(failures++, delays.length - 1)];
    const ms = Math.min(30000, Math.max(base, Number.isFinite(hint) ? hint! : 0));
    const untilSlow = slowAfter - (performance.now() - started);
    report(untilSlow <= 0 ? 'slow' : rebuilding ? 'rebuilding' : 'retrying');
    if (onRecovery && untilSlow > 0 && untilSlow < ms) {
      await delay(untilSlow, signal);
      signal.throwIfAborted();
      report('slow');
      await delay(ms - untilSlow, signal);
    } else await delay(ms, signal);
  }
  const read: ObservationReader = async <Result>(view: string, query: object): Promise<Result> => {
    for (;;) {
      signal.throwIfAborted();
      if (document.hidden) await waitUntilVisible(signal);
      try {
        const value = await observationReadOnce<Result>(view, query, signal, true);
        signal.throwIfAborted();
        return value;
      } catch (error) {
        signal.throwIfAborted();
        if (error === observationReadPaused) continue;
        const action = observationRecoveryAction(error);
        if (action.kind !== 'retry') throw error;
        await backoff(action.hint);
      }
    }
  };
  for (;;) {
    signal.throwIfAborted();
    try {
      const result = await work(read);
      signal.throwIfAborted();
      return result;
    } catch (error) {
      signal.throwIfAborted();
      if (!onRebuild || observationRecoveryAction(error).kind !== 'rebuild') throw error;
      onRebuild();
      rebuilding = true;
      await backoff();
    }
  }
}

/** Read-only calls without a cursor can start a fresh range after a stale reply. */
export function readObservation<T>(view: string, query: Record<string, unknown> & { cursor?: null; after_watermark?: null }, options: Options): Promise<T> {
  return recoverObservation(read => read<T>(view, query), { onRebuild: () => {}, ...options });
}

export function readObservationPages<T extends { next_cursor: string | null }>(
  view: string, query: { cursor: string | null } & Record<string, unknown>, loadedPages: number, options: Options,
  needsNext?: (page: T) => boolean,
): Promise<{ pages: T[]; rebuilt: boolean }> {
  let rebuilt = false;
  return recoverObservation(async read => {
    const pages: T[] = [];
    let cursor = rebuilt ? null : query.cursor;
    const seen = new Set([cursor]);
    const count = rebuilt && query.cursor ? loadedPages + 1 : 1;
    do {
      const page: T = await read(view, { ...query, cursor });
      pages.push(page);
      cursor = page.next_cursor;
      if (cursor && seen.has(cursor)) throw { code: 'RESPONSE_DATA_INVALID' };
      seen.add(cursor);
    } while (cursor && (pages.length < count || needsNext?.(pages[pages.length - 1])));
    return { pages, rebuilt };
  }, { ...options, onRebuild: () => { rebuilt = true; options.onRebuild?.(); } });
}
