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
  const management = facet === 'management_skill';
  const blockedCheckScope = facet ? prerequisiteCheck(preview, facet) : null;
  return <div className="callout warn agent-feedback" role="status">
    <UiIcon name="warning" />
    <div>
    <strong>{text('当前无法应用，未修改配置。', 'Cannot apply yet. Configuration was not changed.')}</strong>
    {preview.blockers.map((block, index) => {
      const capabilities = new Set(block.capabilities?.map(capability => capability.capability) ?? []);
      const message = capabilities.has('ingress_authentication') && selected?.agent_id === 'agent_hermes_default'
        ? text('所选 Hermes 安装尚未满足此协议的原生接口和 SDK 要求。请检查本机依赖后重新预览。', 'The selected Hermes installation does not yet meet this protocol’s native interface and SDK requirements. Check local dependencies, then preview again.')
        : capabilities.has('ingress_authentication')
          ? text('本机认证兼容性尚未确认。', 'Local authentication compatibility is unconfirmed.')
        : block.reason === 'native_model_coverage_unavailable'
          ? `${text('保留原 Codex 模型需要先接入原订阅账号或 API 来源，并证明同账号路由。也可改选“只使用已配置的 HiRoute 模型”。', 'To keep native Codex models, connect the original subscription account or API source and prove same-account routes, or select HiRoute-only.')}${(block.model_ids ?? []).length ? ` ${block.model_ids!.join(', ')}` : ''}`
        : block.reason === 'native_default_invalid'
          ? `${text('原 Codex 默认模型没有同账号可用路由；请接入对应来源，或改选“只使用已配置的 HiRoute 模型”：', 'The native Codex default has no proven same-account route. Connect its source or select HiRoute-only: ')}${(block.model_ids ?? []).join(', ')}`
        : block.reason === 'restore_native_model_invalid'
          ? text('恢复将留下无效的原生默认模型。请在“连接详情与恢复”中明确选择一个原生模型后重试。', 'Restoring would leave an invalid native default. Choose a native model under Connection details and recovery, then retry.')
        : block.reason === 'native_routing_adapter_restore_required'
          ? text('先在连接详情中恢复 OpenCode 模型配置，再重新应用 routing_metadata=false。恢复会移除 HiRoute 拥有的元数据插件；模型路由可重新配置，其他插件保持不变。', 'Restore OpenCode model settings under connection details, then reapply with routing_metadata=false. Restore removes the metadata plugin owned by HiRoute; model routes can be configured again and other plugins stay unchanged.')
        : block.reason === 'native_routing_adapter_external_resource'
          ? text('HiRoute 元数据插件路径存在外部或借用文件。请在所选 OpenCode 配置目录核对并手动停用该文件，再重新预览；HiRoute 不会删除它。其他外部插件仍可能等待依赖安装。', 'An external or borrowed file exists at the HiRoute metadata plugin path. Review and disable it manually in the selected OpenCode configuration directory, then preview again; HiRoute will not delete it. Other external plugins may still wait for dependencies.')
        : block.reason === 'additional_default_in_use'
          ? text('当前默认模型仍引用将被移除的路由。请先在所选客户端中切换到其他模型，然后重试；HiRoute 不会替你更改默认模型。', 'The default model still uses a route being removed. Switch to another model in the selected client, then retry; HiRoute will not change the default for you.')
        : block.reason === 'qoder_default_in_use'
          ? text('Qoder 当前默认模型仍引用将被移除的 HiRoute 路由。请先在 Qoder 中通过 /model 切换到其他模型，再重试；HiRoute 不会替你更改默认模型。', 'Qoder’s current default still uses a HiRoute route being removed. Switch to another model with /model in Qoder, then retry; HiRoute will not change the default for you.')
        : block.reason === 'qoder_model_file_conflict'
          ? text('Qoder 中的受管路由配置已变化。请核对用户修改后重新预览；HiRoute 不会覆盖冲突字段或无关配置，任务协作无需停用。', 'Managed Qoder route settings have changed. Review your edits, then preview again; HiRoute will not overwrite conflicting fields or unrelated settings. Task collaboration can stay enabled.')
        : block.reason === 'hermes_model_file_conflict'
          ? text('无法安全编辑 Hermes 主模型或插件配置。请检查所选配置目录的格式、受管字段和同名插件后重新预览；当前配置未修改。', 'Hermes main-model or plugin configuration cannot be edited safely. Check the selected configuration format, managed fields and existing plugin, then preview again. Configuration was not changed.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default' && defaultChoice.kind === 'preserve_native'
          ? text('当前默认模型名称或所选路由无法用于 Codex Responses。请在“默认选择”中改选已勾选且支持该入口的路由；若要保留原生模型，请先接入对应来源。配置未修改。', 'The current default model name or a selected route cannot be used by Codex Responses. Under Default selection, choose an enabled route that supports this ingress; to preserve a native model, connect its source first. Configuration was not changed.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default'
          ? text('所选路由、固定来源或当前默认模型无法用于 Codex Responses。请核对路由是否已启用并支持该入口协议，或改选可用路由；配置未修改。', 'A selected route, fixed source, or current default model cannot be used by Codex Responses. Check that the route is enabled and supports this ingress protocol, or choose an available route. Configuration was not changed.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_hermes_default'
          ? text('所选路由当前不可用。请确认路由已启用，并支持所选 Hermes 接入协议。', 'The selected route is unavailable. Confirm it is enabled and supports the selected Hermes ingress protocol.')
        : block.reason === 'model_plan_unavailable'
          ? text('所选路由或原生默认模型当前不可用。请检查路由是否已启用、支持当前 Agent 的入口协议；Claude 的 Opus、Sonnet、Haiku 映射都需要可用的目标。', 'A selected route or native default model is unavailable. Check that routes are enabled and support this Agent’s ingress protocol; each Claude Opus, Sonnet and Haiku mapping needs a usable target.')
        : block.reason === 'codex_context_override'
          ? text('Codex 的显式窗口配置会覆盖计划设置。请移除有效配置中的 model_context_window 和 model_auto_compact_token_limit 后重试；HiRoute 不会删除这些配置。', 'Explicit Codex window settings override the plan. Remove model_context_window and model_auto_compact_token_limit from the effective configuration and retry; HiRoute will not delete these settings.')
        : block.reason === 'claude_plan_capability_unavailable'
          ? text('所选计划无法满足 Claude Code 的工具调用或流式请求能力，请检查全部候选模型。', 'A selected plan cannot meet Claude Code tool or streaming requirements. Check all candidate models.')
        : block.reason === 'claude_context_override'
          ? text('Claude Code 存在窗口覆盖或禁用自动压缩的设置。请移除进程、项目或托管配置中的窗口覆盖，以及禁用压缩的环境变量后重试。', 'Claude Code has window overrides or compaction disabled. Remove process, project, or managed window overrides and compaction-disabling environment variables, then retry.')
        : block.reason === 'claude_context_window_unsupported'
          ? text('所选计划的共同窗口低于 Claude Code 的 100K 最低值。请调整计划窗口或使用 Codex。', 'The shared plan window is below Claude Code’s 100K minimum. Adjust the plan window or use Codex.')
        : block.reason === 'skill_file_conflict'
          ? management
            ? text('同名 HiRoute 管理 Skill 内容不同；原文件已保留。请先处理该文件后再预览。', 'A HiRoute management Skill with different content already exists. The original was preserved; resolve it before previewing again.')
            : text('同名任务委派技能内容不同；原文件已保留。请先移走或明确处理该文件后再预览。', 'A task delegation skill with different content already exists. The original was preserved; move or explicitly resolve it before previewing again.')
        : capabilities.has('effective_configuration')
          ? text('无法确认当前有效配置。请检查启动进程的环境变量、项目和受管配置是否覆盖用户设置，处理冲突后重新保存；只更改 HOME 不会清除环境变量。', 'The effective configuration could not be confirmed. Check whether process environment variables, project or managed settings override user settings, resolve conflicts, then save again. Changing HOME does not clear environment variables.')
        : capabilities.has('atomic_managed_replace')
          ? text('无法安全写入 Agent 配置。请检查配置目录及目标文件的所有者、权限和符号链接，再重新保存；现有文件已保留。', 'The Agent configuration cannot be written safely. Check ownership, permissions and symbolic links of its directory and target file, then save again. Existing files were preserved.')
        : capabilities.has('model_catalog')
          ? text('当前模型目录尚不可用。请刷新 Agent 和路由资料，确认所选模型仍可用后重新保存。', 'The current model catalog is unavailable. Refresh Agent and route details, confirm the selected models remain available, then save again.')
        : block.reason === 'restore_point_unavailable'
          ? text('当前配置的恢复记录不可用。请刷新连接详情并核对原来的操作记录，再重试恢复。', 'The restore record for this configuration is unavailable. Refresh connection details and check the original operation before retrying restore.')
        : capabilities.has('skill_loading') || capabilities.has('trusted_cli_execution')
          ? management
            ? text('尚未确认此 Agent 能加载管理 Skill。请检查所选安装及本机 CLI。', 'Management Skill loading is unconfirmed. Check the selected Agent installation and local CLI.')
            : text('任务委派技能尚未确认。', 'Task delegation skill capability is unconfirmed.')
          : text('有前置条件尚未满足，请检查当前 Agent 状态。', 'A prerequisite is unmet. Check the current Agent state.');
      return <p key={index}>{message}</p>;
    })}
    {preview.unproven_native_model_ids?.length ? <p>{text('目录中尚未证明可由原账号调用的模型（不会开放给 Codex）：', 'Catalog names not proven callable on the original account (not exposed to Codex): ')}{preview.unproven_native_model_ids.join(', ')}</p> : null}
    {blockedCheckScope && selected && <button className="btn" type="button" disabled={disabled} onClick={event => onCheck(blockedCheckScope, event.currentTarget)}>{blockedCheckScope === 'native_authentication' ? text('重新检查本机认证', 'Check local authentication again') : text('重新检查任务委派技能', 'Check task delegation skill again')}</button>}
    </div>
  </div>;
}
