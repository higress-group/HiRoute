import { NativeNetworkSettings, NativeNetworkImpact } from './NativeNetworkSettings';
import { manualProxyError, networkDraft, networkEdit, networkFailureText, networkPolicyEqual, networkRepairable, networkTarget, type NativeSourceNetwork } from './native-network';
import { SecretTransportWarning } from '../../host/ConsoleUi';
import { connectionName, connectionTemplate } from '../../ui/provider-identity';
import { ProviderIcon } from '../../ui/ProviderIcon';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke, isTauriHost } from '../../host/index.ts';

import { Dialog } from '../../ui/Dialog';
import { Disclosure } from '../../ui/Disclosure';
import { UiIcon } from '../../ui/UiIcon';
import { confirmDiscard, useDiscardGuard } from '../../ui/discard-guard';
import { saveFailureDefinitelyPreAdmission } from '../subscriptions/model';
import {
  blankModel,
  withCapabilityFallback,
  withRegisteredModels,
  buildCheckDraft,
  buildSaveChange,
  checkMatches,
  clientIdempotencyKey,
  clientOperationId,
  modelCanBeSelected,
  modelSaveCompleted,
  saveEligibility,
  selectedModelRefsForSave,
  selectedModelRefsAfterRecheck,
} from './state';
import type {
  ComputeCandidateRef,
  ComputeConnectionOptions,
  ComputeConnectionApplyRequest,
  ComputeSavePreview,
  ModelConnectionCheckView,
  ModelConnectionDraft,
  ModelConnectionEndpointDraft,
  ModelConnectionFormProps,
  ModelDeclaration,
  ProviderMetadataRecord,
  UpstreamProtocol,
} from './types';
import { checkFailureCode, modelAvailabilityMessage } from './copy';
import { metadataModelPrefill } from './metadata-prefill';
import { customApiPrefillOptions, metadataProviderCandidates, metadataProviderOption, registeredModelCandidates, sortRegisteredOptions, templateEndpointForProtocol } from './registered-metadata';
import { ConnectionDirectory } from './ConnectionDirectory';
import { ConnectionEditingSession, type ConnectionSelection } from './connection-session';
import './connection-flow.css';
import {
  connectionErrorMessage,
  PlaintextHttpWarning,
  ManualModelEditor,
  manualModelError,
  manualModelIdError,
  safeConnectionErrorCode,
  validEndpoint,
} from './form-support';

type Phase = 'editing' | 'checking' | 'saving';
type Screen = 'catalog' | 'connection' | 'manual' | 'models' | 'network_review';
type OptionsRecoveryOwner = { checkId: string; productKey: string | null; editRevision: number };

function emptyAdditionalEndpoint(protocol: UpstreamProtocol, authentication: ModelConnectionDraft['authentication']): ModelConnectionEndpointDraft {
  return {
    base_url: '', base_kind: 'api_root', request_path_override: null,
    inventory_path_override: null, protocol,
    protocol_profile_id: `profile/custom/${protocol}`, protocol_profile_revision: 1,
    authentication,
  };
}

export function ModelConnectionForm(props: ModelConnectionFormProps) {
  const { backend, language } = props;
  const zh = language === 'zh';
  const [networkSource, setNetworkSource] = useState<NativeSourceNetwork | null>(null);
  const [networkReady, setNetworkReady] = useState(!props.initialDraft.existing_source_id);
  const [networkLoadError, setNetworkLoadError] = useState(false);
  const [networkLoadAttempt, setNetworkLoadAttempt] = useState(0);
  const [networkReveal, setNetworkReveal] = useState(0);
  const [networkReview, setNetworkReview] = useState<{ preview: ComputeSavePreview; checked: ModelConnectionCheckView; returnScreen: 'models' | 'manual' } | null>(null);
  const [custom, setCustom] = useState(props.initialDraft.entry_kind === 'custom_api');
  const free = props.initialDraft.entry_kind === 'free_api_key';
  const catalogEnabled = props.initialDraft.entry_kind !== 'custom_api' && !props.initialDraft.existing_source_id;
  const [draft, setDraft] = useState<ModelConnectionDraft>(() => ({ ...props.initialDraft,
    display_name: props.initialDraft.display_name || (catalogEnabled ? '' : zh ? '自定义 API' : 'Custom API'),
  }));
  const [screen, setScreen] = useState<Screen>(catalogEnabled ? 'catalog' : 'connection');
  const [phase, setPhase] = useState<Phase>('editing');
  const [result, setResult] = useState<ModelConnectionCheckView | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState('');
  const [externalLinkError, setExternalLinkError] = useState(false);
  const [connectionOptions, setConnectionOptions] = useState<ComputeConnectionOptions | null>(null);
  const [optionsLoading, setOptionsLoading] = useState(true);
  const [optionsReady, setOptionsReady] = useState(false);
  const [optionsError, setOptionsError] = useState('');
  const [registeredOptionId, setRegisteredOptionId] = useState('');
  const [metadataPrefillKey, setMetadataPrefillKey] = useState('');
  const [credentialReentry, setCredentialReentry] = useState(false);
  const credentialReentryRef = useRef(false);
  const password = useRef<HTMLInputElement>(null);
  const errorFeedback = useRef<HTMLDivElement>(null);
  const draftRef = useRef(draft);
  // Only saved, catalog-prefilled or backend-observed IDs bypass the temporary
  // manual ASCII policy. Editing a draft never adds its input to this set.
  const preservedModelIds = useRef(new Set(props.initialDraft.existing_source_id
    ? props.initialDraft.models.map(model => model.upstream_model_id) : []));
  const protectedInputRef = useRef<ComputeCandidateRef | null>(null);
  const activeCheckRef = useRef<string | null>(null);
  const applyDispatchedRef = useRef(false);
  const aliveRef = useRef(true);
  const saveCompletedRef = useRef(false);
  const editingSession = useRef(new ConnectionEditingSession());
  const editingKey = useRef<string | null>(catalogEnabled ? null : 'custom');
  if (!catalogEnabled) editingSession.current.begin('custom', draft);
  const selectionRef = useRef<ConnectionSelection | null>(null);
  const shortcutDecided = useRef(false);
  const optionsGeneration = useRef(0);
  const optionsRecovery = useRef<OptionsRecoveryOwner | null>(null);
  const backendRef = useRef(backend);
  backendRef.current = backend;
  const selectedOptionRef = useRef<ComputeConnectionOptions['options'][number] | null>(null);
  const pendingSecret = useRef<string | null>(null);
  const flow = useRef<HTMLElement>(null);
  const registeredOptions = useMemo(() => sortRegisteredOptions(connectionOptions?.options.filter(option =>
    option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free')
  ) ?? []), [connectionOptions, free]);
  const registeredOption = registeredOptions.find(option => option.connection_option_id === registeredOptionId)
    ?? null;
  const registeredExecutable = Boolean(optionsReady && registeredOption?.registered_check_available);
  const namedOption = registeredOption ?? selectedOptionRef.current;
  const productName = namedOption ? connectionName(namedOption.connection_option_id, language, namedOption.display_name) : '';
  const registeredCandidates = useMemo(() => registeredOption
    ? registeredModelCandidates(registeredOption, connectionOptions?.metadata_catalog)
    : [], [registeredOption, connectionOptions]);
  const metadataProviders: ProviderMetadataRecord[] = []; // Broad metadata remains internal; only native templates are selectable.
  const metadataPrefills = useMemo(() => customApiPrefillOptions(registeredOptions, metadataProviders), [registeredOptions, metadataProviders]);
  const selectedPrefill = metadataPrefills.find(prefill => prefill.key === metadataPrefillKey) ?? null;
  const metadataCandidates = useMemo(() => selectedPrefill?.kind === 'template'
    ? registeredModelCandidates(selectedPrefill.option, connectionOptions?.metadata_catalog)
    : selectedPrefill?.kind === 'provider' && connectionOptions?.metadata_catalog
      ? metadataProviderCandidates(selectedPrefill.provider, connectionOptions.metadata_catalog, registeredOptions)
      : [], [selectedPrefill, connectionOptions, registeredOptions]);
  function hasUnsavedChanges() {
    return editingSession.current.changed(editingKey.current, draftRef.current, custom ? metadataPrefillKey : undefined)
      || Boolean(password.current?.value || pendingSecret.current || protectedInputRef.current || credentialReentryRef.current || selectionRef.current?.refs.size)
      || editingSession.current.hasUnsavedDrafts;
  }
  useDiscardGuard('models', () => phase === 'saving' || (!saveCompletedRef.current && hasUnsavedChanges()), language, confirmReplacement);

  function markCredentialReentry(value: boolean) {
    credentialReentryRef.current = value;
    setCredentialReentry(value);
  }

  function suggestedName(base: string) {
    const names = new Set(props.existingNames ?? []);
    let value = base;
    for (let suffix = 2; names.has(value); suffix++) value = `${base} · ${suffix}`;
    return value;
  }

  const releaseProtectedInput = useCallback(() => {
    const input = protectedInputRef.current;
    protectedInputRef.current = null;
    if (input && !applyDispatchedRef.current) void backendRef.current.releaseProtectedInput(input).catch(() => undefined);
  }, []);
  const releaseSession = useCallback(() => {
    editingSession.current.clear(input => { void backendRef.current.releaseProtectedInput(input).catch(() => undefined); });
    pendingSecret.current = null;
    if (password.current) password.current.value = '';
  }, []);

  const cancelOptionsRecovery = useCallback(() => {
    if (!optionsRecovery.current) return;
    optionsRecovery.current = null;
    optionsGeneration.current++;
    setOptionsLoading(false);
    setOptionsReady(false);
    setOptionsError('MODEL_CONNECTION_OPTIONS_UNAVAILABLE');
  }, []);

  const loadConnectionOptions = useCallback(async (owner?: OptionsRecoveryOwner) => {
    if (!owner && (activeCheckRef.current || optionsRecovery.current)) {
      const checkId = activeCheckRef.current;
      activeCheckRef.current = null;
      if (checkId) void backendRef.current.cancelModelConnectionCheck(checkId).catch(() => undefined);
      cancelOptionsRecovery();
      setPhase('editing');
    }
    const generation = ++optionsGeneration.current;
    const current = () => aliveRef.current && generation === optionsGeneration.current
      && (!owner || (optionsRecovery.current === owner && activeCheckRef.current === owner.checkId
        && editingKey.current === owner.productKey && draftRef.current.edit_revision === owner.editRevision));
    setOptionsLoading(true);
    setOptionsReady(false);
    setOptionsError('');
    try {
      const options = await backend.listConnectionOptions();
      if (!current()) return false;
      setConnectionOptions(options);
      setOptionsReady(true);
      const executable = sortRegisteredOptions(options.options.filter(option => option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free')));
      if (!custom && executable.length === 0) {
        setOptionsError('MODEL_CONNECTION_OPTION_UNAVAILABLE');
      }
      return true;
    } catch {
      if (!current()) return false;
      if (!custom) setOptionsError('MODEL_CONNECTION_OPTIONS_UNAVAILABLE');
      return true;
    } finally {
      if (current()) setOptionsLoading(false);
    }
  }, [backend, custom, free, cancelOptionsRecovery]);

  function selectMetadataProvider(provider: ProviderMetadataRecord | null, credentialContextChanged = false) {
    if (!provider) return;
    const catalog = connectionOptions?.metadata_catalog;
    const option = catalog ? metadataProviderOption(provider, registeredOptions) : null;
    const endpoint = option?.endpoints?.find(value => provider.base_url_candidates.some(base =>
      `${value.base_url.replace(/\/+$/, '')}/${value.request_path.replace(/^\/+/, '')}`
        .startsWith(`${base.replace(/\/+$/, '')}/`))
      && provider.protocol_candidates.includes(value.protocol));
    const protocol = endpoint?.protocol ?? (provider.protocol_candidates.includes(draftRef.current.protocol)
      ? draftRef.current.protocol : provider.protocol_candidates[0]);
    const candidates = catalog ? metadataProviderCandidates(provider, catalog, registeredOptions) : [];
    for (const candidate of candidates) preservedModelIds.current.add(candidate.upstream_model_id);
    const records = new Map(catalog?.model_records.filter(model =>
      model.provider_record_key === provider.provider_record_key
      && model.usable_for.includes('custom-api-model-prefill')
      && model.execution_fit.state === 'native_text_representable')
      .map(model => [model.upstream_model_id, model] as const) ?? []);
    const otherProtocols = new Set<UpstreamProtocol>();
    const additional_endpoints = (option?.endpoints ?? [])
      .filter(value => value.protocol !== protocol && !otherProtocols.has(value.protocol) && Boolean(otherProtocols.add(value.protocol)))
      .map(value => ({ base_url: value.base_url, base_kind: 'api_root' as const,
        request_path_override: value.request_path, inventory_path_override: value.inventory_path ?? null,
        protocol: value.protocol, protocol_profile_id: `profile/custom/${value.protocol}`,
        protocol_profile_revision: 1, authentication: value.authentication_semantics ?? { kind: 'bearer' as const } }));
    const withProvider = (current: ModelConnectionDraft): ModelConnectionDraft => ({
      ...current,
      display_name: current.display_name && current.display_name !== (zh ? '自定义 API' : 'Custom API') ? current.display_name : provider.display_name,
      display_template_id: null,
      base_url: endpoint?.base_url ?? provider.base_url_candidates[0],
      base_kind: 'api_root',
      request_path_override: endpoint?.request_path ?? null,
      inventory_path_override: endpoint?.inventory_path ?? null,
      protocol,
      protocol_profile_id: `profile/custom/${protocol}`,
      protocol_profile_revision: current.protocol_profile_revision + 1,
      authentication: endpoint?.authentication_semantics ?? (protocol === 'messages'
        ? { kind: 'api_key_header', header: 'x-api-key' }
        : { kind: 'bearer' }),
      additional_endpoints,
      models: candidates.map(candidate => {
        const record = records.get(candidate.upstream_model_id);
        if (candidate.capability_prefill) return {
          ...blankModel(candidate.upstream_model_id, candidate.display_name),
          capabilities: candidate.capability_prefill,
        };
        return record ? withCapabilityFallback(metadataModelPrefill(record, clientOperationId('model')))
          : blankModel(candidate.upstream_model_id, candidate.display_name);
      }),
    });
    if (credentialContextChanged) editingSession.current.beginPrefillFromContext(editingKey.current, metadataPrefillKey, `provider:${provider.provider_record_key}`, withProvider);
    invalidate(withProvider, true, 'connection', { credentialContextChanged });
  }

  function selectRegisteredTemplate(option: ComputeConnectionOptions['options'][number], keepExistingModels = false, credentialContextChanged = false) {
    const endpoint = [...(option.endpoints ?? [])].sort((a, b) => a.stable_preference - b.stable_preference)[0];
    if (!endpoint) return;
    const candidates = registeredModelCandidates(option, connectionOptions?.metadata_catalog);
    for (const candidate of candidates) preservedModelIds.current.add(candidate.upstream_model_id);
    const otherProtocols = new Set<UpstreamProtocol>();
    const additional_endpoints = [...(option.endpoints ?? [])]
      .sort((a, b) => a.stable_preference - b.stable_preference)
      .filter(other => other.protocol !== endpoint.protocol && !otherProtocols.has(other.protocol) && Boolean(otherProtocols.add(other.protocol)))
      .map(other => ({ base_url: other.base_url, base_kind: 'api_root' as const,
        request_path_override: other.request_path, inventory_path_override: other.inventory_path ?? null,
        protocol: other.protocol, protocol_profile_id: `profile/custom/${other.protocol}`,
        protocol_profile_revision: 1, authentication: other.authentication_semantics ?? { kind: 'bearer' as const } }));
    const prefillKey = `template:${option.connection_option_id}`;
    const withTemplate = (current: ModelConnectionDraft): ModelConnectionDraft => ({ ...current,
      entry_kind: 'custom_api', display_template_id: option.connection_option_id,
      display_name: current.display_name.trim() || suggestedName(connectionName(option.connection_option_id, language, option.display_name)),
      base_url: endpoint.base_url, base_kind: 'api_root',
      request_path_override: endpoint.request_path, inventory_path_override: endpoint.inventory_path ?? null,
      protocol: endpoint.protocol, protocol_profile_id: `profile/custom/${endpoint.protocol}`,
      protocol_profile_revision: current.protocol_profile_revision + 1,
      authentication: endpoint.authentication_semantics ?? { kind: 'bearer' },
      additional_endpoints, models: withRegisteredModels(keepExistingModels ? current.models : [], candidates),
    });
    editingSession.current.beginPrefillFromContext(editingKey.current, custom ? metadataPrefillKey : undefined, prefillKey, withTemplate);
    setCustom(true);
    setMetadataPrefillKey(prefillKey);
    invalidate(withTemplate, true, 'connection', { credentialContextChanged });
  }

  function selectMetadataPrefill(key: string) {
    if (key === metadataPrefillKey) return;
    // A prefill can point at another product. Park this context with its own
    // credential rather than restoring a hidden Key into the new template.
    const hadProtectedInput = Boolean(protectedInputRef.current);
    editingSession.current.stashPrefill(editingKey.current, metadataPrefillKey, {
      draft: draftRef.current, custom, metadataPrefillKey,
      secret: password.current?.value || pendingSecret.current || '', protectedInput: null,
      credentialReentry: credentialReentryRef.current || hadProtectedInput, selection: selectionRef.current,
    });
    releaseProtectedInput();
    if (password.current) password.current.value = '';
    pendingSecret.current = null;
    selectionRef.current = null;
    markCredentialReentry(false);
    const saved = editingSession.current.takePrefill(editingKey.current, key);
    setMetadataPrefillKey(key);
    if (saved) {
      setCustom(saved.custom);
      pendingSecret.current = saved.secret;
      selectionRef.current = saved.selection;
      markCredentialReentry(saved.credentialReentry);
      invalidate(() => saved.draft, true, 'connection', { credentialContextChanged: true });
      return;
    }
    const prefill = metadataPrefills.find(value => value.key === key);
    if (prefill?.kind === 'template') selectRegisteredTemplate(prefill.option, false, true);
    else if (prefill?.kind === 'provider') selectMetadataProvider(prefill.provider, true);
    else if (key === '') editingSession.current.beginPrefillFromContext(editingKey.current, metadataPrefillKey, key, current => current);
  }

  useEffect(() => { draftRef.current = draft; }, [draft]);
  useLayoutEffect(() => {
    if (custom) editingSession.current.beginPrefill(editingKey.current, metadataPrefillKey, draftRef.current);
    if (screen === 'connection' && phase === 'editing' && pendingSecret.current !== null && password.current) {
      password.current.value = pendingSecret.current;
      pendingSecret.current = null;
    }
  }, [screen, phase, registeredOptionId, custom, metadataPrefillKey, draft.authentication.kind]);
  useEffect(() => {
    if (phase !== 'editing') return;
    const frame = requestAnimationFrame(() => {
      const visible = (element: HTMLElement) => element.getClientRects().length > 0 && !element.matches(':disabled');
      const target = Array.from(flow.current?.querySelectorAll<HTMLElement>('[data-autofocus]') ?? []).find(visible)
        ?? Array.from(flow.current?.querySelectorAll<HTMLElement>('input, button, summary') ?? []).find(visible);
      target?.focus();
      flow.current?.closest('.modal-body')?.scrollTo({ top: 0 });
    });
    return () => cancelAnimationFrame(frame);
  }, [screen, phase]);
  useEffect(() => {
    if (!error || phase !== 'editing') return;
    requestAnimationFrame(() => errorFeedback.current?.scrollIntoView({ block: 'nearest' }));
  }, [error, phase, screen]);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      optionsGeneration.current++;
      optionsRecovery.current = null;
      const checkId = activeCheckRef.current;
      if (checkId) void backendRef.current.cancelModelConnectionCheck(checkId).catch(() => undefined);
      releaseProtectedInput();
      releaseSession();
    };
  }, [releaseProtectedInput, releaseSession]);
  useEffect(() => { void loadConnectionOptions(); }, [loadConnectionOptions]);
  useEffect(() => {
    const sourceId = props.initialDraft.existing_source_id;
    if (!sourceId) return;
    let current = true; setNetworkReady(false); setNetworkLoadError(false);
    void invoke<NativeSourceNetwork>('native_source_network', { input: { source_id: sourceId } }).then(view => {
      if (!current) return;
      if (view.source_revision !== props.initialDraft.expected_source_revision) throw Error('SOURCE_CHANGED');
      const network = networkDraft(view.policy, view.network_revision);
      setNetworkSource(view);
      editingSession.current.applyNetworkBaseline(editingKey.current, network);
      const next = { ...draftRef.current, network }; draftRef.current = next; setDraft(next); setNetworkReady(true);
    }).catch(() => { if (current) setNetworkLoadError(true); });
    return () => { current = false; };
  }, [props.initialDraft.existing_source_id, props.initialDraft.expected_source_revision, networkLoadAttempt]);


  const invalidate = useCallback((
    update: (current: ModelConnectionDraft) => ModelConnectionDraft,
    securityChanged = false,
    nextScreen?: Screen,
    options?: { credentialContextChanged?: boolean },
  ) => {
    const current = draftRef.current;
    const updated = update(current);
    // Product/prefill changes park credentials separately. Within one context,
    // capture the uncontrolled input before any transition can unmount it.
    if (!options?.credentialContextChanged && current.authentication.kind !== 'none'
      && updated.authentication.kind === 'none' && password.current) pendingSecret.current = password.current.value;
    const checkId = activeCheckRef.current;
    activeCheckRef.current = null;
    if (checkId) void backend.cancelModelConnectionCheck(checkId).catch(() => undefined);
    cancelOptionsRecovery();
    if (securityChanged) {
      if (protectedInputRef.current) markCredentialReentry(true);
      releaseProtectedInput();
    }
    const nextRevision = current.edit_revision + 1;
    const next = securityChanged
      ? {
          ...updated,
          candidate_ref: null,
          provenance: updated.provenance.kind === 'registered'
            ? { kind: 'user_configured' as const, configuration_revision: nextRevision }
            : updated.provenance,
          qualification: { free_access: null, evidence_ref: null },
        }
      : updated;
    const versioned = { ...next, edit_revision: nextRevision, check_id: '' };
    draftRef.current = versioned;
    setDraft(versioned);
    setResult(null);
    setSelected(new Set());
    setPhase('editing');
    setError('');
    if (nextScreen) setScreen(nextScreen);
  }, [backend, releaseProtectedInput, cancelOptionsRecovery]);

  function retainEditingDraft() {
    if (!editingKey.current) return;
    editingSession.current.stash(editingKey.current, {
      draft: draftRef.current, custom, metadataPrefillKey,
      secret: password.current?.value || pendingSecret.current || '', protectedInput: protectedInputRef.current,
      credentialReentry: credentialReentryRef.current, selection: selectionRef.current,
    });
    protectedInputRef.current = null;
    if (password.current) password.current.value = '';
    pendingSecret.current = null;
    markCredentialReentry(false);
    selectionRef.current = null;
    editingKey.current = null;
  }

  function chooseConnection(option: ComputeConnectionOptions['options'][number] | null) {
    if (activeCheckRef.current || optionsRecovery.current) cancelActiveCheck();
    setNetworkReveal(0);
    shortcutDecided.current = true;
    const key = option?.connection_option_id ?? 'custom';
    retainEditingDraft();
    const saved = editingSession.current.take(key);
    const endpoint = [...(option?.endpoints ?? [])].sort((a, b) => a.stable_preference - b.stable_preference)[0];
    const next = saved?.draft ?? { ...props.initialDraft,
      entry_kind: option ? (free ? 'free_api_key' as const : 'preset_api' as const) : 'custom_api' as const,
      display_name: suggestedName(option ? connectionName(option.connection_option_id, language, option.display_name) : (zh ? '自定义 API' : 'Custom API')),
      lineage_ref: clientOperationId('lineage'), candidate_ref: null, check_id: '', models: [],
      authentication: endpoint?.authentication_semantics ?? { kind: 'bearer' as const },
    };
    editingSession.current.begin(key, next);
    setCustom(saved?.custom ?? !option);
    setRegisteredOptionId(option?.connection_option_id ?? '');
    selectedOptionRef.current = option;
    setMetadataPrefillKey(saved?.metadataPrefillKey ?? '');
    protectedInputRef.current = saved?.protectedInput ?? null;
    pendingSecret.current = saved?.secret ?? '';
    markCredentialReentry(saved?.credentialReentry ?? false);
    selectionRef.current = saved?.selection ?? null;
    editingKey.current = key;
    draftRef.current = next;
    setDraft(next);
    setResult(null);
    setSelected(new Set());
    setPhase('editing');
    setError('');
    setScreen('connection');
  }

  function returnToCatalog() {
    if (activeCheckRef.current || optionsRecovery.current) cancelActiveCheck();
    shortcutDecided.current = true;
    retainEditingDraft();
    setResult(null);
    setSelected(new Set());
    setError('');
    setScreen('catalog');
  }

  useEffect(() => {
    if (!props.singleProductShortcut || !catalogEnabled || free || props.appendTo || props.savedSource || screen !== 'catalog' || shortcutDecided.current || optionsLoading || !optionsReady) return;
    const executable = registeredOptions.filter(option => option.registered_check_available);
    if (executable.length === 1) chooseConnection(executable[0]);
  }, [props.singleProductShortcut, props.appendTo, props.savedSource, catalogEnabled, free, screen, optionsLoading, optionsReady, registeredOptions]);

  function updateManualModel(model: ModelDeclaration) {
    invalidate(current => ({ ...current, models: current.models.map(value => value.client_id === model.client_id ? model : value) }), false, 'manual');
  }

  async function saveChecked(checked: ModelConnectionCheckView, selectedRefs: string[], returnScreen: 'models' | 'manual' = 'models') {
    if (!props.mutable) {
      setError('TRUSTED_AUTHORITY_UNAVAILABLE');
      return;
    }
    const eligibility = saveEligibility(checked, true);
    if (!eligibility.allowed || !selectedRefs.length) {
      setError(selectedRefs.length ? 'MODEL_CONNECTION_FAILED' : 'MODEL_SELECTION_REQUIRED');
      setPhase('editing');
      return;
    }
    // Saving this product also leaves the editor. Other product drafts still
    // belong to this session and must not be silently discarded on success.
    const unusedCredential = draftRef.current.authentication.kind === 'none'
      && Boolean(pendingSecret.current || credentialReentryRef.current);
    if ((editingSession.current.hasUnsavedDrafts || unusedCredential) && !(await confirmDiscard(language))) {
      setPhase('editing');
      return;
    }
    setPhase('saving');
    try {
      const change = buildSaveChange({
        result: checked,
        expectedRevisions: props.expectedRevisions,
        selectedModelRefs: selectedRefs,
        enable: (props.appendTo ?? props.savedSource)?.state !== 'disabled',
        protectedInput: protectedInputRef.current,
      });
      if (props.appendTo) { change.edit = { action: 'append_models' }; change.key_edits = []; }
      else if (draftRef.current.display_name.trim()) change.edit = { action: 'rename', display_name: draftRef.current.display_name.trim() };
      const preview = await backend.previewComputeSave(change, networkEdit(draftRef.current.network));
      if (!aliveRef.current) {
        releaseProtectedInput();
        return;
      }
      if (networkSource && !networkPolicyEqual(networkSource.policy, networkEdit(draftRef.current.network).policy)) {
        setNetworkReview({ preview, checked, returnScreen }); setPhase('editing'); setScreen('network_review'); return;
      }
      await applyPreview(preview, checked);
    } catch (caught) {
      if (!aliveRef.current) return;
      setError(safeConnectionErrorCode(caught));
      setPhase('editing');
    }
  }

  async function applyPreview(preview: ComputeSavePreview, checked: ModelConnectionCheckView) {
    if (applyDispatchedRef.current) return;
    const idempotencyKey = clientIdempotencyKey('model-connection-save');
    const request: ComputeConnectionApplyRequest = {
      spec: preview.spec,
      accept_digest: preview.accept_digest,
      expected_revisions: preview.expected_revisions,
      idempotency_key: idempotencyKey,
    };
    applyDispatchedRef.current = true;
    try {
      const accepted = await backend.applyComputeSave(request);
      protectedInputRef.current = null;
      props.onSaveAccepted(accepted.operation);
      try {
        const saved = await backend.getComputeSaveResult(accepted.operation);
        if (modelSaveCompleted(saved)) {
          saveCompletedRef.current = true;
        } else if (saved.disposition === 'pending') {
          finishUncertainSave(idempotencyKey, checked.candidate.candidate);
          return;
        } else {
          applyDispatchedRef.current = false;
          if (aliveRef.current) {
            // A terminal Operation has consumed/released its protected input and
            // validation. The old checked candidate cannot be saved again.
            invalidate(current => ({ ...current, candidate_ref: null }), true, 'connection');
            setError('MODEL_SAVE_RECHECK_REQUIRED');
          }
        }
        props.onSaveResult(saved);
      } catch {
        finishUncertainSave(idempotencyKey, checked.candidate.candidate);
      }
    } catch (caught) {
      const code = safeConnectionErrorCode(caught);
      if (saveFailureDefinitelyPreAdmission(caught) || saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(caught, true))) {
        applyDispatchedRef.current = false;
        if (aliveRef.current) {
          setPhase('editing');
          setError(code);
        }
      } else {
        finishUncertainSave(idempotencyKey, checked.candidate.candidate);
      }
    }
  }

  function finishUncertainSave(idempotencyKey: string, candidate: ComputeCandidateRef) {
    // Once apply may have been admitted, a fresh retry could duplicate the
    // mutation. Hand observation back to the app shell and leave this editor.
    saveCompletedRef.current = true;
    props.onSaveUncertain({ idempotency_key: idempotencyKey, candidate });
    props.onCancel();
    props.restoreFocus();
  }

  async function runCheck(saveAfterCheck = false, inferenceModelId: string | null = null) {
    if (phase !== 'editing') return;
    if (!props.mutable) {
      setError('TRUSTED_AUTHORITY_UNAVAILABLE');
      return;
    }
    setError('');
    const base = !custom && !saveAfterCheck
      ? { ...draftRef.current, models: withRegisteredModels(draftRef.current.models, registeredCandidates) }
      : draftRef.current;
    const automaticInferenceModel = inferenceModelId && !base.models.some(model => model.upstream_model_id === inferenceModelId)
      ? inferenceModelId : null;
    const selectedModels = automaticInferenceModel
      ? [...base.models, blankModel(automaticInferenceModel)] : base.models;
    const current = { ...base, inference_model_id: inferenceModelId, models: selectedModels };
    // The check needs complete execution defaults, but the editable draft must
    // retain the user's declarations (including an explicit unknown value).
    const checkedModels = selectedModels.map(withCapabilityFallback);
    if (current.display_name.trim().length > 60 || (!current.existing_source_id && (props.existingNames ?? []).includes(current.display_name.trim()))) { setError('CONNECTION_NAME_CONFLICT'); return; }
    const allowedIds = new Set([...preservedModelIds.current,
      ...(!custom ? registeredCandidates.map(model => model.upstream_model_id) : [])]);
    // Check every route to the backend, including returning to connection check
    // from the manual editor, before registering credentials or sending inference.
    const idError = current.models.filter(model => model.upstream_model_id !== '')
      .map(model => manualModelIdError(model.upstream_model_id, allowedIds)).find(Boolean);
    if (idError) { setError(idError); return; }
    const passwordElement = password.current;
    if (!networkReady || manualProxyError(current.network ?? networkDraft())) { setScreen('connection'); setNetworkReveal(v => v + 1); return; }
    const enteredSecret = passwordElement?.value ?? '';
    if (!current.display_name.trim() || new TextEncoder().encode(current.display_name.trim()).length > 256 || current.display_name.includes('\0')) {
      setError('MODEL_CONNECTION_NAME_REQUIRED');
      return;
    }
    const connectionReady = custom
      ? validEndpoint(current.base_url)
      : Boolean(connectionOptions && registeredExecutable);
    if (!connectionReady) {
      setError(custom ? 'MODEL_CONNECTION_FIELDS_REQUIRED' : 'MODEL_CONNECTION_OPTION_UNAVAILABLE');
      return;
    }
    if (current.authentication.kind !== 'none' && !enteredSecret && !protectedInputRef.current && !current.existing_source_id) {
      setError('MODEL_CONNECTION_KEY_REQUIRED');
      return;
    }
    if (saveAfterCheck) {
      const validation = (new Set(checkedModels.map(model => model.upstream_model_id)).size !== checkedModels.length ? 'MODEL_ID_DUPLICATE' : '') || checkedModels.map(model => manualModelError(model, allowedIds)).find(Boolean) || (!checkedModels.length ? 'MODEL_ID_REQUIRED' : '');
      if (validation) { setError(validation); return; }
    }
    if (custom && automaticInferenceModel) {
      // Record only the model inserted by this check, never an existing user declaration.
      editingSession.current.recordAutomaticModel(editingKey.current, metadataPrefillKey, selectedModels[selectedModels.length - 1]);
    } else if (!custom) editingSession.current.applyModelDefaults(editingKey.current, models => {
      const withCatalog = !saveAfterCheck ? withRegisteredModels(models, registeredCandidates) : models;
      return automaticInferenceModel && !withCatalog.some(model => model.upstream_model_id === automaticInferenceModel)
        ? [...withCatalog, blankModel(automaticInferenceModel)] : withCatalog;
    });
    const checkId = clientOperationId('check');
    const checkedProductKey = editingKey.current;
    const checkingDraft = { ...current, display_name: current.display_name.trim(), candidate_ref: enteredSecret ? null : current.candidate_ref, check_id: checkId };
    draftRef.current = checkingDraft;
    setDraft(checkingDraft);
    activeCheckRef.current = checkId;
    setResult(null);
    setSelected(new Set());
    // A manual capability declaration must be checked again before it can be
    // saved. Keep that check cancellable; only lock the dialog after the
    // checked candidate crosses into the save phase.
    setPhase('checking');
    let newlyRegistered: ComputeCandidateRef | null = null;
    let inputOwnedBySession = false;
    try {
      if (checkingDraft.authentication.kind !== 'none' && enteredSecret) {
        newlyRegistered = (await backend.registerProtectedInput(enteredSecret)).input_candidate;
        if (activeCheckRef.current !== checkId) {
          await backend.releaseProtectedInput(newlyRegistered);
          return;
        }
        if (passwordElement) passwordElement.value = '';
        releaseProtectedInput();
        protectedInputRef.current = newlyRegistered;
        inputOwnedBySession = true;
        markCredentialReentry(false);
      }
      const inputCandidate = checkingDraft.authentication.kind === 'none' ? null : protectedInputRef.current;
      const checked = custom
        ? await backend.checkModelConnection({
            draft: buildCheckDraft({ ...checkingDraft, models: checkedModels }),
            ...(inputCandidate ? { input_candidate: inputCandidate } : {}),
          }, networkEdit(checkingDraft.network))
        : await backend.checkRegisteredModelConnection({
            display_name: checkingDraft.display_name,
            inference_model_id: inferenceModelId,
            models: checkedModels.filter(model => model.upstream_model_id).map(({ client_id: _clientId, ...model }) => ({ ...model, display_name: model.display_name.trim() || model.upstream_model_id })),
            connection_option_id: registeredOption!.connection_option_id,
            expected_catalog: connectionOptions!.catalog,
            candidate_ref: checkingDraft.candidate_ref,
            lineage_ref: checkingDraft.lineage_ref,
            edit_revision: checkingDraft.edit_revision,
            check_id: checkingDraft.check_id,
            input_candidate: inputCandidate!,
            existing_source_id: checkingDraft.existing_source_id,
            expected_source_revision: checkingDraft.expected_source_revision ?? null,
          }, networkEdit(checkingDraft.network));
      if (!aliveRef.current || activeCheckRef.current !== checkId || draftRef.current.edit_revision !== checkingDraft.edit_revision) return;
      if (!checkMatches(checked, { candidateRef: checkingDraft.candidate_ref, editRevision: checkingDraft.edit_revision, checkId })) {
        activeCheckRef.current = null;
        setError('CHECK_CORRELATION_MISMATCH');
        setPhase('editing');
        return;
      }
      activeCheckRef.current = null;
      const acceptedDraft = { ...draftRef.current, candidate_ref: checked.candidate.candidate.candidate_ref };
      draftRef.current = acceptedDraft;
      setDraft(acceptedDraft);
      setResult(checked);
      if (checked.failure && checked.failure.phase !== 'directory') {
        setPhase('editing'); setScreen(checked.failure.phase === 'inference' ? 'manual' : 'connection');
        if (networkRepairable(checked.failure)) setNetworkReveal(v => v + 1);
        return;
      }
      const checkFailure = checkFailureCode(checked);
      if (checkFailure) {
        setError(checkFailure);
        setPhase('editing');
        return;
      }
      const retained = selectionRef.current;
      const nextSelection = selectedModelRefsAfterRecheck(retained?.result ?? null, retained?.refs ?? new Set(), checked);
      setSelected(nextSelection);
      selectionRef.current = { result: checked, refs: nextSelection };
      for (const model of checked.candidate.models) preservedModelIds.current.add(model.upstream_model_id);
      const selectable = checked.candidate.models.filter(model => model.selectable);
      if (saveAfterCheck) {
        await saveChecked(checked, selectable.filter(model => checkingDraft.models.some(declared => declared.upstream_model_id === model.upstream_model_id)).map(model => model.model_ref), 'manual');
        return;
      }
      if (custom && selectable.length === 0) {
        const observed = checked.candidate.models[0];
        const existing = acceptedDraft.models.find(model =>
          model.upstream_model_id === observed?.upstream_model_id
        );
        const initialManualModel = existing ?? blankModel(observed?.upstream_model_id, observed?.display_name);
        if (!acceptedDraft.models.length) {
          editingSession.current.recordAutoPlaceholder(editingKey.current, initialManualModel);
          editingSession.current.recordAutomaticModel(editingKey.current, metadataPrefillKey, initialManualModel);
        }
        const manualDraft = {
          ...acceptedDraft,
          edit_revision: acceptedDraft.edit_revision + 1,
          check_id: '',
          models: acceptedDraft.models.length ? acceptedDraft.models : [initialManualModel],
        };
        draftRef.current = manualDraft;
        setDraft(manualDraft);
        setResult(null);
        setScreen('manual');
        setPhase('editing');
        return;
      }
      if (!checked.candidate.models.length) {
        setScreen('manual');
        setPhase('editing');
        return;
      }
      // An inventory check starts unselected; a tool recheck retains only the
      // user's existing choices that remain selectable in this exact candidate.
      setScreen('models');
      setPhase('editing');
    } catch (caught) {
      if (aliveRef.current && activeCheckRef.current === checkId && editingKey.current === checkedProductKey
        && draftRef.current.edit_revision === checkingDraft.edit_revision) {
        const code = safeConnectionErrorCode(caught);
        if (!custom && code === 'compute.registered_catalog_changed') {
          const owner = { checkId, productKey: checkedProductKey, editRevision: checkingDraft.edit_revision };
          optionsRecovery.current = owner;
          if (!await loadConnectionOptions(owner)) return;
          optionsRecovery.current = null;
        }
        if (!aliveRef.current || activeCheckRef.current !== checkId || editingKey.current !== checkedProductKey
          || draftRef.current.edit_revision !== checkingDraft.edit_revision) return;
        activeCheckRef.current = null;
        setError(code);
        if (draftRef.current.network?.mode === 'manual' && ['INVALID_ARGUMENTS', 'application.error.invalid_arguments'].includes(code)) { setScreen('connection'); setNetworkReveal(v => v + 1); }
        setPhase('editing');
      } else if (newlyRegistered && !inputOwnedBySession) {
        await backend.releaseProtectedInput(newlyRegistered).catch(() => undefined);
      }
    }
  }

  function cancelActiveCheck() {
    const checkId = activeCheckRef.current;
    activeCheckRef.current = null;
    if (checkId) void backend.cancelModelConnectionCheck(checkId).catch(() => undefined);
    cancelOptionsRecovery();
    setPhase('editing');
    setError('');
    if (screen === 'models') setScreen('connection');
  }

  function discardAndClose(kind: 'back' | 'cancel') {
    // Once a save starts it may already have crossed the native admission
    // boundary. Keep the dialog mounted until the operation reaches a state we
    // can represent instead of making a Close button look like cancellation.
    if (phase === 'saving') return;
    if (phase !== 'editing') {
      cancelActiveCheck();
    }
    releaseProtectedInput();
    releaseSession();
    if (kind === 'back') props.onBack(); else props.onCancel();
    props.restoreFocus();
  }

  async function confirmReplacement() {
    if (phase === 'saving') return false;
    if (!saveCompletedRef.current && hasUnsavedChanges() && !(await confirmDiscard(language))) return false;
    discardAndClose('cancel');
    return true;
  }

  async function requestClose(kind: 'back' | 'cancel') {
    if (phase === 'saving') return;
    if (!saveCompletedRef.current && hasUnsavedChanges() && !(await confirmDiscard(language))) return;
    discardAndClose(kind);
  }

  async function openDocumentation(event: React.MouseEvent<HTMLAnchorElement>, url?: string) {
    if (!isTauriHost()) return;
    event.preventDefault();
    setExternalLinkError(false);
    if (!url) {
      setExternalLinkError(true);
      return;
    }
    try {
      await invoke('open_external_url', { url });
    } catch {
      setExternalLinkError(true);
    }
  }

  function backWithinFlow() {
    setError('');
    setResult(null);
    setSelected(new Set());
    setScreen('connection');
  }

  function openManual() {
    const blank = draftRef.current.models.length ? null : blankModel();
    if (blank) {
      editingSession.current.recordAutoPlaceholder(editingKey.current, blank);
    }
    invalidate(current => ({ ...current, models: current.models.length ? current.models : blank ? [blank] : [] }), false, 'manual');
  }

  async function saveSelected() {
    if (!props.mutable) { setError('TRUSTED_AUTHORITY_UNAVAILABLE'); return; }
    if (!result) return;
    const refs = selectedModelRefsForSave(result, selected, true);
    if (!refs.length) { setError('MODEL_SELECTION_REQUIRED'); return; }
    await saveChecked(result, refs);
  }

  const selectedCount = useMemo(() => result ? selectedModelRefsForSave(result, selected, true).length : 0, [result, selected]);
  const busy = phase !== 'editing';
  const optionUnavailable = error === 'MODEL_CONNECTION_OPTIONS_UNAVAILABLE' || error === 'MODEL_CONNECTION_OPTION_UNAVAILABLE';
  const title = props.appendTo ? (zh ? `向「${props.appendTo.display_name}」添加模型` : `Add models to “${props.appendTo.display_name}”`) : zh ? '连接 API' : 'Connect an API';
  const step = screen === 'catalog' ? 0 : screen === 'connection' ? 1 : 2;
  const footer = networkReview && screen === 'network_review' && !busy
    ? <><button className="btn" type="button" onClick={() => { setNetworkReview(null); setScreen(networkReview.returnScreen); }}>{zh ? '返回修改' : 'Back'}</button><button className="btn btn-primary" type="button" onClick={() => { const review = networkReview; setNetworkReview(null); setPhase('saving'); void applyPreview(review.preview, review.checked); }}>{zh ? '确认并保存' : 'Confirm and save'}</button></>
    : busy
    ? <button className="btn" type="button" disabled={phase === 'saving'} onClick={cancelActiveCheck}>{phase === 'saving' ? (zh ? '正在保存…' : 'Saving…') : (zh ? '取消检查' : 'Cancel check')}</button>
    : screen === 'catalog'
      ? <><button className="btn" type="button" onClick={() => void requestClose('back')}>{zh ? '返回' : 'Back'}</button><span className="field-help">{zh ? '选择服务后继续配置' : 'Choose a service to continue'}</span></>
      : screen === 'connection'
      ? <><button className="btn" type="button" onClick={() => catalogEnabled ? returnToCatalog() : void requestClose('back')}>{catalogEnabled && props.singleProductShortcut ? (zh ? '更换服务' : 'Change service') : (zh ? '返回' : 'Back')}</button><button className="btn btn-primary" type="submit" form="mc-connection-form" disabled={!props.mutable || !networkReady || Boolean(manualProxyError(draft.network ?? networkDraft())) || (!custom && (optionsLoading || !registeredExecutable))}>{zh ? '检查并继续' : 'Check and continue'}</button></>
      : screen === 'manual'
        ? <><button className="btn" type="button" onClick={backWithinFlow}>{zh ? '返回' : 'Back'}</button><button className="btn btn-primary" type="submit" form="mc-manual-form" disabled={!props.mutable}>{zh ? '检查并保存' : 'Check and save'}</button></>
        : <><button className="btn" type="button" onClick={backWithinFlow}>{zh ? '返回' : 'Back'}</button><span className="connection-selection-count">{zh ? `已选择 ${selectedCount} 个模型` : `${selectedCount} models selected`}</span><button className="btn btn-primary" type="button" disabled={!props.mutable || !selectedCount} onClick={() => void saveSelected()}>{zh ? '保存连接' : 'Save connection'}</button></>;

  return <Dialog open title={title} className="connection-dialog" closeLabel={phase === 'saving' ? (zh ? '正在保存' : 'Saving') : (zh ? '取消添加' : 'Cancel')} closeDisabled={phase === 'saving'} onClose={() => void requestClose('cancel')} footer={footer}>
    <section className="connection-flow" ref={flow} aria-label={zh ? '添加模型连接' : 'Add model connection'}>
      <ol className="connection-steps" aria-label={zh ? '连接步骤' : 'Connection steps'}>{(zh ? ['选择服务', '配置连接', '选择模型'] : ['Choose service', 'Configure', 'Choose models']).map((label, index) => <li key={label} className={index < step ? 'complete' : index === step ? 'current' : ''} aria-current={index === step ? 'step' : undefined}><span>{index < step ? <UiIcon name="check" /> : index + 1}</span>{label}</li>)}</ol>
      {props.connectionChoice && <fieldset className="connection-source-choice" disabled={busy || !props.mutable}>{props.connectionChoice}</fieldset>}
      {props.appendTo && <p className="field-help">{zh ? '复用此接入的端点和凭据。检查后添加模型，已有模型与启停状态将保留。' : 'Reuse saved endpoints and credentials. Add models after checking; existing models and enabled state are retained.'}</p>}
      {!props.mutable && !busy && <div className="callout warn" role="status"><UiIcon name="warning" /><span>{zh ? '本机服务连接中断。当前输入已保留，重新连接后再保存。' : 'The local service disconnected. Your input is retained; reconnect before saving.'}</span></div>}
      {busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{phase === 'checking' ? (zh ? '正在检查接入' : 'Checking connection') : (zh ? '正在保存接入' : 'Saving connection')}</strong><p>{phase === 'checking' ? (zh ? '检查配置，并在提供目录时读取模型。' : 'Checking configuration and reading the model directory when configured.') : (zh ? '保存所选模型与凭据。' : 'Saving the selected models and credential.')}</p></div></div>}
      <div className="connection-screen" hidden={busy}>
        {screen === 'catalog' && <ConnectionDirectory options={registeredOptions} loading={optionsLoading} language={language} free={free} onSelect={chooseConnection} onCustom={props.allowCustom === false ? undefined : () => chooseConnection(null)} onReload={() => void loadConnectionOptions()} onInteraction={() => { shortcutDecided.current = true; }} />}
        <form hidden={screen !== 'connection'} id="mc-connection-form" onSubmit={event => { event.preventDefault(); void runCheck(false); }}>
          <fieldset className="connection-fields" disabled={!props.mutable || Boolean(props.appendTo)}>
            <div className="connection-config-heading"><ProviderIcon optionId={custom ? draft.display_template_id : registeredOption?.connection_option_id} language={language} /><div><h3>{custom ? (zh ? '自定义 API' : 'Custom API') : productName}</h3><p>{custom ? (zh ? '填写服务地址，连接兼容的模型服务' : 'Connect a compatible model service') : connectionTemplate(registeredOption?.connection_option_id)?.description[language]}</p></div></div>
            <label className="field"><span className="field-label">{zh ? '连接名称' : 'Connection name'}</span><input className="input" maxLength={60} data-autofocus={screen === 'connection' || undefined} value={draft.display_name} placeholder={zh ? '例如：团队模型服务' : 'e.g. Team models'} onChange={event => invalidate(current => ({ ...current, display_name: event.target.value }), false, 'connection')} /><span className="field-help">{zh ? '用于区分不同账号或用途，保存后可在模型列表中看到。' : 'Distinguish accounts or purposes. This name appears in your model list.'}</span></label>
            {draft.authentication.kind !== 'none' && <label className="field"><SecretTransportWarning language={props.language} /><span className="field-label">API Key</span><input className="input" disabled={!custom && (optionsLoading || !registeredOption)} ref={password} type="password" autoComplete="off" spellCheck={false} placeholder={protectedInputRef.current ? '••••••••••••••••' : draft.existing_source_id ? (zh ? '留空使用已保存的 Key' : 'Leave blank to use the saved key') : (zh ? '粘贴此产品的 API Key' : 'Paste this product’s API key')} onChange={() => invalidate(current => ({ ...current, candidate_ref: null }), true, 'connection')} /></label>}
            {credentialReentry && !password.current?.value && !pendingSecret.current && <p className="field-help" role="status">{zh ? '使用需要认证的方式时，请重新输入之前的 Key。' : 'Re-enter the previous key when using authentication.'}</p>}
            {!custom && registeredOption && <div className="connection-key-help"><span className="field-help">{zh ? '请使用上方产品对应的 Key。' : 'Use the key for the product shown above.'}</span>{connectionTemplate(registeredOption.connection_option_id)?.documentation_url && <a href={connectionTemplate(registeredOption.connection_option_id)?.documentation_url} target="_blank" rel="noreferrer" onClick={event => void openDocumentation(event, connectionTemplate(registeredOption.connection_option_id)?.documentation_url)}>{zh ? '获取 API Key' : 'Get an API key'}<UiIcon name="arrow" /></a>}{externalLinkError && <div className="callout warn" role="alert"><UiIcon name="warning" /><span>{zh ? '无法打开默认浏览器；当前表单内容已保留。' : 'The default browser could not be opened. Your form input is retained.'}</span></div>}</div>}
            {custom ? <>
              {!optionsLoading && metadataPrefills.length > 0 && <Disclosure className="connection-details" language={language} label={zh ? '使用接入资料预填（可选）' : 'Use connection prefill (optional)'}><div className="connection-fields metadata-prefill-fields">
                <label className="field"><span className="field-label">{zh ? '接入资料预填（可选）' : 'Connection prefill (optional)'}</span><select className="select" value={metadataPrefillKey} onChange={event => selectMetadataPrefill(event.target.value)}>
                  <option value="">{zh ? '不使用预填' : 'No prefill'}</option>
                  <optgroup label={zh ? '内置接入' : 'Built-in connections'}>
                    {metadataPrefills.filter(prefill => prefill.kind === 'template').map(prefill => <option key={prefill.key} value={prefill.key}>{connectionName(prefill.option.connection_option_id, language, prefill.option.display_name)}</option>)}
                  </optgroup>

                </select></label>
                {selectedPrefill && <p className="field-help">{zh
                  ? `已收录 ${metadataCandidates.length} 个模型，检查接入后统一选择；未知能力采用可编辑兜底：上下文 200K、输出 32K、工具与流式开启、视觉关闭、思考开关。`
                  : `${metadataCandidates.length} models in this catalog. Choose after checking the connection; missing capabilities use editable defaults (200K context, 32K output, tools and streaming on, vision off, reasoning toggle).`}</p>}
              </div></Disclosure>}
              <label className="field"><span className="field-label">Base URL</span><input className="input" inputMode="url" value={draft.base_url} placeholder="https://example.com/v1" onChange={event => invalidate(current => ({ ...current, base_url: event.target.value }), true, 'connection')} /><PlaintextHttpWarning baseUrl={draft.base_url} zh={zh} /></label>
              <label className="field"><span className="field-label">{zh ? '协议' : 'Protocol'}</span><select className="select" value={draft.protocol} onChange={event => {
                const protocol = event.target.value as UpstreamProtocol;
                invalidate(current => {
                  const template = registeredOptions.find(option => option.connection_option_id === current.display_template_id) ?? null;
                  const endpoint = templateEndpointForProtocol(current, template, protocol);
                  const existingIndex = current.additional_endpoints.findIndex(value => value.protocol === protocol);
                  const existing = current.additional_endpoints[existingIndex];
                  const previousPrimary: ModelConnectionEndpointDraft = {
                    base_url: current.base_url,
                    base_kind: current.base_kind,
                    request_path_override: current.request_path_override,
                    inventory_path_override: current.inventory_path_override,
                    protocol: current.protocol,
                    protocol_profile_id: current.protocol_profile_id,
                    protocol_profile_revision: current.protocol_profile_revision,
                    authentication: current.authentication,
                  };
                  return {
                    ...current,
                    base_url: existing?.base_url ?? endpoint?.base_url ?? current.base_url,
                    base_kind: existing?.base_kind ?? current.base_kind,
                    request_path_override: existing?.request_path_override ?? endpoint?.request_path ?? current.request_path_override,
                    inventory_path_override: existing?.inventory_path_override ?? (endpoint ? endpoint.inventory_path ?? null : current.inventory_path_override),
                    authentication: existing?.authentication ?? endpoint?.authentication_semantics ?? current.authentication,
                    protocol,
                    protocol_profile_id: existing?.protocol_profile_id ?? `profile/custom/${protocol}`,
                    protocol_profile_revision: (existing?.protocol_profile_revision ?? current.protocol_profile_revision) + 1,
                    additional_endpoints: existingIndex < 0 ? current.additional_endpoints : current.additional_endpoints.map((value, index) => index === existingIndex ? previousPrimary : value),
                  };
                }, true, 'connection');
              }}><option value="chat_completions">OpenAI Chat Completions</option><option value="responses">OpenAI Responses</option><option value="messages">Anthropic Messages</option></select></label>
            </> : optionsLoading
              ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{zh ? '正在读取接入方式' : 'Loading connection'}</strong><p>{zh ? '从本机可信目录确认当前支持的产品。' : 'Checking supported products in the local trusted catalog.'}</p></div></div>
              : registeredExecutable && registeredOption
                ? <Disclosure className="connection-details" language={language} label={zh ? '连接配置 · 已自动填写' : 'Connection settings · configured automatically'}><div className="connection-endpoint-list">{registeredOption.endpoints?.map(endpoint => <div key={`${endpoint.protocol}-${endpoint.base_url}-${endpoint.request_path}`}><strong>{({ chat_completions: 'OpenAI Chat Completions', responses: 'OpenAI Responses', messages: 'Anthropic Messages' })[endpoint.protocol]}</strong><code>{endpoint.base_url.replace(/\/+$/, '')}/{endpoint.request_path.replace(/^\/+/, '')}</code></div>)}{props.allowCustom !== false && Boolean(registeredOption.endpoints?.length) && <button className="btn btn-quiet" type="button" onClick={() => selectRegisteredTemplate(registeredOption, true)}>{zh ? '自定义连接配置' : 'Customize connection settings'}</button>}</div></Disclosure>
                : <div className="callout warn"><UiIcon name="warning" /><div><strong>{registeredOptionId ? (zh ? '当前选择的产品暂不可用' : 'The selected product is unavailable') : (zh ? '当前没有可用的内置接入' : 'No built-in connection is available')}</strong><p>{optionsError === 'MODEL_CONNECTION_OPTIONS_UNAVAILABLE' ? connectionErrorMessage(optionsError, zh) : registeredOptionId ? (zh ? '请重新读取，或更换服务后继续。' : 'Reload or change service to continue.') : connectionErrorMessage(optionUnavailable ? error : 'MODEL_CONNECTION_OPTION_UNAVAILABLE', zh)}</p><button className="btn" type="button" onClick={() => void loadConnectionOptions()}>{zh ? '重新读取' : 'Reload'}</button></div></div>}
            {custom && <Disclosure className="connection-details" language={language} label={zh ? '高级设置' : 'Advanced settings'}><div className="connection-fields">
              <label className="field"><span className="field-label">{zh ? '请求路径（可选覆盖）' : 'Request path (optional override)'}</span><input className="input" value={draft.request_path_override ?? ''} onChange={event => invalidate(current => ({ ...current, request_path_override: event.target.value || null }), true, 'connection')} /></label>
              <label className="field"><span className="field-label">{zh ? '模型目录路径（可选）' : 'Model directory path (optional)'}</span><input className="input" placeholder="/v1/models" value={draft.inventory_path_override ?? ''} onChange={event => invalidate(current => ({ ...current, inventory_path_override: event.target.value || null }), true, 'connection')} /></label>
              <label className="field"><span className="field-label">{zh ? '认证方式' : 'Authentication'}</span><select className="select" value={draft.authentication.kind} onChange={event => invalidate(current => ({ ...current, authentication: event.target.value === 'api_key_header' ? { kind: 'api_key_header', header: 'x-api-key' } : event.target.value === 'none' ? { kind: 'none' } : { kind: 'bearer' } }), true, 'connection')}><option value="none">{zh ? '无认证' : 'None'}</option><option value="bearer">Bearer Token</option><option value="api_key_header">{zh ? '自定义 Header' : 'Custom header'}</option></select></label>
              {draft.authentication.kind === 'api_key_header' && <label className="field"><span className="field-label">Header</span><input className="input" value={draft.authentication.header} onChange={event => invalidate(current => ({ ...current, authentication: { kind: 'api_key_header', header: event.target.value } }), true, 'connection')} /></label>}
              <div className="connection-fields" aria-label={zh ? '其他协议端点' : 'Additional protocol endpoints'}>
                {draft.additional_endpoints.map((endpoint, index) => {
                  const updateEndpoint = (change: Partial<ModelConnectionEndpointDraft>) => invalidate(current => ({
                    ...current,
                    additional_endpoints: current.additional_endpoints.map((value, position) => position === index ? { ...value, ...change } : value),
                  }), true, 'connection');
                  return <section key={`${index}-${endpoint.protocol}`} className="connection-fields">
                    <div className="detail-section-head"><strong>{zh ? '端点' : 'Endpoint'} {index + 2}</strong><button className="btn btn-quiet" type="button" onClick={() => invalidate(current => ({ ...current, additional_endpoints: current.additional_endpoints.filter((_, position) => position !== index) }), true, 'connection')}>{zh ? '移除' : 'Remove'}</button></div>
                    <label className="field"><span className="field-label">{zh ? '协议' : 'Protocol'}</span><select className="select" value={endpoint.protocol} onChange={event => { const protocol = event.target.value as UpstreamProtocol; updateEndpoint({ protocol, protocol_profile_id: `profile/custom/${protocol}`, protocol_profile_revision: endpoint.protocol_profile_revision + 1 }); }}><option value="chat_completions">OpenAI Chat Completions</option><option value="responses">OpenAI Responses</option><option value="messages">Anthropic Messages</option></select></label>
                    <label className="field"><span className="field-label">Base URL</span><input className="input" inputMode="url" value={endpoint.base_url} onChange={event => updateEndpoint({ base_url: event.target.value })} /><PlaintextHttpWarning baseUrl={endpoint.base_url} zh={zh} /></label>
                    <label className="field"><span className="field-label">{zh ? '请求路径' : 'Request path'}</span><input className="input" value={endpoint.request_path_override ?? ''} onChange={event => updateEndpoint({ request_path_override: event.target.value || null })} /></label>
                    <label className="field"><span className="field-label">{zh ? '认证方式' : 'Authentication'}</span><select className="select" value={endpoint.authentication.kind} onChange={event => updateEndpoint({ authentication: event.target.value === 'api_key_header' ? { kind: 'api_key_header', header: 'x-api-key' } : event.target.value === 'none' ? { kind: 'none' } : { kind: 'bearer' } })}><option value="none">{zh ? '无认证' : 'None'}</option><option value="bearer">Bearer Token</option><option value="api_key_header">{zh ? '自定义 Header' : 'Custom header'}</option></select></label>
                    {endpoint.authentication.kind === 'api_key_header' && <label className="field"><span className="field-label">Header</span><input className="input" value={endpoint.authentication.header} onChange={event => updateEndpoint({ authentication: { kind: 'api_key_header', header: event.target.value } })} /></label>}
                  </section>;
                })}
                {draft.additional_endpoints.length < 2 && <button className="btn btn-quiet" type="button" onClick={() => {
                  const used = new Set([draft.protocol, ...draft.additional_endpoints.map(endpoint => endpoint.protocol)]);
                  const protocol = (['responses', 'messages', 'chat_completions'] as UpstreamProtocol[]).find(value => !used.has(value));
                  if (protocol) invalidate(current => ({ ...current, additional_endpoints: [...current.additional_endpoints, emptyAdditionalEndpoint(protocol, current.authentication)] }), true, 'connection');
                }}>{zh ? '添加协议端点' : 'Add protocol endpoint'}</button>}
                <p className="field-help">{zh ? '这些端点与上方端点共用同一个 API Key；修改端点无需重新输入已保存的 Key。' : 'All endpoints share one API key. Editing endpoints does not require re-entering a saved key.'}</p>
              </div>
            </div></Disclosure>}
            {!custom && <p className="field-help">{zh ? '检查连接配置；提供目录时读取模型，不发送推理请求。' : 'Checks connection settings and reads the model directory when available, without inference requests.'}</p>}
            <button className="btn btn-quiet connection-manual-entry" type="button" onClick={openManual}>{zh ? '手工维护模型' : 'Edit models manually'}</button>
          </fieldset>
          {networkLoadError ? <div className="callout warn" role="alert">{zh ? '无法读取已保存的网络设置，请重试。' : 'Saved network settings could not be read. Retry.'}<button className="btn" type="button" onClick={() => setNetworkLoadAttempt(v => v + 1)}>{zh ? '重试' : 'Retry'}</button></div> : networkReady ? <NativeNetworkSettings key={editingKey.current ?? 'network'} value={draft.network ?? networkDraft()} language={language} disabled={!props.mutable} target={!custom && registeredOption ? { kind: 'registered', connection_option_id: registeredOption.connection_option_id, protocol: [...(registeredOption.endpoints ?? [])].sort((a, b) => a.stable_preference - b.stable_preference)[0]?.protocol ?? draft.protocol } : networkTarget(draft)} reveal={networkReveal} failure={result?.failure} onChange={network => invalidate(current => ({ ...current, network }), false, 'connection')} /> : <p role="status">{zh ? '正在读取网络设置…' : 'Reading network settings…'}</p>}
        </form>

        {screen === 'manual' && <form id="mc-manual-form" onSubmit={event => { event.preventDefault(); void runCheck(true); }}>
          {draft.models.map(model => <div key={model.client_id}><ManualModelEditor model={model} language={language} disabled={!props.mutable} catalogManaged={!custom && Object.hasOwn(registeredOption?.known_models ?? {}, model.upstream_model_id)} onChange={updateManualModel} /><button className="btn btn-quiet" type="button" onClick={() => invalidate(current => ({ ...current, models: current.models.filter(value => value.client_id !== model.client_id) }), false, 'manual')}>{zh ? '移除模型' : 'Remove model'}</button></div>)}
          <button className="btn" type="button" onClick={() => invalidate(current => ({ ...current, models: [...current.models, blankModel()] }), false, 'manual')}>{zh ? '添加模型 ID' : 'Add model ID'}</button>
        </form>}

        {screen === 'network_review' && networkReview && networkSource && <NativeNetworkImpact before={networkSource.policy} after={networkEdit(draft.network).policy} models={networkSource.models} decisions={networkSource.decisions} language={language} />}
        {screen === 'models' && result && <section className="model-selection" aria-live="polite">
          <div className="connection-config-heading"><ProviderIcon optionId={custom ? draft.display_template_id : registeredOption?.connection_option_id} language={language} /><div><h3>{draft.display_name}</h3><p>{custom ? (zh ? '自定义 API' : 'Custom API') : productName}</p></div></div>
          <p className="v3-select-intro">{zh ? '选择需要接入的模型，已知能力会自动匹配。' : 'Choose models to connect. Known capabilities are matched automatically.'}</p>
          {result.reachability === 'transport_failed' && <p className="field-help">{zh ? '目录连接失败；保留已有模型声明，推理可用性仍未知。' : 'The directory connection failed. Model declarations are retained; inference availability remains unknown.'}</p>}
          {result.authentication === 'unknown' && <p className="field-help">{zh ? '凭据尚未验证。' : 'Credentials have not been verified.'}</p>}
          <p role="status">{result.inference === 'verified' ? (zh ? `${result.inference_model_id} 的函数工具调用已通过；不代表其他模型、联网搜索或客户端专用工具可用。` : `${result.inference_model_id} passed a function tool call; other models, web search and client-specific tools remain unverified.`) : (zh ? '工具调用尚未验证。保存不会自动发起推理。' : 'Tool calling has not been verified. Saving does not send inference requests.')}</p>
          <button className="btn" type="button" disabled={selectedCount !== 1} onClick={() => { const model = result.candidate.models.find(value => selected.has(value.model_ref)); if (model) void runCheck(false, model.upstream_model_id); }}>{zh ? '验证工具调用（仅所选模型，可能计费）' : 'Test tool calling for selected model (may incur cost)'}</button>
          <button className="btn btn-quiet" type="button" onClick={() => invalidate(current => ({ ...current, models: result.candidate.models.filter(model => selected.has(model.model_ref)).map(model => current.models.find(value => value.upstream_model_id === model.upstream_model_id) ?? blankModel(model.upstream_model_id, model.display_name)) }), false, 'manual')}>{zh ? '编辑所选模型 / 添加模型' : 'Edit selected models / add model'}</button>
          {['not_run', 'empty', 'unavailable', 'invalid'].includes(result.directory) && <p className="field-help">{zh ? '未取得模型目录，保留内置或手工声明的模型。' : 'No model directory was obtained. Built-in and manually declared models are retained.'}</p>}
          {result.directory === 'partial' && <div className="callout warn"><UiIcon name="warning" /><span>{zh ? '模型目录只读取到一部分。可以先保存已确认的模型，稍后再重新检查。' : 'Only part of the model catalog was read. You can save confirmed models and check again later.'}</span></div>}
          <fieldset className="v3-catalog model-result-list" disabled={!props.mutable}><legend className="sr-only">{zh ? '选择保存的模型' : 'Select models to save'}</legend>{result.candidate.models.map((model, index) => {
            const canSelect = modelCanBeSelected(result, model, true);
            return <div className="model-result-row" key={model.model_ref}><label className="check-row"><input type="checkbox" data-autofocus={index === 0 || undefined} disabled={!canSelect} checked={canSelect && selected.has(model.model_ref)} onChange={event => { const next = new Set(selected); if (event.target.checked) next.add(model.model_ref); else next.delete(model.model_ref); selectionRef.current = { result, refs: next }; setSelected(next); setError(''); }} /><div><strong>{model.display_name}</strong><span>{draft.display_name}{model.fact_basis === 'runtime_fallback' ? ` · ${zh ? '运行时保守兜底' : 'Conservative runtime fallback'}` : ''}{!canSelect ? ` · ${modelAvailabilityMessage(model.reason, zh)}` : ''}</span></div></label>{custom && !canSelect && <button className="btn btn-quiet" type="button" onClick={() => {
              const prefilled = draftRef.current.models.find(value => value.upstream_model_id === model.upstream_model_id);
              const manualDraft = { ...draftRef.current, edit_revision: draftRef.current.edit_revision + 1, check_id: '', models: [prefilled ?? blankModel(model.upstream_model_id, model.display_name)] };
              draftRef.current = manualDraft; setDraft(manualDraft); setResult(null); setScreen('manual'); setError('');
            }}>{zh ? '补充能力' : 'Add capability details'}</button>}</div>;
          })}</fieldset>
        </section>}
      </div>
      {optionsError && !busy && !custom && (screen === 'catalog' || Boolean(registeredOption)) && <div role="alert" className="callout bad" data-error-code={optionsError}><UiIcon name="warning" /><span>{connectionErrorMessage(optionsError, zh)}</span></div>}
      {result?.failure && !busy && !networkRepairable(result.failure) && <p className="callout warn" role="alert">{networkFailureText(result.failure, zh)}</p>}
      {error && !busy && (!optionUnavailable || Boolean(registeredOption)) && <div ref={errorFeedback} role="alert" className="callout bad" data-error-code={error}><UiIcon name="warning" /><span>{connectionErrorMessage(error, zh)}</span></div>}
    </section>
  </Dialog>;
}
