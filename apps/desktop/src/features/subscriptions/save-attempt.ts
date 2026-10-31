import type { ComputeConnectionApplyRequest, ComputeSaveResult, OperationReference } from '../model-connections/types';
import { safeConnectionErrorCode } from '../model-connections/copy.ts';
import { saveFailureDefinitelyPreAdmission } from './model.ts';

/** Keep the original sealed request across lost replies; retries never mint another key. */
export type SaveAttempt = {
  request: ComputeConnectionApplyRequest;
  submitted: boolean;
  operation?: OperationReference;
  terminal?: ComputeSaveResult;
};
export type SaveAttemptPorts = {
  apply(request: ComputeConnectionApplyRequest): Promise<{ operation: OperationReference }>;
  result(operation: OperationReference): Promise<ComputeSaveResult>;
  find(key: string): Promise<ComputeSaveResult | null>;
  onOperation(operation: OperationReference): void;
  onUncertain(): void;
  current(): boolean;
};

export async function observeSaveAttempt(attempt: SaveAttempt, ports: SaveAttemptPorts): Promise<ComputeSaveResult> {
  if (attempt.terminal) return attempt.terminal;
  const pending = (): ComputeSaveResult => {
    if (attempt.submitted) ports.onUncertain();
    return { disposition: 'pending', bindings: [], operation: attempt.operation };
  };
  if (!ports.current()) return pending();
  let result: ComputeSaveResult | null = null;
  if (attempt.submitted) {
    try {
      result = attempt.operation ? await ports.result(attempt.operation) : await ports.find(attempt.request.idempotency_key);
    } catch { return pending(); }
    // An absent receipt permits replay of this exact request/key, never a new Check/Save.
    if (result?.operation) { attempt.operation = result.operation; ports.onOperation(result.operation); }
  }
  if (!result) {
    if (!ports.current()) return pending();
    const earlierSubmission = attempt.submitted;
    attempt.submitted = true;
    try {
      const accepted = await ports.apply(attempt.request);
      attempt.operation = accepted.operation;
      ports.onOperation(accepted.operation);
    } catch (error) {
      // Refusing this replay cannot disprove admission of an earlier lost call.
      if (!earlierSubmission && (saveFailureDefinitelyPreAdmission(error) || saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(error)))) {
        attempt.submitted = false;
        throw error;
      }
      return pending();
    }
    try { result = await ports.result(attempt.operation!); }
    catch { return pending(); }
  }
  if (result.disposition === 'pending') return pending();
  if (result.disposition === 'saved' && !result.source_id) return pending();
  attempt.terminal = result;
  return result;
}
