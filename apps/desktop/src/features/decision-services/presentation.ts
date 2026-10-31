import type { Draft, Plan } from '../../plan-editor';
import { emptyService, providers, type DecisionConnection, type DecisionService } from './types.ts';
import { presetName } from './form-state.ts';
export { sameEndpointOrigin, testKey } from './form-state.ts';

/** `system_one` is the entry for native decision models (saved V1 System One or V2 native). */
export type DecisionKind = 'system_one' | 'custom';
export type DecisionEntry = DecisionKind | 'compatible';
export type DecisionIntent = { key: string; kind: DecisionEntry };
export type OpenDecisionConnection = (kind: DecisionKind, select: (service: DecisionService) => void) => void;
export type AuthMode = 'none' | 'bearer' | 'header';
export type DecisionTest = { passed: boolean; code?: string; duration?: number; at: number };

export function providerLabel(connection: DecisionConnection, language: 'zh' | 'en'): string {
  if (connection.kind === 'custom') return language === 'zh' ? '自定义扩展' : 'Custom extension';
  if (connection.kind === 'native' && connection.authentication.kind === 'shared_source') return language === 'zh' ? '使用 API 接入凭据' : 'Credentials from API connection';
  if (connection.provider === 'compatible' || connection.provider === 'openai' || connection.provider.startsWith('bailian')) return presetName(connection.provider, language);
  return providers.find(p => p.id === connection.provider)?.name ?? connection.provider;
}

export function newDecisionConnection(kind: DecisionEntry, language: 'zh' | 'en'): DecisionService {
  const service = emptyService();
  if (kind === 'custom') service.connection = { kind: 'custom', endpoint: '', timeout_ms: 10000 };
  if (kind === 'compatible') service.connection = { ...service.connection, kind: 'system_one', provider: 'compatible', model: '', endpoint: '', auth_header: { name: 'Authorization', value_secret_ref: '' } };
  service.name = kind === 'custom' ? (language === 'zh' ? '自定义扩展' : 'Custom extension') : providerLabel(service.connection, language);
  return service;
}

// A saved header has no authentication-scheme metadata. Do not guess its secret.
export function initialAuthMode(connection: DecisionConnection): AuthMode {
  return connection.kind === 'custom' ? connection.auth_header ? 'header' : 'none' : 'bearer';
}

export function protectedInput(connection: DecisionConnection, mode: AuthMode, input: string): string | null {
  if (!input) return null;
  if (connection.kind === 'custom') return connection.auth_header ? mode === 'bearer' ? `Bearer ${input.trim()}` : input : null;
  // The native writers add Bearer to an independent Key.
  return connection.kind === 'system_one' || connection.authentication.kind === 'independent' ? input : null;
}

export function decisionFailureHelp(code: string, language: 'zh' | 'en'): string {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  switch (code) {
    case 'CLASSIFIER_TIMEOUT': return t('连接超时。请检查网络和接入点，或在高级设置中适当增加超时。', 'The connection timed out. Check the network and endpoint, or increase the timeout in advanced settings.');
    case 'CLASSIFIER_AUTH_REJECTED': return t('认证被拒绝。请核对 API Key 及其工作空间权限。', 'Authentication was rejected. Check the API key and its workspace permissions.');
    case 'CLASSIFIER_RATE_LIMITED': return t('服务请求过于频繁，请稍后重试或核对配额。', 'The service rate limit was reached. Retry later or check your quota.');
    case 'CLASSIFIER_ENDPOINT_REJECTED': return t('服务不接受当前工作空间接入点，请核对完整地址和工作空间配置。', 'The service rejected this workspace endpoint. Check the full address and workspace configuration.');
    case 'CLASSIFIER_INPUT_REJECTED': return t('服务拒绝了请求。请核对模型名称、完整接入点及协议兼容性；这不一定是 API Key 问题。', 'The service rejected the request. Check the model, full endpoint and protocol compatibility; this does not necessarily indicate an API key problem.');
    case 'CLASSIFIER_OUTPUT_INVALID': return t('响应不符合决策协议。请检查接入点及模型是否支持决策接口；自定义扩展需实现当前协议并返回允许的类别和模型组。', 'The response does not match the decision protocol. Check decision API support for the endpoint and model; an extension must implement the current contract and return an allowed category and group.');
    case 'CLASSIFIER_CONFIG_INVALID':
    case 'DECISION_SERVICE_INVALID': return t('连接配置无效。请核对完整地址、认证和超时设置。', 'The connection configuration is invalid. Check the complete URL, authentication and timeout.');
    case 'REVISION_CONFLICT':
    case 'DECISION_REVISION_CONFLICT': return t('配置或 API 接入已被更新。已刷新状态并保留非密钥修改，请核对后再保存。', 'The configuration or API connection changed. State was refreshed and non-secret edits were kept; review before saving again.');
    case 'DECISION_INVALID_CONFIGURATION': return t('配置无效。请核对模型、完整地址、协议和超时。', 'The configuration is invalid. Check the model, complete URL, protocol and timeout.');
    case 'DECISION_SOURCE_NOT_FOUND': return t('所选 API 接入已不存在。请选择其他接入。', 'The selected API connection no longer exists. Choose another connection.');
    case 'DECISION_SOURCE_LINEAGE_MISMATCH': return t('所选 API 接入已被替换为不同的接入。请重新选择接入。', 'The selected API connection was replaced by a different connection. Choose a connection again.');
    case 'DECISION_SOURCE_DISABLED': return t('所选 API 接入已停用。可在 API 接入中启用。', 'The selected API connection is disabled. Enable it in the API connection.');
    case 'DECISION_SOURCE_UNAVAILABLE': return t('暂时无法读取 API 接入的状态，请稍后重试。', 'The API connection’s state could not be read. Try again later.');
    case 'DECISION_NO_USABLE_CREDENTIALS': return t('API 接入中没有已启用且已配置的 Key。', 'The API connection has no enabled, configured key.');
    case 'DECISION_IN_USE': return t('仍有路由、草稿或保留的历史版本引用此决策模型。', 'Routes, drafts or retained historical versions still reference this decision model.');
    case 'DECISION_TIMEOUT': return t('请求超时。请稍后重试，或在高级设置中适当增加超时。', 'The request timed out. Retry later, or increase the timeout in advanced settings.');
    case 'DECISION_AUTHENTICATION_REJECTED': return t('服务拒绝了这次使用的凭据。请检查 Key 及其权限。', 'The service rejected the credentials used for this call. Check the key and its permissions.');
    case 'DECISION_RATE_LIMITED': return t('服务请求过于频繁，请稍后重试或核对配额。', 'The service rate limit was reached. Retry later or check your quota.');
    case 'DECISION_INPUT_REJECTED': return t('服务拒绝了请求。请核对模型、完整地址和协议；这不一定是 Key 或工作空间问题。', 'The service rejected the request. Check the model, complete URL and protocol; this does not necessarily indicate a key or workspace problem.');
    case 'DECISION_MALFORMED_OUTPUT': return t('响应不符合所选协议。请核对地址、模型和协议。', 'The response does not match the selected protocol. Check the URL, model and protocol.');
    case 'DECISION_TRANSPORT_UNAVAILABLE': return t('无法连接到服务。请检查网络后重试。', 'The service could not be reached. Check the network and retry.');
    default: return t('暂时无法完成连接。请检查网络、服务状态及认证后重试。', 'The connection could not complete. Check the network, service and credentials, then try again.');
  }
}

export function decisionReferences(id: string, plans: Plan[], drafts: Draft[]) {
  return [
    ...plans.flatMap(plan => {
      const classifier = plan.desired.strategy.routing?.classifier ?? plan.desired.strategy.classifier;
      return classifier && classifier.kind !== 'local_rules' && classifier.service.id === id
        ? [{ key: plan.agent_plan_id, name: plan.desired.display_name, revision: classifier.service.revision, plan, draft: undefined }] : [];
    }),
    ...drafts.flatMap(draft => {
      const editor = draft.editor;
      const classifier = editor.mode === 'custom_branches' ? editor.branch_routing?.classifier : editor.mode === 'smart_saving' ? editor.smart.classifier : undefined;
      return classifier && classifier.kind !== 'local_rules' && classifier.service.id === id
        ? [{ key: draft.draft_id, name: editor.display_name, revision: classifier.service.revision, plan: undefined, draft }] : [];
    }),
  ];
}
