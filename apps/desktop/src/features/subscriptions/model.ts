import { subscriptionFailureCopy } from '../models/subscription-copy.ts';
import type { SubscriptionCandidate, SubscriptionCheckResult, SubscriptionSaveIntent } from './types';

export function saveIntent(enable: boolean): SubscriptionSaveIntent {
  return enable ? 'save_ready' : 'save_disabled';
}

export function validatedSubscriptionCandidate(candidate: SubscriptionCandidate, check?: SubscriptionCheckResult): SubscriptionCandidate | null {
  if (check && (check.status !== 'verified' || !check.checked_candidate)) return null;
  const current = check?.checked_candidate ?? candidate;
  if (current.candidate.candidate_ref !== candidate.candidate.candidate_ref
    || current.candidate.candidate_revision < candidate.candidate.candidate_revision
    || current.fact_state === 'pending_approval' || !current.validation) return null;
  return current;
}

export function saveFailureDefinitelyPreAdmission(failure: unknown): boolean {
  if (typeof failure === 'object' && failure !== null) {
    const value = failure as Record<string, unknown>;
    const envelope = value.envelope;
    if (value.source === 'backend' && typeof envelope === 'object' && envelope !== null) {
      const response = envelope as Record<string, unknown>;
      if (response.error != null && response.operation == null) return true;
    }
  }
  const code = typeof failure === 'string' ? failure : '';
  return new Set([
    'MODEL_SAVE_CANCELLED',
    'CONFIRMATION_EXPIRED',
    'CONFIRMATION_STALE',
    'REVISION_CONFLICT',
    'CHANGE_PREVIEW_STALE',
    'CAPABILITY_DENIED',
    'INVALID_ARGUMENTS',
    'ACTION_REQUIRED',
  ]).has(code);
}

export function isCurrentResult(candidate: SubscriptionCandidate, editRevision: number, checkId: string): boolean {
  return candidate.correlation.edit_revision === editRevision
    && candidate.correlation.check_id === checkId
    && candidate.correlation.candidate_ref === candidate.candidate.candidate_ref;
}

export function canSave(candidate: SubscriptionCandidate, enable: boolean, selected: ReadonlySet<string>): boolean {
  if (candidate.fact_state === 'pending_approval') return false;
  if ([...selected].some(ref => !candidate.models.some(model => model.model_ref === ref && model.selectable))) return false;
  if (candidate.fact_state === 'pending_credential') return !enable;
  if (!enable) return true;
  return selected.size > 0;
}

export function repairSourceAfterLogin(sourceId: string | null, candidate: SubscriptionCandidate): string | null {
  return sourceId !== null && candidate.existing_source_id === sourceId ? sourceId : null;
}

export function canSaveRepair(candidate: SubscriptionCandidate | null | undefined, saved: { source_id: string; models: readonly { model_ref: string }[] } | null | undefined, selected: ReadonlySet<string>): boolean {
  return Boolean(candidate?.validation && candidate.fact_state !== 'pending_approval' && saved
    && candidate.existing_source_id === saved.source_id
    && selected.size === saved.models.length
    && saved.models.every(model => selected.has(model.model_ref)));
}

export function selectionRows(candidate: SubscriptionCandidate, selected: ReadonlySet<string>, saved: readonly { model_ref: string; display_name: string }[] = []) {
  const rows = candidate.models.map(model => ({
    model_ref: model.model_ref, display_name: model.display_name, selectable: model.selectable, missing: false,
  }));
  for (const ref of selected) {
    if (!rows.some(model => model.model_ref === ref)) {
      rows.push({ model_ref: ref, display_name: saved.find(model => model.model_ref === ref)?.display_name ?? ref, selectable: false, missing: true });
    }
  }
  return rows;
}

export type CloseAction =
  | { kind: 'none' }
  | { kind: 'cancel_a'; operation_id: string }
  | { kind: 'release_a'; validation_ref: string }
  | { kind: 'observe_b'; operation_id: string };

export function closeAction(check?: SubscriptionCheckResult, saveOperationId?: string): CloseAction {
  if (saveOperationId) return { kind: 'observe_b', operation_id: saveOperationId };
  if (!check) return { kind: 'none' };
  if (check.save_operation) {
    return { kind: 'observe_b', operation_id: check.save_operation.operation_id };
  }
  if (check.status === 'checking') return { kind: 'cancel_a', operation_id: check.approval_operation.operation_id };
  if ((check.status === 'verified' || check.status === 'source_changed') && check.validation) {
    return { kind: 'release_a', validation_ref: check.validation.validation_ref };
  }
  return { kind: 'none' };
}

export function statusText(candidate: SubscriptionCandidate, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  if (candidate.fact_state === 'pending_credential') return zh ? '需要凭据，可先保存为停用' : 'Credential required; can be saved disabled';
  if (candidate.fact_state === 'pending_approval') return zh ? '确认后检查订阅模型' : 'Confirm before checking subscription models';
  if (candidate.issues?.some(issue => issue.code === 'runtime_unavailable')) return zh ? '订阅运行暂不可用' : 'Subscription runtime unavailable';
  if (candidate.models.length > 0 && candidate.models.every(model => !model.selectable)) return zh ? '已发现模型，暂无可匹配资料' : 'Models observed; no matched catalog data';
  return zh ? '事实已完整，保存后仍需发布与运行检查' : 'Facts complete; save, publication, and runtime remain separate';
}

export function checkStatusText(check: SubscriptionCheckResult, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  const detail = subscriptionFailureCopy(check.reason ?? '', language);
  if (detail && ['needs_auth', 'unavailable', 'failed'].includes(check.status)) return detail;
  switch (check.status) {
    case 'checking': return zh ? '正在检查订阅模型' : 'Checking subscription models';
    case 'verified': return zh ? '检查完成，选择模型后仍需保存' : 'Check complete; select models and save';
    case 'source_changed': return zh ? '登录来源已变化，请重新检查' : 'Login source changed; check again';
    case 'needs_auth': return zh ? '请更新对应的订阅登录后重新检查' : 'Renew the corresponding subscription sign-in and check again';
    case 'unavailable': return zh ? '订阅运行暂不可用，可稍后重试' : 'Subscription runtime unavailable; retry later';
    case 'failed': return zh ? '订阅检查失败，可重试' : 'Subscription check failed; retry available';
    case 'released': return zh ? '本次检查资源已释放' : 'Resources for this check were released';
    case 'retained': return zh ? '保存操作已接管本次检查资源' : 'The save operation retained this check resource';
  }
}
