import { invoke } from '../host/index.ts';
import { observationBusyRetryMs } from './observation-failure';

let readers = 0;
const queue: (() => void)[] = [];

async function readOnce<T>(view: string, query: object, signal?: AbortSignal): Promise<T> {
  if (signal?.aborted) throw new Error('OBSERVATION_QUERY_CANCELLED');
  if (queue.length >= 32) throw new Error('OBSERVATION_QUERY_BUSY');
  if (readers >= 2) await new Promise<void>(resolve => queue.push(resolve));
  else readers++;
  try {
    if (signal?.aborted) throw new Error('OBSERVATION_QUERY_CANCELLED');
    return await invoke<T>('observation_read', {
      request: {
        schema: 'hiroute.observation.query/v2',
        read_failure_details: true,
        intent: { view, query: view === 'facts' || view === 'plan_quality' ? { ...query, include_routing_observations: true } : query },
      },
    });
  } finally {
    const next = queue.shift();
    if (next) next();
    else readers--;
  }
}

/** Web and Desktop share two reader slots and one short retry for a busy read. */
export async function observationRead<T>(view: string, query: object, signal?: AbortSignal): Promise<T> {
  try {
    return await readOnce<T>(view, query, signal);
  } catch (error) {
    const delay = observationBusyRetryMs(error);
    const cancelled = () => signal?.aborted || document.visibilityState === 'hidden';
    if (delay === null || cancelled()) throw error;
    // readOnce has already released its slot, so other reads can make progress.
    await new Promise<void>(resolve => setTimeout(resolve, delay));
    if (cancelled()) throw error;
    return readOnce<T>(view, query, signal);
  }
}
