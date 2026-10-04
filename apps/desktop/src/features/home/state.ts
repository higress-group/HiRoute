import type {
  HomeAgents,
  HomeCandidate,
  HomeCompute,
  HomeFacetState,
  HomeOperation,
  HomePlans,
  HomeRead,
  HomeReads,
  HomeSubscriptionCheck,
} from './types';

export type HomePrimaryMode = 'loading' | 'first-use' | 'partial' | 'configured-empty' | 'daily';

export type DomainReadSlot<T> = {
  requestId: string | null;
  targetKey: string | null;
  read: HomeRead<T>;
};

export function beginDomainRead<T>(
  current: DomainReadSlot<T>,
  requestId: string,
  targetKey: string,
): DomainReadSlot<T> {
  const previous = current.read.status === 'ready' ? current.read.data : current.read.previous;
  return {
    requestId,
    targetKey,
    read: previous === undefined ? { status: 'loading', requestId } : { status: 'loading', requestId, previous },
  };
}

export function resolveDomainRead<T>(
  current: DomainReadSlot<T>,
  requestId: string,
  targetKey: string,
  data: T,
  revision?: string,
): DomainReadSlot<T> {
  if (current.requestId !== requestId || current.targetKey !== targetKey) return current;
  return { requestId, targetKey, read: { status: 'ready', data, revision } };
}

export function failDomainRead<T>(
  current: DomainReadSlot<T>,
  requestId: string,
  targetKey: string,
  code: string,
  message?: string,
): DomainReadSlot<T> {
  if (current.requestId !== requestId || current.targetKey !== targetKey) return current;
  const previous = current.read.status === 'ready' ? current.read.data : current.read.previous;
  return {
    requestId,
    targetKey,
    read: { status: 'error', code, message, previous, retryable: true },
  };
}

export function visibleData<T>(read: HomeRead<T>): T | undefined {
  return read.status === 'ready' ? read.data : read.previous;
}

function configuredFacet(state: HomeFacetState): boolean {
  return state !== 'unconfigured' && state !== 'unknown' && state !== 'unsupported';
}

function hasAnyConfiguration(compute?: HomeCompute, plans?: HomePlans, agents?: HomeAgents): boolean {
  return Boolean(
    compute?.sources.length ||
    compute?.saveAttempts.length ||
    plans?.plans.length ||
    plans?.drafts.length ||
    agents?.agents.some(agent => configuredFacet(agent.model) || configuredFacet(agent.collaboration)),
  );
}

function hasCompleteRoute(compute?: HomeCompute, plans?: HomePlans, agents?: HomeAgents): boolean {
  const hasSavedSource = Boolean(compute?.sources.length);
  const hasPublishedPlan = Boolean(plans?.plans.some(plan => plan.publication === 'published'));
  const hasConfiguredAgent = Boolean(agents?.agents.some(agent =>
    ['configured', 'verified'].includes(agent.model) || ['configured', 'verified'].includes(agent.collaboration),
  ));
  return hasSavedSource && hasPublishedPlan && hasConfiguredAgent;
}

export function deriveHomePrimaryMode(reads: HomeReads): HomePrimaryMode {
  const compute = visibleData(reads.compute);
  const plans = visibleData(reads.plans);
  const agents = visibleData(reads.agents);
  const activity = visibleData(reads.activity);
  const requiredUnresolved = [reads.compute, reads.plans, reads.agents].some(read =>
    read.status !== 'ready' && read.previous === undefined,
  );

  if (requiredUnresolved) return 'loading';
  const configured = hasAnyConfiguration(compute, plans, agents);
  if (!configured) return 'first-use';
  if (activity && (activity.sessions.length > 0 || activity.tasks.length > 0)) return 'daily';
  if (!hasCompleteRoute(compute, plans, agents)) return 'partial';
  return 'configured-empty';
}

export function acceptOperation(current: HomeOperation | undefined, incoming: HomeOperation): HomeOperation {
  if (!current || current.operationId !== incoming.operationId) return incoming;
  return incoming.sequence >= current.sequence ? incoming : current;
}

export type SubscriptionPresentation = 'checking' | 'verified-not-saved' | 'saving' | 'saved-retained' | 'released' | 'failed' | 'unknown';

export function subscriptionPresentation(check: HomeSubscriptionCheck): SubscriptionPresentation {
  if (check.state === 'checking' || check.state === 'cancelling') return 'checking';
  if (check.state === 'verified') return check.saveOperationId ? 'saving' : 'verified-not-saved';
  if (check.state === 'retained') return check.saveOperationId ? 'saved-retained' : 'unknown';
  if (check.state === 'released') return 'released';
  if (check.state === 'failed') return 'failed';
  if (check.state === 'release_pending') return 'checking';
  return 'unknown';
}

export function isExplicitlyFree(priceKnowledge: HomeCompute['sources'][number]['priceKnowledge']): boolean {
  return priceKnowledge === 'free';
}

export type CandidateFactPresentation = 'needs-credential' | 'needs-approved-check' | 'facts-complete';

export function candidateFactPresentation(candidate: HomeCandidate): CandidateFactPresentation {
  if (candidate.factState === 'pending_credential') return 'needs-credential';
  if (candidate.factState === 'pending_approval') return 'needs-approved-check';
  return 'facts-complete';
}
