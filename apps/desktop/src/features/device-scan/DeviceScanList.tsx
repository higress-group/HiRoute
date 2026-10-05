import type { Agent } from '../../agents';
import type { SubscriptionCandidate } from '../subscriptions/types';
import { BrandIcon, UiIcon } from '../../ui';
import { agentBrandFromId } from '../../ui/BrandIcon';

export type ComputeScanItem = {
  agent_id: string;
  supported: boolean;
  native_provider_id?: string;
  configuration_state: string;
  connection_option_id?: string;
  observed_model_id?: string;
  inventory_eligible: boolean;
  discovery?: {
    discovery_ref: string;
    discovery_revision: string;
  };
  actions_required?: string[];
};

export type ComputeScanResult = { items: ComputeScanItem[] };

function agentName(agentId: string, language: 'zh' | 'en') {
  if (agentId.includes('claude')) return 'Claude Code';
  if (agentId.includes('codex')) return 'Codex';
  if (agentId === 'agent_pi_default') return 'Pi';
  if (agentId.includes('qoder')) return 'Qoder';
  return language === 'zh' ? '本机 Agent' : 'Local agent';
}

function connectionName(optionId: string | undefined, language: 'zh' | 'en') {
  if (!optionId) return language === 'zh' ? '本机模型配置' : 'Local model configuration';
  if (optionId.startsWith('zhipu.coding-plan')) return language === 'zh' ? '智谱 Coding Plan' : 'Zhipu Coding Plan';
  if (optionId.startsWith('zhipu.')) return language === 'zh' ? '智谱开放平台' : 'Zhipu Open Platform';
  if (optionId.startsWith('bailian.coding-plan')) return language === 'zh' ? '百炼 Coding Plan' : 'Bailian Coding Plan';
  if (optionId.startsWith('bailian.')) return language === 'zh' ? '百炼模型服务' : 'Bailian Model Service';
  if (optionId.startsWith('kimi.')) return 'Kimi';
  return language === 'zh' ? '已知 API 配置' : 'Known API configuration';
}

function configurationMessage(item: ComputeScanItem, language: 'zh' | 'en') {
  const model = item.observed_model_id?.trim();
  if (item.inventory_eligible) {
    return language === 'zh'
      ? `已识别${model ? ` ${model}` : ''}，可复用现有配置。`
      : model ? `${model} was recognized. The existing configuration can be reused.` : 'The existing configuration can be reused.';
  }
  if (item.actions_required?.includes('registered_model_required')) {
    return language === 'zh' ? '已检测到配置，但模型尚未收录。' : 'Configuration found, but its model is not registered yet.';
  }
  if (item.actions_required?.includes('credential_import_required')) {
    return language === 'zh' ? '已检测到服务配置，但没有找到可复用的凭据。' : 'Service configuration found, but no reusable credential was found.';
  }
  return language === 'zh' ? '已检测到配置，但当前无法安全复用。' : 'Configuration found, but it cannot be reused safely yet.';
}

function agentScanState(agent: Agent, language: 'zh' | 'en') {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (agent.configuration_state === 'executable_not_runnable') {
    return {
      detail: text('已定位安装 · 目标不是普通可执行文件', 'Installation located · the target is not a regular executable file'),
      label: text('不可执行', 'Not executable'),
      tone: 'warn',
    };
  }
  if (agent.configuration_state === 'executable_probe_timed_out') {
    return {
      detail: text('已发现安装 · Agent 版本检查超时', 'Installation found · the Agent version check timed out'),
      label: text('检查超时', 'Check timed out'),
      tone: 'warn',
    };
  }
  if (agent.configuration_state === 'executable_probe_unavailable') {
    return {
      detail: text('已定位安装 · 路径解析或命令启动失败', 'Installation located · path resolution or command launch failed'),
      label: text('检查失败', 'Check failed'),
      tone: 'warn',
    };
  }
  if (['symlink_config', 'wrong_owner', 'unsafe_config_permissions', 'permission_hardening_required', 'config_unavailable', 'conflicting_effective_config'].includes(agent.configuration_state)) {
    return {
      detail: text('已发现安装 · 配置暂时无法安全读取', 'Installation found · settings could not be read safely'),
      label: text('配置受阻', 'Settings blocked'),
      tone: 'warn',
    };
  }
  if (['unregistered_endpoint', 'unregistered_model'].includes(agent.configuration_state)) {
    return {
      detail: text('已发现安装 · 当前模型配置尚未识别', 'Installation found · its model configuration is not recognized yet'),
      label: text('未识别配置', 'Unrecognized'),
      tone: 'warn',
    };
  }
  return {
    detail: text('已安装 · 在 Agent 页面配置路由', 'Installed · configure routing on the Agent page'),
    label: '',
    tone: '',
  };
}

export function DeviceScanList({
  language,
  subscriptions,
  computeItems,
  agents,
  subscriptionScanFailed,
  subscriptionRuntimeUnavailable,
  computeScanFailed,
  repairingSubscriptionSource,
  trustedAuthority,
  onOpenSubscription,
  onViewConnectedSubscription,
  onOpenDiscoveredConfiguration,
}: {
  language: 'zh' | 'en';
  subscriptions: SubscriptionCandidate[];
  computeItems: ComputeScanItem[];
  agents: Agent[];
  subscriptionScanFailed: boolean;
  subscriptionRuntimeUnavailable: boolean;
  computeScanFailed: boolean;
  repairingSubscriptionSource: string | null;
  trustedAuthority: boolean;
  onOpenSubscription(candidate: SubscriptionCandidate): void;
  onViewConnectedSubscription(candidate: SubscriptionCandidate): void;
  onOpenDiscoveredConfiguration(item: ComputeScanItem): void;
}) {
  const configurations = computeItems.filter(item => item.connection_option_id || item.native_provider_id);
  const detectedAgents = agents.filter(agent => agent.configuration_state !== 'not_found_in_scope');
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  return <div className="oc-scan-list">
    <p className="v3-select-intro">{text('逐项查看和接入，不改变 Agent 的现有路由设置。', 'Review and connect each source without changing the agents’ routing settings.')}</p>
    <h3 className="oc-section-label">{text('可复用的订阅', 'Available subscriptions')}</h3>
    {subscriptions.map(candidate => {
      const repairing = Boolean(candidate.existing_source_id && candidate.existing_source_id === repairingSubscriptionSource);
      return <div className="oc-status-row" key={candidate.candidate.candidate_ref}>
        <BrandIcon kind="codex" label="Codex" />
        <div className="row-main">
          <strong>{candidate.display_name}</strong>
          <p>{repairing ? text('重新检查当前订阅登录', 'Check the current subscription sign-in again') : candidate.existing_source_id ? text('已接入，可以在模型页查看', 'Connected. View it on the Models page') : text('已发现本机登录', 'Local sign-in found')}</p>
        </div>
        <button className={`btn${repairing || !candidate.existing_source_id ? ' btn-primary' : ''}`} type="button" disabled={!trustedAuthority && (repairing || !candidate.existing_source_id)} onClick={() => repairing || !candidate.existing_source_id ? onOpenSubscription(candidate) : onViewConnectedSubscription(candidate)}>{repairing ? text('检查', 'Check') : candidate.existing_source_id ? text('查看', 'View') : text('接入', 'Connect')}</button>
        {candidate.existing_source_id && !repairing && <button className="btn" type="button" disabled={!trustedAuthority} onClick={() => onOpenSubscription(candidate)}>{text('调整模型', 'Choose models')}</button>}
      </div>;
    })}
    {!subscriptions.length && !subscriptionScanFailed && !subscriptionRuntimeUnavailable && <p className="oc-meta" role="status">{text('没有发现可复用的 Codex 订阅。', 'No reusable Codex subscription was found.')}</p>}
    {subscriptionRuntimeUnavailable && <div className="callout warn" role="status"><UiIcon name="warning" /><span>{text('当前环境暂时无法读取本机 Codex 登录，请稍后重试。', 'The local Codex sign-in cannot be read in this environment. Try again later.')}</span></div>}
    {subscriptionScanFailed && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{text('暂时无法读取本机 Codex 订阅。', 'The local Codex subscription could not be read.')}</span></div>}

    <h3 className="oc-section-label">{text('Agent 中的模型配置', 'Model configurations in agents')}</h3>
    {configurations.map(item => <div className="oc-status-row" key={item.discovery?.discovery_ref ?? `${item.agent_id}:${item.native_provider_id ?? item.connection_option_id}:${item.observed_model_id ?? 'unknown'}`}>
      <BrandIcon kind={agentBrandFromId(item.agent_id)} label={agentName(item.agent_id, language)} />
      <div className="row-main">
        <strong>{item.native_provider_id ?? connectionName(item.connection_option_id, language)}</strong>
        <p>{agentName(item.agent_id, language)} · {configurationMessage(item, language)}</p>
      </div>
      {item.discovery && item.inventory_eligible
        ? <button className="btn btn-primary" type="button" disabled={!trustedAuthority} onClick={() => onOpenDiscoveredConfiguration(item)}>{text('接入', 'Connect')}</button>
        : <span className="badge warn no-dot">{text('暂不可接入', 'Not connectable')}</span>}
    </div>)}
    {!configurations.length && !computeScanFailed && <p className="oc-meta" role="status">{text('没有发现已识别的 Agent 模型配置。', 'No recognized agent model configuration was found.')}</p>}
    {computeScanFailed && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{text('暂时无法读取本机 API 配置，请重试。', 'Local API configurations could not be read. Try again.')}</span></div>}

    <h3 className="oc-section-label">{text('已发现的 Agent', 'Detected agents')}</h3>
    {detectedAgents.map(agent => {
      const state = agentScanState(agent, language);
      return <div className="oc-status-row" key={agent.agent_id}>
        <BrandIcon kind={agentBrandFromId(agent.agent_id)} label={agentName(agent.agent_id, language)} />
        <div className="row-main"><strong>{agentName(agent.agent_id, language)}</strong><p>{state.detail}</p></div>
        {state.label && <span className={`badge ${state.tone} no-dot`}>{state.label}</span>}
      </div>;
    })}
    {!detectedAgents.length && <p className="oc-meta">{text('尚未发现可接入的 Agent。', 'No compatible agent was detected.')}</p>}
    <p className="oc-meta">{text('未能安全复用的 API，可通过“连接 API”手动添加。', 'APIs that cannot be reused safely can be added manually through Connect an API.')}</p>
  </div>;
}
