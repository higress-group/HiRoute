import type { BrandKind, Language, UiIconName } from '../../ui';
import type { CacheHitSummary } from '../usage-presentation';

/**
 * UI-only read state. Domain adapters project their safe query results into this shape.
 * It is not a backend DTO and must never carry credentials, capabilities, or trusted facts.
 */
export type HomeRead<T> =
  | { status: 'loading'; previous?: T; requestId?: string }
  | { status: 'ready'; data: T; revision?: string }
  | { status: 'error'; code: string; message?: string; previous?: T; retryable?: boolean };

export type HomeService = {
  daemon: 'running' | 'starting' | 'unavailable' | 'read_only' | 'unknown';
  gateway: 'ready' | 'empty' | 'no_new_calls' | 'unavailable' | 'unknown';
  recoveryReady: boolean | null;
};

export type HomeSaveDisposition = 'saved' | 'unchanged' | 'needs_input' | 'conflict' | 'failed' | 'pending';

export type HomeSaveAttempt = {
  attemptId: string;
  displayName: string;
  disposition: HomeSaveDisposition;
  sourceId?: string;
  bindingIds?: string[];
  savedRevision?: string;
  operationId?: string;
  managementState?: 'needs_credential' | 'needs_authorization' | 'ready' | 'disabled';
  safeMessage?: string;
};

export type HomeCandidate = {
  candidateRef: string;
  /** Opaque decimal projections preserve the full backend u64 values in JavaScript. */
  candidateRevision: string;
  editRevision: string;
  checkId: string;
  displayName: string;
  inputState: 'not_required' | 'provided' | 'missing' | 'unavailable';
  factState: 'pending_credential' | 'pending_approval' | 'complete';
  selectableModelCount: number;
  issues: string[];
};

export type HomeSource = {
  sourceId: string;
  displayName: string;
  bindingIds: string[];
  origin: 'catalog' | 'user_configured' | 'connector_owned';
  evidenceRevision?: string;
  authentication: 'bearer' | 'header' | 'none' | 'unknown';
  destinationLabel?: string;
  availability?: 'available' | 'cooling' | 'unavailable' | 'unknown';
  capabilityKnowledge: 'known' | 'partial' | 'unknown';
  priceKnowledge: 'known' | 'partial' | 'unknown' | 'free';
};

export type HomeSubscriptionCheck = {
  checkId: string;
  state: 'checking' | 'verified' | 'failed' | 'cancelling' | 'release_pending' | 'released' | 'retained' | 'unknown';
  checkedCandidateRevision?: string;
  validationRef?: string;
  saveOperationId?: string;
  safeMessage?: string;
};

export type HomeCompute = {
  candidates: HomeCandidate[];
  sources: HomeSource[];
  saveAttempts: HomeSaveAttempt[];
  subscriptionChecks: HomeSubscriptionCheck[];
};

export type HomePlan = {
  planId: string;
  displayName: string;
  publication: 'draft' | 'published' | 'disabled' | 'unknown';
  bindingIds: string[];
};

export type HomeDraft = {
  draftId: string;
  planId?: string;
  displayName: string;
};

export type HomePlans = {
  plans: HomePlan[];
  drafts: HomeDraft[];
};

export type HomeFacetState = 'unconfigured' | 'configured' | 'verified' | 'pending' | 'degraded' | 'unavailable' | 'unsupported' | 'unknown';

export type HomeAgent = {
  agentId: string;
  contextId?: string;
  displayName: string;
  brand: BrandKind;
  model: HomeFacetState;
  collaboration: HomeFacetState;
  safeIssue?: string;
};

export type HomeAgents = {
  agents: HomeAgent[];
};

export type HomeSession = {
  requestCount?: number;
  sessionId: string | null;
  requestId?: string;
  title: string;
  agentName?: string;
  modelLabel?: string;
  occurredAtMs?: number;
  occurredAtLabel: string;
  modelSwitch: boolean | null;
};

export type HomeTask = {
  taskId: string | null;
  runId?: string;
  agentId?: 'codex' | 'claude-code';
  sessionId?: string;
  title: string;
  state: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'unknown';
  agentName?: string;
};

export type HomeActivity = {
  sessions: HomeSession[];
  tasks: HomeTask[];
};

export type HomeMoney = {
  currency: string;
  usageEstimate?: string | null;
  apiEquivalent?: string | null;
  estimatedSavings?: string | null;
};

export type HomeValue = {
  rangeLabel: string;
  coverage: 'complete' | 'partial' | 'unknown';
  pending: number;
  provisional: boolean;
  unknownTrafficRequests: number;
  excludedRequests: number;
  usage: {
    input: number | null;
    output: number | null;
    cacheRead: number | null;
    cacheWrite: number | null;
    inputCacheHit: CacheHitSummary;
  };
  requests?: number;
  modelSwitches?: number;
  money: HomeMoney[];
};

export type HomeOperation = {
  operationId: string;
  sequence: number;
  state: 'accepted' | 'running' | 'succeeded' | 'unchanged' | 'needs_input' | 'conflict' | 'failed' | 'unknown';
  label?: string;
};

export type HomeReads = {
  service: HomeRead<HomeService>;
  compute: HomeRead<HomeCompute>;
  plans: HomeRead<HomePlans>;
  agents: HomeRead<HomeAgents>;
  activity: HomeRead<HomeActivity>;
  value: HomeRead<HomeValue>;
};

export type HomeAction =
  | { kind: 'view-models'; returnTo: 'home' }
  | { kind: 'view-routing'; returnTo: 'home' }
  | { kind: 'view-agents'; returnTo: 'home' }
  | { kind: 'connect-models'; returnTo: 'home' }
  | { kind: 'create-plan'; returnTo: 'home' }
  | { kind: 'configure-agent'; returnTo: 'home'; agentId?: string; contextId?: string; facet?: 'model' | 'collaboration' }
  | { kind: 'open-source'; returnTo: 'home'; sourceId: string; bindingId?: string }
  | { kind: 'open-plan'; returnTo: 'home'; planId: string }
  | { kind: 'open-draft'; returnTo: 'home'; draftId: string; planId?: string }
  | { kind: 'open-session'; returnTo: 'home'; sessionId: string; requestId?: string }
  | { kind: 'open-task'; returnTo: 'home'; taskId: string; runId?: string; sessionId?: string; agentId?: 'codex' | 'claude-code' }
  | { kind: 'inspect-operation'; returnTo: 'home'; operationId: string }
  | { kind: 'retry-read'; domain: keyof HomeReads }
  | { kind: 'view-all-sessions'; returnTo: 'home' }
  | { kind: 'view-all-tasks'; returnTo: 'home' };

export type HomeNavigationItem = {
  id: string;
  label: string;
  icon: UiIconName;
};

export type HomeProps = {
  language: Language;
  reads: HomeReads;
  operation?: HomeOperation;
  onAction: (action: HomeAction) => void;
};
