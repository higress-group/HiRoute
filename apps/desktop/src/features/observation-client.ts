import { invoke } from '../host/index.ts';

let readers = 0;
const queue: (() => void)[] = [];

/** Shared bounded reader for every Desktop observation surface. */
export async function observationRead<T>(view: string, query: object, signal?: AbortSignal, readFailureDetails = false): Promise<T> {
  if (queue.length >= 32) throw new Error('OBSERVATION_QUERY_BUSY');
  if (readers >= 2) await new Promise<void>(resolve => queue.push(resolve));
  else readers++;
  try {
    if (signal?.aborted) throw new Error('OBSERVATION_QUERY_CANCELLED');
    return await invoke<T>('observation_read', {
      request: {
        schema: 'hiroute.observation.query/v2',
        ...(readFailureDetails ? { read_failure_details: true } : {}),
        intent: { view, query: view === 'facts' || view === 'plan_quality' ? { ...query, include_routing_observations: true } : query },
      },
    });
  } finally {
    const next = queue.shift();
    if (next) next();
    else readers--;
  }
}
