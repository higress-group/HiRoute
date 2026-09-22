import type { OperationReference } from './features/model-connections/types';

export type AgentMutationOutcome = {
  state: string;
  operation: OperationReference | null;
};

export type AgentMutationDisposition = 'cancelled' | 'submitted' | 'unverified';

/**
 * A missing Operation is not proof that an apply was a no-op. Once the native
 * host attempted submission, every non-cancelled result without an identity
 * must stay in recovery until the backend can reconcile the idempotency key.
 */
export function classifyAgentMutation(
  mutation: AgentMutationOutcome,
): AgentMutationDisposition {
  if (mutation.state === 'cancelled_before_apply') return 'cancelled';
  if (mutation.operation) return 'submitted';
  return 'unverified';
}

export function agentActionErrorMessage(code: string, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  if (code === 'AGENT_TOKEN_INVALID') return zh
    ? '令牌须为 16–128 位英文字母、数字或 . _ ~ -。当前令牌未更改。'
    : 'Use 16–128 letters, numbers, or . _ ~ -. The current token was not changed.';
  if (['CHANGE_PREVIEW_STALE', 'REVISION_CONFLICT', 'application.error.change_preview_stale', 'application.error.revision_conflict'].includes(code)) {
    return zh
      ? '配置在保存前发生变化，本次未提交。当前选择已保留，请再次保存以重新读取最新状态。'
      : 'The configuration changed before this save was admitted. Your choices are retained; save again to use the latest state.';
  }
  if (code === 'SERVICE_UNAVAILABLE' || code === 'application.error.gateway_unavailable') {
    return zh
      ? '本机服务暂不可用，或旧接入的登录项清理未完成，本次未提交。请检查 HiRoute 服务；若正在恢复旧接入，也检查其登录项状态。'
      : 'The local service is unavailable, or cleanup of a login item from an older connection is incomplete. Nothing was submitted. Check the HiRoute service, and for an older restore also check its login item.';
  }
  if (code === 'RESOURCE_NOT_FOUND' || code === 'application.error.resource_not_found') {
    return zh
      ? 'Agent 安装或配置已变化。请重新检测该 Agent 后保存；当前选择已保留。'
      : 'The Agent installation or configuration changed. Detect it again, then save; your choices are retained.';
  }
  return zh ? '操作未完成，当前输入已保留。' : 'The operation did not complete; your input is retained.';
}
