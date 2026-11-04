const BUSY_BACKOFF_MS = 400;

/** Preserve the native DesktopFailure envelope when reading a backend retry hint. */
export function observationBusyRetryMs(error: unknown): number | null {
  if (error instanceof Error && error.message === 'OBSERVATION_QUERY_BUSY') return BUSY_BACKOFF_MS;
  if (!error || typeof error !== 'object') return null;
  type Failure = { code?: string; observation_read?: { reason?: string; retry_after_ms?: number }; error?: Failure; envelope?: { error?: Failure } };
  const value = error as Failure;
  if (value.code === 'OBSERVATION_QUERY_BUSY') return BUSY_BACKOFF_MS;
  const hint = value.observation_read ?? value.envelope?.error?.observation_read ?? value.error?.observation_read;
  if (hint?.reason !== 'reader_busy') return null;
  // Initial console reads can overlap more than one control-store operation.
  return Number.isFinite(hint.retry_after_ms)
    ? Math.min(1000, Math.max(BUSY_BACKOFF_MS, hint.retry_after_ms!))
    : BUSY_BACKOFF_MS;
}
