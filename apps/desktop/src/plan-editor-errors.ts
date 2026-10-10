/** Read only structured error codes, never render a wire envelope as product copy. */
export function planErrorCode(error: unknown): string {
  if (typeof error === 'string') return /^[A-Z][A-Z0-9_]{0,80}$/.test(error) ? error : 'REQUEST_FAILED';
  if (!error || typeof error !== 'object') return 'REQUEST_FAILED';
  const value = error as { code?: unknown; failure?: { code?: unknown }; envelope?: { error?: { code?: unknown } } };
  return planErrorCode(value.envelope?.error?.code ?? value.failure?.code ?? value.code);
}
export function planErrorMessage(error: unknown, language: 'zh' | 'en'): string {
  const messages: Record<string, [string, string]> = {
    PLAN_UNCHANGED: ['没有待发布更改。', 'No changes to publish.'],
    EDITOR_INVALID: ['请检查名称、使用场景及模型设置，当前输入已保留。', 'Check the name, purpose and model settings. Your input is retained.'],
    INVALID_ARGUMENTS: ['部分路由设置未通过校验，请检查输入后重试。', 'Some routing settings failed validation. Check your input and retry.'],
    CONFLICT: ['配置已在其他地方变化，请重新读取后再提交。当前输入已保留。', 'The configuration changed elsewhere. Reload before submitting. Your input is retained.'],
    QODER_MODEL_BUDGET_CONFLICT: ['新预算低于 Qoder 当前已接入模型声明的容量。请先在 Agent 页面停用 Qoder 的模型路由，再发布此计划并重新配置模型路由；任务协作无需停用。当前输入已保留。', 'The new budget is below the capacity declared by the current Qoder model connection. Disable Qoder model routing on the Agent page, publish this plan, then configure model routing again. Task collaboration can stay enabled. Your input is retained.'],
    PLAN_HEAD_STALE: ['当前路由已在其他地方发布新版本。你的输入已保留，请读取当前生效配置后重新编辑。', 'A newer route version was published elsewhere. Your input is retained; reload the active route before editing again.'],
    DRAFT_REVISION_STALE: ['该草稿已在其他地方更新或移除。你的输入已保留，请读取最新草稿和生效配置后重试。', 'This draft was updated or removed elsewhere. Your input is retained; reload the latest draft and active route before retrying.'],
    RESIDENT_UNAVAILABLE: ['暂时无法连接本机服务，当前输入已保留。请稍后重试。', 'Cannot connect to the local service. Your input is retained. Try again.'],
    DAEMON_UNAVAILABLE: ['暂时无法连接本机服务，当前输入已保留。请稍后重试。', 'Cannot connect to the local service. Your input is retained. Try again.'],
    RESPONSE_DATA_INVALID: ['未能读取操作结果，当前输入已保留。请先核实路由状态再重试。', 'Could not read the operation result. Your input is retained. Check the route state before retrying.'],
  };
  return (messages[planErrorCode(error)] ?? ['操作未能确认完成，当前输入已保留。请先核实路由状态再重试。', 'Completion could not be confirmed. Your input is retained. Check the route state before retrying.'])[language === 'zh' ? 0 : 1];
}
