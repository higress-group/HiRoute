import type { Language } from '../ui/preferences';
import type { DesktopOperation } from './home-projections';

export type OperationPresentation = {
  kind: 'agent-settings' | 'model-connection' | 'routing' | 'background';
  target?: string;
};

export type OperationFeedback = {
  phase: 'pending' | 'succeeded' | 'failed' | 'unverified';
  title: string;
  detail: string;
};

export const OPERATION_RESULT_UNVERIFIED = 'OPERATION_RESULT_UNVERIFIED';
type PendingHint = { operation_id: string | null; idempotency_key: string };

export function isTerminalOperation(state: string): boolean {
  return state === 'succeeded' || state === 'rolled_back' || state === 'needs_attention';
}

/**
 * Observation can advance only the identity that started the poll. A newly
 * submitted operation is installed directly and must never be replaced by a
 * late response from an older observation loop.
 */
export function acceptObservedOperation(
  current: DesktopOperation | null,
  observed: DesktopOperation | null,
): DesktopOperation | null {
  if (!observed) return current;
  if (!current) return observed;
  if (current.operation_id !== observed.operation_id) return current;
  return observed.sequence >= current.sequence ? observed : current;
}

export function shouldObservePendingOperation(
  current: DesktopOperation | null,
  pending: PendingHint | null,
  supersededPendingKey: string | null = null,
): boolean {
  const activePending = currentPendingHint(pending, supersededPendingKey);
  return Boolean(
    current
    && activePending
    && current.operation_id !== activePending.operation_id
    && isTerminalOperation(current.state),
  );
}

export function currentPendingHint(pending: PendingHint | null | undefined, supersededPendingKey: string | null): PendingHint | null {
  return pending && pending.idempotency_key !== supersededPendingKey ? pending : null;
}

export function pendingFeedbackIdentity(
  current: DesktopOperation | null,
  pendingOperationId: string | null | undefined,
): string | null {
  return current?.operation_id ?? pendingOperationId ?? null;
}

/** Local trial progress may replace only feedback for the same known operation. */
export function representedByTrial(identity: string | null, phase: OperationFeedback['phase'], observationError: string, recoveryPending: boolean, locallyRepresented: readonly string[]): boolean {
  return Boolean(identity && !observationError && !recoveryPending && (phase === 'pending' || phase === 'succeeded') && locallyRepresented.includes(identity));
}

function subject(presentation: OperationPresentation, language: Language): string {
  if (presentation.target) return presentation.target;
  const labels = {
    'agent-settings': ['Agent 配置', 'Agent settings'],
    'model-connection': ['模型接入', 'Model connection'],
    routing: ['智能路由', 'Smart routing'],
    background: ['后台操作', 'Background operation'],
  } as const;
  return labels[presentation.kind][language === 'zh' ? 0 : 1];
}

export function operationFeedback(
  operation: DesktopOperation | null,
  observationError: string,
  language: Language,
  presentation: OperationPresentation = { kind: 'background' },
): OperationFeedback {
  const name = subject(presentation, language);
  if (observationError && (!operation || !isTerminalOperation(operation.state))) {
    const detail = observationError === 'OPERATION_IDENTITY_MISMATCH'
      ? ['返回了另一项操作，系统正按原标识重试；可继续编辑。', 'Another operation was returned. Checking continues under the original identity; editing remains available.']
      : observationError === OPERATION_RESULT_UNVERIFIED
        ? ['尚未查到对应操作，系统会继续查询；可继续编辑。', 'The operation is not visible yet. Checking continues; editing remains available.']
        : ['状态查询暂未成功，系统会自动重试；可继续编辑。', 'The status check has not succeeded yet. It will retry automatically; editing remains available.'];
    return {
      phase: 'unverified',
      title: language === 'zh' ? `${name}结果待确认` : `${name} result pending confirmation`,
      detail: detail[language === 'zh' ? 0 : 1],
    };
  }
  if (operation?.state === 'succeeded') {
    return language === 'zh'
      ? { phase: 'succeeded', title: `${name}已完成`, detail: '本机服务已确认实际结果；你可以关闭这条反馈。' }
      : { phase: 'succeeded', title: `${name} completed`, detail: 'The local service confirmed the actual result. You can close this message.' };
  }
  if (operation?.state === 'rolled_back') {
    return language === 'zh'
      ? { phase: 'failed', title: `${name}未完成`, detail: '变更已回滚；请检查保留的输入后重试。' }
      : { phase: 'failed', title: `${name} did not complete`, detail: 'The change was rolled back. Check the retained input before retrying.' };
  }
  if (operation?.state === 'needs_attention') {
    return language === 'zh'
      ? { phase: 'failed', title: `${name}未完成`, detail: '请在对应页面查看当前配置，再决定是否重新保存。' }
      : { phase: 'failed', title: `${name} did not complete`, detail: 'Check the current configuration on its page before saving again.' };
  }
  if (presentation.kind === 'background') {
    return language === 'zh'
      ? { phase: 'pending', title: '正在确认后台操作', detail: '正在查询本机服务的实际状态。' }
      : { phase: 'pending', title: 'Checking background operation', detail: 'Checking its actual state with the local service.' };
  }
  return language === 'zh'
    ? { phase: 'pending', title: `正在保存${name}`, detail: '正在等待本机服务确认实际结果。' }
    : { phase: 'pending', title: `Saving ${name}`, detail: 'Waiting for the local service to confirm the actual result.' };
}
