import type { Plan } from './src/plan-editor';
import type { PlanQualitySample } from './src/features/PlanQuality';
import type { DesktopSnapshot } from './src/product/home-projections';

// Real model labels, fabricated executions. Never a benchmark or live account data.
export function decisionDocsData(language: 'zh' | 'en', now = Date.now()) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const models = [
    { model_configuration_id: 'deepseek-v4-1-flash', display_name: 'DeepSeek V4.1 Flash' },
    { model_configuration_id: 'claude-sonnet-5', display_name: 'Claude Sonnet 5' },
  ];
  const plan: Plan = {
    agent_plan_id: 'plan/order-service-maintenance', agent_plan_revision: 3,
    head: { head_revision: 3, status: 'enabled' },
    model_alias: 'order-service-maintenance', publication: { revision: 3, digest: 'documentation-fixture' },
    execution: 'ready', desired: {
      display_name: text('订单服务维护', 'Order service maintenance'),
      purpose: text('接口修复、支付回调排错与回归验证。', 'API fixes, payment callback debugging, and regression checks.'),
      mode: 'smart_saving', delegation_enabled: false, requirements: {},
      limits: { maximum_attempts: 3, request_timeout_ms: 60000, attempt_timeout_ms: 30000 },
      strategy: { mode: 'smart_saving', economy: [{ binding_id: models[0].model_configuration_id }],
        primary: [{ binding_id: models[1].model_configuration_id }], primary_fallback: false, reselect_on_user_message: false, complex_keywords: [],
        classifier: { kind: 'rest', endpoint: 'http://127.0.0.1:8080/v1/decisions', timeout_ms: 3000 } },
    },
  };
  // Distinct sessions with their own ranges; one stage has an unassessed newer round.
  const specs = [
    { id: 'callback-retry', model: 1, from: 1, to: 3, scoredThrough: 3, score: null, partial: false, age: 27, duration: 12 },
    { id: 'payment-idempotency', model: 1, from: 6, to: 10, scoredThrough: 9, score: 0.91, partial: false, age: 183, duration: 38 },
    { id: 'order-reconciliation', model: 0, from: 1, to: 6, scoredThrough: 6, score: 0.37, partial: true, age: 1271, duration: 51 },
    { id: 'pagination-validation', model: 0, from: 1, to: 4, scoredThrough: 4, score: 0.88, partial: false, age: 1566, duration: 19 },
  ];
  const clock = Math.floor(now / 60_000) * 60_000;
  const samples: PlanQualitySample[] = specs.map(s => ({
    segment_id: `docs-stage-${s.id}`, session_id: `session-${s.id}`,
    plan_id: plan.agent_plan_id, plan_revision: 3,
    selected_branch_id: s.model ? 'smart_saving_complex' : 'smart_saving_simple',
    executed_branch_id: s.model ? 'smart_saving_complex' : 'smart_saving_simple',
    model_configuration_id: models[s.model].model_configuration_id,
    attribution: 'single', first_turn_ordinal: s.from, last_observed_turn_ordinal: s.to,
    first_at_ms: clock - (s.age + s.duration) * 60_000,
    last_at_ms: clock - s.age * 60_000,
    history_partial: s.partial,
    first_request_id: `req-${s.id}-first`, last_request_id: `req-${s.id}-last`,
    execution_evidence_available: false,
    assessment: s.score === null ? null : {
      trigger_request_id: `req-${s.id}-followup`, target_from_ordinal: s.from,
      target_through_ordinal: s.scoredThrough, score: s.score, partial: s.partial,
      // Official Jev emits no text reason. No transcript exists in this fixture.
      evidence_available: false,
    },
  }));
  const otherPlans = [
    ['local-fix', text('局部修复与测试', 'Local fixes & tests'), text('单模块修改，明确输入与验收条件。', 'Single-module changes with explicit acceptance criteria.')],
    ['research', text('资料检索与归纳', 'Research & synthesis'), text('检索资料、核对来源并整理结论。', 'Find sources, cross-check evidence, and summarize findings.')],
  ].map(([id, name, purpose]): Plan => ({
    ...plan, agent_plan_id: `plan/${id}`, model_alias: id,
    desired: { ...plan.desired, display_name: name, purpose },
  }));
  const snapshot: DesktopSnapshot = {
    catalog_error: null, service: { daemon_role: 'control', recovery_ready: true, mutation_available: true, gateway: 'ready' },
    catalog: { plans: [plan, ...otherPlans], drafts: [] },
    trusted_authority: true, restore_names: [], pending: null,
  };
  return { models, plan, samples, snapshot };
}

export type DocsQualityQuery = {
  plan_id?: string | null; session_id?: string | null; plan_revision?: number | null;
  model_configuration_id?: string | null; from_ms?: number; to_ms?: number;
  score_lt?: number | null; score_gt?: number | null; limit?: number; cursor?: string | null;
};

export function queryDocsSamples(samples: PlanQualitySample[], q: DocsQualityQuery) {
  if (q.cursor) throw new Error('DOCUMENTATION_CURSOR_UNSUPPORTED');
  return {
    samples: samples.filter(s =>
      (!q.plan_id || s.plan_id === q.plan_id) &&
      (!q.session_id || s.session_id === q.session_id) &&
      (q.plan_revision == null || s.plan_revision === q.plan_revision) &&
      (!q.model_configuration_id || s.model_configuration_id === q.model_configuration_id) &&
      (q.from_ms == null || s.last_at_ms >= q.from_ms) &&
      (q.to_ms == null || s.last_at_ms < q.to_ms) &&
      (q.score_lt == null || (s.assessment != null && s.assessment.score < q.score_lt)) &&
      (q.score_gt == null || (s.assessment != null && s.assessment.score > q.score_gt))
    ).slice(0, q.limit ?? 20), next_cursor: null,
  };
}
