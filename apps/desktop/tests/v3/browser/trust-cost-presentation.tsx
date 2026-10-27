import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { PlanQuality, type PlanQualityModel } from '../../../src/features/PlanQuality';
import { RoutingTimeline } from '../../../src/features/RoutingTimeline';
import { PlanEditor } from '../../../src/plan-editor';
import { PresentationRoot } from '../../../src/ui';
import { dailyPlan, fixturePlanAuthoring, mockProductInvoke } from './product-fixtures';
import '../../../src/occami/styles.css';

const counts = { executed: 5, complete: 2, meets_floor: 2, partial: 1, awaiting_assessment: 1, unscored: 1, sessions: 2 };
const comparison = { currency: 'USD', coverage: 'unknown', request_count: 1, compared_request_count: 0,
  missing_request_count: 1, excluded_request_count: 0, routed_estimate_micros: 2764,
  reference_estimate_micros: null, same_usage_bounds: { reference_min_micros: 92400, reference_max_micros: 115200,
    difference_min_micros: -112436, difference_max_micros: -89636 },
  compared_routed_subtotal_micros: null, compared_reference_subtotal_micros: null,
  retention_boundary_partial: false, arithmetic_overflow: false };
const report = { plan_id: 'plan/fixture', active_revision: 8, revision: 'active', from_ms: 0, to_ms: 2000000000000,
  rows: [{ branch_id: 'writing', group: 'regular', model_configuration_id: 'model/regular', reasoning_profile_id: null,
    attribution: 'single', executor: null, identity_stability: 'pinned', counts, competence_rate: 1, scoring_coverage: .4,
    execution_share: 1, protections: [], availability_relays: 0, duration_p50_ms: null, duration_p90_ms: null, output_tokens_per_second: null }],
  branches: [{ branch_id: 'writing', threshold_millis: 800, full_history: counts, upward_cohort: counts,
    upward_from_ms: Date.UTC(2026, 9, 14), suggestion: { kind: 'keep', missing: ['complete_scores'] },
    replay: { included: 5, excluded_by_reason: {}, recorded_share_at_current: 1, estimate_only: true, thresholds: [] } }],
  sessions: [], session_total: 2, executors: [], announcements: [], cost: { kind: 'available', difference_micros: null, comparison } };
const models: PlanQualityModel[] = [{ plan_revision: 8, branch_id: 'writing', branch_name: 'A long category name for evidence and model presentation',
  group: 'regular', candidate_index: 0, model_configuration_id: 'model/regular', display_name: 'A very long configured model name for wrapping verification', reasoning_profile_id: null }];

const fixture = { language: 'en' as 'en' | 'zh', theme: 'light' as 'light' | 'dark', report: structuredClone(report),
  models: structuredClone(models), scored: 4, unrated: 1, generation: 0, refresh: () => {}, reads: [] as any[],
  view: 'quality' as 'quality' | 'timeline' | 'editor', timelineMicros: 1 as number | null, timelineCurrency: 'USD' as string | null,
  editorPlan: structuredClone(dailyPlan), editorCandidates: [] as any[] };
Object.assign(window, { trustCostFixture: fixture });
mockIPC(async (command, payload: any) => {
  if (command === 'plan_editor_options') return { ...mockProductInvoke(command, payload, 'ready'),
    authoring: fixturePlanAuthoring(fixture.editorPlan), candidates: fixture.editorCandidates };
  if (command !== 'observation_read') return mockProductInvoke(command, payload, 'ready');
  const { view, query } = payload.request.intent;
  fixture.reads.push({ view, query });
  if (view === 'plan_trust') {
    const value = structuredClone(fixture.report);
    value.revision = query.revision;
    if (query.revision === 'all' || (typeof query.revision === 'number' && query.revision !== value.active_revision)) {
      value.branches.forEach(branch => { branch.suggestion = { kind: 'keep', missing: ['active_revision_only'] }; });
    }
    return value;
  }
  if (view === 'plan_quality') return { samples: [], summary: { models: [], scored_stage_count: fixture.scored,
    unrated_stage_count: fixture.unrated, session_count: 2, available_revisions: [7, 8] }, next_cursor: null };
  if (view === 'routing_timeline') {
    const cost = { model_configuration_id: 'model/regular', currency: fixture.timelineCurrency, valuation_kind: 'api_equivalent',
      known_micros: fixture.timelineMicros, unknown_contributions: 1 };
    const request = { request_id: 'request/fixture', started_at_ms: 1, duration_ms: 1, native_model: 'Fixture model',
      input_tokens: 1, output_tokens: 1, cache_read_tokens: null, cache_read_share: null, costs: [cost], outcome: 'accepted' };
    return { watermark: 'fixture', complete: true, turns: query.after_watermark ? [] : [{ turn_id: 'request/fixture', agent_turn_id: 'turn/fixture',
      segment_id: 'stage/fixture', started_at_ms: 1, finished_at_ms: 2, boundary: 'new_user_message', plan_id: 'plan/fixture', plan_revision: 8,
      branch_id: 'writing', model_configuration_id: 'model/regular', reasoning_profile_id: null, group: 'regular',
      selection: null, tool_continuations: 0, relays: [], assessment_state: 'unscored', assessment: null, competence_floor_millis: null,
      protected_turn_ids: [], status: 'completed', requests: [request], cost_by_model: [cost], evidence_partial: false, content_digest: 'fixture' }] };
  }
  if (view === 'timeline') return { requests: [] };
  throw new Error(`Unexpected observation view ${view}`);
});

function Harness() {
  const [generation, setGeneration] = useState(0);
  fixture.refresh = () => { fixture.generation += 1; setGeneration(fixture.generation); };
  return <PresentationRoot language={fixture.language} theme={fixture.theme} textScale={1}>
    <div className="app-window"><header className="titlebar" /><aside className="sidebar" /><main className="main">
      <div className="detail-pane" style={{ height: '100%' }}><section className="editor-section quality-performance-panel" data-fixture-generation={generation}>
        {fixture.view === 'quality' ? <PlanQuality key={generation} planId="plan/fixture" planRevision={8} currentModels={fixture.models} language={fixture.language} onThreshold={() => {}} onDivision={() => {}} />
          : fixture.view === 'editor' ? <PlanEditor key={generation} plan={fixture.editorPlan} initialView="performance" language={fixture.language} onDone={async () => {}} onClose={() => {}} />
            : <RoutingTimeline key={generation} session="session/fixture" language={fixture.language} active displayed refreshVersion={0} requestId={null} contentCleared onSelectRequest={() => {}} onOpenPerformance={() => {}} />}
      </section></div>
    </main></div>
  </PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
