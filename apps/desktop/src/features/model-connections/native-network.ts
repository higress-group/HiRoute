import type { CanonicalDigest, ModelConnectionCheckView, ModelConnectionDraft, UpstreamProtocol } from './types';

export type NativeNetworkPolicy = { mode: 'direct' } | { mode: 'inherit' } | { mode: 'manual'; proxy_url: string; no_proxy: string };
/** Inactive manual fields belong to the editing session, never to another mode's wire payload. */
export type NativeNetworkDraft = { mode: NativeNetworkPolicy['mode']; proxy_url: string; no_proxy: string; expected_network_revision: number };
export type NativeNetworkEdit = { expected_network_revision: number; policy: NativeNetworkPolicy };
export type NativeNetworkFailure = { phase: 'route_unavailable' | 'proxy_connect' | 'proxy_tls' | 'proxy_auth' | 'proxy_negotiation' | 'upstream_connect' | 'upstream_tls' | 'upstream_auth' | 'directory' | 'inference' | 'transport_unknown'; code: string; retryable: boolean };
export type NativeNetworkRoute = { mode: NativeNetworkPolicy['mode']; route: 'direct' | 'proxy' | null; reason: 'policy_direct' | 'loopback' | 'bypass_matched' | 'no_proxy_configured' | 'selected_proxy' | null; destination: string; state_revision: CanonicalDigest; failure: NativeNetworkFailure | null };
export type NativeSourceNetwork = { source_id: string; source_revision: number; network_revision: number; policy: NativeNetworkPolicy; applied_revision: number | null; application_state: 'applied' | 'pending' | 'unavailable'; operation: import('./types').OperationReference | null; routes: NativeNetworkRoute[]; models: { binding_id: string; display_name: string }[]; decisions: { service_id: string; revision: number; display_name: string; model: string }[] };
export type NativeNetworkTarget = { kind: 'draft'; endpoint_url: string } | { kind: 'registered'; connection_option_id: string; protocol: UpstreamProtocol } | { kind: 'saved'; source_id: string; expected_source_revision: number; protocol: UpstreamProtocol | null } | { kind: 'decision'; id: string; revision: number };
export type NetworkCheckResponse = { check: ModelConnectionCheckView; network: { policy: NativeNetworkPolicy; expected_network_revision: number; route: NativeNetworkRoute }; failure: NativeNetworkFailure | null };
export const networkDraft = (policy: NativeNetworkPolicy = { mode: 'direct' }, revision = 0): NativeNetworkDraft => ({ mode: policy.mode, proxy_url: policy.mode === 'manual' ? policy.proxy_url : '', no_proxy: policy.mode === 'manual' ? policy.no_proxy : '', expected_network_revision: revision });
export function networkEdit(draft: NativeNetworkDraft = networkDraft()): NativeNetworkEdit {
  return { expected_network_revision: draft.expected_network_revision, policy: draft.mode === 'manual' ? { mode: 'manual', proxy_url: draft.proxy_url.trim(), no_proxy: draft.no_proxy } : { mode: draft.mode } };
}
export function networkPolicyEqual(a: NativeNetworkPolicy, b: NativeNetworkPolicy) {
  const normalized = (policy: NativeNetworkPolicy) => {
    if (policy.mode !== 'manual') return policy;
    let url = policy.proxy_url.trim();
    try { url = new URL(url).toString(); } catch { /* Keep invalid input distinct. */ }
    return { mode: policy.mode, proxy_url: url, no_proxy: policy.no_proxy.split(',').map(s => s.trim()).filter(Boolean).join(',') };
  };
  return JSON.stringify(normalized(a)) === JSON.stringify(normalized(b));
}
/** An explicit refresh rebases the revision, preserving local intent for the same source. */
export function reconcileNetworkDraft(
  before: Pick<NativeSourceNetwork, 'source_id' | 'policy'> | null,
  current: Pick<NativeSourceNetwork, 'source_id' | 'policy' | 'network_revision'>,
  draft: NativeNetworkDraft,
): NativeNetworkDraft {
  if (!before || before.source_id !== current.source_id) return networkDraft(current.policy, current.network_revision);
  if (!networkPolicyEqual(before.policy, networkEdit(draft).policy) || networkPolicyEqual(before.policy, current.policy)) {
    return { ...draft, expected_network_revision: current.network_revision };
  }
  return current.policy.mode === 'manual' ? networkDraft(current.policy, current.network_revision)
    : { ...draft, mode: current.policy.mode, expected_network_revision: current.network_revision };
}
export function manualProxyError(draft: NativeNetworkDraft): 'proxy_url' | null {
  if (draft.mode !== 'manual') return null;
  try {
    const url = new URL(draft.proxy_url.trim());
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || draft.proxy_url.includes('@') || /[\x00-\x1f\x7f\\]/.test(draft.proxy_url) || (url.pathname !== '' && url.pathname !== '/') || url.search || url.hash || url.port === '0') return 'proxy_url';
    return null;
  } catch { return 'proxy_url'; }
}
export const networkRepairable = (failure?: NativeNetworkFailure | null) => Boolean(failure && ['route_unavailable', 'proxy_connect', 'proxy_tls', 'proxy_auth', 'proxy_negotiation', 'transport_unknown'].includes(failure.phase));
export function networkFailureText(failure: NativeNetworkFailure, zh: boolean) {
  const copy = {
    route_unavailable: ['代理配置暂不可用，请检查网络设置。', 'The network policy is unavailable. Review network settings.'],
    proxy_connect: ['无法连接代理，请检查代理地址和服务状态。', 'Could not connect to the proxy. Check its address and availability.'],
    proxy_tls: ['代理的安全连接校验失败，请检查代理地址和证书。', 'The proxy TLS check failed. Check its address and certificate.'],
    proxy_auth: ['代理要求认证，请检查服务启动环境中的代理认证配置。API Key 已保留。', 'The proxy requires authentication. Check proxy credentials in the service startup environment. Your API key is retained.'],
    proxy_negotiation: ['代理未能建立转发连接，请检查代理配置。', 'The proxy could not establish forwarding. Check its configuration.'],
    upstream_connect: ['无法连接模型服务，请检查服务地址和状态。', 'Could not connect to the model service. Check its endpoint and availability.'],
    upstream_tls: ['模型服务的安全连接校验失败，请检查服务地址和证书。', 'The model service TLS check failed. Check its endpoint and certificate.'],
    upstream_auth: ['模型服务拒绝了认证，请检查 API Key 和访问权限。', 'The model service rejected authentication. Check the API key and access permissions.'],
    directory: ['模型目录暂时无法读取，请重试或检查服务。', 'The model directory is unavailable. Retry or check the service.'],
    inference: ['模型验证未通过，请检查模型和服务后重试。', 'Model verification failed. Check the model and service, then retry.'],
    transport_unknown: ['连接未完成，暂时无法确定失败环节。输入已保留，可重试或检查网络设置。', 'The connection did not complete and the failing stage is unknown. Your input is retained. Retry or review network settings.'],
  } as const;
  return copy[failure.phase][zh ? 0 : 1];
}
export function networkSummary(policy: Pick<NativeNetworkPolicy, 'mode'>, zh: boolean) {
  return policy.mode === 'direct' ? (zh ? '直连（默认）' : 'Direct (default)') : policy.mode === 'inherit' ? (zh ? '服务环境代理' : 'Service environment proxy') : (zh ? '手动代理' : 'Manual proxy');
}
export function networkTarget(draft: ModelConnectionDraft): NativeNetworkTarget {
  if (draft.provenance.kind === 'user_configured') return { kind: 'draft', endpoint_url: draft.base_url };
  return draft.existing_source_id ? { kind: 'saved', source_id: draft.existing_source_id, expected_source_revision: draft.expected_source_revision!, protocol: draft.protocol }
    : { kind: 'registered', connection_option_id: draft.provenance.connection_option_id, protocol: draft.protocol };
}
export const flattenNetworkCheck = (result: NetworkCheckResponse): ModelConnectionCheckView => ({ ...result.check, network: result.network, failure: result.failure });
