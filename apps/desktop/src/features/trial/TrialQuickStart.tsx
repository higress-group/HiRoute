import { safeDiagnosticCode } from '../../error-code';
import { randomId } from '../../random-id.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog, Disclosure, ProductPage, UiIcon } from '../../ui';
import { requestEditorReplacement } from '../../ui/discard-guard';
import { type Editor, type Plan, type Selection } from '../../plan-editor';
import type { DesktopOperation, DesktopSnapshot } from '../../product/home-projections';
import type { TrialCatalogSnapshot, TrialManagementSnapshot, TrialMember, TrialRetainedApiConnection, TrialSource, TrialSetupSnapshot } from './types';
import type { ComputeSaveResult, OperationReference } from '../model-connections/types';
import type { AvailableModel, ModelAvailabilityResult } from '../model-feed/types';
import { agentDisplayName } from '../agents/ecosystems';
import { DecisionSelector } from '../decision-services/DecisionSelector';
import { DecisionServicesPage } from '../decision-services/DecisionServicesPage';
import { emptyService, isCustomExtension, type Classifier, type DecisionService } from '../decision-services/types';
import type { DecisionIntent, OpenDecisionConnection } from '../decision-services/presentation';
import { TrustLevelControl } from '../trust/TrustLevelControl';
import { agentEcosystem } from '../agents/ecosystems';
import { TrialConnectModel, type TrialConnectionState } from './TrialConnectModel';
import { TrialAgentChoice, TrialAgentMapping } from './TrialAgentMapping';
import { TrialReview, type TrialReviewHandle } from './TrialReview';
import { createTrialAlias, currentPrimary, defaultTrialName, initialTrialMapping, trialCriteria, trialSelection, type TrialMapping } from './model';
import { FeedDisclosure } from '../model-feed/FeedDisclosure';
import { factTags, publisherName, rollingIdentity, shortDate } from '../model-feed/presentation';
import './trial.css';
import { TrialProtectionExplanation } from './TrialProtectionExplanation';
import { agentEditorSeed } from '../agents/editor-state';
import { useTrialAgents } from './useTrialAgents';
import { ModelManagementPage } from '../../product/ModelManagementPage';
import { modelAnnouncements } from '../model-feed/announcements';
import { ReasoningDialog } from '../../ui/ReasoningDialog';
import { trialReadiness, type TrialBlocker } from './readiness';
import { candidateUnavailableMessage } from '../../plan-candidate-availability';
import type { Options } from '../../plan-editor';
import { TrialConnectedModelSelect, type ConnectedTrialModel } from './TrialConnectedModelSelect';
import { TrialRequiredMark } from './TrialRequiredMark';

const ignoreRepresentedOperations = (_ids: string[]) => {};

function retainedApiIndex(management?: TrialManagementSnapshot) {
  const index = new Map<string, TrialRetainedApiConnection | null>();
  if (!management) return index;
  const sourceIds = new Set<string>(), bindingIds = new Set<string>();
  for (const source of management.sources) {
    sourceIds.add(source.source_id);
    for (const member of source.models) bindingIds.add(member.binding_id);
  }
  for (const record of management.retained_api_connections) {
    if (index.has(record.binding_id)) { index.set(record.binding_id, null); continue; }
    const valid = !!record.source_id && !!record.display_name && Number.isSafeInteger(record.source_revision) && record.source_revision > 0 && Number.isSafeInteger(record.binding_revision) && record.binding_revision > 0 && !sourceIds.has(record.source_id) && !bindingIds.has(record.binding_id);
    index.set(record.binding_id, valid ? record : null);
  }
  return index;
}

function retainedApiFor(current: TrialCatalogSnapshot | null | undefined, candidate: Options['candidates'][number] | undefined, index?: Map<string, TrialRetainedApiConnection | null>) {
  if (!current || !candidate?.routable) return undefined;
  const record = (index ?? retainedApiIndex(current.management)).get(candidate.binding_id);
  return record?.model_configuration_id === candidate.model_configuration_id ? record : undefined;
}

async function readTrialSetup(): Promise<TrialSetupSnapshot> {
  for (let attempt = 0; ; attempt += 1) {
    try { return await invoke<TrialSetupSnapshot>('trial_setup_snapshot'); }
    catch (error) {
      const code = safeDiagnosticCode(error, 'UNKNOWN');
      if (attempt >= 1 || !['READ_SNAPSHOT_CHANGED', 'CHANGE_PREVIEW_STALE', 'application.error.change_preview_stale'].includes(code)) throw error;
    }
  }
}

export function TrialQuickStart({ initialModelId, initialBindingId, language, mutable, snapshot, operation, onOperation, onRefresh, onClose, onOpenAgent, onOpenPerformance, onOpenSessions, onOpenModels, active = true, onRepresentedOperations = ignoreRepresentedOperations }: {
  initialModelId?: string; initialBindingId?: string; language: 'zh' | 'en'; mutable: boolean; snapshot: DesktopSnapshot | null; operation: DesktopOperation | null;
  onOpenPerformance?(plan: Plan): void; onOpenSessions?(sessionId?: string): void;
  onOpenModels?(): void; active?: boolean;
  onRepresentedOperations?(ids: string[]): void;
  onOperation(operation: DesktopOperation | null): void; onRefresh(): Promise<void>; onClose(): void; onOpenAgent(id: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [data, setData] = useState<TrialCatalogSnapshot | null>(null);
  const retainedApi = useMemo(() => retainedApiIndex(data?.management), [data?.management]);
  const [trialId, setTrialId] = useState(initialModelId ?? '');
  const [primaryId, setPrimaryId] = useState('');
  const [primarySelection, setPrimarySelection] = useState<Selection>();
  const [trialChosen, setTrialChosen] = useState<Selection>();
  const [reasoningRole, setReasoningRole] = useState<'trial' | 'primary' | null>(null);
  const [trialBinding, setTrialBinding] = useState(''), [primaryBinding, setPrimaryBinding] = useState('');
  const [mapping, setMapping] = useState<TrialMapping>({ agentId: '', presets: ['opus'] });
  const agentRead = useTrialAgents(mapping.agentId);
  const [classifier, setClassifier] = useState<Classifier>(() => ({ kind: 'decision_service', service: emptyService() }));
  const custom = isCustomExtension(classifier);
  const [services, setServices] = useState<DecisionService[]>([]);
  const [servicesLoading, setServicesLoading] = useState(true), [servicesError, setServicesError] = useState(false);
  const [connect, setConnect] = useState<AvailableModel | null>(null), [subscriptions, setSubscriptions] = useState(false);
  const [connectionHidden, setConnectionHidden] = useState(false);
  const [connectionState, setConnectionState] = useState<TrialConnectionState>('editing');
  const [pickerRole, setPickerRole] = useState<'trial' | 'primary' | null>(null);
  const [pickerBusy, setPickerBusy] = useState(false), [pickerError, setPickerError] = useState('');
  const [manageSource, setManageSource] = useState<string | null>(null);
  const [appendIntent, setAppendIntent] = useState(false);
  const reuseConnectionOrigin = useRef(false);
  const [unlinked, setUnlinked] = useState('');
  const [decision, setDecision] = useState<DecisionIntent | null>(null);
  const decisionSelect = useRef<((service: DecisionService) => void) | null>(null);
  const [savedConnections, setSavedConnections] = useState<string[]>([]);
  const [savedReceipt, setSavedReceipt] = useState(false);
  const [persistencePending, setPersistencePending] = useState(false);
  const [review, setReview] = useState<Editor | null>(null), [published, setPublished] = useState<Plan | null>(null);
  const reviewHandle = useRef<TrialReviewHandle>(null);
  const [reviewDraft, setReviewDraft] = useState<Editor | null>(null);
  const [threshold, setThreshold] = useState(900);
  const [attempted, setAttempted] = useState(false);
  const [catalogLoading, setLoading] = useState(true), [error, setError] = useState('');
  const [recommendationState, setRecommendationState] = useState<'loading' | 'ready' | 'error'>('loading');
  const loading = catalogLoading || agentRead.loading || servicesLoading;
  const [name, setName] = useState<string | null>(null);
  const [choosingPrimary, setChoosingPrimary] = useState(false), [allPrimary, setAllPrimary] = useState(false);
  const [mapped, setMapped] = useState(false);
  const initialized = useRef(false);
  const latestData = useRef<TrialCatalogSnapshot | null>(null);
  const descriptive = useRef<ModelAvailabilityResult>({ featured: [], models: [] });
  const inFlight = useRef<Promise<TrialCatalogSnapshot | undefined> | null>(null);
  const recommendationFlight = useRef<Promise<void> | null>(null), recommendationGeneration = useRef(0);
  const replacingConnection = useRef(0);
  const live = useRef(true), selectionEpoch = useRef(0), pickerEpoch = useRef(0), connectionEpoch = useRef(0), nativeInitialized = useRef(false), pickerBusyRef = useRef(false);
  const serviceGeneration = useRef(0), serviceInitialized = useRef(false), serviceTouched = useRef(false);
  const [nonce] = useState(() => randomId().replaceAll('-', '').slice(0, 8));
  async function refreshRecommendations() {
    if (recommendationFlight.current) return recommendationFlight.current;
    const generation = ++recommendationGeneration.current;
    setRecommendationState('loading');
    const reading = (async () => {
      try {
        const availability = await invoke<ModelAvailabilityResult>('model_availability');
        if (!live.current || generation !== recommendationGeneration.current) return;
        descriptive.current = availability;
        if (latestData.current) {
          latestData.current = { ...latestData.current, availability };
          setData(latestData.current);
        }
        setRecommendationState('ready');
      } catch {
        if (live.current && generation === recommendationGeneration.current) setRecommendationState('error');
      }
    })();
    recommendationFlight.current = reading;
    try { await reading; } finally { if (recommendationFlight.current === reading) recommendationFlight.current = null; }
  }
  async function refreshServices() {
    const generation = ++serviceGeneration.current;
    setServicesLoading(true); setServicesError(false);
    try {
      const result = await invoke<{services: DecisionService[]}>('decision_services');
      if (!live.current || generation !== serviceGeneration.current) return;
      setServices(result.services);
      if (!serviceInitialized.current) {
        serviceInitialized.current = true;
        if (!serviceTouched.current && result.services.length === 1) setClassifier({kind:'decision_service', service:result.services[0]});
      }
    } catch { if (live.current && generation === serviceGeneration.current) setServicesError(true); }
    finally { if (live.current && generation === serviceGeneration.current) setServicesLoading(false); }
  }
  async function refresh(rediscover = false, requireFresh = false, receipt?: Pick<ComputeSaveResult, 'source_id' | 'bindings'> & { skipCatalogForNonRegistered?: boolean }): Promise<TrialCatalogSnapshot | undefined> {
    if (rediscover) { void agentRead.refresh(true); void refreshServices(); if (recommendationState === 'error') void refreshRecommendations(); }
    if (inFlight.current) {
      const previous = await inFlight.current;
      if (!requireFresh || !live.current) return previous;
      return refresh(false, true, receipt);
    }
    const reading = (async () => {
    setLoading(true); setError('');
    try {
      const { options, management } = await readTrialSetup();
      const availability = descriptive.current;
      if (!live.current) return;
      latestData.current = { availability, options, management };
      setData(latestData.current);
      if (!initialized.current) {
        initialized.current = true;
        const requested = options.candidates.find(c => c.binding_id === initialBindingId && c.routable);
        if (requested) { setTrialId(requested.model_configuration_id); setTrialBinding(requested.binding_id); }
      }
      return { availability, options, management };
    } catch { if (live.current) setError(t('试用所需信息暂时无法读取。请重试，已完成的配置仍保留。', 'Trial setup information could not be read. Retry; completed configuration is retained.')); }
    finally { if (live.current) setLoading(false); }
    })();
    inFlight.current = reading;
    try { return await reading; } finally { if (inFlight.current === reading) inFlight.current = null; }
  }
  useEffect(() => {
    live.current = true;
    // Recommendations are descriptive. They neither block saved selection nor prove admission.
    void refreshRecommendations();
    void refresh(); void refreshServices();
    return () => { live.current = false; ++selectionEpoch.current; ++pickerEpoch.current; ++serviceGeneration.current; ++recommendationGeneration.current; recommendationFlight.current = null; };
  }, []);
  useEffect(() => {
    if (active && initialized.current) void refresh(false, true);
  }, [active]);
  const mappingInitialized = useRef(false);
  const primaryOrigin = useRef<{ kind: 'unset' | 'agent' | 'manual'; agentId: string }>({ kind: 'unset', agentId: '' });
  useEffect(() => {
    const agents = agentRead.snapshot;
    if (!data || !agentRead.ready || !agents) return;
    // Choosing Primary while details load must not suppress the default Agent;
    // choosing an Agent explicitly must not be undone by a late default read.
    if (!mappingInitialized.current) {
      mappingInitialized.current = true;
      setMapping(value => value.agentId ? value : initialTrialMapping(agents));
    }
    const agentId = mapping.agentId || agents.selected?.agent_id || '';
    if (primaryOrigin.current.kind === 'manual' && !primaryOrigin.current.agentId) primaryOrigin.current.agentId = agentId;
    if (primaryOrigin.current.kind !== 'unset' || !agentId || agents.selected?.agent_id !== agentId) return;
    const current = currentPrimary(agents.selected ?? undefined, agents.plans.plans);
    const candidate = data.options.candidates.find(c => c.binding_id === current?.binding_id && c.routable);
    if (candidate && candidate.model_configuration_id !== trialId) {
      primaryOrigin.current = { kind: 'agent', agentId };
      setPrimaryId(candidate.model_configuration_id); setPrimaryBinding(candidate.binding_id); setPrimarySelection(current);
    }
  }, [data, agentRead.ready, agentRead.snapshot, mapping.agentId, trialId]);
  const openDecision: OpenDecisionConnection = (kind, select) => { decisionSelect.current = select; setDecision({ key: randomId(), kind }); };
  const operationRef = (value: OperationReference) => onOperation({ ...value, safe_error_code: null });
  type TrialModel = Pick<AvailableModel, 'model_configuration_id' | 'display_name' | 'identity_stability' | 'availability'> & { catalog?: AvailableModel };
  const models: TrialModel[] = (data?.availability.models ?? []).filter(m => m.origin.kind !== 'published').map(m => ({ ...m, catalog: m }));
  for (const candidate of data?.options.candidates ?? []) {
    if (candidate.routable && !models.some(m => m.model_configuration_id === candidate.model_configuration_id)) models.push({ model_configuration_id: candidate.model_configuration_id, display_name: candidate.display_name, identity_stability: 'unknown', availability: { kind: 'executable' } });
  }
  const trial = models.find(m => m.model_configuration_id === trialId), primary = models.find(m => m.model_configuration_id === primaryId);
  function candidates(id: string) { return data?.options.candidates.filter(c => c.model_configuration_id === id && c.routable) ?? []; }
  const trialCandidates = candidates(trialId), primaryCandidates = candidates(primaryId);
  const soleTrial = trialCandidates.length === 1 ? trialCandidates[0] : undefined;
  const soleTrialMember = data?.management.sources.flatMap(source => source.models).find(member => member.binding_id === soleTrial?.binding_id);
  const soleTrialCatalogMatch = !!soleTrial && ((soleTrialMember?.catalog_configuration_id === trialId && soleTrialMember.catalog_admitted) || !!retainedApiFor(data, soleTrial, retainedApi));
  const selectedTrial = trialBinding ? trialCandidates.find(c => c.binding_id === trialBinding) : soleTrialCatalogMatch ? soleTrial : undefined;
  const selectedPrimary = primaryBinding ? primaryCandidates.find(c => c.binding_id === primaryBinding) : primaryCandidates.length === 1 ? primaryCandidates[0] : undefined;
  const actualTrialBinding = selectedTrial?.binding_id ?? trialBinding;
  const actualTrialSource = data?.management.sources.find(source => source.models.some(member => member.binding_id === actualTrialBinding));
  const actualTrialMember = actualTrialSource?.models.find(member => member.binding_id === actualTrialBinding);
  const unlinkedTrial = !!actualTrialBinding && (unlinked === actualTrialBinding || (!!actualTrialMember && (!actualTrialMember.catalog_admitted || actualTrialMember.catalog_configuration_id !== trialId)));
  const explicitUnlinkedTrial = unlinkedTrial && (actualTrialSource?.provenance !== 'registered' || !actualTrialMember?.catalog_configuration_id);
  const trialName = unlinkedTrial ? actualTrialMember?.display_name ?? selectedTrial?.display_name ?? trial?.display_name : trial?.display_name;
  const trialReasoning = selectedTrial && trialSelection(selectedTrial, trialChosen);
  const primaryReasoning = selectedPrimary && trialSelection(selectedPrimary, primarySelection);
  useEffect(() => {
    if (!trialChosen && trialReasoning?.state === 'selected') setTrialChosen(trialReasoning.selection);
  }, [trialChosen, trialReasoning?.state, selectedTrial?.binding_id]);
  useEffect(() => {
    if (!primarySelection && primaryReasoning?.state === 'selected') setPrimarySelection(primaryReasoning.selection);
  }, [primarySelection, primaryReasoning?.state, selectedPrimary?.binding_id]);
  // The inline connection form owns protected input; its guard confirms, then releases and closes it.
  const releaseConnection = async () => {
    if (!connect) return true;
    replacingConnection.current++;
    try { return await requestEditorReplacement('models'); }
    finally { replacingConnection.current--; }
  };
  const backToSetup = (editor: Editor) => {
    setReviewDraft(editor);
    setThreshold(editor.smart.judgment.degree.simple_threshold_millis);
    setName(editor.display_name); setClassifier(editor.smart.classifier);
    const regular = editor.smart.economy[0], primary = editor.smart.primary[0];
    if (regular) {
      const candidate = data?.options.candidates.find(value => value.binding_id === regular.binding_id);
      const member = data?.management.sources.flatMap(source => source.models).find(value => value.binding_id === regular.binding_id);
      setTrialId(candidate?.model_configuration_id ?? ''); setTrialBinding(regular.binding_id); setTrialChosen(regular);
      setUnlinked(candidate && member?.catalog_admitted && member.catalog_configuration_id === candidate.model_configuration_id ? '' : regular.binding_id);
    } else { setTrialId(''); setTrialBinding(''); setTrialChosen(undefined); setUnlinked(''); }
    if (primary) {
      const candidate = data?.options.candidates.find(value => value.binding_id === primary.binding_id);
      primaryOrigin.current = {kind:'manual', agentId:mapping.agentId};
      setPrimaryId(candidate?.model_configuration_id ?? ''); setPrimaryBinding(primary.binding_id); setPrimarySelection(primary);
    } else { primaryOrigin.current = {kind:'manual', agentId:mapping.agentId}; setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); }
    setReview(null);
  };
  const leave = () => void (async () => {
    const current = review ? reviewHandle.current?.currentEditor() : undefined;
    if (current) { backToSetup(current); return; }
    if (connect && (connectionState === 'saving' || connectionState === 'uncertain')) { setConnectionHidden(false); return; }
    if (persistencePending || (await releaseConnection() && await requestEditorReplacement('routing'))) onClose();
  })();
  const defaultName = trial && primary ? defaultTrialName(trialName ?? trial.display_name, primary.display_name, language) : '';
  const criteriaHelp = t('评价已完成阶段的核心结果。用户明确表示不满意可以作为结果未达到要求的证据，不必先诊断技术问题；区分局部偏好、新要求、无关反馈和操纵评分的指令。', trialCriteria);
  const alias = trial ? createTrialAlias(trialName ?? trial.display_name, nonce) : '';
  const announcements = data ? modelAnnouncements(data.availability).entries : [];
  const featured = announcements.map(entry => entry.model_configuration_id);
  const cards = [...new Set([...(initialModelId ? [initialModelId] : []), ...featured, ...(trialId ? [trialId] : [])])].flatMap(id => models.filter(m => m.catalog && m.model_configuration_id === id)).slice(0, 6);
  const primaryOptions = models.filter(m => m.model_configuration_id !== trialId && candidates(m.model_configuration_id).length > 0);
  const source = (binding?: string) => retainedApiFor(data, data?.options.candidates.find(c => c.binding_id === binding), retainedApi)?.display_name ?? data?.management.sources.find(s => s.models.some(m => m.binding_id === binding))?.display_name;
  const bindingInfo = (binding?: string) => {
    const owner = data?.management.sources.find(s => s.models.some(m => m.binding_id === binding));
    return owner && { owner, member: owner.models.find(m => m.binding_id === binding)! };
  };
  const sourceKind = (binding?: string) => retainedApiFor(data, data?.options.candidates.find(c => c.binding_id === binding), retainedApi) ? 'API' : bindingInfo(binding)?.owner.access_kind === 'subscription' ? t('订阅', 'Subscription') : bindingInfo(binding)?.owner.access_kind === 'api' ? 'API' : t('类型未知', 'Unknown type');
  const exactCatalogBinding = (modelId: string, bindingId: string) => !!retainedApiFor(data, data?.options.candidates.find(c => c.binding_id === bindingId && c.model_configuration_id === modelId), retainedApi) || (bindingInfo(bindingId)?.member.catalog_configuration_id === modelId && !!bindingInfo(bindingId)?.member.catalog_admitted);
  const sourceLabel = (model: TrialModel) => {
    const connections = candidates(model.model_configuration_id);
    if (connections.length > 1) return t('多个接入', 'Multiple connections');
    if (connections.length === 1) {
      const label = source(connections[0].binding_id) ?? t('来源未确认', 'Source unconfirmed');
      return model.catalog && !(explicitUnlinkedTrial && model.model_configuration_id === trialId) && !exactCatalogBinding(model.model_configuration_id, connections[0].binding_id) ? `${label} · ${t('目录匹配未确认', 'Catalog match unconfirmed')}` : label;
    }
    return [model.catalog?.facts ? publisherName(model.catalog.facts.publisher_id, language) : t('来源未确认', 'Source unconfirmed'), t('未接入', 'Not connected')].join(' · ');
  };
  const mappedAgent = agentRead.snapshot?.selected;
  const bindingUsable = (owner: TrialSource, member: TrialMember, candidate: typeof selectedTrial, current = data) => {
    if (!current || !candidate || !candidate.routable || owner.state !== 'ready' || member.availability !== 'available') return false;
    if (owner.provenance === 'registered' && member.catalog_configuration_id) {
      return candidate.model_configuration_id === member.catalog_configuration_id && member.catalog_admitted;
    }
    return true;
  };
  const usable = (candidate: typeof selectedTrial) => {
    if (!candidate || !data) return false;
    return !!retainedApiFor(data, candidate, retainedApi) || data.management.sources.some(s => s.models.some(m => m.binding_id === candidate.binding_id && bindingUsable(s, m, candidate)));
  };
  useEffect(() => {
    if (!trialBinding && selectedTrial && usable(selectedTrial)) {
      setTrialBinding(selectedTrial.binding_id);
      if (actualTrialMember && (!actualTrialMember.catalog_admitted || actualTrialMember.catalog_configuration_id !== selectedTrial.model_configuration_id)) setUnlinked(selectedTrial.binding_id);
    }
  }, [data, trialBinding, selectedTrial?.binding_id]);
  useEffect(() => {
    if (!primaryBinding && selectedPrimary && usable(selectedPrimary)) setPrimaryBinding(selectedPrimary.binding_id);
  }, [data, primaryBinding, selectedPrimary?.binding_id]);
  const unavailableReason = (owner: TrialSource, member: TrialMember) => {
    const reason = data?.options.unavailable_candidates?.find(candidate => candidate.binding_id === member.binding_id)?.reason;
    if (reason) return candidateUnavailableMessage(reason, language);
    if (owner.state === 'disabled' || member.availability === 'disabled') return t('此接入或模型已停用。', 'This connection or model is disabled.');
    if (owner.state === 'needs_authorization' || member.availability === 'needs_credentials') return t('需要登录或更新凭据。', 'Sign in or update credentials.');
    if (owner.state === 'needs_credential') return t('需要更新凭据。', 'Update the connection credentials.');
    if (member.availability === 'cooling_down') return t('此模型暂时不可用，请稍后重试。', 'This model is cooling down. Try again later.');
    if (member.availability === 'unknown') return t('当前状态尚未确认，请检查接入。', 'Its current state is unconfirmed. Review the connection.');
    return t('此模型当前不能用于路由，请检查接入。', 'This model cannot currently be used for routing. Review the connection.');
  };
  const connectedModels: ConnectedTrialModel[] = (data?.management.sources ?? []).flatMap(owner => owner.models.map(member => ({
    bindingId: member.binding_id, sourceId: owner.source_id, name: member.display_name, source: owner.display_name,
    available: bindingUsable(owner, member, data?.options.candidates.find(candidate => candidate.binding_id === member.binding_id)),
    reason: unavailableReason(owner, member),
  })));
  for (const [bindingId, connection] of retainedApi) {
    const candidate = data?.options.candidates.find(value => value.binding_id === bindingId);
    if (connection && retainedApiFor(data, candidate, retainedApi)) connectedModels.push({
      bindingId, name: candidate!.display_name, source: connection.display_name, available: true,
    });
  }
  const agentMatches = !!mapping.agentId && agentRead.snapshot?.selected?.agent_id === mapping.agentId;
  const readiness = trialReadiness({ language, trialId, trialUsable: usable(selectedTrial), trialReasoning: trialReasoning?.state,
    primaryId, primaryUsable: usable(selectedPrimary), primaryReasoning: primaryReasoning?.state, connectionOpen: !!connect && !connectionHidden && connectionState === 'editing',
    connectionPending: connect && (connectionState === 'saving' || connectionState === 'uncertain') ? connectionState : null,
    decisionUsable: classifier.kind === 'decision_service' && !!classifier.service.name && services.some(s => s.id === classifier.service.id && s.revision === classifier.service.revision),
    agentId: mapping.agentId, agentMatches, agentSeedKnown: !!agentRead.snapshot?.selected && agentEditorSeed(agentRead.snapshot.selected, 'model', []).known,
    claude: agentEcosystem(mapping.agentId) === 'claude', presets: mapping.presets.length,
    name: name ?? (defaultName || (!trial || !primary ? 'pending' : '')), threshold, custom,
    authoring: !!data?.options.authoring, loading, readError: !!error, decisionError: servicesError, agentError: agentRead.error, mutable, authority: !!agentRead.snapshot?.trusted_authority });
  const ready = readiness.canAdvance;
  const missing = (readiness.blockers.find(blocker => blocker.category !== 'user_input') ?? readiness.blockers[0])?.message ?? '';
  const fieldError = (target: TrialBlocker['target']) => attempted ? readiness.blockers.find(blocker => blocker.target === target && blocker.category === 'user_input')?.message : undefined;
  function focusBlocker(blocker: TrialBlocker) {
    if (blocker.target === 'primary') setChoosingPrimary(true);
    requestAnimationFrame(() => {
      const region = document.querySelector<HTMLElement>(`[data-trial-target="${blocker.target}"]`);
      const recovery = blocker.category !== 'user_input' ? region?.querySelector<HTMLElement>('[data-trial-retry]:not(:disabled)') : null;
      const target = recovery ?? region?.querySelector<HTMLElement>('[data-trial-focus]:not(:disabled)') ?? region?.querySelector<HTMLElement>('button:not(:disabled), select:not(:disabled), input:not(:disabled)') ?? region;
      target?.scrollIntoView({ block: 'center', behavior: 'instant' });
      target?.focus({ preventScroll: true });
    });
  }
  function cardState(model: TrialModel) {
    if (model.availability.kind === 'needs_newer_client' && !(unlinkedTrial && model.model_configuration_id === trialId)) return { tone: 'warn', text: t(`需要 HiRoute ${model.availability.version}`, `Requires HiRoute ${model.availability.version}`) };
    const available = candidates(model.model_configuration_id);
    if (model.catalog && !(unlinkedTrial && model.model_configuration_id === trialId) && available.some(usable) && !available.some(candidate => exactCatalogBinding(model.model_configuration_id, candidate.binding_id) && usable(candidate))) return { tone: 'warn', text: t('请选择已保存模型，目录匹配未确认', 'Choose a saved model; catalog match unconfirmed') };
    if (available.some(usable)) return { tone: 'good', text: t('已连接', 'Connected') };
    if (available.length) return { tone: 'warn', text: t('接入当前不可用', 'Connection currently unavailable') };
    return { tone: '', text: t('需要先连接，下面可以直接完成', 'Connect it first, right below') };
  }
  function connectionPicker(role: 'trial' | 'primary') {
    const available = role === 'trial' ? trialCandidates : primaryCandidates;
    const selected = role === 'trial' ? selectedTrial : selectedPrimary;
    if (available.length < 2) return null;
    const label = (candidate: typeof available[number]) => `${source(candidate.binding_id) ?? t('来源未确认', 'Source unconfirmed')} · ${candidate.billing_class}`;
    return <label className="field trial-connection-field"><span className="field-label">{t('使用的接入', 'Connection')}</span><select className="input" value={selected?.binding_id ?? ''} onChange={e => {
      const element = e.currentTarget, value = element.value;
      if (role === 'primary') { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimaryBinding(value); setPrimarySelection(undefined); return; }
      if (value === selected?.binding_id) return;
      const generation = selectionEpoch.current;
      void (async () => {
        if (connect && connectionState === 'editing' && !(await releaseConnection())) { element.value = selected?.binding_id ?? ''; return; }
        if (!live.current || generation !== selectionEpoch.current) return;
        ++selectionEpoch.current; ++pickerEpoch.current;
        setTrialBinding(value); setTrialChosen(undefined);
        const member = bindingInfo(value)?.member;
        setUnlinked(member && (!member.catalog_admitted || member.catalog_configuration_id !== trialId) ? value : '');
        if (connect && connectionState !== 'editing') setConnectionHidden(true);
        else if (connect) { setConnect(null); setConnectionHidden(false); }
      })();
    }}><option value="">{t('请选择', 'Select')}</option>{available.map((c, index) => <option key={c.binding_id} value={c.binding_id} disabled={!usable(c)}>{label(c)}{available.filter(other => label(other) === label(c)).length > 1 ? ` · ${index + 1}` : ''}</option>)}</select></label>;
  }
  function reasoningPicker(role: 'trial' | 'primary') {
    const candidate = role === 'trial' ? selectedTrial : selectedPrimary;
    const result = role === 'trial' ? trialReasoning : primaryReasoning;
    if (!candidate || !result) return null;
    const value = result.selection.reasoning;
    const label = value?.kind === 'profile' ? value.profile : value?.kind === 'toggle' ? (value.enabled ? t('开启', 'On') : t('关闭', 'Off')) : value?.kind === 'budget' ? `${value.tokens} tokens` : candidate.reasoning.kind === 'fixed' ? t('固定', 'Fixed') : t('请选择', 'Choose reasoning');
    const hasError = result.state === 'invalid' || (attempted && result.state === 'choice_required');
    return <div className="field trial-reasoning" data-trial-target={`${role}-reasoning`} tabIndex={-1}>
      <span className="field-label" id={`${role}-reasoning-label`}>{candidate.reasoning.kind === 'fixed' ? t('思考设置', 'Reasoning') : <>{t('思考强度', 'Reasoning')}<TrialRequiredMark language={language} /></>}</span>
      {candidate.reasoning.kind === 'fixed' && result.state !== 'invalid' ? <span>{label}</span>
        : candidate.reasoning.kind === 'fixed' ? <button type="button" className="btn" aria-invalid={hasError} aria-describedby={`${role}-reasoning-error`} onClick={() => { const value = { binding_id: candidate.binding_id }; if (role === 'trial') setTrialChosen(value); else setPrimarySelection(value); }}>{t('使用模型固定设置', 'Use the model’s fixed setting')}</button>
          : <button type="button" className="btn trial-reasoning-control" aria-haspopup="dialog" aria-labelledby={`${role}-reasoning-label ${role}-reasoning-value`} aria-invalid={hasError} aria-describedby={hasError ? `${role}-reasoning-error` : undefined} onClick={() => setReasoningRole(role)}>
            <span id={`${role}-reasoning-value`}>{label}</span><UiIcon name="chevron" />
          </button>}
      {hasError && <p id={`${role}-reasoning-error`} className="oc-inline-error">{result.state === 'invalid' ? t('当前思考设置或能力信息无效，请重新选择或检查接入。', 'The current reasoning setting or capability is invalid. Choose again or review the connection.') : t('请选择有效的思考设置。', 'Choose a valid reasoning setting.')}</p>}
    </div>;
  }
  const chooseTrial = (id: string, explicitSavedModel = false) => void (async () => {
    if (id === trialId || (connect && connectionState === 'editing' && !(await releaseConnection()))) return;
    if (explicitSavedModel) {
      const matches = candidates(id);
      const owner = matches.length === 1 ? data?.management.sources.find(source => source.models.some(member => member.binding_id === matches[0].binding_id)) : undefined;
      const member = owner?.models.find(value => value.binding_id === matches[0]?.binding_id);
      if (owner && member && matches[0] && bindingUsable(owner, member, matches[0])) { await chooseSavedBinding('trial', owner.source_id, matches[0].binding_id, true); return; }
      if (matches.length === 1 && retainedApiFor(data, matches[0], retainedApi)) { await chooseRetainedBinding('trial', matches[0].binding_id, true); return; }
    }
    ++selectionEpoch.current; ++pickerEpoch.current; setUnlinked(''); setTrialId(id); setTrialBinding(''); setTrialChosen(undefined);
    if (connect && connectionState !== 'editing') setConnectionHidden(true); else { setConnect(null); setConnectionHidden(false); }
    if (id === primaryId) { primaryOrigin.current = { kind: 'unset', agentId: mapping.agentId }; setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); }
  })();
  const openPicker = (role: 'trial' | 'primary') => { ++pickerEpoch.current; pickerBusyRef.current = false; setPickerBusy(false); setPickerError(''); setPickerRole(role); };
  const closePicker = () => { ++pickerEpoch.current; pickerBusyRef.current = false; setPickerBusy(false); setPickerError(''); setPickerRole(null); };
  const chooseSavedBinding = async (role: 'trial' | 'primary', sourceId: string, bindingId: string, alreadyReleased = false) => {
    const source = data?.management.sources.find(s => s.source_id === sourceId);
    const member = source?.models.find(m => m.binding_id === bindingId);
    const candidate = data?.options.candidates.find(c => c.binding_id === bindingId && c.routable);
    if (!source || !member || !candidate || !bindingUsable(source, member, candidate) || pickerBusyRef.current) return;
    if (role === 'trial' && connect && connectionState === 'editing' && !alreadyReleased && !(await releaseConnection())) return;
    const epoch = ++pickerEpoch.current, selection = role === 'trial' ? ++selectionEpoch.current : selectionEpoch.current;
    pickerBusyRef.current = true; setPickerBusy(true); setPickerError('');
    const current = await refresh(false, true, {source_id: sourceId, bindings:[{binding_id: bindingId, model_ref:member.model_ref, revision:member.revision}], skipCatalogForNonRegistered:true});
    if (!live.current || epoch !== pickerEpoch.current || selection !== selectionEpoch.current) return;
    pickerBusyRef.current = false; setPickerBusy(false);
    const freshSource = current?.management.sources.find(s => s.source_id === sourceId);
    const freshMember = freshSource?.models.find(m => m.binding_id === bindingId && m.model_ref === member.model_ref && m.revision === member.revision && m.catalog_configuration_id === member.catalog_configuration_id);
    const freshCandidate = current?.options.candidates.find(c => c.binding_id === bindingId && c.model_configuration_id === candidate.model_configuration_id);
    if (!freshSource || !freshMember || !freshCandidate || !bindingUsable(freshSource, freshMember, freshCandidate, current)) {
      setPickerRole(role);
      setPickerError(t('所选接入已变化或暂时无法读取，请核对后重新选择。', 'The selected connection changed or could not be read. Review it and choose again.'));
      return;
    }
    if (role === 'trial') {
      setTrialId(freshCandidate.model_configuration_id); setTrialBinding(bindingId); setTrialChosen(undefined);
      setUnlinked(freshMember.catalog_admitted && freshMember.catalog_configuration_id === freshCandidate.model_configuration_id ? '' : bindingId);
      if (connect && connectionState !== 'editing') setConnectionHidden(true); else { setConnect(null); setConnectionHidden(false); }
      if (freshCandidate.model_configuration_id === primaryId) { setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); }
    } else {
      primaryOrigin.current = {kind:'manual', agentId:mapping.agentId};
      setPrimaryId(freshCandidate.model_configuration_id); setPrimaryBinding(bindingId); setPrimarySelection(undefined); setChoosingPrimary(false);
    }
    setPickerRole(null);
  };
  const chooseRetainedBinding = async (role: 'trial' | 'primary', bindingId: string, alreadyReleased = false) => {
    const candidate = data?.options.candidates.find(value => value.binding_id === bindingId);
    const prior = retainedApiFor(data, candidate, retainedApi);
    if (!candidate || !prior || pickerBusyRef.current) return;
    if (role === 'trial' && connect && connectionState === 'editing' && !alreadyReleased && !(await releaseConnection())) return;
    const epoch = ++pickerEpoch.current, selection = role === 'trial' ? ++selectionEpoch.current : selectionEpoch.current;
    pickerBusyRef.current = true; setPickerBusy(true); setPickerError('');
    const current = await refresh(false, true);
    if (!live.current || epoch !== pickerEpoch.current || selection !== selectionEpoch.current) return;
    pickerBusyRef.current = false; setPickerBusy(false);
    const freshCandidate = current?.options.candidates.find(value => value.binding_id === bindingId && value.model_configuration_id === candidate.model_configuration_id);
    const fresh = retainedApiFor(current, freshCandidate);
    if (!fresh || !freshCandidate || fresh.source_id !== prior.source_id || fresh.source_revision !== prior.source_revision || fresh.binding_revision !== prior.binding_revision || fresh.display_name !== prior.display_name) {
      setPickerRole(role);
      setPickerError(t('所选接入已变化或暂时无法读取，请核对后重新选择。', 'The selected connection changed or could not be read. Review it and choose again.'));
      return;
    }
    if (role === 'trial') {
      setTrialId(freshCandidate.model_configuration_id); setTrialBinding(bindingId); setTrialChosen(undefined); setUnlinked('');
      if (connect && connectionState !== 'editing') setConnectionHidden(true); else { setConnect(null); setConnectionHidden(false); }
      if (freshCandidate.model_configuration_id === primaryId) { setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); }
    } else {
      primaryOrigin.current = {kind:'manual', agentId:mapping.agentId};
      setPrimaryId(freshCandidate.model_configuration_id); setPrimaryBinding(bindingId); setPrimarySelection(undefined); setChoosingPrimary(false);
    }
    setPickerRole(null);
  };
  const step = mapped ? 4 : published ? 3 : review ? 2 : 1;
  const steps = [t('选择模型', 'Choose models'), t('核对并发布', 'Review and publish'), t('映射 Agent', 'Map the Agent'), t('开始使用', 'Start using it')];
  const setup = data && <fieldset disabled={catalogLoading || !mutable} className="trial-setup">
    <p className="field-help trial-required-hint">{t('* 表示需要完成的项目', '* marks required items')}</p>
    <section className="trial-card" data-trial-target="trial" tabIndex={-1}>
      <div className="trial-card-head"><h3>{t('要试用的模型', 'Model to try')}<TrialRequiredMark language={language} /></h3>{featured.length > 0 && <span>{t('来自 hiroute.ai 模型目录和内置目录', 'From the hiroute.ai model directory and the built-in catalog')}</span>}</div>
      {recommendationState === 'error' && <p role="alert" className="oc-inline-error">{t('推荐模型暂时无法读取。', 'Recommended models could not be read.')} <button type="button" className="btn btn-quiet" onClick={() => void refreshRecommendations()}>{t('重试', 'Retry')}</button></p>}
      {recommendationState === 'loading' && <p role="status" className="field-help">{t('正在读取推荐模型…', 'Reading recommended models…')}</p>}
      <div className="trial-model-grid" role="group" aria-label={t('要试用的模型', 'Model to try')}>{cards.map(model => { const state = cardState(model); const actual = unlinkedTrial && model.model_configuration_id === trialId; const facts = actual ? null : model.catalog?.facts ?? null; return <button type="button" key={model.model_configuration_id} className={`trial-model-card${trialId === model.model_configuration_id ? ' selected' : ''}`} aria-pressed={trialId === model.model_configuration_id} disabled={model.availability.kind === 'needs_newer_client' && !actual} onClick={() => chooseTrial(model.model_configuration_id)}>
        <span className="trial-model-title"><strong>{actual ? trialName : model.display_name}</strong>{trialId === model.model_configuration_id && <UiIcon name="check" className="trial-model-check" />}</span>
        <span className="trial-model-meta">{sourceLabel(model)}</span>
        {facts && <span className="model-fact-tags">{factTags(facts, language).map(tag => <span key={tag} className="badge no-dot">{tag}</span>)}</span>}
        <span className="trial-model-note new-model-note">{t('从范围明确的小任务开始试用，并观察实际表现。', 'Start with a well-scoped task and observe the results.')}</span>
        <span className={`trial-model-state ${state.tone}`}>{state.text}</span>
      </button>; })}</div>
      <TrialConnectedModelSelect choices={connectedModels} selected={selectedTrial?.binding_id} language={language} busy={pickerBusy} expanded={pickerRole === 'trial' && !manageSource} onOpen={() => openPicker('trial')} />
      {trial && !unlinkedTrial && <Disclosure className="trial-model-facts" label={t('版本与来源', 'Version and source')} language={language}>
        {trial.catalog?.origin.kind === 'feed' && <p className="field-help">{t(`模型信息来源：hiroute.ai 模型目录（${shortDate(trial.catalog.origin.as_of, language)} 资料日期，内容已校验）。`, `Model information: the hiroute.ai model directory (information as of ${shortDate(trial.catalog.origin.as_of, language)}, content validated).`)}</p>}
        {rollingIdentity(trial.identity_stability) && <p className="field-help">{t('供应商可能更新同名模型；后续证据可能混合多个版本。', 'The provider may update this model alias; later evidence may mix versions.')}</p>}
        <p className="field-help">{t('目录或公告日期不能证明当前接入实际使用的版本。', 'A catalog or announcement date does not prove the version served by this connection.')}</p>
      </Disclosure>}
      {connectionPicker('trial')}
      {trialBinding && !selectedTrial && <p className="oc-inline-error" role="alert">{t('所选试用接入当前不可用，请重新选择。', 'The selected trial connection is unavailable. Choose again.')}</p>}
      {selectedTrial && <p className="field-help"><strong>{bindingInfo(selectedTrial.binding_id)?.member.display_name ?? selectedTrial.display_name}</strong> · {source(selectedTrial.binding_id) ?? t('来源未确认', 'Source unconfirmed')} · {sourceKind(selectedTrial.binding_id)}{explicitUnlinkedTrial && <> · {t('使用此接入中的模型，目录版本未核实', 'Using the model from this connection; catalog version unconfirmed')}</>}</p>}
      {!connect && trial?.catalog && trialCandidates.length > 0 && <button type="button" className="btn btn-quiet trial-inline-action" onClick={() => { connectionEpoch.current = ++selectionEpoch.current; setConnectionState('editing'); setConnectionHidden(false); setConnect(trial.catalog!); }}>{t('连接其他接入点', 'Connect another source')}</button>}
      {connectionHidden && <button type="button" className="btn btn-quiet trial-inline-action" data-trial-retry onClick={() => setConnectionHidden(false)}>{t('查看接入结果', 'Review connection result')}</button>}
      {fieldError('trial') && <p id="trial-model-error" className="oc-inline-error">{fieldError('trial')}</p>}
      {reasoningPicker('trial')}
      {alias && <p className="field-help">{t('路由别名', 'Route alias')}: <code>{alias}</code></p>}
      {connect && data && <div className="trial-connect-card" hidden={connectionHidden}><TrialConnectModel key={connect.model_configuration_id} model={connect} language={language} management={data.management} onReuseSource={async sourceId => {
        const savedSource = data.management.sources.find(source => source.source_id === sourceId && source.actions.includes('append_models'));
        if (!savedSource || !await releaseConnection() || !live.current) return;
        setConnect(null); setConnectionHidden(false); setConnectionState('editing');
        reuseConnectionOrigin.current = true;
        openPicker('trial'); setManageSource(sourceId); setAppendIntent(true);
      }} mutable={mutable} onOperation={operationRef} onResolve={receipt => refresh(false, true, receipt)} onReceipt={() => setSavedReceipt(true)} onSelectionIntent={() => { ++pickerEpoch.current; return ++selectionEpoch.current; }} onStateChange={setConnectionState} onBack={recoverable => { if (recoverable) setConnectionHidden(true); else { if (!replacingConnection.current) ++selectionEpoch.current; setConnect(null); setConnectionState('editing'); setConnectionHidden(false); } void refresh(); }} onSaved={(_result, selection, intent) => { if (!live.current || (intent ?? connectionEpoch.current) !== selectionEpoch.current) return false; setSavedConnections(values => [...new Set([...values, selection.source])]); setTrialId(selection.modelId); setTrialBinding(selection.bindingId); setTrialChosen(undefined); setUnlinked(selection.linked ? '' : selection.bindingId); setConnect(null); setConnectionState('editing'); setConnectionHidden(false); void onRefresh(); return true; }} /></div>}
      {!connect && trial?.catalog && !unlinkedTrial && !trialCandidates.length && trial.availability.kind !== 'needs_newer_client' && data && <div className="trial-connect-card"><div><strong>{t(`先接入 ${trial.display_name}`, `Connect ${trial.display_name} first`)}</strong><p className="field-help">{t('只读取这个服务的模型目录，不会发起付费推理。API Key 保存在本机。', 'Only the service model list is read; no paid inference runs. The API key stays on this device.')}</p></div><button type="button" className="btn btn-primary" onClick={() => { connectionEpoch.current = ++selectionEpoch.current; setConnectionState('editing'); setConnectionHidden(false); setConnect(trial.catalog!); }}>{t('在此接入', 'Connect here')}</button></div>}
      <TrialProtectionExplanation classifier={classifier} language={language} />
    </section>
    <section className="trial-card trial-card-pair">
      <div className="trial-column" data-trial-target="primary" tabIndex={-1}>
        <h3>{t('主力模型', 'Primary model')}<TrialRequiredMark language={language} /></h3>
        <div className="actions"><button type="button" data-trial-focus className="btn" aria-invalid={!!fieldError('primary')} aria-describedby={fieldError('primary') ? 'trial-primary-error' : undefined} onClick={() => openPicker('primary')}>{t('选择已接入模型', 'Choose a connected model')}</button><button type="button" className="btn btn-quiet" onClick={() => onOpenModels?.()}>{t('添加模型…', 'Add a model…')}</button></div>
        {fieldError('primary') && <p id="trial-primary-error" className="oc-inline-error">{fieldError('primary')}</p>}
        {selectedPrimary && primary && !choosingPrimary ? <button type="button" className="trial-choice selected" aria-expanded={false} onClick={() => setChoosingPrimary(true)}><strong>{bindingInfo(selectedPrimary.binding_id)?.member.display_name ?? primary.display_name}</strong><span>{[source(selectedPrimary.binding_id) ?? t('来源未确认', 'Source unconfirmed'), sourceKind(selectedPrimary.binding_id), exactCatalogBinding(selectedPrimary.model_configuration_id, selectedPrimary.binding_id) ? '' : t('使用此接入中的模型，目录版本未核实', 'Using the model from this connection; catalog version unconfirmed'), primaryOrigin.current.kind === 'agent' && mappedAgent ? t(`来自 ${agentDisplayName(mappedAgent.agent_id, language)} 现在使用的模型`, `What ${agentDisplayName(mappedAgent.agent_id, language)} uses now`) : ''].filter(Boolean).join(' · ')}</span><em>{t('更换', 'Change')}</em></button>
          : <div className="trial-choice-list" role="group" aria-label={t('主力模型', 'Primary model')}>{(allPrimary ? primaryOptions : primaryOptions.slice(0, 4)).map(model => <button type="button" key={model.model_configuration_id} className={`trial-choice${primaryId === model.model_configuration_id ? ' selected' : ''}`} aria-pressed={primaryId === model.model_configuration_id} onClick={() => { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimaryId(model.model_configuration_id); setPrimaryBinding(''); setPrimarySelection(undefined); setChoosingPrimary(false); }}><strong>{model.display_name}</strong><span>{sourceLabel(model)}</span></button>)}{!allPrimary && primaryOptions.length > 4 && <button type="button" className="btn btn-quiet trial-inline-action" onClick={() => setAllPrimary(true)}>{t(`显示全部 ${primaryOptions.length} 个已连接模型`, `Show all ${primaryOptions.length} connected models`)}</button>}{!primaryOptions.length && <p className="field-help">{t('还没有其他已连接的模型。', 'No other connected model yet.')}</p>}</div>}
        {connectionPicker('primary')}
        {primaryId && !selectedPrimary && <p className="oc-inline-error" role="alert">{t('所选主力接入当前不可用，请重新选择。', 'The selected Primary connection is unavailable. Choose again.')}</p>}
        {reasoningPicker('primary')}
        <p className="field-help">{custom ? t('扩展选择主力组时使用此模型。仅在已保存配置明确指向单个模型时，才自动填入当前主力。', 'This model is used when the extension selects Primary. It is prefilled only when saved settings identify a single model.') : t('负责复杂任务和需要保护的轮次。仅在已保存配置明确指向单个模型时，才自动填入当前主力。', 'It takes complex tasks and turns that need protection. It is prefilled only when saved settings identify a single model.')}</p>
        <button type="button" className="btn btn-quiet trial-inline-action" onClick={() => setSubscriptions(true)}>{t('扫描本机订阅', 'Scan local subscriptions')}</button>
      </div>
      <div className="trial-column trial-decision" data-trial-target="decision" tabIndex={-1}>
        <h3>{t('决策连接', 'Decision connection')}<TrialRequiredMark language={language} /></h3>
        <DecisionSelector classifier={classifier} smart allowHeuristic={false} announceError={false} trialFocus error={fieldError('decision')} services={services} language={language} onChange={value => { serviceTouched.current = true; setClassifier(value); }} onOpenServices={openDecision} />
        {servicesError && <p role="alert">{t('决策连接暂时无法读取。', 'Decision connections could not be read.')} <button type="button" data-trial-retry className="btn btn-quiet" onClick={() => void refreshServices()}>{t('重试', 'Retry')}</button></p>}
        <p className="field-help">{custom ? t('扩展直接选择执行组，可以提供仅用于观察的评分。', 'The extension selects the execution group directly and may provide scores for observation.') : t('试用需要给每个阶段打胜任分，所以不提供启发式规则。', 'A trial needs a competence score for every stage, so heuristic rules are not offered.')}</p>
      </div>
    </section>
    <section className="trial-card trial-card-pair">
      <div className="trial-column" data-trial-target="agent" tabIndex={-1}><h3>{t('接入的 Agent', 'Agent')}<TrialRequiredMark language={language} /></h3>{agentRead.snapshot && <TrialAgentChoice snapshot={agentRead.snapshot} value={mapping} onChange={value => { mappingInitialized.current = true; if (value.agentId !== mapping.agentId) { ++selectionEpoch.current; ++pickerEpoch.current; primaryOrigin.current = { kind: 'unset', agentId: value.agentId }; setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); } setMapping(value); }} language={language} loading={agentRead.loading} presetTarget agentError={fieldError('agent')} presetError={fieldError('presets')} onRetry={() => void agentRead.refresh()} />}{agentRead.error && <p role="alert">{t('Agent 配置暂时无法读取，请重试。', 'Agent settings could not be read. Retry.')}<button type="button" data-trial-retry className="btn btn-quiet" onClick={() => void agentRead.refresh(true)}>{t('重试', 'Retry')}</button></p>}</div>
      <div className="trial-column">
        <h3>{custom ? t('路由名称', 'Route name') : t('名称与起始信任度', 'Name and starting trust')}</h3>
        <label className="field" data-trial-target="name"><span className="field-label">{t('路由名称', 'Route name')}</span><input className="input" value={name ?? defaultName} placeholder={t('选好模型后自动填写', 'Filled in after choosing models')} aria-invalid={!!fieldError('name')} aria-describedby={fieldError('name') ? 'trial-name-error' : undefined} onChange={e => setName(e.target.value)} />{fieldError('name') && <span id="trial-name-error" className="oc-inline-error">{fieldError('name')}</span>}</label>
        {!custom && data.options.authoring && <><div data-trial-target="trust"><TrustLevelControl context="initial-trial" id="trial-initial" language={language} classifier={classifier} value={{ ...(reviewDraft?.smart.judgment ?? data.options.authoring.editor.smart.judgment), degree: { ...(reviewDraft?.smart.judgment.degree ?? data.options.authoring.editor.smart.judgment.degree), simple_threshold_millis: threshold } }} onChange={value => setThreshold(value.degree.simple_threshold_millis)} /></div>
        <Disclosure className="trial-criteria" label={t('胜任评分标准', 'Competence criteria')} language={language}><p>{criteriaHelp}</p><p className="field-help">{t('下一步核对时可以修改实际提示词。', 'You can edit the actual prompt in the next step.')}</p></Disclosure></>}
      </div>
    </section>
    {attempted && readiness.blockers.length > 0 && <div className="trial-validation-summary callout warn" role="alert"><strong>{t(`还需完成 ${readiness.blockers.length} 项才能继续`, `Complete ${readiness.blockers.length} items before continuing`)}</strong><ul>{readiness.blockers.map(blocker => <li key={blocker.code}><button type="button" className="btn btn-quiet" onClick={() => focusBlocker(blocker)}>{blocker.message}</button></li>)}</ul></div>}
    <div className="trial-footer" data-trial-target="refresh">
      <span className="trial-footer-hint" role="status">{missing}</span>
      <button type="button" data-trial-retry className="btn btn-quiet" onClick={() => void refresh(true)}>{t('刷新可用状态', 'Refresh availability')}</button>
      <button type="button" className="btn" onClick={leave}>{t('取消', 'Cancel')}</button>
      <button type="button" className="btn btn-primary" disabled={readiness.systemBlocked} onClick={() => {
        setAttempted(true);
        if (!ready) { if (readiness.blockers[0]) focusBlocker(readiness.blockers[0]); return; }
        if (!selectedTrial || !selectedPrimary || !trialReasoning || !primaryReasoning || classifier.kind !== 'decision_service') return;
        if (!data?.options.authoring) return;
        const editor = structuredClone(reviewDraft ?? data.options.authoring.editor); editor.mode = 'smart_saving';
        if (!reviewDraft || reviewDraft.smart.economy[0]?.binding_id !== trialReasoning.selection.binding_id) editor.custom_alias = alias;
        editor.display_name = name ?? defaultName;
        if (!reviewDraft) editor.purpose = custom ? t('由自定义扩展选择执行组，观察新模型在自己任务中的表现。', 'Let the custom extension choose the execution group and observe the new model on your own tasks.') : t('在主力保护下试用新模型，根据自己的任务证据调整路由。', 'Try a new model with Primary protection and adjust routing using evidence from your own tasks.');
        editor.smart.economy = [trialReasoning.selection, ...(reviewDraft?.smart.economy.slice(1) ?? [])]; editor.smart.primary = [primaryReasoning.selection, ...(reviewDraft?.smart.primary.slice(1) ?? [])];
        editor.smart.classifier = structuredClone(classifier);
        if (!custom) { if (!nativeInitialized.current) { if (!reviewDraft) { editor.smart.judgment.competence.standard = 'custom'; editor.smart.judgment.competence.criteria[0] = trialCriteria; } nativeInitialized.current = true; } editor.smart.judgment.degree.simple_threshold_millis = threshold; }
        setReview(editor);
      }}>{t('检查改动', 'Review changes')}</button>
    </div>
  </fieldset>;
  return <ProductPage className="trial-page" title={t('试用新模型', 'Try a new model')} subtitle={custom ? t('由自定义扩展选择执行组。发布前会列出全部改动。', 'Your custom extension chooses the execution group. Every change is listed before publishing.') : t('新模型先上，主力兜底。发布前会列出全部改动。', 'The new model goes first and Primary catches what it misses. Every change is listed before publishing.')} actions={<button type="button" className="btn btn-quiet" onClick={leave}><UiIcon name="arrowLeft" />{t('返回', 'Back')}</button>}>
    <div className="trial-wizard">
      <ol className="trial-steps" aria-label={t('试用步骤', 'Trial steps')}>{steps.map((label, i) => <li key={label} className={i + 1 < step ? 'done' : i + 1 === step ? 'current' : undefined} aria-current={i + 1 === step ? 'step' : undefined}><span>{i + 1}</span>{label}</li>)}</ol>
      {data?.availability.models.some(model => model.origin.kind === 'feed') && <FeedDisclosure language={language} mutable={mutable} />}
      {error && <div className="callout warn" role="alert"><p>{error}</p><button className="btn" onClick={() => void refresh()}>{t('重试', 'Retry')}</button></div>}
      {savedConnections.length > 0 ? <p className="trial-saved" role="status">{t('本次已保存接入：', 'Connections saved in this flow: ')}{savedConnections.join(' · ')}</p> : savedReceipt && <p className="trial-saved" role="status">{t('接入已保存；请选择实际可用的模型。', 'Connection saved; choose an available model.')}</p>}
      {setup && <div hidden={!!review || !!published}>{setup}</div>}
      {published ? <section className="trial-card"><TrialAgentMapping onConfirmed={() => setMapped(true)} onOpenPerformance={() => onOpenPerformance?.(published)} onOpenSessions={onOpenSessions} plan={published} initial={mapping} initialChoices={agentRead.snapshot} language={language} mutable={mutable} onOperation={operationRef} onChanged={() => void onRefresh()} onOpenAgent={onOpenAgent} /></section>
        : review ? <section className="trial-card trial-review-card"><div className="trial-card-head"><h3>{t('核对改动', 'Review changes')}</h3><span>{t('将映射', 'Mapping to review')}: {agentDisplayName(mapping.agentId, language)}{agentEcosystem(mapping.agentId) === 'claude' ? ` · ${mapping.presets.join(', ')}` : ''}</span></div><TrialReview ref={reviewHandle} editor={review} previewAgentId={mapping.agentId} language={language} mutable={mutable} snapshot={snapshot} operation={operation} onOperation={onOperation} onRefresh={onRefresh} onPublished={setPublished} onPendingChange={setPersistencePending} onRepresentedOperations={onRepresentedOperations} onClose={onClose} onBack={backToSetup} onOpenServices={openDecision} /></section>
        : !setup && !error ? <p role="status">{t('正在读取配置…', 'Reading configuration…')}</p> : null}
    </div>
    {reasoningRole && (() => { const candidate = reasoningRole === 'trial' ? selectedTrial : selectedPrimary; const result = reasoningRole === 'trial' ? trialReasoning : primaryReasoning; return candidate && result ? <ReasoningDialog name={candidate.display_name} native={candidate.reasoning} value={result.selection.reasoning} language={language} onClose={() => setReasoningRole(null)} onApply={reasoning => { const selection = { binding_id: candidate.binding_id, reasoning }; if (reasoningRole === 'trial') setTrialChosen(selection); else { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimarySelection(selection); } setReasoningRole(null); }} /> : null; })()}
    {pickerRole && !manageSource && !subscriptions && data && <Dialog open title={t('选择已接入模型', 'Choose a connected model')} closeLabel={t('返回试用', 'Back to trial')} onClose={closePicker} className="trial-dialog">
      <div className="trial-picker"><p className="field-help">{t('选择此角色使用的实际模型；不会创建接入或发起推理。', 'Choose the actual model for this role. This does not create a connection or run inference.')}</p>{pickerBusy && <p role="status">{t('正在确认所选接入…', 'Checking the selected connection…')}</p>}{pickerError && <p role="alert" className="oc-inline-error">{pickerError}</p>}
        {data.management.sources.map(source => <section className="trial-picker-source" key={source.source_id}>
          <h3>{source.display_name}{data.management.sources.filter(s => s.display_name === source.display_name).length > 1 ? ` · ${data.management.sources.filter(s => s.display_name === source.display_name).indexOf(source) + 1}` : ''} <span className="badge no-dot">{source.access_kind === 'subscription' ? t('订阅', 'Subscription') : source.access_kind === 'api' ? 'API' : t('类型未知', 'Unknown type')}</span></h3>
          {source.models.map(member => { const candidate = data.options.candidates.find(c => c.binding_id === member.binding_id); const sameAsTrial = pickerRole === 'primary' && candidate?.model_configuration_id === trialId; const available = bindingUsable(source, member, candidate) && !sameAsTrial; const recommended = available && member.catalog_admitted && !!member.catalog_configuration_id && member.catalog_configuration_id === (pickerRole === 'trial' ? trialId : primaryId) && candidate?.model_configuration_id === member.catalog_configuration_id; return <div className="trial-picker-row" key={member.binding_id}><button type="button" className="trial-choice" disabled={!available} aria-disabled={pickerBusy || !available} onClick={() => void chooseSavedBinding(pickerRole, source.source_id, member.binding_id)}><strong>{member.display_name}</strong><span>{member.upstream_model_id}{recommended ? ` · ${t('与推荐模型匹配', 'Matches recommendation')}` : ` · ${t('其他已接入模型', 'Other connected models')}`}</span>{!available && <span>{sameAsTrial ? t('已选为试用模型，请为主力选择其他模型。', 'Already selected for trial. Choose a different Primary model.') : unavailableReason(source, member)}</span>}</button>{pickerRole === 'primary' && !available && !sameAsTrial && <button type="button" className="btn btn-quiet" disabled={pickerBusy} onClick={() => { setAppendIntent(false); setManageSource(source.source_id); }}>{t('检查接入', 'Review connection')}</button>}</div>; })}
          {pickerRole === 'primary' && source.actions.includes('append_models') && <button type="button" className="btn btn-quiet" disabled={pickerBusy} onClick={() => { setAppendIntent(true); setManageSource(source.source_id); }}>{t('向此接入添加模型', 'Add model to this connection')}</button>}
        </section>)}
        {data.management.retained_api_connections.map(connection => {
          const candidate = data.options.candidates.find(value => value.binding_id === connection.binding_id);
          const exact = retainedApiFor(data, candidate, retainedApi);
          const sameAsTrial = pickerRole === 'primary' && candidate?.model_configuration_id === trialId;
          const available = !!exact && !sameAsTrial;
          return <section className="trial-picker-source" key={`retained-${connection.binding_id}`}>
            <h3>{connection.display_name} <span className="badge no-dot">API</span></h3>
            <div className="trial-picker-row"><button type="button" className="trial-choice" disabled={!available} aria-disabled={pickerBusy || !available} onClick={() => void chooseRetainedBinding(pickerRole, connection.binding_id)}><strong>{candidate?.display_name ?? t('模型不可用', 'Unavailable model')}</strong><span>{available ? t('已保存的 API 模型', 'Saved API model') : sameAsTrial ? t('已选为试用模型，请为主力选择其他模型。', 'Already selected for trial. Choose a different Primary model.') : t('此接入当前不可用。', 'This connection is currently unavailable.')}</span></button></div>
          </section>;
        })}
        {!data.management.sources.length && !data.management.retained_api_connections.length && <p>{t('还没有已保存的接入。', 'No saved connections yet.')}</p>}
        <div className="actions"><button type="button" className="btn" disabled={pickerBusy} onClick={() => { closePicker(); void (async () => { if (!connect || connectionState !== 'editing' || await releaseConnection()) onOpenModels?.(); })(); }}>{t('添加模型…', 'Add a model…')}</button></div>
      </div>
    </Dialog>}
    {pickerRole && manageSource && <Dialog open title={t('管理所选接入', 'Manage selected connection')} closeLabel={t('返回已接入模型', 'Back to connected models')} onClose={() => void (async () => { if (await requestEditorReplacement('models')) { setManageSource(null); setAppendIntent(false); if (reuseConnectionOrigin.current) { reuseConnectionOrigin.current = false; closePicker(); } void refresh(false, true); } })()} className="trial-dialog"><ModelManagementPage agents={agentRead.snapshot?.choices} language={language} active trustedAuthority={mutable} initialSourceId={manageSource} startAppending={appendIntent} scanOnMount={false} onOperation={operationRef} onRecoveryRefresh={() => void refresh()} onChanged={() => { void refresh(false, true); void onRefresh(); }} plans={snapshot?.catalog.plans} /></Dialog>}
    {decision && <Dialog open title={t('接入决策连接', 'Connect a decision service')} closeLabel={t('返回试用', 'Back to trial')} onClose={() => void (async () => { if (await requestEditorReplacement('decisions')) setDecision(null); })()} className="trial-dialog"><DecisionServicesPage language={language} active mutable={mutable} intent={decision} plans={snapshot?.catalog.plans} drafts={snapshot?.catalog.drafts} onChanged={() => void refreshServices()} onReturn={() => setDecision(null)} onUse={service => { serviceTouched.current = true; decisionSelect.current?.(service); setDecision(null); void refreshServices(); }} /></Dialog>}
    {subscriptions && <Dialog open title={t('复用本机订阅', 'Reuse a local subscription')} closeLabel={t('返回试用', 'Back to trial')} onClose={() => void (async () => { if (await requestEditorReplacement('models')) { setSubscriptions(false); void refresh(); } })()} className="trial-dialog"><ModelManagementPage agents={agentRead.snapshot?.choices} language={language} active trustedAuthority={mutable} startScanning onOperation={operationRef} onRecoveryRefresh={() => void refresh()} onChanged={() => { void refresh(); void onRefresh(); }} plans={snapshot?.catalog.plans} /></Dialog>}
  </ProductPage>;
}
