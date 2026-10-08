import type { Plan, Options, Selection } from './src/plan-editor';
import type { PlanQualitySample } from './src/features/PlanQuality';
import type { DesktopSnapshot } from './src/product/home-projections';
import type { DecisionService, Judgment } from './src/features/decision-services/types';
import { defaultJudgment } from './src/features/decision-services/types.ts';
import { qualityExecutionKey, type QualityExecution, type QualitySummary } from './src/features/plan-quality-state.ts';

// Real model labels with fabricated executions, never model-performance claims.
export function decisionDocsData(language: 'zh' | 'en', now = Date.now()) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const models = [
    { model_configuration_id: 'qwen-flash', display_name: 'Qwen 3.8 Flash', native_model: 'qwen3.8-flash' },
    { model_configuration_id: 'qwen-max', display_name: 'Qwen 3.8 Max', native_model: 'qwen3.8-max' },
  ];
  const services: DecisionService[] = [
    { id: 'docs-bailian', revision: 1, name: text('百炼决策模型', 'Bailian decision model'), connection: { kind: 'system_one', provider: 'bailian-token-plan', model: 'decision-model-preview', endpoint: 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone', timeout_ms: 10000, auth_header: { name: 'Authorization', value_secret_ref: 'docs-key-not-a-real-credential' } } },
    { id: 'docs-openrouter', revision: 1, name: 'Jev · OpenRouter', connection: { kind: 'system_one', provider: 'openrouter', model: 'typesafe/jev-1.13', endpoint: 'https://openrouter.ai/api/alpha/decisions', timeout_ms: 10000, auth_header: { name: 'Authorization', value_secret_ref: 'docs-key-not-a-real-credential' } } },
    { id: 'docs-custom', revision: 1, name: text('团队决策扩展', 'Team decision extension'), connection: { kind: 'custom', endpoint: 'https://classifier.example/v1/decisions', timeout_ms: 10000 } },
  ];
  const select = (model: number, profile: string): Selection => ({ binding_id: models[model].model_configuration_id, reasoning: { kind: 'profile', profile } });
  const classifier = { kind: 'decision_service' as const, service: services[0] };
  const plan: Plan = {
    agent_plan_id: 'plan/order-maintenance', agent_plan_revision: 3,
    head: { head_revision: 3, status: 'enabled' }, model_alias: 'order-maintenance',
    publication: { revision: 3, digest: 'documentation-fixture' }, execution: 'ready',
    desired: { display_name: text('订单服务维护', 'Order service maintenance'),
      purpose: text('接口修复、支付回调排错与回归验证。', 'API fixes, payment callback debugging and regression checks.'),
      mode: 'smart_saving', delegation_enabled: false, requirements: {},
      limits: { maximum_attempts: 6, request_timeout_ms: 3600000, attempt_timeout_ms: 30000 },
      strategy: { mode: 'smart_saving', economy: [select(0, 'low')], primary: [select(1, 'high')],
        judgment: structuredClone(defaultJudgment), reselect_on_user_message: false, complex_keywords: [], classifier },
    },
  };
  const writingJudgment: Judgment = structuredClone(defaultJudgment);
  writingJudgment.degree.simple = text('材料齐备，范围明确，能按已有标准完成写作或核验。', 'Complete a bounded writing or review task using sufficient materials and established criteria.');
  writingJudgment.degree.complex = text('需要整合冲突资料、深入论证或重新组织全文结构。', 'Resolve conflicting sources, examine a complex argument or restructure the whole article.');
  const customPlan: Plan = { ...plan, agent_plan_id: 'plan/editorial', model_alias: 'editorial', desired: {
    ...plan.desired, display_name: text('文章写作与审稿', 'Writing and review'),
    purpose: text('按任务选择写稿或审稿，再判断所需投入。', 'Choose writing or review, then judge the effort required.'), mode: 'custom_branches',
    strategy: { mode: 'branches', routing: { classifier, judgment: writingJudgment,
      default_branch_id: 'review', reselect_on_user_message: false, branches: [
        { id: 'writing', name: text('写稿', 'Writing'), condition: text('撰写初稿、扩写内容，或根据材料修改文章。', 'Draft, expand or revise an article using the source material.'), candidates: [select(1, 'medium')], primary_candidates: [select(1, 'high')] },
        { id: 'review', name: text('审稿', 'Review'), condition: text('核验事实与引用，评价结构、论证和可读性。', 'Check facts and citations; assess structure, reasoning and readability.'), candidates: [select(0, 'low')], primary_candidates: [select(1, 'high')] },
      ] } },
  } };
  const specs = [
    { id: 'review-main', branch: 'review', model: 1, group: 'primary', profile: 'high', from: 4, to: 6, through: 6, score: .91, partial: false, age: 20 },
    { id: 'review-start', branch: 'review', model: 0, group: 'regular', profile: 'low', from: 1, to: 3, through: 3, score: .38, partial: false, age: 40 },
    { id: 'writing-main', branch: 'writing', model: 1, group: 'primary', profile: 'high', from: 1, to: 2, through: 2, score: null, partial: false, age: 70 },
    { id: 'writing-start', branch: 'writing', model: 1, group: 'regular', profile: 'medium', from: 1, to: 5, through: 4, score: .88, partial: false, age: 110 },
    { id: 'writing-partial', branch: 'writing', model: 1, group: 'regular', profile: 'medium', from: 1, to: 3, through: 3, score: .37, partial: true, age: 140 },
  ] as const;
  const clock = Math.floor(now / 60000) * 60000;
  const samples: PlanQualitySample[] = specs.map(s => ({
    segment_id: `docs-${s.id}`, session_id: s.branch === 'review' ? 'session-review' : `session-${s.id}`,
    plan_id: customPlan.agent_plan_id, plan_revision: 3, selected_branch_id: s.branch, executed_branch_id: s.branch,
    model_configuration_id: `published-${models[s.model].model_configuration_id}`, native_model: models[s.model].native_model,
    profile_digest: `docs-${s.profile}`, reasoning_profile_id: s.profile, attribution: 'single',
    branch_execution: { policy: { name: s.branch === 'review' ? text('审稿', 'Review') : text('写稿', 'Writing'), floor_millis: 500 }, group: s.group, candidate_index: 0 },
    selection: { execution_group: s.group, simple_probability: s.id === 'review-main' ? .86 : s.group === 'regular' ? .92 : .21,
      simple_threshold_millis: 800, selection_reason: s.id === 'review-main' ? 'low_competence' : s.group === 'regular' ? 'simple_task' : 'complex_task',
      competence_trigger: s.id === 'review-main' ? { score: .38, floor_millis: 500, segment_id: 'docs-review-start' } : null },
    upgrade: s.id === 'review-start' ? { trigger_request_id: 'req-review-main-first', decision: { segment_id: 'docs-review-start', score: .38, floor_millis: 500, from_group: 'regular' } } : null,
    first_turn_ordinal: s.from, last_observed_turn_ordinal: s.to,
    first_at_ms: clock - (s.age + 12) * 60000, last_at_ms: clock - s.age * 60000,
    history_partial: s.partial, first_request_id: `req-${s.id}-first`, last_request_id: `req-${s.id}-last`, execution_evidence_available: false,
    assessment: s.score == null ? null : { trigger_request_id: s.id === 'review-start' ? 'req-review-main-first' : `req-${s.id}-next`, assessed_at_ms: clock - (s.id === 'review-start' ? 32 : s.age - 1) * 60000,
      target_from_ordinal: s.from, target_through_ordinal: s.through, score: s.score, partial: s.partial, evidence_available: false },
  }));
  const options: Options = { context_window: { maximum_tokens: 272000, default_tokens: 272000 }, suggested_alias: null,
    free_suggestions: null, codex_capabilities: null, candidates: models.map(m => ({ ...m, binding_id: m.model_configuration_id,
      reasoning: { kind: 'discrete', parameter: 'reasoning_effort', profiles: ['low', 'medium', 'high', 'max'] },
      billing_class: 'paid', routable: true, ingress_protocols: ['responses', 'messages', 'chat_completions'] })) };
  const snapshot: DesktopSnapshot = { catalog_error: null,
    service: { daemon_role: 'control', recovery_ready: true, mutation_available: true, gateway: 'ready' },
    catalog: { plans: [plan, customPlan], drafts: [] }, trusted_authority: true, restore_names: [], pending: null };
  return { models, services, plan, customPlan, samples, snapshot, options };
}

export type DocsQualityQuery = {
  plan_id?: string | null; session_id?: string | null; plan_revision?: number | null;
  execution?: QualityExecution | null; from_ms?: number; to_ms?: number;
  competence?: 'below_floor' | 'meets_floor' | null; unrated_only?: boolean; limit?: number; cursor?: string | null;
};
const execution = (s: PlanQualitySample): QualityExecution => ({
  plan_revision: s.plan_revision, selected_branch_id: s.selected_branch_id, executed_branch_id: s.executed_branch_id ?? null,
  model_configuration_id: s.model_configuration_id ?? null, profile_digest: s.profile_digest ?? null, attribution: s.attribution,
  group: s.branch_execution?.group ?? null, candidate_index: s.branch_execution?.candidate_index ?? null,
});
const scored = (s: PlanQualitySample) => !!s.assessment && !s.assessment.partial;

// Bounded fixture projection, not another production aggregation implementation.
export function queryDocsSamples(samples: PlanQualitySample[], q: DocsQualityQuery) {
  if (q.cursor) throw new Error('DOCUMENTATION_CURSOR_UNSUPPORTED');
  const scope = samples.filter(s => (!q.plan_id || s.plan_id === q.plan_id) && (!q.session_id || s.session_id === q.session_id)
    && (q.plan_revision == null || s.plan_revision === q.plan_revision)
    && (q.from_ms == null || s.last_at_ms >= q.from_ms) && (q.to_ms == null || s.last_at_ms < q.to_ms));
  const summary: QualitySummary = { models: [], scored_stage_count: scope.filter(scored).length,
    unrated_stage_count: scope.filter(s => !scored(s)).length, session_count: new Set(scope.map(s => s.session_id)).size,
    available_revisions: [...new Set(scope.map(s => s.plan_revision))] };
  for (const s of scope) {
    if (summary.models.some(m => qualityExecutionKey(m.execution) === qualityExecutionKey(execution(s)))) continue;
    const group = scope.filter(other => qualityExecutionKey(execution(other)) === qualityExecutionKey(execution(s)));
    const scores = group.filter(scored).map(s => s.assessment!.score);
    summary.models.push({ execution: execution(s), native_model: s.native_model!, reasoning_profile_id: s.reasoning_profile_id!,
      branch_policy: s.branch_execution!.policy, scored_stage_count: scores.length, unrated_stage_count: group.length - scores.length,
      average_score: scores.length ? scores.reduce((a,b) => a+b, 0) / scores.length : null });
  }
  return { summary, samples: scope.filter(s => (!q.execution || qualityExecutionKey(execution(s)) === qualityExecutionKey(q.execution))
    && (!q.unrated_only || !scored(s)) && (!q.competence || (scored(s) && (q.competence === 'below_floor'
      ? s.assessment!.score * 1000 < s.branch_execution!.policy.floor_millis : s.assessment!.score * 1000 >= s.branch_execution!.policy.floor_millis))))
    .slice(0, q.limit ?? 20), next_cursor: null };
}
