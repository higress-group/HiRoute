import { planDisplayNameError } from '../../plan-display-name.ts';

export type TrialBlocker = {
  code: string;
  category: 'user_input' | 'loading' | 'recoverable_read' | 'authority' | 'unavailable';
  target: 'trial' | 'trial-reasoning' | 'primary' | 'primary-reasoning' | 'decision' | 'agent' | 'presets' | 'name' | 'trust' | 'refresh';
  message: string;
};

type Inputs = {
  language: 'zh' | 'en'; trialId: string; trialUsable: boolean; trialReasoning?: 'selected' | 'choice_required' | 'invalid';
  primaryId: string; primaryUsable: boolean; primaryReasoning?: 'selected' | 'choice_required' | 'invalid';
  connectionOpen: boolean; decisionUsable: boolean; agentId: string; agentMatches: boolean; agentSeedKnown: boolean;
  connectionPending?: 'saving' | 'uncertain' | null;
  claude: boolean; presets: number; name: string; threshold: number; custom: boolean;
  authoring: boolean; loading: boolean; readError: boolean; decisionError: boolean; agentError: boolean; mutable: boolean; authority: boolean;
};

/** One ordered source for presentation and the transition guard. */
export function trialReadiness(i: Inputs) {
  const blockers: TrialBlocker[] = [];
  const add = (code: string, category: TrialBlocker['category'], target: TrialBlocker['target'], zh: string, en: string) =>
    blockers.push({code, category, target, message: i.language === 'zh' ? zh : en});
  if (!i.trialId) add('trial-model', 'user_input', 'trial', '选择要试用的模型', 'Choose a model to try');
  else if (!i.trialUsable) add('trial-binding', 'user_input', 'trial', '选择可用的试用接入', 'Choose an available trial connection');
  if (i.connectionOpen) add('trial-connection-open', 'user_input', 'trial', '完成或返回接入流程', 'Finish or return from the connection flow');
  if (i.connectionPending) add('trial-connection-pending', 'recoverable_read', 'trial', '接入保存结果仍待核实，请查看接入结果', 'The connection save is still being confirmed. Review the connection result');
  if (i.trialReasoning && i.trialReasoning !== 'selected') add('trial-reasoning', 'user_input', 'trial-reasoning', '选择有效的试用思考设置', 'Choose valid trial reasoning');
  if (!i.primaryId || !i.primaryUsable) add('primary-binding', 'user_input', 'primary', '选择主力模型及可用接入', 'Choose a Primary model and available connection');
  else if (i.trialId === i.primaryId) add('primary-distinct', 'user_input', 'primary', '主力模型须与试用模型不同', 'Choose a different model for Primary');
  if (i.primaryReasoning && i.primaryReasoning !== 'selected') add('primary-reasoning', 'user_input', 'primary-reasoning', '选择有效的主力思考设置', 'Choose valid Primary reasoning');
  if (!i.decisionUsable) add('decision', 'user_input', 'decision', '选择可用的决策连接', 'Choose an available decision connection');
  if (!i.agentId) add('agent', 'user_input', 'agent', '选择 Agent', 'Choose an Agent');
  if (i.agentId && !i.agentMatches && !i.loading && !i.agentError) add('agent-current', 'recoverable_read', 'agent', '重新读取所选 Agent', 'Reload the selected Agent');
  if (i.agentId && i.agentMatches && !i.agentSeedKnown) add('agent-seed', 'unavailable', 'agent', '检查 Agent 的现有配置', 'Check the Agent’s current configuration');
  if (i.claude && !i.presets) add('presets', 'user_input', 'presets', '至少选择一个预设', 'Choose at least one preset');
  const nameError = planDisplayNameError(i.name, i.language);
  if (nameError) blockers.push({code: 'name', category: 'user_input', target: 'name', message: nameError});
  if (!i.custom && (!Number.isInteger(i.threshold) || i.threshold < 0 || i.threshold > 1000)) add('threshold', 'user_input', 'trust', '选择有效的起始信任度', 'Choose a valid starting trust');
  if (i.loading) add('loading', 'loading', 'refresh', '正在读取当前配置', 'Reading the current configuration');
  if (i.readError) add('read', 'recoverable_read', 'refresh', '无法读取试用配置，请重试', 'Trial configuration could not be read. Retry');
  if (i.decisionError) add('decision-read', 'recoverable_read', 'decision', '无法读取决策连接，请重试', 'Decision connections could not be read. Retry');
  if (i.agentError) add('agent-read', 'recoverable_read', 'agent', '无法读取 Agent 配置，请重试', 'Agent settings could not be read. Retry');
  if (!i.loading && !i.readError && !i.authoring) add('authoring', 'unavailable', 'refresh', '路由编辑信息不可用，请重试', 'Route authoring information is unavailable. Retry');
  if (!i.mutable || (!i.loading && i.agentMatches && !i.authority)) add('authority', 'authority', 'refresh', '当前配置不可写，请恢复可写状态后重试', 'The current configuration is read-only. Restore write access and retry');
  return { blockers, canAdvance: blockers.length === 0, systemBlocked: blockers.some(b => b.category !== 'user_input') };
}
