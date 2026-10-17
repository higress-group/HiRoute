export type ModelBudgetConflict = {
  schema: 'hiroute.agent-model-budget-conflict/v1';
  blockers: {
    agent_kind: 'qoder' | 'pi' | 'deepseek_harness'; context_id: string;
    plan_id: string; model_alias: string;
    reason: 'capacity_reduced' | 'plan_missing' | 'budget_unavailable';
    declared: { context_window_tokens: number; max_output_tokens: number };
    proposed: { context_window_tokens: number; max_output_tokens: number } | null;
  }[];
};

/** Accept only the safe typed capacity projection; raw wire/native error text is never copy. */
export function modelBudgetConflict(error: unknown): ModelBudgetConflict | null {
  if (!error || typeof error !== 'object') return null;
  const source = error as { model_budget_conflict?: unknown; envelope?: { error?: { model_budget_conflict?: unknown } } };
  const value = (source.model_budget_conflict ?? source.envelope?.error?.model_budget_conflict) as ModelBudgetConflict | undefined;
  const positive = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
  const budget = (value: ModelBudgetConflict['blockers'][number]['declared'] | null | undefined) => value
    && positive(value.context_window_tokens) && positive(value.max_output_tokens);
  if (value?.schema !== 'hiroute.agent-model-budget-conflict/v1' || !Array.isArray(value.blockers) || !value.blockers.length) return null;
  const valid = value.blockers.every(blocker => blocker
    && ['qoder', 'pi', 'deepseek_harness'].includes(blocker.agent_kind)
    && typeof blocker.context_id === 'string' && /^agent-context\/[A-Za-z0-9/_:-]{1,242}$/.test(blocker.context_id)
    && typeof blocker.plan_id === 'string' && /^[A-Za-z0-9/_:-]{1,256}$/.test(blocker.plan_id)
    && typeof blocker.model_alias === 'string' && /^(?:hiroute\/[0-9a-f]{16}|[a-z0-9][a-z0-9._-]{0,63})$/.test(blocker.model_alias)
    && budget(blocker.declared)
    && (blocker.reason === 'capacity_reduced'
      ? budget(blocker.proposed) && (blocker.proposed!.context_window_tokens < blocker.declared.context_window_tokens || blocker.proposed!.max_output_tokens < blocker.declared.max_output_tokens)
      : ['plan_missing', 'budget_unavailable'].includes(blocker.reason) && blocker.proposed === null));
  return valid ? value : null;
}

/** Read only structured error codes, never render a wire envelope as product copy. */
export function planErrorCode(error: unknown): string {
  if (typeof error === 'string') return /^[A-Z][A-Z0-9_]{0,80}$/.test(error) ? error : 'REQUEST_FAILED';
  if (!error || typeof error !== 'object') return 'REQUEST_FAILED';
  const value = error as { code?: unknown; safe_error_code?: unknown; failure?: { code?: unknown }; envelope?: { error?: { code?: unknown } } };
  const code = planErrorCode(value.envelope?.error?.code ?? value.failure?.code ?? value.code ?? value.safe_error_code);
  return modelBudgetConflict(error) && ['REVISION_CONFLICT', 'QODER_MODEL_BUDGET_CONFLICT', 'AGENT_MODEL_BUDGET_CONFLICT'].includes(code)
    ? 'AGENT_MODEL_BUDGET_CONFLICT' : code;
}
export function planErrorMessage(error: unknown, language: 'zh' | 'en'): string {
  const conflict = modelBudgetConflict(error);
  if (conflict && planErrorCode(error) === 'AGENT_MODEL_BUDGET_CONFLICT') {
    const zh = language === 'zh';
    const names = { qoder: 'Qoder', pi: 'Pi', deepseek_harness: 'DeepSeek Harness' };
    const number = (value: number) => value.toLocaleString('en-US');
    const blockers = conflict.blockers.map(blocker => {
      const changes: string[] = [];
      if (blocker.proposed) {
        if (blocker.proposed.context_window_tokens < blocker.declared.context_window_tokens) changes.push(`${zh ? '上下文' : 'context'} ${number(blocker.declared.context_window_tokens)} → ${number(blocker.proposed.context_window_tokens)}`);
        if (blocker.proposed.max_output_tokens < blocker.declared.max_output_tokens) changes.push(`${zh ? '最大输出' : 'max output'} ${number(blocker.declared.max_output_tokens)} → ${number(blocker.proposed.max_output_tokens)}`);
      } else changes.push(blocker.reason === 'plan_missing' ? (zh ? '计划不可用' : 'plan unavailable') : (zh ? '预算不可计算' : 'budget unavailable'));
      return `${names[blocker.agent_kind]}「${blocker.model_alias}」: ${changes.join(', ')}`;
    }).join('; ');
    return zh
      ? `已接入的模型路由容量受保护。${blockers}。请在 Agent 页面调整这些连接，取消接入受影响计划或停用其模型路由，再发布并重新接入。任务协作无需停用，当前输入已保留。`
      : `Installed model routing capacity is protected. ${blockers}. On the Agent page, remove the affected plan from these connections or disable their model routing, then publish and reconnect. Task collaboration can stay enabled. Your input is retained.`;
  }
  const messages: Record<string, [string, string]> = {
    PLAN_UNCHANGED: ['没有待发布更改。', 'No changes to publish.'],
    EDITOR_INVALID: ['请检查名称、使用场景及模型设置，当前输入已保留。', 'Check the name, purpose and model settings. Your input is retained.'],
    INVALID_ARGUMENTS: ['部分路由设置未通过校验，请检查输入后重试。', 'Some routing settings failed validation. Check your input and retry.'],
    REVISION_CONFLICT: ['另一会话更改了此路由。已重新读取当前状态，你的草稿已保留。请核对最新配置后重新编辑并提交。', 'Another session changed this route. The current state was reloaded and your draft is retained. Review the latest configuration before editing and submitting again.'],
    CONFLICT: ['配置已在其他地方变化，请重新读取后再提交。当前输入已保留。', 'The configuration changed elsewhere. Reload before submitting. Your input is retained.'],
    QODER_MODEL_BUDGET_CONFLICT: ['新预算低于 Qoder 当前已接入模型声明的容量。请先在 Agent 页面停用 Qoder 的模型路由，再发布此计划并重新配置模型路由；任务协作无需停用。当前输入已保留。', 'The new budget is below the capacity declared by the current Qoder model connection. Disable Qoder model routing on the Agent page, publish this plan, then configure model routing again. Task collaboration can stay enabled. Your input is retained.'],
    AGENT_MODEL_BUDGET_CONFLICT: ['已接入的 Agent 模型路由容量受保护。请读取连接状态，调整受影响的模型路由后重新发布。任务协作无需停用，当前输入已保留。', 'Installed Agent model routing capacity is protected. Read connection status and adjust the affected model routing before publishing again. Task collaboration can stay enabled. Your input is retained.'],
    PLAN_HEAD_STALE: ['当前路由已在其他地方发布新版本。你的输入已保留，请读取当前生效配置后重新编辑。', 'A newer route version was published elsewhere. Your input is retained; reload the active route before editing again.'],
    DRAFT_REVISION_STALE: ['该草稿已在其他地方更新或移除。你的输入已保留，请读取最新草稿和生效配置后重试。', 'This draft was updated or removed elsewhere. Your input is retained; reload the latest draft and active route before retrying.'],
    RESIDENT_UNAVAILABLE: ['暂时无法连接本机服务，当前输入已保留。请稍后重试。', 'Cannot connect to the local service. Your input is retained. Try again.'],
    DAEMON_UNAVAILABLE: ['暂时无法连接本机服务，当前输入已保留。请稍后重试。', 'Cannot connect to the local service. Your input is retained. Try again.'],
    RESPONSE_DATA_INVALID: ['未能读取操作结果，当前输入已保留。请先核实路由状态再重试。', 'Could not read the operation result. Your input is retained. Check the route state before retrying.'],
  };
  return (messages[planErrorCode(error)] ?? ['操作未能确认完成，当前输入已保留。请先核实路由状态再重试。', 'Completion could not be confirmed. Your input is retained. Check the route state before retrying.'])[language === 'zh' ? 0 : 1];
}
