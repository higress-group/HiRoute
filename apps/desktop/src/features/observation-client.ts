import { invoke } from '../host/index.ts';
import { observationBusyRetryMs } from './observation-failure.ts';

let readers = 0;
const queue: (() => void)[] = [];

export const observationReadPaused = Symbol('observation read paused');

export async function observationReadOnce<T>(view: string, query: object, signal?: AbortSignal, visibleOnly = false): Promise<T> {
  signal?.throwIfAborted();
  if (visibleOnly && document.hidden) throw observationReadPaused;
  if (readers >= 2) {
    if (queue.length >= 32) throw Object.assign(new Error('OBSERVATION_QUERY_BUSY'), { code: 'OBSERVATION_QUERY_BUSY' });
    await new Promise<void>((resolve, reject) => {
      const admit = () => { signal?.removeEventListener('abort', cancel); resolve(); };
      const cancel = () => {
        const index = queue.indexOf(admit);
        if (index >= 0) queue.splice(index, 1);
        reject(signal?.reason);
      };
      queue.push(admit);
      signal?.addEventListener('abort', cancel, { once: true });
    });
  } else readers++;
  try {
    signal?.throwIfAborted();
    if (visibleOnly && document.hidden) throw observationReadPaused;
    return await invoke<T>('observation_read', {
      request: {
        schema: 'hiroute.observation.query/v2',
        read_failure_details: true,
        intent: { view, query: view === 'facts' || view === 'plan_quality' ? { ...query, include_routing_observations: true } : query },
      },
    }, { recoveringObservation: visibleOnly });
  } finally {
    const next = queue.shift();
    if (next) next();
    else readers--;
  }
}

/** Web and Desktop share two reader slots and one short retry for a busy read. */
export async function observationRead<T>(view: string, query: object, signal?: AbortSignal): Promise<T> {
  try {
    return await observationReadOnce<T>(view, query, signal);
  } catch (error) {
    const delay = observationBusyRetryMs(error);
    const cancelled = () => signal?.aborted || document.visibilityState === 'hidden';
    if (delay === null || cancelled()) throw error;
    // The attempt has already released its slot, so other reads can make progress.
    await new Promise<void>(resolve => setTimeout(resolve, delay));
    if (cancelled()) throw error;
    return observationReadOnce<T>(view, query, signal);
  }
}
