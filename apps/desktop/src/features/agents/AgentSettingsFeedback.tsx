import React from 'react';
import { UiIcon } from '../../ui';
import { prerequisiteCheck } from './settings-request';
import type { Agent, AgentCheckScope, AgentDefaultChoice, AgentFacet, Preview } from './types';

/** Translate authoritative Preview blockers without making capability or recovery decisions. */
export function AgentSettingsFeedback({ preview, agent: selected, facet, defaultChoice, language, disabled, onCheck }: {
  preview: Preview | null;
  agent?: Agent;
  facet?: AgentFacet;
  defaultChoice: AgentDefaultChoice;
  language: 'zh' | 'en';
  disabled: boolean;
  onCheck: (scope: AgentCheckScope, source: HTMLElement) => void;
}) {
  if (!preview || preview.applicable) return null;
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const blockedCheckScope = facet ? prerequisiteCheck(preview, facet) : null;
  return <div className="callout warn agent-feedback" role="status">
    <UiIcon name="warning" />
    <div>
    <strong>{text('当前无法应用，未修改配置。', 'Cannot apply yet. Configuration was not changed.')}</strong>
    {preview.blockers.map((block, index) => {
      const capabilities = new Set(block.capabilities?.map(capability => capability.capability) ?? []);
      const message = capabilities.has('ingress_authentication')
        ? text('本机认证兼容性尚未确认。', 'Local authentication compatibility is unconfirmed.')
        : block.reason === 'native_model_coverage_unavailable'
          ? `${text('保留原 Codex 模型需要先接入原订阅账号或 API 来源，并证明同账号路由。也可改选“只使用已配置的 HiRoute 模型”。', 'To keep native Codex models, connect the original subscription account or API source and prove same-account routes, or select HiRoute-only.')}${(block.model_ids ?? []).length ? ` ${block.model_ids!.join(', ')}` : ''}`
        : block.reason === 'native_default_invalid'
          ? `${text('原 Codex 默认模型没有同账号可用路由；请接入对应来源，或改选“只使用已配置的 HiRoute 模型”：', 'The native Codex default has no proven same-account route. Connect its source or select HiRoute-only: ')}${(block.model_ids ?? []).join(', ')}`
        : block.reason === 'restore_native_model_invalid'
          ? text('恢复将留下无效的原生默认模型。请在“连接详情与恢复”中明确选择一个原生模型后重试。', 'Restoring would leave an invalid native default. Choose a native model under Connection details and recovery, then retry.')
        : block.reason === 'additional_default_in_use'
          ? text('当前默认模型仍引用将被移除的路由。请先在 Pi 中通过 /model 切换到其他模型，然后重试。', 'The default model still uses a route being removed. Switch to another model with /model in Pi, then retry.')
        : block.reason === 'qoder_default_in_use'
          ? text('Qoder 当前默认模型仍引用将被移除的 HiRoute 路由。请先在 Qoder 中通过 /model 切换到其他模型，再重试；HiRoute 不会替你更改默认模型。', 'Qoder’s current default still uses a HiRoute route being removed. Switch to another model with /model in Qoder, then retry; HiRoute will not change the default for you.')
        : block.reason === 'qoder_model_file_conflict'
          ? text('Qoder 中的受管路由配置已变化。请核对用户修改后重新预览；HiRoute 不会覆盖冲突字段或无关配置，任务协作无需停用。', 'Managed Qoder route settings have changed. Review your edits, then preview again; HiRoute will not overwrite conflicting fields or unrelated settings. Task collaboration can stay enabled.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default' && defaultChoice.kind === 'preserve_native'
          ? text('当前默认模型名称或所选路由无法用于 Codex Responses。请在“默认选择”中改选已勾选且支持该入口的路由；若要保留原生模型，请先接入对应来源。配置未修改。', 'The current default model name or a selected route cannot be used by Codex Responses. Under Default selection, choose an enabled route that supports this ingress; to preserve a native model, connect its source first. Configuration was not changed.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default'
          ? text('所选路由、固定来源或当前默认模型无法用于 Codex Responses。请核对路由是否已启用并支持该入口协议，或改选可用路由；配置未修改。', 'A selected route, fixed source, or current default model cannot be used by Codex Responses. Check that the route is enabled and supports this ingress protocol, or choose an available route. Configuration was not changed.')
        : block.reason === 'codex_context_override'
          ? text('Codex 的显式窗口配置会覆盖计划设置。请移除有效配置中的 model_context_window 和 model_auto_compact_token_limit 后重试；HiRoute 不会删除这些配置。', 'Explicit Codex window settings override the plan. Remove model_context_window and model_auto_compact_token_limit from the effective configuration and retry; HiRoute will not delete these settings.')
        : block.reason === 'claude_plan_capability_unavailable'
          ? text('所选计划无法满足 Claude Code 的工具调用或流式请求能力，请检查全部候选模型。', 'A selected plan cannot meet Claude Code tool or streaming requirements. Check all candidate models.')
        : block.reason === 'claude_context_override'
          ? text('Claude Code 存在窗口覆盖或禁用自动压缩的设置。请移除进程、项目或托管配置中的窗口覆盖，以及禁用压缩的环境变量后重试。', 'Claude Code has window overrides or compaction disabled. Remove process, project, or managed window overrides and compaction-disabling environment variables, then retry.')
        : block.reason === 'claude_context_window_unsupported'
          ? text('所选计划的共同窗口低于 Claude Code 的 100K 最低值。请调整计划窗口或使用 Codex。', 'The shared plan window is below Claude Code’s 100K minimum. Adjust the plan window or use Codex.')
        : block.reason === 'skill_file_conflict'
          ? text('同名任务委派技能内容不同；原文件已保留。请先移走或明确处理该文件后再预览。', 'A task delegation skill with different content already exists. The original was preserved; move or explicitly resolve it before previewing again.')
        : capabilities.has('skill_loading') || capabilities.has('trusted_cli_execution')
          ? text('任务委派技能尚未确认。', 'Task delegation skill capability is unconfirmed.')
          : text('有前置条件尚未满足，请检查当前 Agent 状态。', 'A prerequisite is unmet. Check the current Agent state.');
      return <p key={index}>{message}</p>;
    })}
    {preview.unproven_native_model_ids?.length ? <p>{text('目录中尚未证明可由原账号调用的模型（不会开放给 Codex）：', 'Catalog names not proven callable on the original account (not exposed to Codex): ')}{preview.unproven_native_model_ids.join(', ')}</p> : null}
    {blockedCheckScope && selected && <button className="btn" type="button" disabled={disabled} onClick={event => onCheck(blockedCheckScope, event.currentTarget)}>{blockedCheckScope === 'native_authentication' ? text('重新检查本机认证', 'Check local authentication again') : text('重新检查任务委派技能', 'Check task delegation skill again')}</button>}
    </div>
  </div>;
}
