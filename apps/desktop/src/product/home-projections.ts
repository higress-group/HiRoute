import type { Agent, AgentSnapshot } from '../agents';
import { agentHasNoModelConnection, agentModelStatus } from '../features/agents/status.ts';
import type {
  HomeActivity,
  HomeAgents,
  HomeCompute,
  HomeFacetState,
  HomeMoney,
  HomeOperation,
  HomePlans,
  HomeService,
  HomeValue,
} from '../features/home';
import type { ManagementSnapshot, ManagedSource } from '../features/models/types';
import type { CacheHitSummary } from '../features/usage-presentation';
import type { Draft, Plan, Selection } from '../plan-editor';

export type DesktopSnapshot = {
  catalog_error: unknown | null;
  service: {
    daemon_role: string;
    recovery_ready: boolean;
    mutation_available: boolean;
    gateway: string;
    revisions?: { target: number; dependencies: Record<string, number> };
  };
  catalog: { plans: Plan[]; drafts: Draft[] };
  trusted_authority: boolean;
  restore_names: { plan_id: string; previous_name: string }[];
  pending: {
    plan_id: string;
    operation_id: string | null;
    idempotency_key: string;
    latest_edit_not_applied?: boolean;
  } | null;
};

export type DesktopOperation = {
  operation_id: string;
  state: string;
  sequence: number;
  cancellable: boolean;
  safe_error_code: string | null;
};

export type ObservationSessionPage = {
  sessions: {
    session_id: string;
    agent_id: string;
    first_request_at_ms: number;
    last_request_at_ms: number;
    request_count: number;
    fallback_request_count: number;
    unknown_model_request_count: number;
    correlation_kind: string;
  }[];
  next_cursor: string | null;
};

type ObservationCoverage = 'complete' | 'partial' | 'unknown';

export type ObservationValueSummary = {
  from_ms: number;
  to_ms: number;
  pending_requests: number;
  provisional_requests: number;
  unknown_traffic_requests: number;
  excluded_requests: number;
  amounts: {
    currency: string;
    valuation_kind: 'usage_estimate' | 'api_equivalent' | string;
    known_sum_micros: number | null;
    coverage: ObservationCoverage;
    missing_contribution_count: number;
  }[];
  archive_boundary_partial: boolean;
  retention_boundary_partial: boolean;
  usage: {
    metric: string;
    known_sum: number | null;
    coverage: ObservationCoverage;
    missing_attempt_count: number;
  }[];
  input_cache_hit: CacheHitSummary;
};

export function projectHomeService(snapshot: DesktopSnapshot): HomeService {
  const gateway = {
    ready: 'ready',
    empty: 'empty',
    no_new_calls: 'no_new_calls',
    unavailable: 'unavailable',
    not_composed: 'unknown',
  }[snapshot.service.gateway] as HomeService['gateway'] | undefined;
  return {
    daemon: snapshot.trusted_authority ? 'running' : 'read_only',
    gateway: gateway ?? 'unknown',
    recoveryReady: snapshot.service.recovery_ready,
  };
}

function selections(plan: Plan): Selection[] {
  const strategy = plan.desired.strategy;
  return [
    ...(strategy.candidates ?? []),
    ...(strategy.economy ?? []),
    ...(strategy.primary ?? []),
  ];
}

export function projectHomePlans(snapshot: DesktopSnapshot): HomePlans {
  return {
    plans: snapshot.catalog.plans.map(plan => ({
      planId: plan.agent_plan_id,
      displayName: plan.desired.display_name,
      publication: plan.head.status === 'enabled'
        ? 'published'
        : plan.head.status === 'disabled' ? 'disabled' : 'unknown',
      bindingIds: [...new Set(selections(plan).map(selection => selection.binding_id))],
    })),
    drafts: snapshot.catalog.drafts.map(draft => ({
      draftId: draft.draft_id,
      planId: draft.plan_id,
      displayName: draft.editor.display_name || draft.draft_id,
    })),
  };
}

function sourceAvailability(source: ManagedSource): HomeCompute['sources'][number]['availability'] {
  if (
    source.state === 'disabled'
    || source.state === 'needs_credential'
    || source.state === 'needs_authorization'
  ) return 'unavailable';
  if (source.state !== 'ready') return 'unknown';
  if (source.authentication.kind === 'none' && source.keys.length === 0) return 'available';
  const states = source.keys.flatMap(key =>
    key.model_statuses.map(status => status.availability),
  );
  if (states.includes('available')) return 'available';
  if (states.includes('cooling_down')) return 'cooling';
  if (states.includes('unavailable') || states.includes('disabled')) return 'unavailable';
  return 'unknown';
}

function targetLabel(source: ManagedSource): string {
  const defaultPort = (source.target.scheme === 'https' && source.target.port === 443)
    || (source.target.scheme === 'http' && source.target.port === 80);
  return `${source.target.scheme}://${source.target.authority}${defaultPort ? '' : `:${source.target.port}`}${source.target.request_path}`;
}

export function projectHomeCompute(snapshot: ManagementSnapshot): HomeCompute {
  return {
    candidates: [],
    saveAttempts: [],
    subscriptionChecks: [],
    sources: snapshot.sources.map(source => ({
      sourceId: source.source_id,
      displayName: source.display_name,
      bindingIds: source.models.map(model => model.binding_id),
      origin: source.provenance === 'registered' ? 'catalog' : source.provenance,
      authentication: source.authentication.kind === 'api_key_header'
        ? 'header'
        : source.authentication.kind,
      destinationLabel: targetLabel(source),
      availability: sourceAvailability(source),
      capabilityKnowledge: 'unknown',
      priceKnowledge: 'unknown',
    })),
  };
}

function facet(state: string | undefined): HomeFacetState {
  if (!state || state === 'not_configured' || state === 'restored') return 'unconfigured';
  if (state === 'configured') return 'configured';
  if (state === 'pending') return 'pending';
  if (state === 'drift' || state === 'needs_attention') return 'degraded';
  if (state === 'unavailable' || state === 'failed') return 'unavailable';
  return 'unknown';
}

function projectAgent(agent: Agent): HomeAgents['agents'][number] {
  const model = agentModelStatus(agent);
  const normalized = agent.agent_id.toLowerCase();
  const brand = normalized.includes('codex')
    ? 'codex'
    : normalized.includes('claude') ? 'claude-code' : normalized.includes('qoder') ? 'qoder' : 'agent';
  return {
    agentId: agent.agent_id,
    contextId: agent.context_id ?? undefined,
    displayName: agent.agent_id === 'agent_codex_default'
      ? 'Codex'
      : agent.agent_id === 'agent_claude_default' ? 'Claude Code'
        : agent.agent_id === 'agent_qoder_default' ? 'Qoder' : agent.agent_id,
    brand,
    model: agentHasNoModelConnection(agent) ? 'unsupported'
      : model ? facet(model.state) : 'unknown',
    collaboration: facet(agent.settings?.collaboration?.state),
    safeIssue: agent.status_error ?? undefined,
  };
}

export function projectHomeAgents(snapshot: AgentSnapshot): HomeAgents {
  return { agents: snapshot.agents.map(projectAgent) };
}

function dateTimeLabel(milliseconds: number): string {
  const date = new Date(milliseconds);
  if (!Number.isFinite(date.valueOf())) return '—';
  return date.toLocaleString('sv-SE', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });
}

export function projectHomeActivity(page: ObservationSessionPage): HomeActivity {
  const agentName = (id: string) => id.toLocaleLowerCase().includes('codex')
    ? 'Codex'
    : id.toLocaleLowerCase().includes('claude')
      ? 'Claude Code'
      : 'Agent';
  return {
    sessions: page.sessions.map(session => ({
      sessionId: session.session_id || null,
      title: '',
      requestCount: session.request_count,
      agentName: session.agent_id ? agentName(session.agent_id) : undefined,
      occurredAtMs: session.last_request_at_ms,
      occurredAtLabel: dateTimeLabel(session.last_request_at_ms),
      modelSwitch: session.fallback_request_count > 0,
    })),
    // Task history has its own paginated read model; Home only needs session activity here.
    tasks: [],
  };
}

function formatMicros(value: number | null): string | null {
  if (value === null) return null;
  return (value / 1_000_000).toFixed(2);
}

function observedUsageValue(summary: ObservationValueSummary, metric: string): number | null {
  return summary.usage.find(item => item.metric === metric)?.known_sum ?? null;
}

export function projectHomeValue(summary: ObservationValueSummary): HomeValue {
  const money = new Map<string, HomeMoney>();
  for (const amount of summary.amounts) {
    const current = money.get(amount.currency) ?? { currency: amount.currency };
    if (amount.valuation_kind === 'usage_estimate') {
      current.usageEstimate = formatMicros(amount.known_sum_micros);
    }
    if (amount.valuation_kind === 'api_equivalent') {
      current.apiEquivalent = formatMicros(amount.known_sum_micros);
    }
    money.set(amount.currency, current);
  }
  const coverage = [
    ...summary.amounts.map(amount => amount.coverage),
    ...summary.usage.map(metric => metric.coverage),
    summary.input_cache_hit.coverage,
  ];
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'local';
  return {
    rangeLabel: `${dateTimeLabel(summary.from_ms)} – ${dateTimeLabel(summary.to_ms)} · ${timeZone}`,
    coverage: summary.archive_boundary_partial || summary.retention_boundary_partial || coverage.includes('partial')
      ? 'partial'
      : coverage.length === 0 || coverage.includes('unknown') ? 'unknown' : 'complete',
    pending: summary.pending_requests,
    provisional: summary.provisional_requests > 0,
    unknownTrafficRequests: summary.unknown_traffic_requests,
    excludedRequests: summary.excluded_requests,
    usage: {
      input: observedUsageValue(summary, 'input'),
      output: observedUsageValue(summary, 'output'),
      cacheRead: observedUsageValue(summary, 'cache_read'),
      cacheWrite: observedUsageValue(summary, 'cache_write'),
      inputCacheHit: summary.input_cache_hit,
    },
    money: [...money.values()],
  };
}

export function projectHomeOperation(
  operation: DesktopOperation | null,
): HomeOperation | undefined {
  if (!operation) return undefined;
  const state = {
    accepted: 'accepted',
    submitted: 'accepted',
    running: 'running',
    succeeded: 'succeeded',
    unchanged: 'unchanged',
    conflict: 'conflict',
    needs_input: 'needs_input',
    rolled_back: 'failed',
    failed: 'failed',
    needs_attention: 'failed',
  }[operation.state] as HomeOperation['state'] | undefined;
  return {
    operationId: operation.operation_id,
    sequence: operation.sequence,
    state: state ?? 'unknown',
    label: operation.safe_error_code ?? undefined,
  };
}
