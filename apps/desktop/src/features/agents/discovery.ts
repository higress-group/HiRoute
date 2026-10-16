import type { StorageLike } from '../../ui/preferences';
import type { Agent, AgentNativeTarget } from './types';

export const AGENT_DISCOVERY_STORAGE_KEY = 'hiroute.agent-discovery';

function browserStorage(): StorageLike | undefined {
  try { return typeof window === 'undefined' ? undefined : window.localStorage; }
  catch { return undefined; }
}

function validTarget(value: unknown): value is AgentNativeTarget {
  if (!value || typeof value !== 'object') return false;
  const target = value as Partial<AgentNativeTarget>;
  return target.kind === 'hermes' && typeof target.config_root === 'string'
    && target.config_root.startsWith('/') && target.config_root.length <= 4096
    && !/[\x00-\x1f\x7f-\x9f]/u.test(target.config_root) && !target.config_root.split('/').includes('..');
}

/** Preferences are discovery hints. The native scanner must validate them on every read. */
export function loadAgentDiscoveryTargets(storage: StorageLike | undefined = browserStorage()): AgentNativeTarget[] {
  try {
    const encoded = storage?.getItem(AGENT_DISCOVERY_STORAGE_KEY);
    if (!encoded || encoded.length > 512 * 1024) return [];
    const saved: unknown = JSON.parse(encoded);
    if (!saved || typeof saved !== 'object' || !('version' in saved) || saved.version !== 1
      || !('targets' in saved) || !Array.isArray(saved.targets) || saved.targets.length > 128
      || !saved.targets.every(validTarget)) return [];
    return saved.targets;
  } catch { return []; }
}

export function saveAgentDiscoveryTargets(targets: AgentNativeTarget[], storage: StorageLike | undefined = browserStorage()): boolean {
  if (!storage || targets.length > 128 || !targets.every(validTarget)) return false;
  try {
    storage.setItem(AGENT_DISCOVERY_STORAGE_KEY, JSON.stringify({ version: 1, targets }));
    return true;
  } catch { return false; }
}

export function agentDiscoveryState(agent: Agent): 'not_discovered' | 'unavailable' | 'detected' {
  if (agent.status_error || agent.configuration_state === 'profile_unavailable') return 'unavailable';
  if (!agent.context_id || ['profile_not_selected', 'not_discovered', 'not_found_in_scope'].includes(agent.configuration_state)) return 'not_discovered';
  return 'detected';
}
