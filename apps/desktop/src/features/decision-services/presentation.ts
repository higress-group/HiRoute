import type { Draft, Plan } from '../../plan-editor';
import { emptyService, providers, type DecisionConnection, type DecisionService } from './types.ts';

export type DecisionKind = 'system_one' | 'custom';
export type DecisionEntry = DecisionKind | 'compatible';
export type DecisionIntent = { key: string; kind: DecisionEntry };
export type OpenDecisionConnection = (kind: DecisionKind, select: (service: DecisionService) => void) => void;
export type AuthMode = 'none' | 'bearer' | 'header';
export type DecisionTest = { passed: boolean; code?: string; duration?: number; at: number };

export function providerLabel(connection: DecisionConnection, language: 'zh' | 'en'): string {
  if (connection.kind === 'custom') return language === 'zh' ? '自定义扩展' : 'Custom extension';
  if (connection.provider === 'compatible') return language === 'zh' ? '兼容接入' : 'Compatible connection';
  if (language === 'en' && connection.provider.startsWith('bailian')) return connection.provider === 'bailian-token-plan' ? 'Bailian Token Plan' : 'Bailian workspace';
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
  return connection.kind === 'system_one' ? 'bearer' : connection.auth_header ? 'header' : 'none';
}

export function protectedInput(connection: DecisionConnection, mode: AuthMode, input: string): string | null {
  if (!connection.auth_header || !input) return null;
  // The native System One writer adds Bearer; custom extensions store the full value.
  return connection.kind === 'custom' && mode === 'bearer' ? `Bearer ${input.trim()}` : input;
}

export function sameEndpointOrigin(left: string, right: string): boolean {
  try { return new URL(left).origin === new URL(right).origin; } catch { return false; }
}

export function testKey(service: DecisionService): string { return `${service.id}/${service.revision}`; }

export function decisionFailureHelp(code: string, language: 'zh' | 'en'): string {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  switch (code) {
    case 'CLASSIFIER_TIMEOUT': return t('连接超时。请检查网络和接入点，或在高级设置中适当增加超时。', 'The connection timed out. Check the network and endpoint, or increase the timeout in advanced settings.');
    case 'CLASSIFIER_AUTH_REJECTED': return t('认证被拒绝。请核对 API Key 及其工作空间权限。', 'Authentication was rejected. Check the API key and its workspace permissions.');
    case 'CLASSIFIER_RATE_LIMITED': return t('服务请求过于频繁，请稍后重试或核对配额。', 'The service rate limit was reached. Retry later or check your quota.');
    case 'CLASSIFIER_ENDPOINT_REJECTED': return t('服务不接受当前工作空间接入点，请核对完整地址和工作空间配置。', 'The service rejected this workspace endpoint. Check the full address and workspace configuration.');
    case 'CLASSIFIER_INPUT_REJECTED': return t('服务拒绝了请求。请核对模型名称、完整接入点及协议兼容性；这不一定是 API Key 问题。', 'The service rejected the request. Check the model, full endpoint and protocol compatibility; this does not necessarily indicate an API key problem.');
    case 'CLASSIFIER_OUTPUT_INVALID': return t('响应不符合决策协议。请检查接入点及模型是否支持决策接口；自定义扩展需返回允许的分支。', 'The response does not match the decision protocol. Check decision API support for the endpoint and model; an extension must return an allowed branch.');
    case 'CLASSIFIER_CONFIG_INVALID':
    case 'DECISION_SERVICE_INVALID': return t('连接配置无效。请核对完整地址、认证和超时设置。', 'The connection configuration is invalid. Check the complete URL, authentication and timeout.');
    case 'REVISION_CONFLICT': return t('连接已被更新。请刷新后重新编辑。', 'The connection was updated. Refresh before editing again.');
    default: return t('暂时无法完成连接。请检查网络、服务状态及认证后重试。', 'The connection could not complete. Check the network, service and credentials, then try again.');
  }
}

export function decisionReferences(id: string, plans: Plan[], drafts: Draft[]) {
  return [
    ...plans.flatMap(plan => {
      const classifier = plan.desired.strategy.routing?.classifier ?? plan.desired.strategy.classifier;
      return classifier?.kind === 'decision_service' && classifier.service.id === id
        ? [{ key: plan.agent_plan_id, name: plan.desired.display_name, revision: classifier.service.revision, plan, draft: undefined }] : [];
    }),
    ...drafts.flatMap(draft => {
      const editor = draft.editor;
      const classifier = editor.mode === 'custom_branches' ? editor.branch_routing?.classifier : editor.mode === 'smart_saving' ? editor.smart.classifier : undefined;
      return classifier?.kind === 'decision_service' && classifier.service.id === id
        ? [{ key: draft.draft_id, name: editor.display_name, revision: classifier.service.revision, plan: undefined, draft }] : [];
    }),
  ];
}
