import { requestEditorReplacement } from '../ui/discard-guard';
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModelConnectionForm } from '../features/model-connections/ModelConnectionForm';
import { checkMatches, clientIdempotencyKey, modelSaveCompleted } from '../features/model-connections/state';
import type {
  ComputeCandidateView,
  ComputeConnectionApplyRequest,
  ComputeSaveResult,
  ModelConnectionCheckView,
  ModelConnectionBackend,
  ModelConnectionDraft,
  OperationReference,
  ProtectedInputRegistration,
} from '../features/model-connections/types';
import { Models } from '../features/models/Models';
import {
  DeviceScanList,
  type ComputeScanItem,
  type ComputeScanResult,
} from '../features/device-scan/DeviceScanList';
import { PriceEditor } from '../features/model-reference/PriceEditor';
import type { PriceContext, PriceOutcome } from '../features/model-reference/types';
import { agentBrandFromId, BrandIcon, Dialog, Disclosure, ProductPage, UiIcon } from '../ui';
import type { Agent } from '../agents';
import type {
  CandidateRef,
  ManagedModel,
  ManagedSource,
  ManagementChange,
  ManagementSnapshot,
  ProtectedKeyInput,
  SavePreview,
} from '../features/models/types';
import type { Plan } from '../plan-editor';
import type {
  SubscriptionCandidate,
  SubscriptionCheckResult,
} from '../features/subscriptions/types';
import { canSave as canSaveSubscription, saveFailureDefinitelyPreAdmission, saveIntent, validatedSubscriptionCandidate } from '../features/subscriptions/model';
import { safeDiagnosticCode } from '../error-code';

type SubscriptionSaveSelection = {
  candidate: SubscriptionCandidate['candidate'];
  validation?: SubscriptionCandidate['validation'];
  selected_model_refs: string[];
  intent: 'save_ready' | 'save_disabled';
};

type SubscriptionFailure = {
  code: string;
  phase: 'scan' | 'check' | 'save';
};

type SubscriptionScanResult = {
  candidates: SubscriptionCandidate[];
  discovery_state: 'complete' | 'runtime_unavailable';
  reason_code?: string;
};

type DiscoveryAction = 'preparing' | 'saving' | null;

function failureCode(error: unknown): string {
  return safeDiagnosticCode(error, 'CLIENT_ERROR');
}

function subscriptionFailureMessage(failure: SubscriptionFailure, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  const code = failure.code.toLocaleUpperCase();
  if (code.includes('AUTH') || code.includes('LOGIN') || code.includes('CREDENTIAL') || code.includes('UNAUTHORIZED')) {
    return zh ? '无法使用当前 Codex 登录。请先在 Codex 中登录，再重新检查。' : 'The current Codex sign-in could not be used. Sign in to Codex, then check again.';
  }
  if (code.includes('REVISION_CONFLICT') || code.includes('CHANGE_PREVIEW_STALE') || code.includes('SOURCE_CHANGED')) {
    return zh ? '接入状态已经变化。请重新检查订阅后再保存，当前选择已保留。' : 'The connection changed. Check the subscription again before saving; your selection is retained.';
  }
  if (failure.phase === 'scan') {
    return zh ? '暂时无法扫描本机订阅，请确认本机服务正在运行后重试。' : 'Local subscriptions could not be scanned. Confirm the local service is running, then try again.';
  }
  if (failure.phase === 'check') {
    return zh ? '订阅检查未完成。请确认 Codex 已登录后重试。' : 'The subscription check did not complete. Confirm Codex is signed in and try again.';
  }
  return zh ? '订阅接入未保存。请重新检查当前状态后再试，当前选择已保留。' : 'The subscription connection was not saved. Check the current state and try again; your selection is retained.';
}

function discoveryFailureMessage(code: string, language: 'zh' | 'en'): string {
  const zh = language === 'zh';
  switch (code.toLocaleLowerCase()) {
    case 'compute.discovery_changed':
    case 'revision_conflict':
    case 'change_preview_stale':
      return zh ? '本机配置或保存依据已发生变化，请重新扫描后再接入。' : 'The local configuration or save context changed. Scan again before connecting it.';
    case 'compute.discovery_unavailable':
      return zh ? '暂时无法重新读取这项配置，请重新扫描。' : 'This configuration cannot be read right now. Scan again.';
    case 'compute.discovery_not_importable':
      return zh ? '已发现配置，但当前无法安全接入。可以使用“连接 API”手动添加。' : 'The configuration was found, but it cannot be imported safely. Add it manually through Connect an API.';
    case 'registered_option_unavailable':
      return zh ? '当前版本暂不支持接入这项配置。' : 'This configuration is not supported by the current version.';
    default:
      return zh ? '这项配置暂时无法接入，请重试。' : 'This configuration could not be connected. Try again.';
  }
}

function discoveredAgentName(agentId: string, language: 'zh' | 'en') {
  if (agentId.toLocaleLowerCase().includes('claude')) return 'Claude Code';
  if (agentId.toLocaleLowerCase().includes('codex')) return 'Codex';
  return language === 'zh' ? '本机 Agent' : 'Local agent';
}

export function newModelConnectionDraft(kind: 'known' | 'custom' | 'free' = 'custom'): ModelConnectionDraft {
  const known = kind === 'known';
  return {
    entry_kind: kind === 'free' ? 'free_api_key' : known ? 'preset_api' : 'custom_api',
    candidate_ref: null,
    lineage_ref: `lineage/native/${crypto.randomUUID()}`,
    display_name: '',
    existing_source_id: null,
    edit_revision: 1,
    check_id: '',
    base_url: '',
    base_kind: 'api_root',
    request_path_override: null,
    inventory_path_override: null,
    protocol: 'chat_completions',
    protocol_profile_id: 'profile/custom/chat_completions',
    protocol_profile_revision: 1,
    authentication: { kind: 'bearer' },
    provenance: { kind: 'user_configured', configuration_revision: 1 },
    qualification: { free_access: null, evidence_ref: null },
    models: [],
  };
}

export function ModelManagementPage({
  language,
  active,
  trustedAuthority,
  initialSourceId = null,
  startAdding = false,
  notice,
  onOperation,
  onRecoveryRefresh,
  onChanged,
  refreshVersion = 0,
  plans,
  agents = [],
  onOpenPlan,
  onCreatePlan,
}: {
  language: 'zh' | 'en';
  active: boolean;
  trustedAuthority: boolean;
  initialSourceId?: string | null;
  startAdding?: boolean;
  notice?: string;
  onOperation(operation: OperationReference): void;
  onRecoveryRefresh(): void;
  onChanged(): void;
  refreshVersion?: number;
  plans?: Plan[];
  agents?: Agent[];
  onOpenPlan?(planId: string): void;
  onCreatePlan?(bindingId: string): void;
}) {
  const [management, setManagement] = useState<ManagementSnapshot | null>(null);
  const [focusedSource, setFocusedSource] = useState<string | null>(initialSourceId);
  const [subscriptions, setSubscriptions] = useState<SubscriptionCandidate[]>([]);
  const [subscriptionChecks, setSubscriptionChecks] = useState<Record<string, SubscriptionCheckResult | undefined>>({});
  const [subscriptionBusy, setSubscriptionBusy] = useState(false);
  const [subscriptionAction, setSubscriptionAction] = useState<'checking' | 'saving' | null>(null);
  const [selectedSubscriptionRef, setSelectedSubscriptionRef] = useState<string | null>(null);
  const [subscriptionSelections, setSubscriptionSelections] = useState<Record<string, string[]>>({});
  const [repairingSubscriptionSource, setRepairingSubscriptionSource] = useState<string | null>(null);
  const [subscriptionNotice, setSubscriptionNotice] = useState('');
  const [subscriptionLoading, setSubscriptionLoading] = useState(true);
  const [subscriptionError, setSubscriptionError] = useState<SubscriptionFailure | null>(null);
  const [subscriptionDiscoveryState, setSubscriptionDiscoveryState] = useState<'complete' | 'runtime_unavailable'>('complete');
  const [computeScanItems, setComputeScanItems] = useState<ComputeScanItem[]>([]);
  const [computeScanError, setComputeScanError] = useState('');
  const [selectedDiscovery, setSelectedDiscovery] = useState<ComputeScanItem | null>(null);
  const [preparedDiscovery, setPreparedDiscovery] = useState<ComputeCandidateView | null>(null);
  const [discoverySelections, setDiscoverySelections] = useState<string[]>([]);
  const [discoveryAction, setDiscoveryAction] = useState<DiscoveryAction>(null);
  const [discoveryError, setDiscoveryError] = useState('');
  const [adding, setAdding] = useState(startAdding && trustedAuthority);
  const [reconnectingFrom, setReconnectingFrom] = useState<string | null>(null);
  const [addStage, setAddStage] = useState<'choose' | 'api' | 'scan'>('choose');
  const [addNotice, setAddNotice] = useState('');
  const [modelNotice, setModelNotice] = useState('');
  const [priceTarget, setPriceTarget] = useState<{
    source: ManagedSource;
    model: ManagedModel;
    contexts: PriceContext[];
    writable: boolean;
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const addButton = useRef<HTMLButtonElement | null>(null);
  const protectedInputs = useRef<CandidateRef[]>([]);
  const initialDraft = useRef<ModelConnectionDraft>(newModelConnectionDraft());
  const checksRef = useRef<Record<string, SubscriptionCheckResult | undefined>>({});
  const subscriptionAttempt = useRef(0);
  const subscriptionInFlight = useRef(false);
  const handedOffValidations = useRef(new Set<string>());
  const wasActive = useRef(active);
  const alive = useRef(true);
  const managementRequest = useRef(0);
  const subscriptionView = useRef(0);
  const subscriptionScan = useRef(0);
  const discoveryGeneration = useRef(0);
  const discoveryInFlight = useRef(false);

  useEffect(() => {
    if (!subscriptionNotice) return;
    const timer = window.setTimeout(() => setSubscriptionNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [subscriptionNotice]);

  useEffect(() => {
    if (!modelNotice) return;
    const timer = window.setTimeout(() => setModelNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [modelNotice]);

  async function refreshManagement() {
    const request = ++managementRequest.current;
    setLoading(true);
    setError('');
    try {
      const snapshot = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (alive.current && managementRequest.current === request) setManagement(snapshot);
    } catch (cause) {
      if (alive.current && managementRequest.current === request) setError(failureCode(cause));
    } finally {
      if (alive.current && managementRequest.current === request) setLoading(false);
    }
  }

  async function refreshSubscriptions() {
    const scan = ++subscriptionScan.current;
    setSubscriptionLoading(true);
    setSubscriptionError(current => current?.phase === 'scan' ? null : current);
    setComputeScanError('');
    const [subscriptionResult, computeResult] = await Promise.allSettled([
      invoke<SubscriptionScanResult>('compute_subscriptions'),
      invoke<ComputeScanResult>('compute_scan'),
    ]);
    if (!alive.current || subscriptionScan.current !== scan) return;
    if (subscriptionResult.status === 'fulfilled') {
      setSubscriptions(subscriptionResult.value.candidates);
      setSubscriptionDiscoveryState(subscriptionResult.value.discovery_state);
    } else {
      setSubscriptionDiscoveryState('complete');
      setSubscriptionError({ code: failureCode(subscriptionResult.reason), phase: 'scan' });
    }
    if (computeResult.status === 'fulfilled') setComputeScanItems(computeResult.value.items);
    else setComputeScanError(failureCode(computeResult.reason));
    setSubscriptionLoading(false);
  }

  function closeSubscriptionView() {
    subscriptionView.current += 1;
    subscriptionScan.current += 1;
    subscriptionAttempt.current += 1;
    subscriptionInFlight.current = false;
    discoveryInFlight.current = false;
    checksRef.current = {};
    setSubscriptionChecks({});
    setSubscriptionBusy(false);
    setSubscriptionAction(null);
    setSelectedSubscriptionRef(null);
    setSubscriptionSelections({});
    setRepairingSubscriptionSource(null);
    setSubscriptionNotice('');
    setSubscriptionError(null);
    discoveryGeneration.current += 1;
    setSelectedDiscovery(null);
    setPreparedDiscovery(null);
    setDiscoverySelections([]);
    setDiscoveryAction(null);
    setDiscoveryError('');
    void invoke('close_subscription_check')
      .catch(() => undefined)
      .finally(onRecoveryRefresh);
  }

  function currentSubscriptionAttempt(view: number, attempt: number) {
    return alive.current && subscriptionView.current === view && subscriptionAttempt.current === attempt;
  }

  async function recoverSubscriptionCheck() {
    if (subscriptionInFlight.current || discoveryInFlight.current) return;
    const view = subscriptionView.current;
    const attempt = ++subscriptionAttempt.current;
    try {
      const result = await invoke<SubscriptionCheckResult | null>('recover_subscription_check');
      if (!result || !currentSubscriptionAttempt(view, attempt)) return;
      subscriptionInFlight.current = true;
      setSubscriptionBusy(true);
      setSubscriptionAction('checking');
      setSubscriptionError(null);
      await observeSubscriptionCheck(result.candidate.candidate_ref, result, view, attempt);
    } catch (cause) {
      if (currentSubscriptionAttempt(view, attempt)) setSubscriptionError({ code: failureCode(cause), phase: 'check' });
    } finally {
      if (currentSubscriptionAttempt(view, attempt)) {
        subscriptionInFlight.current = false;
        setSubscriptionBusy(false);
        setSubscriptionAction(null);
      }
    }
  }

  function rememberCheck(candidateRef: string, result: SubscriptionCheckResult) {
    checksRef.current = { ...checksRef.current, [candidateRef]: result };
    setSubscriptionChecks(checksRef.current);
  }

  async function observeSubscriptionCheck(candidateRef: string, initial: SubscriptionCheckResult, view: number, attempt: number) {
    let result = initial;
    rememberCheck(candidateRef, result);
    let delay = 300;
    while (result.status === 'checking') {
      await new Promise(resolve => window.setTimeout(resolve, delay));
      if (!currentSubscriptionAttempt(view, attempt)) return;
      result = await invoke<SubscriptionCheckResult>('get_subscription_check_result', {
        operation: result.approval_operation,
      });
      if (!currentSubscriptionAttempt(view, attempt)) return;
      rememberCheck(candidateRef, result);
      delay = Math.min(2000, delay + 250);
    }
    if (!currentSubscriptionAttempt(view, attempt)) return;
    if (!['verified', 'retained'].includes(result.status)) {
      setSubscriptionError({ code: result.reason || `SUBSCRIPTION_${result.status.toLocaleUpperCase()}`, phase: 'check' });
    }
    // Approval advances the control head even before a save is admitted.
    await Promise.all([refreshManagement(), refreshSubscriptions()]);
    onRecoveryRefresh();
  }

  async function checkSubscription(candidate: SubscriptionCandidate['candidate']) {
    if (subscriptionInFlight.current || discoveryInFlight.current) return;
    if (!trustedAuthority) {
      setSubscriptionError({ code: 'TRUSTED_AUTHORITY_UNAVAILABLE', phase: 'check' });
      return;
    }
    const view = subscriptionView.current;
    const attempt = ++subscriptionAttempt.current;
    subscriptionInFlight.current = true;
    setSubscriptionBusy(true);
    setSubscriptionAction('checking');
    setSubscriptionError(null);
    try {
      const result = await invoke<SubscriptionCheckResult>('check_subscription', { candidate, language });
      if (!currentSubscriptionAttempt(view, attempt)) return;
      await observeSubscriptionCheck(candidate.candidate_ref, result, view, attempt);
    } catch (cause) {
      const code = failureCode(cause);
      if (currentSubscriptionAttempt(view, attempt)) {
        if (code === 'SUBSCRIPTION_CHECK_CANCELLED' || code === 'SUBSCRIPTION_CHECK_CLOSED') {
          setSubscriptionNotice(language === 'zh' ? '已取消检查。' : 'Check cancelled.');
        } else {
          setSubscriptionError({ code, phase: 'check' });
        }
      }
    } finally {
      if (currentSubscriptionAttempt(view, attempt)) {
        subscriptionInFlight.current = false;
        setSubscriptionBusy(false);
        setSubscriptionAction(null);
      }
      onRecoveryRefresh();
    }
  }

  async function saveSubscription(selection: SubscriptionSaveSelection) {
    if (subscriptionInFlight.current || discoveryInFlight.current) return;
    if (!trustedAuthority) {
      setSubscriptionError({ code: 'TRUSTED_AUTHORITY_UNAVAILABLE', phase: 'save' });
      return;
    }
    if (!selection.validation) return;
    const view = subscriptionView.current;
    const repairing = repairingSubscriptionSource !== null;
    subscriptionAttempt.current += 1;
    subscriptionInFlight.current = true;
    setSubscriptionBusy(true);
    setSubscriptionAction('saving');
    setSubscriptionError(null);
    const validationKey = selection.validation.validation_ref;
    try {
      // A save must seal the control head that exists when B is submitted. The
      // displayed snapshot can lag behind an operation/recovery refresh, so read
      // the authoritative head again instead of closing over a rendered revision.
      const currentManagement = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (!alive.current || subscriptionView.current !== view) return;
      setManagement(currentManagement);
      const change: ManagementChange = {
        schema: 'hiroute.compute-management-change/v2',
        subject: { kind: 'candidate', candidate: selection.candidate },
        expected_revisions: currentManagement.revisions,
        selected_model_refs: selection.selected_model_refs,
        intent: selection.intent,
        key_edits: [],
        validation: selection.validation,
      };
      const preview = await invoke<SavePreview>('preview_compute_save', { change });
      if (!alive.current || subscriptionView.current !== view) return;
      handedOffValidations.current.add(validationKey);
      let accepted: { operation: OperationReference };
      try {
        accepted = await invoke<{ operation: OperationReference }>('apply_compute_save', {
          request: {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: clientIdempotencyKey('subscription-save'),
          },
          language,
        });
      } catch (cause) {
        if (saveFailureDefinitelyPreAdmission(cause) || saveFailureDefinitelyPreAdmission(failureCode(cause))) {
          handedOffValidations.current.delete(validationKey);
          throw cause;
        }
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view) closeScan();
        return;
      }
      onOperation(accepted.operation);
      let saved: ComputeSaveResult;
      try {
        saved = await invoke<ComputeSaveResult>('get_compute_save_result', { operation: accepted.operation });
      } catch {
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view) closeScan();
        return;
      }
      if (saved.disposition === 'pending') {
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view) closeScan();
        return;
      }
      if (!modelSaveCompleted(saved)) {
        if (alive.current && subscriptionView.current === view) setSubscriptionError({ code: saved.reason || `COMPUTE_SAVE_${saved.disposition.toLocaleUpperCase()}`, phase: 'save' });
        onRecoveryRefresh();
        return;
      }
      onChanged();
      if (!alive.current || subscriptionView.current !== view) return;
      const current = checksRef.current[selection.candidate.candidate_ref];
      if (current && alive.current && subscriptionView.current === view) {
        const result = await invoke<SubscriptionCheckResult>('get_subscription_check_result', {
          operation: current.approval_operation,
        });
        if (alive.current && subscriptionView.current === view) rememberCheck(selection.candidate.candidate_ref, result);
      }
      if (!alive.current || subscriptionView.current !== view) return;
      await Promise.all([refreshManagement(), refreshSubscriptions()]);
      if (!alive.current || subscriptionView.current !== view) return;
      onChanged();
      closeScan();
      setModelNotice(repairing
        ? (language === 'zh' ? '订阅授权已更新。' : 'Subscription access updated.')
        : (language === 'zh' ? '订阅已接入。' : 'Subscription connected.'));
    } catch (cause) {
      if (alive.current && subscriptionView.current === view) setSubscriptionError({ code: failureCode(cause), phase: 'save' });
      onRecoveryRefresh();
    } finally {
      if (alive.current && subscriptionView.current === view) {
        subscriptionInFlight.current = false;
        setSubscriptionBusy(false);
        setSubscriptionAction(null);
      }
    }
  }

  async function prepareDiscoveredConfiguration(item: ComputeScanItem) {
    if (discoveryInFlight.current || subscriptionInFlight.current) return;
    if (!trustedAuthority || !item.discovery) {
      setDiscoveryError('compute.discovery_not_importable');
      return;
    }
    const view = subscriptionView.current;
    const generation = ++discoveryGeneration.current;
    subscriptionAttempt.current += 1;
    discoveryInFlight.current = true;
    setSelectedSubscriptionRef(null);
    setSelectedDiscovery(item);
    setPreparedDiscovery(null);
    setDiscoverySelections([]);
    setDiscoveryError('');
    setSubscriptionNotice('');
    setDiscoveryAction('preparing');
    try {
      const candidate = await invoke<ComputeCandidateView>('prepare_discovered_model_connection', {
        discovery: item.discovery,
      });
      if (!alive.current || subscriptionView.current !== view || discoveryGeneration.current !== generation) return;
      setPreparedDiscovery(candidate);
      setDiscoverySelections(candidate.models.filter(model => model.selectable).map(model => model.model_ref));
    } catch (cause) {
      if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) {
        setDiscoveryError(failureCode(cause));
      }
    } finally {
      if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) {
        discoveryInFlight.current = false;
        setDiscoveryAction(null);
      }
    }
  }

  function returnToScanList() {
    discoveryGeneration.current += 1;
    discoveryInFlight.current = false;
    setSelectedDiscovery(null);
    setPreparedDiscovery(null);
    setDiscoverySelections([]);
    setDiscoveryAction(null);
    setDiscoveryError('');
  }

  function rescanDevice() {
    returnToScanList();
    setSelectedSubscriptionRef(null);
    setSubscriptionError(null);
    void refreshSubscriptions();
  }

  function toggleDiscoveryModel(modelRef: string) {
    setDiscoverySelections(current => current.includes(modelRef)
      ? current.filter(item => item !== modelRef)
      : [...current, modelRef]);
  }

  async function saveDiscoveredConfiguration() {
    if (!trustedAuthority || !preparedDiscovery || discoveryFailureNeedsRescan || discoveryInFlight.current || subscriptionInFlight.current) return;
    const selectable = new Set(preparedDiscovery.models.filter(model => model.selectable).map(model => model.model_ref));
    const selectedModelRefs = discoverySelections.filter(modelRef => selectable.has(modelRef));
    if (!selectedModelRefs.length || preparedDiscovery.fact_state !== 'complete'
      || ['missing', 'unavailable'].includes(preparedDiscovery.input_state)) return;
    const view = subscriptionView.current;
    const generation = discoveryGeneration.current;
    discoveryInFlight.current = true;
    setDiscoveryAction('saving');
    setDiscoveryError('');
    try {
      const currentManagement = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (!alive.current || subscriptionView.current !== view || discoveryGeneration.current !== generation) return;
      setManagement(currentManagement);
      const change: ManagementChange = {
        schema: 'hiroute.compute-management-change/v2',
        subject: { kind: 'candidate', candidate: preparedDiscovery.candidate },
        expected_revisions: currentManagement.revisions,
        selected_model_refs: selectedModelRefs,
        intent: 'save_ready',
        key_edits: [],
      };
      const preview = await invoke<SavePreview>('preview_compute_save', { change });
      if (!alive.current || subscriptionView.current !== view || discoveryGeneration.current !== generation) return;
      let accepted: { operation: OperationReference };
      try {
        accepted = await invoke<{ operation: OperationReference }>('apply_compute_save', {
          request: {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: clientIdempotencyKey('discovered-model-save'),
          },
          language,
        });
      } catch (cause) {
        if (saveFailureDefinitelyPreAdmission(cause) || saveFailureDefinitelyPreAdmission(failureCode(cause))) throw cause;
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) closeScan();
        return;
      }
      onOperation(accepted.operation);
      let saved: ComputeSaveResult;
      try {
        saved = await invoke<ComputeSaveResult>('get_compute_save_result', { operation: accepted.operation });
      } catch {
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) closeScan();
        return;
      }
      if (saved.disposition === 'pending') {
        onRecoveryRefresh();
        if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) closeScan();
        return;
      }
      if (!modelSaveCompleted(saved)) {
        if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) setDiscoveryError(saved.reason || `COMPUTE_SAVE_${saved.disposition.toLocaleUpperCase()}`);
        onRecoveryRefresh();
        return;
      }
      onChanged();
      if (!alive.current || subscriptionView.current !== view || discoveryGeneration.current !== generation) return;
      await Promise.all([refreshManagement(), refreshSubscriptions()]);
      if (!alive.current || subscriptionView.current !== view || discoveryGeneration.current !== generation) return;
      setFocusedSource(saved.source_id ?? null);
      closeScan();
      setModelNotice(language === 'zh' ? '模型接入已保存。' : 'Model connection saved.');
    } catch (cause) {
      if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) {
        setDiscoveryError(failureCode(cause));
      }
      onRecoveryRefresh();
    } finally {
      if (alive.current && subscriptionView.current === view && discoveryGeneration.current === generation) {
        discoveryInFlight.current = false;
        setDiscoveryAction(null);
      }
    }
  }

  useEffect(() => {
    alive.current = true;
    void Promise.all([refreshManagement(), refreshSubscriptions(), recoverSubscriptionCheck()]);
    return () => {
      alive.current = false;
      managementRequest.current += 1;
      subscriptionView.current += 1;
      subscriptionAttempt.current += 1;
      subscriptionScan.current += 1;
      subscriptionInFlight.current = false;
      discoveryInFlight.current = false;
      void invoke('close_subscription_check')
        .catch(() => undefined)
        .finally(onRecoveryRefresh);
    };
  }, []);

  useEffect(() => {
    const previous = wasActive.current;
    wasActive.current = active;
    if (previous && !active) {
      closeSubscriptionView();
    } else if (!previous && active) {
      void Promise.all([refreshSubscriptions(), recoverSubscriptionCheck()]);
    }
  }, [active]);

  useEffect(() => { if (refreshVersion > 0) void refreshManagement(); }, [refreshVersion]);

  const backend = useMemo<ModelConnectionBackend>(() => ({
    registerProtectedInput: secret => invoke<ProtectedInputRegistration>(
      'register_protected_model_input',
      { secret },
    ),
    releaseProtectedInput: input => invoke<void>('release_protected_model_input', { input }),
    listConnectionOptions: () => invoke('compute_connection_options'),
    checkModelConnection: request => invoke('check_model_connection', { request }),
    checkRegisteredModelConnection: request => invoke('check_registered_model_connection', { request }),
    cancelModelConnectionCheck: checkId => invoke('cancel_model_connection_check', { checkId }),
    previewComputeSave: async change => {
      // The editor may stay open across unrelated saves. Bind each new preview to
      // current revisions, just like the existing-source editor; Apply still CASes
      // this preview and the candidate retains its own source-revision binding.
      const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
      return invoke('preview_compute_save', {
        change: { ...change, expected_revisions: current.revisions },
      });
    },
    applyComputeSave: request => invoke('apply_compute_save', { request, language }),
    getComputeSaveResult: operation => invoke('get_compute_save_result', { operation }),
  }), [language]);

  function discardProtectedInputs() {
    const inputs = protectedInputs.current.splice(0);
    for (const input of inputs) {
      void backend.releaseProtectedInput(input).catch(() => undefined);
    }
  }

  async function prepareKeyInput(_sourceId: string, input: ProtectedKeyInput): Promise<CandidateRef> {
    const registration = await backend.registerProtectedInput(input.value);
    protectedInputs.current.push(registration.input_candidate);
    return registration.input_candidate;
  }

  async function applyExisting(preview: SavePreview): Promise<'saved' | 'uncertain'> {
    const request: ComputeConnectionApplyRequest = {
      spec: preview.spec as ComputeConnectionApplyRequest['spec'],
      accept_digest: preview.accept_digest as ComputeConnectionApplyRequest['accept_digest'],
      expected_revisions: preview.expected_revisions,
      idempotency_key: clientIdempotencyKey('model-save'),
    };
    let accepted: Awaited<ReturnType<ModelConnectionBackend['applyComputeSave']>>;
    try {
      accepted = await backend.applyComputeSave(request);
    } catch (cause) {
      if (saveFailureDefinitelyPreAdmission(cause) || saveFailureDefinitelyPreAdmission(failureCode(cause))) throw cause;
      // Native recovery now owns any protected inputs that may have crossed the
      // admission boundary. Closing prevents a retry with a fresh idempotency key.
      protectedInputs.current = [];
      onRecoveryRefresh();
      return 'uncertain';
    }
    protectedInputs.current = [];
    onOperation(accepted.operation);
    let result: ComputeSaveResult;
    try {
      result = await backend.getComputeSaveResult(accepted.operation);
    } catch {
      onRecoveryRefresh();
      return 'uncertain';
    }
    if (result.disposition === 'pending') {
      onRecoveryRefresh();
      return 'uncertain';
    }
    if (!modelSaveCompleted(result)) {
      throw { code: result.reason || `COMPUTE_SAVE_${result.disposition.toLocaleUpperCase()}` };
    }
    onChanged();
    return 'saved';
  }

  async function recheckSavedSource(
    renderedSource: ManagedSource,
    checkId: string,
    editRevision: number,
  ): Promise<'saved' | 'uncertain'> {
    if (!trustedAuthority) throw { code: 'TRUSTED_AUTHORITY_UNAVAILABLE' };
    const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
    const source = current.sources.find(item => item.source_id === renderedSource.source_id);
    if (!source || !source.actions.includes('recheck')) throw { code: 'RECHECK_CONTEXT_UNAVAILABLE' };
    const checked = await invoke<ModelConnectionCheckView>('check_saved_model_connection', {
      request: {
        source_id: source.source_id,
        expected_source_revision: source.revision,
        edit_revision: editRevision,
        check_id: checkId,
      },
    });
    if (!checkMatches(checked, { candidateRef: null, editRevision, checkId })) {
      throw { code: 'CHECK_CORRELATION_MISMATCH' };
    }
    if (checked.reachability === 'transport_failed') throw { code: 'MODEL_CONNECTION_TRANSPORT_FAILED' };
    if (checked.authentication === 'rejected') throw { code: 'MODEL_CONNECTION_AUTHENTICATION_REJECTED' };
    if (checked.directory === 'empty') throw { code: 'MODEL_DIRECTORY_EMPTY' };
    if (checked.directory === 'unavailable' || checked.directory === 'invalid') throw { code: 'MODEL_DIRECTORY_UNAVAILABLE' };
    const selectable = new Set(checked.candidate.models.filter(model => model.selectable).map(model => model.model_ref));
    const selectedModelRefs = source.models.map(model => model.model_ref);
    if (!selectedModelRefs.length || selectedModelRefs.some(modelRef => !selectable.has(modelRef))) {
      throw { code: 'SAVED_MODEL_SELECTION_CHANGED' };
    }
    const change: ManagementChange = {
      schema: 'hiroute.compute-management-change/v2',
      subject: { kind: 'candidate', candidate: checked.candidate.candidate },
      expected_revisions: current.revisions,
      selected_model_refs: selectedModelRefs,
      intent: source.state === 'disabled' ? 'save_disabled' : 'save_ready',
      key_edits: [],
      validation: checked.candidate.validation,
    };
    const preview = await invoke<SavePreview>('preview_compute_save', { change });
    const outcome = await applyExisting(preview);
    if (outcome !== 'uncertain') await refreshManagement();
    return outcome;
  }

  const connectionForm = adding && addStage === 'api' && management ? <ModelConnectionForm
        language={language}
        mutable={trustedAuthority}
        initialDraft={initialDraft.current}
        expectedRevisions={management.revisions}
        backend={backend}
        onBack={() => { setAddStage('choose'); setAddNotice(''); setReconnectingFrom(null); }}
        onCancel={() => { setAdding(false); setReconnectingFrom(null); }}
        onSaveAccepted={onOperation}
        onSaveResult={(result: ComputeSaveResult) => {
          if (modelSaveCompleted(result)) {
            setFocusedSource(result.source_id ?? null);
            setAdding(false);
            setAddStage('choose');
            if (reconnectingFrom) setModelNotice(language === 'zh'
              ? '新接入已保存；旧接入和已有路由未改变。请在路由中改选新模型，再停用旧接入。'
              : 'The new connection is saved. The old connection and routes are unchanged. Select the new model in routes, then disable the old connection.');
            setReconnectingFrom(null);
            requestAnimationFrame(() => addButton.current?.focus());
          }
          void refreshManagement();
          onChanged();
        }}
        onSaveUncertain={onRecoveryRefresh}
        restoreFocus={() => addButton.current?.focus()}
      /> : null;

  const text = language === 'zh'
    ? {
        title: '模型', detail: '管理已连接的模型与接入',
        add: '添加模型', loading: '正在读取模型来源…',
      }
    : {
        title: 'Models', detail: 'Manage connected models and connections',
        add: 'Add models', loading: 'Reading model sources…',
      };

  function openAdd() {
    if (!trustedAuthority) return;
    void requestEditorReplacement('models').then(allowed => {
      if (!allowed) return;
      setAddNotice('');
      setAddStage('choose');
      setAdding(true);
    });
  }

  function openConnection(kind: 'known' | 'custom' | 'free') {
    if (!trustedAuthority) return;
    setReconnectingFrom(null);
    initialDraft.current = newModelConnectionDraft(kind);
    setAddNotice('');
    setAddStage('api');
  }

  function reconnectSource(source: ManagedSource) {
    if (!trustedAuthority || source.provenance !== 'user_configured') return;
    const host = source.target.authority.includes(':') && !source.target.authority.startsWith('[')
      ? `[${source.target.authority}]`
      : source.target.authority;
    const draft = newModelConnectionDraft('custom');
    initialDraft.current = {
      ...draft,
      display_name: source.display_name,
      base_url: `${source.target.scheme}://${host}:${source.target.port}`,
      request_path_override: source.target.request_path,
      protocol: source.target.upstream_protocol as ModelConnectionDraft['protocol'],
      protocol_profile_id: `profile/custom/${source.target.upstream_protocol}`,
      authentication: source.authentication,
    };
    setReconnectingFrom(source.source_id);
    setAdding(true);
    setAddStage('api');
  }

  function openPrice(source: ManagedSource, model: ManagedModel) {
    const contexts: PriceContext[] = (model.presentation?.price_contexts ?? []).map(item => ({
      target_locator: { kind: 'binding', binding_id: model.binding_id },
      currency: item.currency,
      valuation_kind: item.valuation_kind,
    }));
    if (!contexts.length) return;
    setModelNotice('');
    setPriceTarget({
      source,
      model,
      contexts,
      // GetEffectivePrices returns the exact edit context, including a real
      // server-observed zero revision when no override exists yet.
      writable: true,
    });
  }

  const hasSources = Boolean(management?.sources.length);
  const selectedSubscription = subscriptions.find(item => item.candidate.candidate_ref === selectedSubscriptionRef) ?? null;
  const repairingSource = repairingSubscriptionSource
    ? management?.sources.find(source => source.source_id === repairingSubscriptionSource) ?? null
    : null;
  const repairingSelectedSubscription = Boolean(repairingSource
    && selectedSubscription?.existing_source_id === repairingSource.source_id);
  const selectedSubscriptionCheck = selectedSubscription
    ? subscriptionChecks[selectedSubscription.candidate.candidate_ref]
    : undefined;
  const validatedSubscription = selectedSubscription
    ? validatedSubscriptionCandidate(selectedSubscription, selectedSubscriptionCheck)
    : null;
  const effectiveSubscription = validatedSubscription ?? selectedSubscription;
  const subscriptionModels = effectiveSubscription?.models ?? [];
  const selectableSubscriptionModels = effectiveSubscription?.models.filter(model => model.selectable) ?? [];
  const selectedSubscriptionModels = new Set(effectiveSubscription
    ? (subscriptionSelections[effectiveSubscription.candidate.candidate_ref]
      ?? (repairingSelectedSubscription
        ? repairingSource?.models.map(model => model.model_ref) ?? []
        : selectableSubscriptionModels.map(model => model.model_ref)))
    : []);
  const subscriptionChecking = subscriptionAction === 'checking';
  const subscriptionWorking = subscriptionLoading || subscriptionChecking || subscriptionAction === 'saving';
  const subscriptionReady = Boolean(validatedSubscription);
  const selectableDiscoveryModels = preparedDiscovery?.models.filter(model => model.selectable) ?? [];
  const selectedDiscoveryModels = new Set(discoverySelections);
  const discoveryReady = Boolean(preparedDiscovery
    && preparedDiscovery.fact_state === 'complete'
    && !['missing', 'unavailable'].includes(preparedDiscovery.input_state)
    && selectableDiscoveryModels.length
    && discoverySelections.length);

  function toggleSubscriptionModel(modelRef: string) {
    if (!effectiveSubscription || repairingSelectedSubscription) return;
    const next = new Set(selectedSubscriptionModels);
    if (next.has(modelRef)) next.delete(modelRef); else next.add(modelRef);
    setSubscriptionSelections(current => ({
      ...current,
      [effectiveSubscription.candidate.candidate_ref]: [...next],
    }));
    setSubscriptionError(null);
  }

  function openSubscription(candidate: SubscriptionCandidate) {
    returnToScanList();
    setSelectedSubscriptionRef(candidate.candidate.candidate_ref);
    const repairingModelRefs = repairingSource
      && candidate.existing_source_id === repairingSource.source_id
      ? repairingSource.models.map(model => model.model_ref)
      : null;
    if (repairingModelRefs) {
      setSubscriptionSelections(current => ({
        ...current,
        [candidate.candidate.candidate_ref]: repairingModelRefs,
      }));
    }
    setSubscriptionError(null);
    setSubscriptionNotice('');
  }

  function repairSubscription(sourceId: string) {
    if (!trustedAuthority) return;
    setSelectedSubscriptionRef(null);
    setRepairingSubscriptionSource(sourceId);
    setSubscriptionError(null);
    setSubscriptionNotice('');
    setAddStage('scan');
    setAdding(true);
    void Promise.all([refreshSubscriptions(), recoverSubscriptionCheck()]);
  }

  function viewConnectedSubscription(candidate: SubscriptionCandidate) {
    if (candidate.existing_source_id) setFocusedSource(candidate.existing_source_id);
    setAdding(false);
    closeSubscriptionView();
  }

  function closeScan() {
    setAdding(false);
    closeSubscriptionView();
    requestAnimationFrame(() => addButton.current?.focus());
  }

  const subscriptionSaving = subscriptionAction === 'saving';
  const discoverySaving = discoveryAction === 'saving';
  const scanWorking = subscriptionWorking || discoveryAction !== null;
  const discoveryFailureNeedsRescan = ['compute.discovery_changed', 'compute.discovery_unavailable', 'revision_conflict', 'change_preview_stale']
    .includes(discoveryError.toLocaleLowerCase());
  const discoveryFailureCanRetry = Boolean(discoveryError
    && !['compute.discovery_not_importable', 'registered_option_unavailable'].includes(discoveryError.toLocaleLowerCase()));
  const repairSelectionValid = Boolean(repairingSelectedSubscription
    && effectiveSubscription?.validation
    && repairingSource
    && selectedSubscriptionModels.size === repairingSource.models.length
    && repairingSource.models.every(model => selectedSubscriptionModels.has(model.model_ref)));
  const scanFooter = scanWorking
    ? <button type="button" className="btn" disabled={subscriptionSaving || discoverySaving} onClick={closeScan}>{subscriptionSaving || discoverySaving
      ? (language === 'zh' ? '正在保存…' : 'Saving…')
      : subscriptionChecking
        ? (language === 'zh' ? '取消检查' : 'Cancel check')
        : (language === 'zh' ? '关闭' : 'Close')}</button>
    : selectedDiscovery
      ? <>
          <button type="button" className="btn" onClick={returnToScanList}>{language === 'zh' ? '返回' : 'Back'}</button>
          {discoveryFailureNeedsRescan
            ? <button type="button" className="btn btn-primary" disabled={!trustedAuthority} onClick={rescanDevice}>{language === 'zh' ? '重新扫描' : 'Scan again'}</button>
            : preparedDiscovery
            ? <button type="button" className="btn btn-primary" disabled={!trustedAuthority || !discoveryReady} onClick={() => void saveDiscoveredConfiguration()}>{language === 'zh' ? '保存接入' : 'Save connection'}</button>
            : discoveryFailureCanRetry && <button type="button" className="btn btn-primary" disabled={!trustedAuthority} onClick={() => discoveryFailureNeedsRescan ? rescanDevice() : void prepareDiscoveredConfiguration(selectedDiscovery)}>{discoveryFailureNeedsRescan ? (language === 'zh' ? '重新扫描' : 'Scan again') : (language === 'zh' ? '重试' : 'Try again')}</button>}
        </>
      : !selectedSubscription
      ? <>
          <button type="button" className="btn" onClick={closeScan}>{language === 'zh' ? '完成' : 'Done'}</button>
          {(subscriptionError?.phase === 'scan' || computeScanError || subscriptionDiscoveryState === 'runtime_unavailable') && <button type="button" className="btn btn-primary" onClick={rescanDevice}>{language === 'zh' ? '重新扫描' : 'Scan again'}</button>}
        </>
      : <>
          <button type="button" className="btn" onClick={() => { setSelectedSubscriptionRef(null); setSubscriptionError(null); }}>{language === 'zh' ? '返回' : 'Back'}</button>
          {subscriptionReady
            ? <button type="button" className="btn btn-primary" disabled={!trustedAuthority || subscriptionBusy || !effectiveSubscription || (repairingSelectedSubscription ? !repairSelectionValid : !canSaveSubscription(effectiveSubscription, true, selectedSubscriptionModels))} onClick={() => effectiveSubscription && void saveSubscription({ candidate: effectiveSubscription.candidate, validation: effectiveSubscription.validation, selected_model_refs: [...selectedSubscriptionModels], intent: saveIntent(true) })}>{repairingSelectedSubscription ? (language === 'zh' ? '更新订阅' : 'Update subscription') : (language === 'zh' ? '保存接入' : 'Save connection')}</button>
            : selectedSubscriptionCheck?.status === 'checking'
              ? <button type="button" className="btn btn-primary" disabled={subscriptionBusy} onClick={() => void recoverSubscriptionCheck()}>{language === 'zh' ? '重新查询检查结果' : 'Retry check status'}</button>
              : <button type="button" className="btn btn-primary" disabled={!trustedAuthority || subscriptionBusy || !selectedSubscription} onClick={() => selectedSubscription && void checkSubscription(selectedSubscription.candidate)}>{language === 'zh' ? '检查订阅' : 'Check subscription'}</button>}
        </>;

  return <ProductPage
    title={text.title}
    subtitle={text.detail}
    flush
    className="hr-models-page"
    actions={hasSources ? <button ref={addButton} className="btn btn-primary" disabled={!trustedAuthority || loading} onClick={openAdd}><UiIcon name="plus" />{text.add}</button> : undefined}
  >
    {connectionForm}
    {priceTarget && <PriceEditor
      modelName={priceTarget.model.display_name}
      sourceName={priceTarget.source.connection_identity?.product_label || priceTarget.source.display_name}
      contexts={priceTarget.contexts}
      language={language}
      writable={priceTarget.writable && trustedAuthority}
      onRefresh={async () => { await refreshManagement(); onChanged(); }}
      onOperation={(outcome: PriceOutcome) => { if (outcome.operation) onOperation(outcome.operation); }}
      onClose={() => setPriceTarget(null)}
      onSaved={() => { setPriceTarget(null); setModelNotice(language === 'zh' ? '价格已保存。' : 'Pricing saved.'); }}
    />}
    <Dialog
      open={adding && addStage === 'choose'}
      title={text.add}
      description={language === 'zh' ? '连接你已有的模型资源' : 'Connect your existing model resources'}
      closeLabel={language === 'zh' ? '关闭添加模型' : 'Close add models'}
      onClose={() => { setAdding(false); setAddNotice(''); }}
    >
      <div className="v3-add-choices">
        <button data-autofocus className="add-choice" type="button" disabled={!trustedAuthority} onClick={() => openConnection('known')}>
          <span className="choice-icon"><UiIcon name="plug" /></span>
          <div><strong>{language === 'zh' ? '添加 API' : 'Add API'}</strong><span>{language === 'zh' ? '选择已支持的服务，填写 Key 后选择模型' : 'Choose a supported service and connect with an API key'}</span></div>
          <UiIcon name="chevronRight" />
        </button>
        <button className="add-choice" type="button" disabled={!trustedAuthority} onClick={() => { returnToScanList(); setSelectedSubscriptionRef(null); setRepairingSubscriptionSource(null); setSubscriptionNotice(''); setAddStage('scan'); void Promise.all([refreshSubscriptions(), recoverSubscriptionCheck()]); }}>
          <span className="choice-icon"><UiIcon name="scan" /></span>
          <div><strong>{language === 'zh' ? '扫描本机' : 'Scan this device'}</strong><span>{language === 'zh' ? '发现已有的 Agent、Codex 订阅和模型配置' : 'Find local agents, Codex subscriptions, and model configurations'}</span></div>
          <UiIcon name="chevronRight" />
        </button>
        <button className="add-choice" type="button" disabled={!trustedAuthority} onClick={() => openConnection('free')}>
          <span className="choice-icon"><UiIcon name="sparkles" /></span><div><strong>{language === 'zh' ? '浏览免费模型' : 'Browse free models'}</strong><span>{language === 'zh' ? '查看免费条件和获取 Key 指引' : 'Explore free conditions and API key guides'}</span></div><UiIcon name="chevronRight" />
        </button>
      </div>
      <Disclosure className="oc-advanced" label={language === 'zh' ? '高级接入' : 'Advanced connection'} language={language}><p className="oc-meta">{language === 'zh' ? '使用自己的兼容服务；未知模型需要补充能力信息。' : 'Use a compatible endpoint. Unknown models require capability details.'}</p><button className="btn" type="button" disabled={!trustedAuthority} onClick={() => openConnection('custom')}>{language === 'zh' ? '自定义 API' : 'Custom API'}</button></Disclosure>
      {addNotice && <div className="callout" role="status">{addNotice}</div>}
    </Dialog>
    <Dialog open={adding && addStage === 'scan'} title={language === 'zh' ? '扫描本机' : 'Scan this device'} closeLabel={subscriptionSaving || discoverySaving ? (language === 'zh' ? '正在保存' : 'Saving') : (language === 'zh' ? '关闭扫描' : 'Close scan')} closeDisabled={subscriptionSaving || discoverySaving} onClose={closeScan} footer={scanFooter}>
      {scanWorking ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{discoveryAction === 'preparing'
        ? (language === 'zh' ? '正在读取配置' : 'Reading configuration')
        : discoveryAction === 'saving' || subscriptionAction === 'saving'
          ? (language === 'zh' ? '正在保存接入' : 'Saving connection')
          : subscriptionLoading
            ? (language === 'zh' ? '正在扫描本机' : 'Scanning this device')
            : (language === 'zh' ? '正在检查订阅' : 'Checking subscription')}</strong><p>{discoveryAction === 'preparing'
          ? (language === 'zh' ? '只读取这项本机配置，不会改动 Agent。' : 'This only reads the local configuration and does not change the agent.')
          : (language === 'zh' ? '完成后会在这里更新状态。' : 'The result will appear here.')}</p></div></div> : selectedDiscovery ? <div className="oc-scan-detail">
        <div className="oc-status-row"><BrandIcon kind={agentBrandFromId(selectedDiscovery.agent_id)} label={discoveredAgentName(selectedDiscovery.agent_id, language)} /><div className="row-main"><strong>{preparedDiscovery?.display_name || selectedDiscovery.observed_model_id || (language === 'zh' ? '已发现的模型配置' : 'Discovered model configuration')}</strong><p>{language === 'zh' ? `已从 ${discoveredAgentName(selectedDiscovery.agent_id, language)} 读取现有配置。保存后作为独立接入，不修改 Agent 当前配置。` : `Read from ${discoveredAgentName(selectedDiscovery.agent_id, language)}. Saving creates an independent connection without changing the agent configuration.`}</p></div>{discoveryReady && !discoveryFailureNeedsRescan && <span className="badge good">{language === 'zh' ? '可保存' : 'Ready to save'}</span>}</div>
        {preparedDiscovery && (selectableDiscoveryModels.length === 1
          ? <p className="oc-meta">{language === 'zh' ? '将接入模型：' : 'Model to connect: '} {selectableDiscoveryModels[0].display_name}</p>
          : selectableDiscoveryModels.length > 1
            ? <><p className="oc-meta">{language === 'zh' ? '选择要加入 HiRoute 的模型。' : 'Choose the models to add to HiRoute.'}</p><div className="v3-catalog">{selectableDiscoveryModels.map(model => <label className="check-row" key={model.model_ref}><input type="checkbox" disabled={!trustedAuthority} checked={selectedDiscoveryModels.has(model.model_ref)} onChange={() => toggleDiscoveryModel(model.model_ref)} /><div><strong>{model.display_name}</strong></div></label>)}</div></>
            : <p className="oc-meta">{language === 'zh' ? '这项配置中没有可接入的模型。' : 'No connectable model was found in this configuration.'}</p>)}
      </div> : selectedSubscription && effectiveSubscription ? <div className="oc-scan-detail">
        <div className="oc-status-row"><BrandIcon kind="codex" label="Codex" /><div className="row-main"><strong>{effectiveSubscription.display_name}</strong><p>{language === 'zh' ? '复用本机登录，无需复制订阅凭据。' : 'Reuse the local sign-in without copying credentials.'}</p></div>{subscriptionReady && <span className="badge good">{language === 'zh' ? '可用' : 'Ready'}</span>}</div>
        {subscriptionReady && repairingSelectedSubscription
          ? <p className="oc-meta">{language === 'zh' ? '将保留原有模型、绑定和路由，仅更新当前订阅授权与可执行资格。' : 'Existing models, bindings, and routes will be retained; only current subscription access and execution eligibility will be updated.'}</p>
          : subscriptionReady ? subscriptionModels.length === 1 && selectableSubscriptionModels.length === 1
          ? <p className="oc-meta">{language === 'zh' ? '检查通过，可接入以下模型：' : 'Check passed. Available model: '} {selectableSubscriptionModels[0].display_name}</p>
          : subscriptionModels.length
            ? <><p className="oc-meta">{language === 'zh' ? '已读取此账号可见的模型。选择资料完整、可接入路由的模型。' : 'Models visible to this account were loaded. Select models with complete routing data.'}</p><div className="v3-catalog">{subscriptionModels.map(model => <label className="check-row" key={model.model_ref}><input type="checkbox" disabled={!trustedAuthority || !model.selectable} checked={selectedSubscriptionModels.has(model.model_ref)} onChange={() => toggleSubscriptionModel(model.model_ref)} /><div><strong>{model.display_name}</strong>{!model.selectable && <span>{language === 'zh' ? '账号可见 · 能力资料待补充，暂不能用于路由' : 'Visible to this account · capability data pending; not yet routable'}</span>}</div></label>)}</div></>
            : <p className="oc-meta">{language === 'zh' ? '检查已完成，但没有可接入的模型。' : 'The check completed, but no connectable model was found.'}</p>
          : <p className="oc-meta">{language === 'zh' ? '检查会使用这项本机订阅验证连接；保存后才加入模型列表。' : 'The check uses this local subscription to verify the connection. Save it to add the model.'}</p>}
      </div> : <DeviceScanList
        language={language}
        subscriptions={subscriptions}
        computeItems={computeScanItems}
        agents={agents}
        subscriptionScanFailed={subscriptionError?.phase === 'scan'}
        subscriptionRuntimeUnavailable={subscriptionDiscoveryState === 'runtime_unavailable'}
        computeScanFailed={Boolean(computeScanError)}
        repairingSubscriptionSource={repairingSubscriptionSource}
        trustedAuthority={trustedAuthority}
        onOpenSubscription={openSubscription}
        onViewConnectedSubscription={viewConnectedSubscription}
        onOpenDiscoveredConfiguration={item => void prepareDiscoveredConfiguration(item)}
      />}
      {subscriptionNotice && !scanWorking && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{subscriptionNotice}</span></div></div>}
      {subscriptionError && subscriptionError.phase !== 'scan' && !scanWorking && <div className="callout bad" role="alert" data-error-code={subscriptionError.code}><UiIcon name="warning" /><span>{subscriptionFailureMessage(subscriptionError, language)}</span></div>}
      {discoveryError && !scanWorking && <div className="callout bad" role="alert" data-error-code={discoveryError}><UiIcon name="warning" /><span>{discoveryFailureMessage(discoveryError, language)}</span></div>}
    </Dialog>
    {(notice || modelNotice) && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{notice || modelNotice}</span></div></div>}
    {loading && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text.loading}</p></div></div>}
    {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{language === 'zh' ? '暂时无法读取模型，请重试。' : 'Models could not be loaded. Try again.'}</span><button className="btn" type="button" onClick={() => void refreshManagement()}>{language === 'zh' ? '重试' : 'Retry'}</button></div>}
    {management && !management.sources.length && !loading && !error && <div className="empty-state v3-empty"><div>
      <span className="empty-icon"><UiIcon name="models" /></span>
      <h3>{language === 'zh' ? '先连接一个模型' : 'Connect your first model'}</h3>
      <p>{language === 'zh' ? '连接一个 API，或复用本机已有的 Codex 订阅。' : 'Connect an API or reuse a local Codex subscription.'}</p>
      <button ref={hasSources ? undefined : addButton} className="btn btn-primary" disabled={!trustedAuthority} onClick={openAdd}><UiIcon name="plus" />{text.add}</button>
      {!trustedAuthority && <p className="oc-meta">{language === 'zh' ? '本机服务尚未就绪，连接恢复后即可添加。' : 'The local service is not ready. Reconnect before adding a model.'}</p>}
    </div></div>}
    {management && management.sources.length > 0 && <Models
      language={language}
      snapshot={management}
      initialSourceId={focusedSource}
      busy={loading}
      onRefresh={refreshManagement}
      onPrepareKeyInput={prepareKeyInput}
      onPreview={(change: ManagementChange) => invoke<SavePreview>('preview_compute_save', { change })}
      onApply={applyExisting}
      onDiscardProtectedInputs={discardProtectedInputs}
      onCredentialsSaved={() => setModelNotice(language === 'zh' ? '接入凭据已保存。' : 'Connection credentials saved.')}
      plans={plans}
      onOpenPlan={onOpenPlan}
      onCreatePlan={onCreatePlan}
      onEditPrice={openPrice}
      onReauthorize={repairSubscription}
      onRecheck={recheckSavedSource}
      onCancelRecheck={checkId => invoke<void>('cancel_model_connection_check', { checkId })}
      onReconnect={reconnectSource}
      onSourceStateSaved={enabled => setModelNotice(enabled
        ? language === 'zh' ? '接入已启用。' : 'Connection enabled.'
        : language === 'zh' ? '接入已停用。' : 'Connection disabled.')}
      mutable={trustedAuthority}
    />}
  </ProductPage>;
}
