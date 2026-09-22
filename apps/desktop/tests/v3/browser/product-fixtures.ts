import type { AgentSnapshot } from '../../../src/agents';
import type { HomeReads } from '../../../src/features/home';
import type { AgentTaskRead } from '../../../src/features/AgentTasks';
import type { ManagementSnapshot } from '../../../src/features/models/types';
import type { ComputeCandidateView } from '../../../src/features/model-connections/types';
import type { SubscriptionCandidate, SubscriptionCheckResult } from '../../../src/features/subscriptions/types';
import type { Draft, Plan } from '../../../src/plan-editor';
import type { DesktopSnapshot } from '../../../src/product/home-projections';
import { fixture as legacyHomeFixture } from '../../mvp16/browser/fixtures';

export type ProductScenario = 'fresh' | 'ready' | 'collaboration' | 'gap' | 'configured_no_sessions' | 'drift' | 'cooling' | 'free_exhausted' | 'api_failure' | 'unknown_model' | 'agent_blocked' | 'task_cancelled';

export const fixtureTrace: { commands: string[]; clipboard: string[] } = { commands: [], clipboard: [] };
let agentPrerequisiteChecked = false;
let preparedWorkerSelection: Record<string, any> | null = null;
export function resetFixtureTrace() { fixtureTrace.commands.length = 0; fixtureTrace.clipboard.length = 0; agentPrerequisiteChecked = false; preparedWorkerSelection = null; }

const fixtureNow = new Date();
fixtureNow.setHours(14, 28, 0, 0);
const now = fixtureNow.getTime();

export const readyManagement: ManagementSnapshot = {
  schema: 'hiroute.compute-management-snapshot/v2',
  revisions: { target: 19, dependencies: {} },
  runtime_state: 'complete',
  sources: [
    {
      source_id: 'source/codex/subscription',
      revision: 7,
      display_name: 'Codex 订阅',
      provenance: 'connector_owned',
      target: { scheme: 'https', authority: 'chatgpt.com', port: 443, request_path: '/backend-api/codex/responses', upstream_protocol: 'responses', protocol_profile_id: 'profile/codex/responses', protocol_profile_revision: 1 },
      authentication: { kind: 'none' },
      state: 'ready',
      models: [
        { model_ref: 'model-ref/sol', binding_id: 'binding/codex/gpt-5.6-sol', revision: 5, upstream_model_id: 'gpt-5.6-sol', display_name: 'GPT-5.6-Sol', membership: 'observed', native_reasoning: { kind: 'discrete', parameter: 'reasoning.effort', profiles: ['low', 'medium', 'high'] } },
        { model_ref: 'model-ref/astra', binding_id: 'binding/codex/gpt-6-astra', revision: 3, upstream_model_id: 'gpt-6-astra', display_name: 'GPT-6-Astra', membership: 'catalog', native_reasoning: { kind: 'discrete', parameter: 'reasoning.effort', profiles: ['low', 'medium', 'high'] } },
      ],
      keys: [],
      ready_model_count: 2,
      actions: ['recheck'],
    },
    {
      source_id: 'source/bailian/coding',
      revision: 4,
      display_name: '百炼 Coding Plan',
      provenance: 'registered',
      target: { scheme: 'https', authority: 'coding.dashscope.aliyuncs.com', port: 443, request_path: '/v1/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/bailian/chat', protocol_profile_revision: 1 },
      authentication: { kind: 'bearer' },
      state: 'ready',
      models: [{ model_ref: 'model-ref/qwen', binding_id: 'binding/bailian/qwen-coder', revision: 2, upstream_model_id: 'qwen3-coder-plus', display_name: 'Qwen3-Coder-Plus', membership: 'catalog', native_reasoning: { kind: 'fixed', profile: 'provider-default' } }],
      keys: [{ key_id: 'key/bailian/1', generation: 2, fingerprint_hint: '••72D1', ordinal: 1, enabled: true, model_statuses: [{ binding_id: 'binding/bailian/qwen-coder', availability: 'available' }] }],
      ready_model_count: 1,
      actions: ['edit', 'add_key', 'recheck'],
    },
    {
      source_id: 'source/free/deepseek',
      revision: 2,
      display_name: 'OpenRouter 免费模型',
      provenance: 'registered',
      target: { scheme: 'https', authority: 'openrouter.ai', port: 443, request_path: '/api/v1/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/openrouter/chat', protocol_profile_revision: 1 },
      authentication: { kind: 'none' },
      state: 'ready',
      models: [{ model_ref: 'model-ref/deepseek-free', binding_id: 'binding/free/deepseek', revision: 1, upstream_model_id: 'deepseek-v3.1-free', display_name: 'DeepSeek V3.1 Free', membership: 'catalog', native_reasoning: { kind: 'fixed', profile: 'provider-default' } }],
      keys: [],
      ready_model_count: 1,
      actions: ['recheck'],
    },
    {
      source_id: 'source/free/glm',
      revision: 2,
      display_name: '智谱免费模型',
      provenance: 'registered',
      target: { scheme: 'https', authority: 'open.bigmodel.cn', port: 443, request_path: '/api/paas/v4/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/zhipu/chat', protocol_profile_revision: 1 },
      authentication: { kind: 'none' },
      state: 'ready',
      models: [{ model_ref: 'model-ref/glm-free', binding_id: 'binding/free/glm', revision: 1, upstream_model_id: 'glm-4.5-flash', display_name: 'GLM-4.5-Flash', membership: 'catalog', native_reasoning: { kind: 'fixed', profile: 'provider-default' } }],
      keys: [],
      ready_model_count: 1,
      actions: ['recheck'],
    },
    {
      source_id: 'source/free/qwen',
      revision: 2,
      display_name: '硅基流动免费模型',
      provenance: 'registered',
      target: { scheme: 'https', authority: 'api.siliconflow.cn', port: 443, request_path: '/v1/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/siliconflow/chat', protocol_profile_revision: 1 },
      authentication: { kind: 'none' },
      state: 'ready',
      models: [{ model_ref: 'model-ref/qwen-free', binding_id: 'binding/free/qwen', revision: 1, upstream_model_id: 'qwen2.5-7b-instruct', display_name: 'Qwen2.5-7B-Instruct', membership: 'catalog', native_reasoning: { kind: 'fixed', profile: 'provider-default' } }],
      keys: [],
      ready_model_count: 1,
      actions: ['recheck'],
    },
  ],
};

// Mirror the B3 presentation contract so browser review exercises the same UI
// shape that the real management snapshot exposes after backend convergence.
for (const source of readyManagement.sources) {
  const subscription = source.source_id.includes('/subscription');
  const free = source.source_id.includes('/free/');
  source.connection_identity = {
    access_kind: subscription ? 'subscription' : 'api',
    connection_option_id: `option/${source.source_id}`,
    product_label: subscription ? 'Codex' : source.display_name,
  };
  for (const model of source.models) {
    model.presentation = {
      billing_class: subscription ? 'subscription' : free ? 'free' : 'paid',
      availability: 'available',
      reason_code: null,
      evaluated_at_ms: now,
      price_contexts: subscription || free ? [] : [{
        currency: 'CNY',
        valuation_kind: 'usage_estimate',
      }],
    };
  }
}

const localSubscriptionCandidate: SubscriptionCandidate = {
  candidate: { candidate_ref: 'candidate/codex-local-subscription', candidate_revision: 1 },
  correlation: { candidate_ref: 'candidate/codex-local-subscription', edit_revision: 1, check_id: 'scan/codex', input_digest: 'sha256:fixture-subscription' },
  producer: 'cpa',
  provenance: 'connector_owned',
  display_name: 'Codex 本机订阅',
  models: [],
  input_state: 'not_required',
  fact_state: 'pending_approval',
};

const checkedLocalSubscription: SubscriptionCandidate = {
  ...localSubscriptionCandidate,
  candidate: { ...localSubscriptionCandidate.candidate, candidate_revision: 2 },
  models: [{ model_ref: 'model-ref/sol-local', upstream_model_id: 'gpt-5.6-sol', display_name: 'GPT-5.6-Sol', membership: 'observed', selectable: true }],
  fact_state: 'complete',
  validation: {
    approval_operation: { operation_id: 'operation/check-codex-subscription', state: 'succeeded', sequence: 2, cancellable: false },
    validation_ref: 'validation/codex-local-subscription',
    validation_revision: '2',
  },
};

const checkedLocalSubscriptionResult: SubscriptionCheckResult = {
  candidate: checkedLocalSubscription.candidate,
  approval_operation: checkedLocalSubscription.validation!.approval_operation,
  status: 'verified',
  validation: checkedLocalSubscription.validation,
  checked_candidate: checkedLocalSubscription,
};

const discoveredClaudeCandidate: ComputeCandidateView = {
  candidate: { candidate_ref: 'candidate/claude-zhipu-local', candidate_revision: 1 },
  correlation: {
    candidate_ref: 'candidate/claude-zhipu-local',
    edit_revision: 1,
    check_id: 'prepare/claude-zhipu-local',
    input_digest: 'sha256:fixture-claude-zhipu-local',
  },
  producer: 'native',
  provenance: 'registered',
  display_name: '智谱 Coding Plan',
  models: [{
    model_ref: 'model-ref/glm-5.3-local',
    upstream_model_id: 'glm-5.3',
    display_name: 'GLM-5.3',
    membership: 'catalog',
    selectable: true,
  }],
  input_state: 'provided',
  fact_state: 'complete',
};

export const dailyPlan: Plan = {
  agent_plan_id: 'plan/daily-coding',
  desired: {
    display_name: '日常编码',
    purpose: '实现功能、修复问题与复杂代码设计',
    mode: 'smart_saving',
    strategy: {
      mode: 'smart_saving',
      economy: [{ binding_id: 'binding/bailian/qwen-coder' }],
      primary: [{ binding_id: 'binding/codex/gpt-5.6-sol', reasoning: { kind: 'profile', profile: 'high' } }],
      primary_fallback: true,
      classifier: { kind: 'local_rules' },
      complex_keywords: ['架构', '重构', '并发'],
    },
    delegation_enabled: true,
    work: { harness: 'claude_code', protocol: 'messages' },
    requirements: {},
    limits: { maximum_attempts: 6, request_timeout_ms: 60000, attempt_timeout_ms: 30000 },
  },
  head: { head_revision: 8, status: 'enabled' },
  agent_plan_revision: 8,
  model_alias: 'hiroute-daily-coding',
  publication: { revision: 8, digest: 'sha256:fixture' },
  execution: 'published',
};

export const freeResearchPlan: Plan = {
  agent_plan_id: 'plan/free-research',
  desired: {
    display_name: '免费资料整理',
    purpose: '搜索、摘要和低成本资料整理',
    mode: 'free_first',
    strategy: {
      mode: 'free_first',
      candidates: [
        { binding_id: 'binding/free/deepseek' },
        { binding_id: 'binding/free/glm' },
        { binding_id: 'binding/free/qwen' },
      ],
      primary: [],
      primary_fallback: false,
    },
    delegation_enabled: true,
    work: { harness: 'codex_cli', protocol: 'responses' },
    requirements: {},
    limits: { maximum_attempts: 4, request_timeout_ms: 60000, attempt_timeout_ms: 30000 },
  },
  head: { head_revision: 5, status: 'enabled' },
  agent_plan_revision: 5,
  model_alias: 'hiroute-free-research',
  publication: { revision: 5, digest: 'sha256:fixture-free' },
  execution: 'published',
};

export const releaseWritingPlan: Plan = {
  agent_plan_id: 'plan/release-writing',
  desired: {
    display_name: '发布文案',
    purpose: '按固定顺序生成和润色发布内容',
    mode: 'fixed_model',
    strategy: { mode: 'fixed_model', candidates: [{ binding_id: 'binding/bailian/qwen-coder' }, { binding_id: 'binding/codex/gpt-5.6-sol' }] },
    delegation_enabled: false,
    requirements: {},
    limits: { maximum_attempts: 4, request_timeout_ms: 60000, attempt_timeout_ms: 30000 },
  },
  head: { head_revision: 3, status: 'enabled' },
  agent_plan_revision: 3,
  model_alias: 'hiroute-release-writing',
  publication: { revision: 3, digest: 'sha256:fixture-release' },
  execution: 'published',
};

export const codeImplementationPlan: Plan = {
  agent_plan_id: 'plan/code-implementation',
  desired: {
    display_name: '代码实现',
    purpose: '实现明确的编码任务，补齐测试并汇报变更',
    mode: 'fixed_model',
    strategy: { mode: 'fixed_model', candidates: [{ binding_id: 'binding/codex/gpt-5.6-sol', reasoning: { kind: 'profile', profile: 'high' } }] },
    delegation_enabled: true,
    work: { harness: 'claude_code', protocol: 'messages' },
    requirements: {},
    limits: { maximum_attempts: 4, request_timeout_ms: 60000, attempt_timeout_ms: 30000 },
  },
  head: { head_revision: 2, status: 'enabled' },
  agent_plan_revision: 2,
  model_alias: 'hiroute-code-implementation',
  publication: { revision: 2, digest: 'sha256:fixture-code' },
  execution: 'published',
};

export const savedDraft: Draft = {
  draft_id: 'draft/daily-review',
  plan_id: dailyPlan.agent_plan_id,
  base_head_revision: dailyPlan.head.head_revision,
  revision: 3,
  editor: {
    schema: 'hiroute.plan-editor/v2',
    display_name: '日常编码 · review',
    purpose: dailyPlan.desired.purpose,
    mode: 'smart_saving',
    candidates: [],
    smart: { economy: [{ binding_id: 'binding/bailian/qwen-coder' }], primary: [{ binding_id: 'binding/codex/gpt-5.6-sol', reasoning: { kind: 'profile', profile: 'high' } }], primary_fallback: true, classifier: { kind: 'local_rules' }, complex_keywords: ['架构', '重构', '并发'] },
    free: { candidates: [], primary: [], primary_fallback: false },
    delegation_enabled: false,
    requirements: {},
    limits: dailyPlan.desired.limits,
  },
};

export const readyDesktop: DesktopSnapshot = {
  catalog_error: null,
  service: { daemon_role: 'all', recovery_ready: true, mutation_available: true, gateway: 'ready', revisions: { target: 19, dependencies: {} } },
  catalog: { plans: [dailyPlan, freeResearchPlan, releaseWritingPlan, codeImplementationPlan], drafts: [] },
  trusted_authority: true,
  restore_names: [],
  pending: null,
};

export const readyAgents: AgentSnapshot = {
  trusted_authority: true,
  plans: { plans: [dailyPlan, freeResearchPlan, releaseWritingPlan, codeImplementationPlan] },
  agents: [
    {
      agent_id: 'agent_codex_default',
      version: '0.147.0',
      context_id: 'agent-context/codex/default',
      configuration_state: 'configured',
      available_surfaces: ['codex_desktop', 'codex_cli'],
      native_model_catalog: {
        metadata_source: 'target_cache',
        native_default_model: dailyPlan.model_alias,
        models: [{
          client_model_id: 'gpt-5.6-sol',
          display_name: 'GPT-5.6 Sol',
          source_options: [{
            binding_id: 'binding/codex/gpt-5.6-sol',
            source_label: 'Codex account',
            account_scope_ref: 'source/codex/default',
            account_scope_digest: 'sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
            state: 'ready',
            reasoning: { kind: 'discrete', parameter: 'reasoning_effort', profiles: ['low', 'medium', 'high'] },
          }],
        }],
      },
      status_error: null,
      settings: {
        state: 'configured',
        model_verified: true,
        restore_point_ref: 'model-restore/codex',
        applied_revision: 19,
        surface_results: [
          { surface: 'codex_cli', applied_revision: 19, state: 'passed', reason_code: null },
          { surface: 'codex_desktop', applied_revision: 19, state: 'passed', reason_code: null },
        ],
        live_check_targets: [
          { context_id: 'agent-context/codex/default', surface: 'codex_cli', expected_applied_revision: 19, client_model_ids: ['hiroute.plan.daily'] },
          { context_id: 'agent-context/codex/default', surface: 'codex_desktop', expected_applied_revision: 19, client_model_ids: ['hiroute.plan.daily'] },
        ],
        current_selection: {
          mode: 'codex_default', native_model_mode: 'hiroute_only', fixed_models: [],
          allowed_plan_ids: [dailyPlan.agent_plan_id],
          default_selection: { kind: 'plan', plan_id: dailyPlan.agent_plan_id },
        },
        collaboration: { state: 'configured', restore_point_ref: 'task-restore/codex', current_selection: { trigger_mode: 'explicit' } },
      },
    },
    {
      agent_id: 'agent_claude_default',
      version: '2.1.231',
      context_id: 'agent-context/claude/default',
      configuration_state: 'configured',
      available_surfaces: ['claude_cli'],
      native_model_catalog: null,
      status_error: null,
      settings: {
        state: 'configured',
        model_verified: false,
        restore_point_ref: 'model-restore/claude',
        applied_revision: 19,
        surface_results: [{ surface: 'claude_cli', applied_revision: 19, state: 'not_verified', reason_code: null }],
        live_check_targets: [{ context_id: 'agent-context/claude/default', surface: 'claude_cli', expected_applied_revision: 19, client_model_ids: ['hiroute.plan.daily'] }],
        current_selection: {
          mode: 'claude_launcher', surfaces: ['claude_cli'], fixed_models: [],
          preset_mappings: {
            opus: { kind: 'plan', plan_id: dailyPlan.agent_plan_id },
            sonnet: { kind: 'preserve_native' },
            haiku: { kind: 'preserve_native' },
          },
        },
        collaboration: { state: 'configured', restore_point_ref: 'task-restore/claude', current_selection: { trigger_mode: 'delegate_by_default' } },
      },
    },
  ],
};

export function productTaskRead(scenario: ProductScenario): AgentTaskRead {
  if (scenario === 'fresh') return { status: 'ready', tasks: [] };
  const cancelled = scenario === 'task_cancelled';
  return { status: 'ready', tasks: [
    {
      taskId: 'task/code-boundaries', runId: 'run/code-boundaries/1', title: '补齐边界条件测试', mainAgent: 'Claude Code', executor: 'Codex', routeName: '代码实现', createdAtMs: now - 8 * 60_000, status: 'complete',
      brief: '只修改路由编译器测试，覆盖空候选和不支持的能力。不要修改生产逻辑。', result: '已补齐两个边界测试，测试范围内通过；未修改生产逻辑。', sessionId: 'session/boundary-test', nativeSessionId: 'codex-thread-code', mainUsesOriginalModel: true,
      facts: [{ model: 'GPT-5.6-Sol', reasoning: 'max', tokens: '8.1K' }], scope: { directory: '~/Projects/HiRoute', files: '读写项目文件', network: '未授予网络访问' },
    },
    {
      taskId: 'task/route-review', runId: 'run/route-review/1', title: '评审路由编译边界', mainAgent: 'Codex', executor: 'Claude Code', routeName: '日常编码', createdAtMs: now - 40 * 60_000, status: 'complete',
      brief: '检查请求级与 Attempt 级作用域是否正确，给出证据，不修改文件。', result: '完成评审并给出两处代码定位。执行期间发生提交前模型接力，详见会话。', sessionId: 'session/route-review', nativeSessionId: 'claude-session-review',
      facts: [{ model: 'Claude Sonnet', reasoning: '16,000 tokens' }, { model: 'GPT-5.6-Sol', reasoning: 'max', tokens: '42.8K' }], scope: { directory: '~/Projects/HiRoute', files: '只读项目文件', network: '未授予网络访问' },
    },
    {
      taskId: 'task/fix-empty-list', runId: 'run/fix-empty-list/1', title: '修复空列表显示', mainAgent: 'Claude Code', executor: 'Codex', routeName: '代码实现', createdAtMs: now - 46 * 60_000, status: cancelled ? 'cancelled' : 'running',
      brief: '按现有组件修复空列表显示，不修改路由规则。', result: null, sessionId: null, nativeSessionId: null, mainUsesOriginalModel: true, facts: [], scope: { directory: '~/Projects/HiRoute', files: '读写项目文件', network: '未授予网络访问' },
    },
  ] };
}

const sessionRows = [
  { session_id: 'session/gateway-refactor', agent_id: 'agent_codex_default', request_count: 3, fallback_request_count: 1, last_request_at_ms: now, correlation_kind: 'agent_supplied' },
  { session_id: 'session/rust-ownership', agent_id: 'agent_claude_default', request_count: 2, fallback_request_count: 0, last_request_at_ms: now - 34 * 60_000, correlation_kind: 'agent_supplied' },
  { session_id: 'session/release-notes', agent_id: 'agent_codex_default', request_count: 1, fallback_request_count: 0, last_request_at_ms: new Date(new Date(now).getFullYear(), new Date(now).getMonth(), new Date(now).getDate(), 11, 20).getTime(), correlation_kind: 'agent_supplied' },
  { session_id: 'session/stream-gap', agent_id: 'agent_claude_default', request_count: 1, fallback_request_count: 0, last_request_at_ms: now - 24 * 60 * 60_000, correlation_kind: 'agent_supplied' },
  { session_id: 'session/boundary-test', agent_id: 'agent_codex_default', request_count: 1, fallback_request_count: 0, last_request_at_ms: now - 24 * 60 * 60_000 - 34 * 60_000, correlation_kind: 'agent_supplied' },
  { session_id: 'session/route-review', agent_id: 'agent_claude_default', request_count: 2, fallback_request_count: 1, last_request_at_ms: now - 3 * 24 * 60 * 60_000, correlation_kind: 'agent_supplied' },
];

const requestBySession: Record<string, string> = {
  'session/gateway-refactor': 'request/gateway-refactor',
  'session/rust-ownership': 'request/rust-ownership',
  'session/release-notes': 'request/release-notes',
  'session/stream-gap': 'request/stream-gap',
  'session/boundary-test': 'request/boundary-test',
  'session/route-review': 'request/route-review',
};

const messages: Record<string, { user: string; assistant: string }> = {
  'request/gateway-refactor': { user: '结合当前设计，把 Gateway 的请求级计划和 Attempt 级计划彻底拆开，并补齐相关测试。', assistant: '已完成作用域拆分：请求级阶段不再从主 Provider 继承，Attempt 仅拥有上游相关阶段，并补充了 fallback 回归测试。' },
  'request/rust-ownership': { user: '解释 Rust ownership 错误', assistant: '这个借用跨过了可变引用的作用域，可以缩短借用范围来修复。' },
  'request/release-notes': { user: '整理三篇发布说明', assistant: '已按影响范围整理为三组变更。' },
  'request/stream-gap': { user: '分析一次流式响应中断', assistant: '从已交付的前缀看，连接在上游心跳间隔之后关闭。下一步应核对……' },
  'request/boundary-test': { user: '补齐边界条件测试', assistant: '已补齐边界条件并验证。' },
  'request/route-review': { user: '评审路由编译边界', assistant: '已完成路由编译边界评审。' },
};

export function productHomeReads(scenario: ProductScenario): HomeReads {
  if (scenario === 'fresh') return structuredClone(legacyHomeFixture('fresh'));
  const source = scenario === 'drift' ? legacyHomeFixture('drift') : legacyHomeFixture('daily');
  const reads = structuredClone(source);
  if (reads.activity.status === 'ready') {
    reads.activity.data.sessions = scenario === 'configured_no_sessions' ? [] : [
      { sessionId: 'session/gateway-refactor', title: '重构 Gateway 的路由编译边界', agentName: 'Codex', modelLabel: '日常编码', occurredAtLabel: '14:28', modelSwitch: true },
      { sessionId: 'session/rust-ownership', title: '解释 Rust ownership 错误', agentName: 'Claude Code', modelLabel: '日常编码', occurredAtLabel: '13:54', modelSwitch: false },
      { sessionId: 'session/release-notes', title: '整理三篇发布说明', agentName: 'Codex', modelLabel: '免费资料整理', occurredAtLabel: '11:20', modelSwitch: false },
      { sessionId: 'session/stream-gap', title: '分析一次流式响应中断', agentName: 'Claude Code', modelLabel: '日常编码', occurredAtLabel: '昨天', modelSwitch: false },
      { sessionId: 'session/boundary-test', title: '补齐边界条件测试', agentName: 'Codex', modelLabel: '代码实现', occurredAtLabel: '昨天', modelSwitch: false },
      { sessionId: 'session/route-review', title: '评审路由编译边界', agentName: 'Claude Code', modelLabel: '日常编码', occurredAtLabel: '09-07', modelSwitch: true },
    ];
    reads.activity.data.tasks = scenario === 'configured_no_sessions' ? [] : [{ taskId: 'task/visual', runId: 'run/visual', title: '修复空列表显示', state: 'running', agentName: 'Codex' }];
  }
  if (reads.value.status === 'ready') {
    reads.value.data.modelSwitches = 2;
    reads.value.data.requests = 18;
  }
  if (scenario === 'collaboration' && reads.agents.status === 'ready') {
    reads.agents.data.agents = reads.agents.data.agents.map(agent => agent.agentId === 'agent_claude_default'
      ? { ...agent, model: 'unconfigured', collaboration: 'configured' }
      : agent.agentId === 'agent_codex_default' ? { ...agent, model: 'unconfigured', collaboration: 'unconfigured' } : agent);
  }
  return reads;
}

function agentSnapshotFor(scenario: ProductScenario): AgentSnapshot {
  if (scenario !== 'collaboration') return readyAgents;
  return {
    ...readyAgents,
    agents: readyAgents.agents.map(agent => agent.agent_id === 'agent_claude_default'
      ? { ...agent, settings: { ...agent.settings!, state: 'restored', model_verified: false, current_selection: null, collaboration: { ...agent.settings!.collaboration!, current_selection: { trigger_mode: 'explicit' } } } }
      : agent.agent_id === 'agent_codex_default' ? { ...agent, settings: null, configuration_state: 'not_configured' } : agent),
  };
}

export function mockProductInvoke(command: string, payload: Record<string, any> | undefined, scenario: ProductScenario) {
  fixtureTrace.commands.push(command);
  const fresh = scenario === 'fresh';
  if (command === 'compute_management_snapshot') return fresh ? { ...readyManagement, sources: [] } : readyManagement;
  if (command === 'compute_connection_options') return {
    schema: 'hiroute.compute-connection-options/v1',
    catalog: {
      product_release: 'mvp-current', catalog_binding_id: 'client-bundled/current', release_sequence: 1,
      connector_registry_digest: 'sha256:fixture-registry', model_data_digest: 'sha256:fixture-models',
      cross_reference_digest: 'sha256:fixture-cross-reference',
    },
    options: [{
      connection_option_id: 'bailian.payg.cn.v1', display_name: 'Bailian Pay As You Go China',
      connector_id: 'connector.bailian.p0', connector_revision: 1,
      endpoint_profile_id: 'endpoint.bailian.payg.cn.v1', endpoint_profile_revision: 1,
      billing_class: 'paid', authentication: 'provider_api_key',
      model_configuration_ids: ['model.bailian.qwen3-max-2026-01-23'],
    }],
  };
  if (command === 'compute_subscriptions') return { discovery_state: 'complete', candidates: [{
    ...localSubscriptionCandidate,
    ...(fresh ? {} : { existing_source_id: 'source/codex/subscription' }),
  }] };
  if (command === 'compute_scan') return { items: fresh ? [] : [{
    agent_id: 'agent_claude_default',
    supported: false,
    configuration_state: 'registered_with_protected_input',
    connection_option_id: 'zhipu.coding-plan.cn.v1',
    observed_model_id: 'glm-5.3',
    inventory_eligible: true,
    discovery: {
      discovery_ref: `discovery/${'a'.repeat(64)}`,
      discovery_revision: '8091638379121373450',
    },
    actions_required: [],
  }] };
  if (command === 'prepare_discovered_model_connection') return discoveredClaudeCandidate;
  if (command === 'recover_subscription_check') return null;
  if (command === 'check_subscription' || command === 'get_subscription_check_result') return checkedLocalSubscriptionResult;
  if (command === 'close_subscription_check') return undefined;
  if (command === 'desktop_snapshot') return fresh ? { ...readyDesktop, catalog: { plans: [], drafts: [] } } : readyDesktop;
  if (command === 'agent_snapshot') return fresh ? { ...readyAgents, agents: [] } : agentSnapshotFor(scenario);
  if (command === 'worker_executor_availability') {
    const ready = { state: 'ready' as const };
    const unavailable = { state: 'unavailable' as const, reason: 'installation_not_configured' as const };
    return {
      schema: 'hiroute.worker-executor-availability-list/v1',
      executors: fresh ? [
        { harness: 'codex_cli', state: 'unavailable', reason: 'installation_not_configured', start_approve_all: unavailable, cancel: unavailable, continue_session: unavailable, restricted_policy: unavailable },
        { harness: 'claude_code', state: 'unavailable', reason: 'installation_not_configured', start_approve_all: unavailable, cancel: unavailable, continue_session: unavailable, restricted_policy: unavailable },
      ] : [
        {
          harness: 'codex_cli', state: 'ready',
          start_approve_all: ready, cancel: ready, continue_session: ready,
          restricted_policy: { state: 'unknown', reason: 'restricted_policy_unverified' },
        },
        {
          harness: 'claude_code', state: 'ready',
          start_approve_all: ready, cancel: ready,
          continue_session: { state: 'unavailable', reason: 'resume_unavailable' },
          restricted_policy: { state: 'unknown', reason: 'restricted_policy_unverified' },
        },
      ],
    };
  }
  if (command === 'worker_dependencies_discover' || command === 'worker_dependencies_select_confirm') {
    const confirmed = command === 'worker_dependencies_select_confirm' ? preparedWorkerSelection : null;
    const harness = confirmed?.harness ?? payload?.input?.harness ?? 'claude_code';
    const name = harness === 'codex_cli' ? 'codex' : 'claude';
    const selection = confirmed ?? {
      harness,
      cli_path: `/usr/local/bin/${name}`,
      adapter_path: `/usr/local/lib/${name}-acp/adapter.js`,
      node_path: '/usr/local/bin/node',
    };
    const data = {
      schema: 'hiroute.worker-dependencies-view/v1',
      selection_revisions: [{ harness, revision: 1 }],
      selected: [selection],
      candidates: [
        { harness, component: 'cli', path: selection.cli_path, source: 'selected', state: 'found' },
        { harness, component: 'adapter', path: selection.adapter_path, source: 'selected', state: 'found' },
        { harness, component: 'node', path: selection.node_path, source: 'selected', state: 'found' },
      ],
      install_hints: [],
    };
    return { status: 'succeeded', data, operation: null, next_actions: [], error: null };
  }
  if (command === 'worker_dependencies_select_prepare') {
    preparedWorkerSelection = payload?.input ?? null;
    return {
      schema: 'hiroute.worker-dependency-selection-confirmation/v1',
      confirmation_id: `worker-dependency-confirmation/${'a'.repeat(64)}`,
      selection: payload?.input,
      expires_at_ms: now + 60_000,
    };
  }
  if (command === 'worker_dependencies_select_cancel') { preparedWorkerSelection = null; return undefined; }
  if (command === 'worker_settings_get') {
    return { schema: 'hiroute.worker-settings/v1', max_concurrent: 10 };
  }
  if (command === 'worker_settings_set') {
    return payload?.input;
  }
  if (command === 'effective_price_query') {
    const targets = payload?.input?.targets ?? [];
    return {
      result: {
        pending_activation: false,
        evaluated_at: now,
        generation_ref: null,
        items: targets.map((target: any) => ({ query_id: target.query_id, quote: { origin: 'catalog', quote_digest: 'sha256:fixture-price', unknown_reasons: [] }, edit_context: { target_locator: target.target_locator, expected_source_revision: 1, expected_binding_revision: 1, expected_override_revision: 0 } })),
      },
      display_rates: targets.map(() => ['0.800000', '2.000000', null, null]),
    };
  }
  if (command === 'preview_price_change') return {
    state: 'applied',
    operation: {
      operation_id: 'operation/fixture-price-save',
      state: 'succeeded',
      sequence: 1,
      cancellable: false,
    },
  };
  if (command === 'plan_editor_options') return {
    suggested_alias: 'hiroute-daily-coding',
    candidates: fresh ? [] : readyManagement.sources.flatMap(source => source.models.map(model => ({ binding_id: model.binding_id, model_configuration_id: model.catalog_configuration_id ?? model.upstream_model_id, display_name: model.display_name, reasoning: model.native_reasoning ?? { kind: 'fixed', profile: 'provider-default' }, billing_class: model.presentation?.billing_class ?? 'unknown', routable: source.state === 'ready', ingress_protocols: ['responses', 'messages'] }))),
    free_suggestions: fresh ? { candidates: [], unavailable: {} } : { candidates: [{ selection: { binding_id: 'binding/free/deepseek' } }, { selection: { binding_id: 'binding/free/glm' } }, { selection: { binding_id: 'binding/free/qwen' } }], unavailable: {} },
    codex_capabilities: fresh || !payload?.input?.editor ? null : {
      state: 'available',
      context_window: 64000,
      input_modalities: ['text'],
      reasoning: 'route_configuration',
      limitations: [
        { kind: 'context_window', binding_ids: ['binding/bailian/qwen-coder'] },
        { kind: 'image_input', binding_ids: ['binding/bailian/qwen-coder'] },
      ],
      fixed_limits: ['parallel_tool_calls_disabled'],
    },
  };
  if (command === 'preview_agent_settings') return scenario === 'agent_blocked' && !agentPrerequisiteChecked
    ? { preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [{ capability: 'ingress_authentication', reason: 'unverified' }] }] }, mutation: null }
    : { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { state: 'succeeded' } } };
  if (command === 'check_agent_authentication') { agentPrerequisiteChecked = true; return true; }
  if (command === 'check_agent_live') return { accepted: true, scope: 'live', model_call: true, state: 'passed', call_count: 1, requested_call_count: 1 };
  if (command === 'preview_plan_editor') return { state: 'succeeded', operation: { state: 'succeeded' } };
  if (command === 'preview_restore_name') return { operation: { state: 'succeeded' } };
  if (command === 'preview_compute_save') return {
    candidate: payload?.change?.subject?.candidate,
    spec: { schema_version: { major: 2, minor: 0 }, command_id: 'fixture-model-save', desired_state: {} },
    accept_digest: 'sha256:fixture-save',
    expected_revisions: payload?.change?.expected_revisions ?? readyManagement.revisions,
    changes: [{ resource_kind: 'model_source', resource_id: 'source/fixture/new', action: 'create' }],
    affected_plan_refs: [],
  };
  if (command === 'apply_compute_save') {
    const operation = { operation_id: 'operation/fixture-model-save', state: 'succeeded', sequence: 1, cancellable: false };
    return { result: { operation_id: operation.operation_id, accepted_digest: 'sha256:fixture-save', state: 'accepted' }, operation };
  }
  if (command === 'get_compute_save_result') return {
    disposition: 'saved', source_id: 'source/fixture/new', saved_revision: 1, management_state: 'ready',
    bindings: [
      { model_ref: 'model-ref/qwen3-coder-plus', binding_id: 'binding/fixture/qwen3-coder-plus', revision: 1 },
      { model_ref: 'model-ref/qwen3-max', binding_id: 'binding/fixture/qwen3-max', revision: 1 },
    ],
  };
  if (command === 'observation_delete') return { cancelled: false, managed_native_cleanup_pending: false, managed_object_cleanup_pending: false, object_cleanup_pending: false };
  if (command === 'observation_read') {
    const intent = payload?.request?.intent ?? {};
    const view = intent.view;
    const query = intent.query ?? {};
    if (view === 'sessions') {
      const rows = query.only_model_switch
        ? sessionRows.filter(row => row.fallback_request_count > 0)
        : sessionRows;
      return { sessions: fresh || scenario === 'configured_no_sessions' ? [] : rows, next_cursor: null };
    }
    if (view === 'search') {
      const needle = String(query.keyword ?? '').trim().toLocaleLowerCase();
      const allowedSessions = new Set((query.only_model_switch
        ? sessionRows.filter(row => row.fallback_request_count > 0)
        : sessionRows).map(row => row.session_id));
      const hits = fresh || scenario === 'configured_no_sessions' || !needle ? [] : Object.entries(requestBySession).flatMap(([sessionId, requestId]) => {
        if (!allowedSessions.has(sessionId)) return [];
        const content = messages[requestId];
        if (!content) return [];
        return (['user', 'assistant'] as const).flatMap(role => {
          const normalized = content[role].toLocaleLowerCase();
          const characterOffset = normalized.indexOf(needle);
          if (characterOffset < 0) return [];
          return [{
            session_id: sessionId,
            request_id: requestId,
            content_id: `${requestId}/${role}`,
            original_text_offset: new TextEncoder().encode(content[role].slice(0, characterOffset)).length,
          }];
        });
      });
      return { hits, next_cursor: null, index_partial: false, budget_exhausted: false };
    }
    if (view === 'timeline') {
      const requestId = requestBySession[query.session_id];
      const routeName = query.session_id === 'session/release-notes' ? '免费资料整理' : query.session_id === 'session/boundary-test' ? '代码实现' : '日常编码';
      const finalModel = query.session_id === 'session/rust-ownership'
        ? 'Qwen3-Coder-Plus'
        : query.session_id === 'session/release-notes'
          ? 'DeepSeek V3.1 Free'
          : 'GPT-5.6-Sol';
      return { requests: requestId ? [{ request_id: requestId, started_at_ms: sessionRows.find(row => row.session_id === query.session_id)?.last_request_at_ms ?? now, outcome: query.session_id === 'session/stream-gap' ? 'failed' : 'completed', native_turn_id: null, within_request_fallback: query.session_id === 'session/gateway-refactor', final_native_model: finalModel, between_turn_model_change: false, previous_native_turn_id: null, previous_turn_model: null, turn_final_native_model: null, routing_context: { state: 'recorded', display_name: routeName, name_state: 'recorded' } }] : [], next_cursor: null };
    }
    if (view === 'catalog') {
      const requestId = query.request_id as string;
      const incomplete = requestId === 'request/stream-gap';
      const gatewayContents = requestId === 'request/gateway-refactor' ? [
        { content_id: `${requestId}/user`, role: 'user', kind: 'text', state: 'complete', direction: 'ingress', media_type: 'text/plain', message_occurrence_id: '1' },
        { content_id: `${requestId}/assistant-initial`, role: 'assistant', kind: 'text', state: 'complete', direction: 'egress', media_type: 'text/plain', message_occurrence_id: '2' },
        { content_id: `${requestId}/tool-start`, role: 'assistant', kind: 'tool_call_started', state: 'complete', direction: 'response_delivered', media_type: 'application/vnd.hiroute.model-stream-event+json;version=1', message_occurrence_id: '3' },
        { content_id: `${requestId}/tool-args`, role: 'assistant', kind: 'tool_arguments_delta', state: 'complete', direction: 'response_delivered', media_type: 'application/vnd.hiroute.model-stream-event+json;version=1', message_occurrence_id: '4' },
        { content_id: `${requestId}/tool-ready`, role: 'assistant', kind: 'tool_call_finished', state: 'complete', direction: 'response_delivered', media_type: 'application/vnd.hiroute.model-stream-event+json;version=1', message_occurrence_id: '5' },
        { content_id: `${requestId}/assistant`, role: 'assistant', kind: 'text', state: 'complete', direction: 'egress', media_type: 'text/plain', message_occurrence_id: '6' },
      ] : null;
      return { contents: messages[requestId] ? gatewayContents ?? [{ content_id: `${requestId}/user`, role: 'user', kind: 'text', state: 'complete', direction: 'ingress', media_type: 'text/plain', message_occurrence_id: '1' }, { content_id: `${requestId}/assistant`, role: 'assistant', kind: 'text', state: incomplete ? 'partial' : 'complete', direction: 'egress', media_type: 'text/plain', message_occurrence_id: '2' }] : [], transcript_roots: [], roots_partial: false, next_cursor: null };
    }
    if (view === 'content') {
      const requestId = query.request_id as string;
      const contentId = String(query.content_id);
      const role = contentId.endsWith('/user') ? 'user' : 'assistant';
      const special: Record<string, string> = {
        'request/gateway-refactor/assistant-initial': '我先检查 execution plan 与 runtime driver 的作用域关系。',
        'request/gateway-refactor/tool-start': JSON.stringify({ schema_version: 'hiroute.model-stream-event/v1', sequence: 1, event: { kind: 'tool_call_started', logical_id: 'tool/read', namespace: '', name: 'read' } }),
        'request/gateway-refactor/tool-args': JSON.stringify({ schema_version: 'hiroute.model-stream-event/v1', sequence: 2, event: { kind: 'tool_arguments_delta', logical_id: 'tool/read', delta: '{"paths":["execution_plan.rs","driver.rs"]}' } }),
        'request/gateway-refactor/tool-ready': JSON.stringify({ schema_version: 'hiroute.model-stream-event/v1', sequence: 3, event: { kind: 'tool_call_finished', logical_id: 'tool/read', namespace: '', name: 'read' } }),
      };
      return { state: requestId === 'request/stream-gap' && role === 'assistant' ? 'partial' : 'complete', chunks: [{ text: special[contentId] ?? messages[requestId]?.[role] ?? '', original_byte_offset: 0 }], next_cursor: null };
    }
    if (view === 'facts') {
      const fallback = query.request_id === 'request/gateway-refactor';
      return {
        facts: fallback ? [
          { event_id: 'event/1', sequence: 1, occurred_at_ms: now - 300, event_kind: 'model_attempt', attempt_ordinal: 1, native_model: 'Claude Sonnet 4.5', input_tokens: null, output_tokens: null, cache_read_tokens: null, cache_write_tokens: null, reasoning_tokens: null, outcome: 'failed', sensitive_fields_deleted: true },
          { event_id: 'event/2', sequence: 2, occurred_at_ms: now, event_kind: 'model_attempt', attempt_ordinal: 2, native_model: 'GPT-5.6-Sol', input_tokens: 38_200, output_tokens: 4_600, cache_read_tokens: null, cache_write_tokens: null, reasoning_tokens: null, outcome: 'completed', sensitive_fields_deleted: true },
        ] : [{ event_id: 'event/1', sequence: 1, occurred_at_ms: now, event_kind: 'model_attempt', attempt_ordinal: 1, native_model: 'GPT-5.6-Sol', input_tokens: 38_200, output_tokens: 4_600, cache_read_tokens: null, cache_write_tokens: null, reasoning_tokens: null, outcome: 'completed', sensitive_fields_deleted: true }],
        projection_partial: false,
        next_cursor: null,
      };
    }
    if (view === 'home_value') return query.session_id ? {
      pending_requests: 0,
      provisional_requests: 0,
      unknown_traffic_requests: 0,
      excluded_requests: 1,
      amounts: [{ known_sum_micros: 6_420_000, currency: '¥', valuation_kind: 'usage_estimate', coverage: 'complete' }],
      usage: [
        { metric: 'input', known_sum: 38_200, coverage: 'complete' },
        { metric: 'output', known_sum: 4_600, coverage: 'complete' },
        { metric: 'cache_read', known_sum: 22_920, coverage: 'complete' },
        { metric: 'cache_write', known_sum: 810, coverage: 'complete' },
      ],
      input_cache_hit: { state: 'available', ratio_basis_points: 6000, cache_read_tokens: 22_920, total_input_tokens: 38_200, eligible_attempt_count: 1, total_attempt_count: 1, zero_input_attempt_count: 0, missing_attempt_count: 0, invalid_attempt_count: 0, arithmetic_overflow: false, archive_coverage_partial: false, coverage: 'complete' },
      archive_boundary_partial: false,
      retention_boundary_partial: false,
    } : {
      pending_requests: 0,
      provisional_requests: 0,
      unknown_traffic_requests: 0,
      excluded_requests: 0,
      amounts: [],
      usage: [
        { metric: 'input', known_sum: 2_400, coverage: 'complete' },
        { metric: 'output', known_sum: 1_100, coverage: 'complete' },
        { metric: 'cache_read', known_sum: 0, coverage: 'complete' },
        { metric: 'cache_write', known_sum: 0, coverage: 'complete' },
      ],
      input_cache_hit: { state: 'available', ratio_basis_points: 0, cache_read_tokens: 0, total_input_tokens: 2_400, eligible_attempt_count: 1, total_attempt_count: 1, zero_input_attempt_count: 0, missing_attempt_count: 0, invalid_attempt_count: 0, arithmetic_overflow: false, archive_coverage_partial: false, coverage: 'complete' },
      archive_boundary_partial: false,
      retention_boundary_partial: false,
    };
    if (view === 'status') return { running: true, index_running: true, error_count: 0 };
    if (view === 'ancestry') return { roots: [], gap: null };
  }
  if (command === 'check_saved_model_connection') {
    if (scenario === 'api_failure') throw { code: 'MODEL_CONNECTION_AUTHENTICATION_REJECTED' };
    const request = payload?.request ?? {};
    const source = readyManagement.sources.find(item => item.source_id === request.source_id);
    if (!source) throw { code: 'RECHECK_CONTEXT_UNAVAILABLE' };
    const candidateRef = `candidate/recheck/${source.source_id}`;
    const digest = 'sha256:fixture-saved-check';
    return {
      candidate: {
        candidate: { candidate_ref: candidateRef, candidate_revision: source.revision + 1 },
        correlation: { candidate_ref: candidateRef, edit_revision: request.edit_revision, check_id: request.check_id, input_digest: digest },
        producer: source.provenance === 'connector_owned' ? 'cpa' : 'native',
        provenance: source.provenance,
        display_name: source.display_name,
        existing_source_id: source.source_id,
        models: source.models.map(model => ({
          model_ref: model.model_ref,
          upstream_model_id: model.upstream_model_id,
          display_name: model.display_name,
          membership: model.membership,
          selectable: true,
        })),
        input_state: source.authentication.kind === 'none' ? 'not_required' : 'provided',
        fact_state: 'complete',
        issues: [],
      },
      target: source.target,
      inventory_path: '/v1/models', reachability: 'reachable',
      authentication: source.authentication.kind === 'none' ? 'not_required' : 'verified',
      directory: 'available', protocol: 'selected', inference: 'not_run',
      checked_model_count: source.models.length, invalid_model_count: 0, pages_read: 1,
      checked_at_unix_ms: now, input_digest: digest, issues: [],
    };
  }
  if (command === 'check_model_connection' || command === 'check_registered_model_connection') {
    if (scenario === 'api_failure') throw { code: 'MODEL_CONNECTION_AUTHENTICATION_REJECTED' };
    const registered = command === 'check_registered_model_connection';
    const draft = payload?.request?.draft ?? payload?.request ?? {};
    const digest = 'sha256:fixture-check';
    const customUnknown = !registered && scenario === 'unknown_model';
    const declaredModels = Array.isArray(draft.models) ? draft.models : [];
    return {
      candidate: {
        candidate: { candidate_ref: 'candidate/protected', candidate_revision: 2 },
        correlation: { candidate_ref: 'candidate/protected', edit_revision: draft.edit_revision, check_id: draft.check_id, input_digest: digest },
        producer: 'native', provenance: registered ? 'registered' : 'user_configured', display_name: registered ? 'Bailian Pay As You Go China' : draft.display_name,
        models: customUnknown ? declaredModels.map((model: any, index: number) => ({
          model_ref: `model-ref/custom-${index}`,
          upstream_model_id: model.upstream_model_id,
          display_name: model.display_name || model.upstream_model_id,
          membership: 'user_declared',
          selectable: true,
        })) : [
          { model_ref: 'model-ref/qwen3-coder-plus', upstream_model_id: 'qwen3-coder-plus', display_name: 'Qwen3-Coder-Plus', membership: 'observed', selectable: true },
          { model_ref: 'model-ref/qwen3-max', upstream_model_id: 'qwen3-max', display_name: 'Qwen3-Max', membership: 'observed', selectable: true },
        ],
        input_state: 'provided', fact_state: 'complete', issues: [],
      },
      target: { scheme: 'https', authority: 'coding.dashscope.aliyuncs.com', port: 443, request_path: '/v1/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/custom/chat_completions', protocol_profile_revision: 1 },
      inventory_path: '/v1/models', reachability: 'reachable', authentication: 'verified', directory: 'available', protocol: 'selected', inference: 'not_run', checked_model_count: 2, invalid_model_count: 0, pages_read: 1, checked_at_unix_ms: now, input_digest: digest, issues: [],
    };
  }
  if (command === 'release_protected_model_input' || command === 'cancel_model_connection_check') return undefined;
  if (command === 'register_protected_model_input') return { input_candidate: { candidate_ref: 'candidate/protected', candidate_revision: 1 } };
  throw new Error(`Unexpected V3 fixture command: ${command}`);
}
