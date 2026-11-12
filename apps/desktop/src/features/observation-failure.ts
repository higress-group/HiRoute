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

type MachineFailure = {
  source?: string;
  code?: string;
  envelope?: { error?: MachineFailure };
  error?: MachineFailure;
  failure?: { code?: string };
  retryable?: boolean;
  details_schema?: string;
  observation_read?: { reason?: string; retry_after_ms?: number };
};

function machineFailure(error: unknown): MachineFailure {
  if (!error || typeof error !== 'object') return {};
  const failure = error as MachineFailure;
  return failure.source === 'transport' ? failure.failure ?? {}
    : failure.envelope?.error ?? failure.error ?? failure;
}

export function observationFailureCode(error: unknown): string | undefined {
  return machineFailure(error).code;
}

export function observationCancelled(error: unknown, signal?: AbortSignal): boolean {
  return Boolean(signal?.aborted) || (error instanceof DOMException && error.name === 'AbortError')
    || observationFailureCode(error) === 'observation_stopped';
}

export function observationAccessLost(error: unknown): boolean {
  return [
    'CAPABILITY_DENIED', 'CAPABILITY_UNAVAILABLE', 'CLIENT_CONTEXT_INVALID', 'CONSOLE_AUTH_REQUIRED', 'peer_rejected',
    'TRUSTED_AUTHORITY_UNAVAILABLE', 'PROTECTED_CHANNEL_TIMEOUT', 'PROTECTED_CHANNEL_CLOSED',
    'PROTECTED_CHANNEL_UNAVAILABLE', 'REGISTRATION_ACK_INVALID', 'REGISTRATION_ACK_MISMATCH',
  ].includes(observationFailureCode(error) ?? '');
}

/** Only the observation read owner may use these recovery decisions. */
export function observationRecoveryAction(error: unknown): { kind: 'retry'; hint?: number } | { kind: 'rebuild' | 'stop' } {
  const failure = machineFailure(error);
  switch (failure.code) {
    case 'OBSERVATION_QUERY_BUSY':
    case 'TRUSTED_AUTHORITY_BUSY':
    case 'READ_SNAPSHOT_CHANGED':
    case 'transport_unavailable':
    case 'deadline':
    case 'CONSOLE_UNAVAILABLE':
      return { kind: 'retry' };
    case 'CHANGE_PREVIEW_STALE':
      return { kind: 'rebuild' };
    case 'OBSERVATION_UNAVAILABLE':
      if (failure.retryable === true && failure.details_schema === 'hiroute.observation.read-failure/v1'
        && ['reader_busy', 'timeout', 'storage_unavailable'].includes(failure.observation_read?.reason ?? '')) {
        return { kind: 'retry', hint: failure.observation_read?.retry_after_ms };
      }
      break;
  }
  return { kind: 'stop' };
}
