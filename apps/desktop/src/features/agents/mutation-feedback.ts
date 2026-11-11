import type { AgentMutationOutcome } from './types';
export type { AgentMutationOutcome } from './types';

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

export function agentDisableMessage(state: string, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  if (state === 'succeeded') return zh
    ? '模型配置已恢复，连接已停用。'
    : 'Model settings restored; the connection is disabled.';
  if (state === 'rolled_back') return zh
    ? '停用未完成，本次变更已回滚，原连接仍有效。请处理冲突或失败原因后重新点击“停用”。'
    : 'Disable did not complete. This change was rolled back; the original connection remains active. Resolve the conflict or failure, then click Disable again.';
  if (state === 'needs_attention') return zh
    ? '停用未完成，回滚尚未确认。请查看操作状态并处理恢复问题。'
    : 'Disable did not complete and rollback is unconfirmed. Check the operation status and resolve the recovery issue.';
  return zh ? '正在恢复模型配置并停用连接…' : 'Restoring model settings and disabling the connection…';
}

export function agentActionErrorMessage(code: string, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  if (code === 'CODEX_DESKTOP_NOT_INSTALLED') return zh
    ? '未发现 Codex Desktop，请安装后重新扫描。模型路由配置已保留。'
    : 'Codex Desktop was not found. Install it and scan again. Your routing settings are retained.';
  if (code === 'CODEX_DESKTOP_PREPARE_FAILED') return zh
    ? '无法准备独立实例或共享技能。请检查实例目录的访问权限，再重试启动。已保存的模型路由保持有效。'
    : 'Could not prepare the instance or shared skills. Check directory access, then retry opening. Saved model routing remains active.';
  if (code === 'CODEX_DESKTOP_LAUNCH_FAILED') return zh
    ? '独立窗口未能启动。已保存的模型路由保持有效，可再次点击“启动独立窗口”。'
    : 'The separate window could not open. Saved model routing remains active; try Open separate window again.';
  if (code === 'CODEX_DESKTOP_HOST_REQUIRED') return zh
    ? '请在 macOS 或 Windows 版 HiRoute 中启动，或在服务电脑的桌面终端运行启动命令。'
    : 'Open from HiRoute on macOS or Windows, or run the launch command in a desktop terminal on the service computer.';
  if (code === 'CODEX_DESKTOP_DISPLAY_REQUIRED') return zh
    ? '请在服务电脑已登录的图形桌面终端运行启动命令。已保存的路由和会话会保留。'
    : 'Run the launch command from the service computer’s logged-in graphical desktop. Saved routing and sessions are retained.';
  if (code === 'AGENT_STATUS_UNVERIFIED') return zh
    ? '接入状态已变化，请重新读取并确认模型路由已启用后再启动。'
    : 'Connection state changed. Refresh and confirm model routing is enabled before opening.';
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

/** Only a recognized backend reason becomes recovery guidance; never display raw native errors. */
export function agentSettingsFailureMessage(error: unknown, language: 'zh' | 'en'): string | null {
  if (!error || typeof error !== 'object' || !('source' in error) || error.source !== 'backend'
    || !('envelope' in error) || !error.envelope || typeof error.envelope !== 'object'
    || !('error' in error.envelope)) return null;
  const detail = error.envelope.error;
  if (!detail || typeof detail !== 'object' || !('code' in detail) || !('message_key' in detail)) return null;
  const zh = language === 'zh';
  if (detail.code === 'CAPABILITY_DENIED' && detail.message_key === 'error.codex_profile_conflict') {
    return zh
      ? 'Codex 配置无法安全合并，尚未保存。请检查配置文件格式、访问权限及已有的同名 HiRoute 配置。当前选择已保留，处理冲突后再保存。'
      : 'Codex settings cannot be merged safely. Nothing was saved. Check the file format, access permissions and existing HiRoute names. Your choices are retained; resolve the conflict before saving again.';
  }
  if (detail.code !== 'ACTION_REQUIRED') return null;
  switch (detail.message_key) {
    case 'error.dsh_desktop_patch_invalid':
      return zh
        ? 'DeepSeek Harness Desktop 配置校验失败。请检查用户目录和 Desktop profile 下的 cordis.patch.yml 的格式和访问权限。'
        : 'DeepSeek Harness Desktop settings failed validation. Check the format and permissions of cordis.patch.yml in the DSH home and Desktop profile.';
    case 'error.dsh_desktop_manifest_invalid':
      return zh
        ? 'DeepSeek Harness Desktop 的 profile 清单无效。请检查 Desktop profile 下的 package.json 的格式和访问权限。'
        : 'DeepSeek Harness Desktop could not verify the profile manifest. Check the format and permissions of package.json in the Desktop profile.';
    case 'error.dsh_desktop_custom_profile':
      return zh
        ? '模型路由接入要求 DeepSeek Harness Desktop 使用默认组合包，且没有额外依赖。请检查 Desktop profile 的插件配置。'
        : 'Model routing requires the default DeepSeek Harness Desktop bundles with no extra dependencies. Check the plugins configured in the Desktop profile.';
    case 'error.dsh_desktop_runtime_unavailable':
      return zh
        ? 'DeepSeek Harness Desktop 的安装文件无法验证。请检查安装后重新扫描。'
        : 'DeepSeek Harness Desktop installation files could not be verified. Check the installation and scan again.';
    case 'error.dsh_desktop_probe_failed':
      return zh
        ? 'DeepSeek Harness Desktop 内置 CLI 的默认配置检查失败。请检查安装后重试。'
        : 'The bundled DeepSeek Harness Desktop CLI could not check the default configuration. Check the installation and try again.';
    case 'error.dsh_desktop_model_adapter_missing':
      return zh
        ? 'DeepSeek Harness Desktop 的默认配置缺少模型适配器 llm-pi-ai。请检查客户端版本。'
        : 'The default DeepSeek Harness Desktop configuration is missing the llm-pi-ai model adapter. Check the client version.';
    default:
      return null;
  }
}
