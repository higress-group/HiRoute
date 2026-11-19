import type { CandidateModelView, ModelConnectionCheckView } from './types';

export function checkFailureCode(check: Pick<ModelConnectionCheckView, 'authentication' | 'inference' | 'issues'>): string | null {
  if (check.authentication === 'rejected') return 'MODEL_CONNECTION_AUTHENTICATION_REJECTED';
  if (check.inference !== 'failed') return null;
  return check.issues?.some(issue => issue.code === 'TOOL_CALL_NOT_VERIFIED')
    ? 'MODEL_TOOL_CALL_NOT_VERIFIED'
    : 'MODEL_INFERENCE_FAILED';
}

/** Presentation keys may be more specific; retry classification needs the machine code. */
export function safeConnectionErrorCode(error: unknown, machineCode = false): string {
  const pending: unknown[] = [error];
  const visited = new Set<object>();
  while (pending.length > 0 && visited.size < 8) {
    const current = pending.shift();
    if (typeof current !== 'object' || current === null || visited.has(current)) continue;
    visited.add(current);
    const record = current as Record<string, unknown>;
    const fields = machineCode ? [record.code, record.message_key] : [record.message_key, record.code];
    for (const value of fields) {
      if (typeof value === 'string' && /^[A-Za-z0-9_.\/-]{1,80}$/.test(value)) return value;
    }
    pending.push(record.error, record.envelope, record.failure);
  }
  return 'MODEL_CONNECTION_FAILED';
}

export function modelAvailabilityMessage(reason: string | null | undefined, zh: boolean): string {
  if (reason === 'compute.registered_model_unmatched') {
    return zh
      ? '这个模型尚无可信能力资料；请通过高级自定义接入补充。'
      : 'Trusted capability data is unavailable. Add it through Advanced custom connection.';
  }
  if (reason === 'model_connections.capability_required') {
    return zh ? '需要补充模型能力信息。' : 'Model capability details are required.';
  }
  if (reason === 'model_connections.connection_check_required') {
    return zh ? '需要重新读取并核对连接。' : 'Read and verify the connection again.';
  }
  return zh
    ? '当前不可选择，请检查模型能力与接入状态。'
    : 'Unavailable; check model capabilities and connection status.';
}


/**
 * Independent current-observation label from the exact directory result. Catalog membership and
 * selection eligibility stay separate from whether this check actually returned the ID.
 */
export function modelObservationMessage(
  result: Pick<ModelConnectionCheckView, 'directory' | 'reachability' | 'observed_model_ids'>,
  model: Pick<CandidateModelView, 'upstream_model_id'>,
  zh: boolean,
): string | null {
  const observed = new Set(result.observed_model_ids ?? []);
  if (observed.has(model.upstream_model_id)) {
    return zh ? '本次检查已返回' : 'Returned by this check';
  }
  const complete = result.reachability === 'reachable'
    && (result.directory === 'available' || result.directory === 'empty');
  return complete
    ? (zh ? '本次检查未返回' : 'Not returned by this check')
    : (zh ? '本次检查未能确认' : 'Not confirmed by this check');
}
