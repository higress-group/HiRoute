import { UiIcon } from '../../ui';
import { agentDiscoveryState } from './discovery';
import type { Agent } from './types';

export function AgentDiscoveryFeedback({ agent, language, scanning, onScan }: {
  agent: Agent; language: 'zh' | 'en'; scanning: boolean; onScan: () => void;
}) {
  const state = agentDiscoveryState(agent);
  if (state === 'detected' || (!agent.status_error && ['not_found_in_scope', 'executable_not_runnable', 'executable_probe_unavailable'].includes(agent.configuration_state))) return null;
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const reason = agent.status_error;
  const detail = reason === 'AGENT_PROFILE_PATH_INVALID'
    ? text('配置目录无效，需要本机目录的绝对路径。请更改所选目录。', 'The configuration directory is invalid. Select an absolute local directory path.')
    : reason === 'AGENT_PROFILE_NOT_FOUND'
    ? text('所选配置目录不存在。保留此选择，可重新扫描或更改目录。', 'The selected directory does not exist. The selection is retained; scan again or change the directory.')
    : reason === 'AGENT_PROFILE_CONFIG_MISSING'
    ? text('所选目录没有 Hermes 配置。请先完成 Hermes 初始化，或更改目录。', 'The selected directory has no Hermes configuration. Initialize Hermes or change the directory.')
    : text('保留当前目录。请确认目录仍可访问后重新扫描，也可选择其他目录；读取成功前无法修改此配置。', 'The selected directory is retained. Check that it is accessible and scan again, or select another directory. Changes require a successful read.');
  return <div className="callout warn" role="status" data-agent-discovery={state}>
    <UiIcon name="warning" /><div>
      <strong>{state === 'unavailable'
        ? text('暂时无法读取 Agent 配置', 'Agent configuration is temporarily unavailable')
        : text('未发现本机配置', 'No local configuration found')}</strong>
      <p>{state === 'unavailable'
        ? detail
        : text('当前用户环境中未找到可用配置。安装或完成 Agent 初始化后可重新扫描；使用自定义配置时可手动选择目录。', 'No usable configuration was found in the current user environment. Scan again after installing or initializing the Agent; select a directory manually for a custom configuration.')}</p>
      <button className="btn" type="button" disabled={scanning} onClick={onScan}>{scanning ? text('扫描中…', 'Scanning…') : text('重新扫描', 'Scan again')}</button>
    </div>
  </div>;
}
