const FAILURE_SCHEMA = 'hiroute.agent-collaboration-check-failure/v1';

const messages = {
  login_required: {
    zh: '当前客户端需要登录。请正常打开已选客户端完成登录，然后重试协作检查。',
    en: 'The selected client needs sign-in. Open it normally, sign in, then retry the collaboration check.',
  },
  installed_skill_missing: {
    zh: '未找到已安装的协作 Skill。请检查协作目标，再选择安全恢复或重新启用协作，然后重试检查。现有内容不会自动覆盖。',
    en: 'The installed collaboration Skill was not found. Review the collaboration target, then safely restore or re-enable collaboration and retry the check. Existing content will not be overwritten automatically.',
  },
  installed_skill_changed: {
    zh: '已安装的协作 Skill 内容发生变化。请检查协作目标，再选择安全恢复或重新启用协作，然后重试检查。现有内容不会自动覆盖。',
    en: 'The installed collaboration Skill has changed. Review the collaboration target, then safely restore or re-enable collaboration and retry the check. Existing content will not be overwritten automatically.',
  },
  installed_skill_invalid: {
    zh: '已安装的协作 Skill 无法用于本次检查。请检查协作目标，再选择安全恢复或重新启用协作，然后重试检查。现有内容不会自动覆盖。',
    en: 'The installed collaboration Skill cannot be used for this check. Review the collaboration target, then safely restore or re-enable collaboration and retry the check. Existing content will not be overwritten automatically.',
  },
  native_context_unavailable: {
    zh: '所选客户端上下文暂不可用。请重新选择可用的客户端上下文，然后重试协作检查。',
    en: 'The selected client context is unavailable. Select an available client context again, then retry the collaboration check.',
  },
  native_context_changed: {
    zh: '所选客户端上下文已经变化。请重新选择并确认客户端上下文，然后重试协作检查。',
    en: 'The selected client context has changed. Select and confirm the client context again, then retry the collaboration check.',
  },
  dependency_unavailable: {
    zh: '协作检查所需的客户端或命令行入口不可用。请重新检测并选择可用的安装，然后重试检查。',
    en: 'A client or command-line entry required for the collaboration check is unavailable. Detect and select an available installation again, then retry the check.',
  },
  check_timed_out: {
    zh: '协作检查超时，尚未确认协作可用。请确认所选客户端可正常使用，然后重试检查。',
    en: 'The collaboration check timed out; collaboration has not been verified. Confirm the selected client is available, then retry the check.',
  },
  verification_failed: {
    zh: '协作验证未通过。请检查所选客户端和协作 Skill 的状态，然后重试检查。',
    en: 'Collaboration verification failed. Check the selected client and collaboration Skill, then retry the check.',
  },
};

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/** Project only the versioned backend reason; native error details are not display text. */
export function collaborationCheckFailureMessage(error: unknown, language: 'zh' | 'en'): string | null {
  if (!record(error) || error.source !== 'backend' || !record(error.envelope)) return null;
  const { envelope } = error;
  if (!record(envelope.error) || envelope.error.details_schema !== FAILURE_SCHEMA
    || !record(envelope.data) || envelope.data.schema !== FAILURE_SCHEMA) return null;
  const { reason } = envelope.data;
  if (typeof reason !== 'string' || !Object.hasOwn(messages, reason)) return null;
  return messages[reason as keyof typeof messages][language];
}
