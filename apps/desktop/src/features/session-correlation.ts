import { agentDisplayName } from './agents/ecosystems.ts';

export type CorrelationKind =
  | 'agent_supplied'
  | 'verified_worker'
  | 'inferred'
  | 'request_scoped'
  | 'unknown';

const labels: Record<CorrelationKind, [string, string]> = {
  agent_supplied: ['Agent 提供', 'Agent supplied'],
  verified_worker: ['执行关联已验证', 'Execution link verified'],
  inferred: ['推断关联', 'Inferred'],
  request_scoped: ['请求级', 'Request scoped'],
  unknown: ['关联未知', 'Correlation unknown'],
};

export function sessionCorrelationLabel(kind: CorrelationKind | null | undefined, language: 'zh' | 'en' = 'zh'): string {
  return (labels[kind ?? 'unknown'] ?? labels.unknown)[language === 'en' ? 1 : 0];
}

const executorNames = new Map([
  ['codex', 'Codex'], ['claude', 'Claude Code'], ['dsh', 'DeepSeek Harness'],
  ['pi', 'Pi'], ['qoder', 'Qoder'], ['opencode', 'OpenCode'],
]);

export function sessionAgentLabel(session: { agent_id?: string; executor?: string | null; correlation_kind?: string } | undefined, language: 'zh' | 'en'): string {
  if (session?.executor != null) {
    const name = executorNames.get(session.executor);
    return name ? language === 'zh' ? `${name}（委派执行）` : `${name} (delegated)`
      : language === 'zh' ? '委派执行器未确认' : 'Delegated executor unknown';
  }
  if (session?.agent_id) return agentDisplayName(session.agent_id, language);
  return session?.correlation_kind === 'verified_worker'
    ? language === 'zh' ? '委派执行器未确认' : 'Delegated executor unknown'
    : language === 'zh' ? '未关联 Agent' : 'Unlinked Agent';
}
