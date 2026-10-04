import React from 'react';
import type { Dispatch, SetStateAction } from 'react';
import type { Plan } from '../../plan-editor';
import type { Agent, AgentModelSelection } from './types';
import type { AgentEditorValues } from './editor-state';
import { commonClaudePlan } from './editor-state';
import { agentEcosystem } from './ecosystems';
import { CodexModelEditor } from './CodexModelEditor';
import { ClaudeModelEditor } from './ClaudeModelEditor';
import { QoderModelEditor } from './QoderModelEditor';

export type AgentModelEditorProps = {
  agent: Agent;
  values: AgentEditorValues;
  plans: Plan[];
  language: 'zh' | 'en';
  disabled: boolean;
  invalid: boolean;
  codexMode: 'profile' | 'root';
  onCodexMode: (mode: 'profile' | 'root') => void;
  onChange: Dispatch<SetStateAction<AgentEditorValues>>;
  onEdited: (clearNotice?: boolean) => void;
  onCreatePlan?: () => void;
};

export function AgentModelEditor(props: AgentModelEditorProps) {
  switch (agentEcosystem(props.agent.agent_id)) {
    case 'codex': return <CodexModelEditor {...props} />;
    case 'claude': return <ClaudeModelEditor {...props} />;
    case 'qoder': return <QoderModelEditor {...props} />;
    default: return null;
  }
}

export function modelSelectionSummary(selection: AgentModelSelection | null | undefined, plans: Plan[], language: 'zh' | 'en'): string {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const planName = (id: string) => plans.find(plan => plan.agent_plan_id === id)?.desired.display_name;
  switch (selection?.mode) {
    case 'codex_default':
      return selection.default_selection.kind === 'plan'
        ? planName(selection.default_selection.plan_id) ?? text('当前路由不可用', 'Current route unavailable')
        : selection.default_selection.kind === 'fixed_model'
          ? selection.default_selection.client_model_id
          : text('使用 Codex 当前默认模型名称', 'Use the current Codex default model name');
    case 'claude_launcher': {
      const sharedPlan = commonClaudePlan(selection.preset_mappings);
      return sharedPlan
        ? text('三个档位共用：', 'All presets use: ') + (planName(sharedPlan) ?? text('当前路由不可用', 'Current route unavailable'))
        : Object.entries(selection.preset_mappings).map(([preset, choice]) => `${preset[0].toUpperCase() + preset.slice(1)}: ${choice.kind === 'plan' ? planName(choice.plan_id) ?? text('当前路由不可用', 'Current route unavailable') : text('保留原生', 'Native')}`).join(' · ');
    }
    case 'qoder_additional':
      return selection.allowed_plan_ids.map(id => planName(id) ?? text('当前路由不可用', 'Current route unavailable')).join(' · ');
    default: return '';
  }
}
