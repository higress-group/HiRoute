/** Preserve the native DesktopFailure envelope when reading a backend retry hint. */
export function observationBusyRetryMs(error: unknown): number | null {
  if (error instanceof Error && error.message === 'OBSERVATION_QUERY_BUSY') return 100;
  if (!error || typeof error !== 'object') return null;
  type Failure = { code?: string; observation_read?: { reason?: string; retry_after_ms?: number }; error?: Failure; envelope?: { error?: Failure } };
  const value = error as Failure;
  if (value.code === 'OBSERVATION_QUERY_BUSY') return 100;
  const hint = value.observation_read ?? value.envelope?.error?.observation_read ?? value.error?.observation_read;
  if (hint?.reason !== 'reader_busy') return null;
  return Number.isFinite(hint.retry_after_ms) ? Math.min(1000, Math.max(100, hint.retry_after_ms!)) : 100;
}
