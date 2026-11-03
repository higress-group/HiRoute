import React from 'react';
import { AgentSkillDetails } from './AgentSkillDetails';
import { CodexDesktopDetails } from './CodexDesktopPanel';
import { agentSupportsCollaboration, agentSupportsManagement } from './ecosystems';
import { agentModelStatus } from './status';
import type { Agent } from './types';

/** Secondary guidance is mounted only when the user opens connection details. */
export function AgentUsageDetails({ agent, language }: { agent: Agent; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const selection = agentModelStatus(agent)?.current_selection;
  return <div className="agent-usage-details">
    {agent.agent_id === 'agent_codex_desktop' && <CodexDesktopDetails agent={agent} language={language} />}
    {selection?.mode === 'claude_launcher' && <section><h4>{text('在 Claude Code 中使用', 'Using Claude Code')}</h4><p>{text('直接启动 claude，通过 Opus、Sonnet、Haiku 选择已映射路由。Claude Default 由当前账号决定，需通过实际调用验证。', 'Start claude and use Opus, Sonnet or Haiku to select mapped routes. Claude Default depends on the account and needs a live call to verify.')}</p></section>}
    {selection?.mode === 'qoder_additional' && <section><h4>{text('在 Qoder 中使用', 'Using Qoder')}</h4><p>{text('重新启动 Qoder，在模型列表选择已添加的 HiRoute 路由。需要登录时，在终端启动 Qoder CLI 并完成 /login。', 'Restart Qoder and select an added HiRoute route. If sign-in is needed, start Qoder CLI in a terminal and complete /login.')}</p></section>}
    {agentSupportsCollaboration(agent.agent_id) && <section><h4>{text('任务路由', 'Task routing')}</h4><p>{text('对 Agent 说：“使用 HiRoute，将当前任务委派给【路由名称】执行。”可用路由在“智能路由”中管理。', 'Ask the Agent: “Use HiRoute to delegate this task to [route name].” Manage available routes under Smart routing.')}</p><AgentSkillDetails resource={agent.settings?.collaboration?.resource} language={language} /></section>}
    {agentSupportsManagement(agent.agent_id) && <section><h4>{text('管理技能', 'Management skill')}</h4><p>{text('让此 Agent 通过本机 CLI 管理模型来源和智能路由。它可独立于模型与任务路由启用。', 'Let this Agent manage sources and smart routes through the local CLI, independently of model and task routing.')}</p><AgentSkillDetails resource={agent.settings?.management_skill?.resource} language={language} management /></section>}
  </div>;
}
