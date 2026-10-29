import { observationRead } from '../features/observation-client';
import { observationBusyRetryMs } from '../features/observation-failure';

/** Home shares reader capacity with suggestions and excerpts. Only a typed busy
 * response may be retried; authority, data and transport failures stay failures. */
export async function readHomeObservation<T>(view: string, query: object, signal: AbortSignal): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    if (signal.aborted) throw new Error('OBSERVATION_QUERY_CANCELLED');
    try {
      return await observationRead<T>(view, query, signal, true);
    } catch (error) {
      const delay = observationBusyRetryMs(error);
      if (signal.aborted || delay === null || attempt >= 2) throw error;
      await new Promise<void>(resolve => setTimeout(resolve, delay));
    }
  }
}
