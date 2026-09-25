import { connectionName, connectionTemplate } from '../../ui/provider-identity';
import { ProviderIcon } from '../../ui/ProviderIcon';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { Dialog } from '../../ui/Dialog';
import { UiIcon } from '../../ui/UiIcon';
import { confirmDiscard, useDiscardGuard } from '../../ui/discard-guard';
import { saveFailureDefinitelyPreAdmission } from '../subscriptions/model';
import {
  blankModel,
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
import { customApiPrefillOptions, metadataProviderCandidates, metadataProviderOption, registeredModelCandidates, sortMetadataProviders, sortRegisteredOptions, templateEndpointForProtocol } from './registered-metadata';
import {
  connectionErrorMessage,
  ManualModelEditor,
  manualModelError,
  reasoningValue,
  registeredConnectionLabel,
  safeConnectionErrorCode,
  validEndpoint,
} from './form-support';

type Phase = 'editing' | 'checking' | 'saving';
type Screen = 'connection' | 'manual' | 'models';

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
  const [custom, setCustom] = useState(props.initialDraft.entry_kind === 'custom_api');
  const free = props.initialDraft.entry_kind === 'free_api_key';
  const [draft, setDraft] = useState<ModelConnectionDraft>(props.initialDraft);
  const [screen, setScreen] = useState<Screen>('connection');
  const [phase, setPhase] = useState<Phase>('editing');
  const [result, setResult] = useState<ModelConnectionCheckView | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState('');
  const [externalLinkError, setExternalLinkError] = useState(false);
  const [connectionOptions, setConnectionOptions] = useState<ComputeConnectionOptions | null>(null);
  const [optionsLoading, setOptionsLoading] = useState(true);
  const [registeredOptionId, setRegisteredOptionId] = useState('');
  const [metadataPrefillKey, setMetadataPrefillKey] = useState('');
  const password = useRef<HTMLInputElement>(null);
  const errorFeedback = useRef<HTMLDivElement>(null);
  const draftRef = useRef(draft);
  const protectedInputRef = useRef<ComputeCandidateRef | null>(null);
  const activeCheckRef = useRef<string | null>(null);
  const applyDispatchedRef = useRef(false);
  const aliveRef = useRef(true);
  const saveCompletedRef = useRef(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(props.initialDraft) || Boolean(protectedInputRef.current);
  const registeredOptions = useMemo(() => sortRegisteredOptions(connectionOptions?.options.filter(option =>
    option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free')
  ) ?? []), [connectionOptions, free]);
  const registeredOption = registeredOptions.find(option => option.connection_option_id === registeredOptionId)
    ?? registeredOptions[0]
    ?? null;
  const registeredLabel = registeredConnectionLabel(registeredOption, zh);
  const registeredCandidates = useMemo(() => registeredOption
    ? registeredModelCandidates(registeredOption, connectionOptions?.metadata_catalog)
    : [], [registeredOption, connectionOptions]);
  const metadataProviders = useMemo(() => sortMetadataProviders(connectionOptions?.metadata_catalog?.provider_records.filter(provider =>
    provider.usable_for.includes('custom-api-endpoint-prefill')
    && provider.base_url_candidates.length > 0
    && provider.protocol_candidates.length > 0
  ) ?? []), [connectionOptions]);
  const metadataPrefills = useMemo(() => customApiPrefillOptions(registeredOptions, metadataProviders), [registeredOptions, metadataProviders]);
  const selectedPrefill = metadataPrefills.find(prefill => prefill.key === metadataPrefillKey) ?? null;
  const metadataCandidates = useMemo(() => selectedPrefill?.kind === 'template'
    ? registeredModelCandidates(selectedPrefill.option, connectionOptions?.metadata_catalog)
    : selectedPrefill?.kind === 'provider' && connectionOptions?.metadata_catalog
      ? metadataProviderCandidates(selectedPrefill.provider, connectionOptions.metadata_catalog, registeredOptions)
      : [], [selectedPrefill, connectionOptions, registeredOptions]);
  useDiscardGuard('models', () => dirty && !saveCompletedRef.current, language, confirmReplacement);

  const releaseProtectedInput = useCallback(() => {
    const input = protectedInputRef.current;
    protectedInputRef.current = null;
    if (input && !applyDispatchedRef.current) void backend.releaseProtectedInput(input).catch(() => undefined);
  }, [backend]);

  const loadConnectionOptions = useCallback(async () => {
    setOptionsLoading(true);
    if (!custom) setError('');
    try {
      const options = await backend.listConnectionOptions();
      if (!aliveRef.current) return;
      setConnectionOptions(options);
      const executable = sortRegisteredOptions(options.options.filter(option => option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free')));
      setRegisteredOptionId(current => executable.some(option => option.connection_option_id === current)
        ? current
        : executable[0]?.connection_option_id ?? '');
      if (!custom && executable.length === 0) {
        setError('MODEL_CONNECTION_OPTION_UNAVAILABLE');
      }
    } catch {
      if (aliveRef.current && !custom) setError('MODEL_CONNECTION_OPTIONS_UNAVAILABLE');
    } finally {
      if (aliveRef.current) setOptionsLoading(false);
    }
  }, [backend, custom, free]);

  function selectMetadataProvider(provider: ProviderMetadataRecord | null) {
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
    invalidate(current => ({
      ...current,
      display_name: provider.display_name,
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
        return record ? metadataModelPrefill(record, clientOperationId('model'))
          : blankModel(candidate.upstream_model_id, candidate.display_name);
      }),
    }), true, 'connection');
  }

  function selectRegisteredTemplate(option: ComputeConnectionOptions['options'][number], keepExistingModels = false) {
    const endpoint = [...(option.endpoints ?? [])].sort((a, b) => a.stable_preference - b.stable_preference)[0];
    if (!endpoint) return;
    const candidates = registeredModelCandidates(option, connectionOptions?.metadata_catalog);
    const otherProtocols = new Set<UpstreamProtocol>();
    const additional_endpoints = [...(option.endpoints ?? [])]
      .sort((a, b) => a.stable_preference - b.stable_preference)
      .filter(other => other.protocol !== endpoint.protocol && !otherProtocols.has(other.protocol) && Boolean(otherProtocols.add(other.protocol)))
      .map(other => ({ base_url: other.base_url, base_kind: 'api_root' as const,
        request_path_override: other.request_path, inventory_path_override: other.inventory_path ?? null,
        protocol: other.protocol, protocol_profile_id: `profile/custom/${other.protocol}`,
        protocol_profile_revision: 1, authentication: other.authentication_semantics ?? { kind: 'bearer' as const } }));
    setCustom(true);
    setMetadataPrefillKey(`template:${option.connection_option_id}`);
    invalidate(current => ({ ...current,
      entry_kind: 'custom_api', display_template_id: option.connection_option_id,
      display_name: connectionName(option.connection_option_id, language, option.display_name),
      base_url: endpoint.base_url, base_kind: 'api_root',
      request_path_override: endpoint.request_path, inventory_path_override: endpoint.inventory_path ?? null,
      protocol: endpoint.protocol, protocol_profile_id: `profile/custom/${endpoint.protocol}`,
      protocol_profile_revision: current.protocol_profile_revision + 1,
      authentication: endpoint.authentication_semantics ?? { kind: 'bearer' },
      additional_endpoints, models: withRegisteredModels(keepExistingModels ? current.models : [], candidates),
    }), true, 'connection');
  }

  function selectMetadataPrefill(key: string) {
    setMetadataPrefillKey(key);
    const prefill = metadataPrefills.find(value => value.key === key);
    if (prefill?.kind === 'template') selectRegisteredTemplate(prefill.option);
    else if (prefill?.kind === 'provider') selectMetadataProvider(prefill.provider);
  }

  useEffect(() => { draftRef.current = draft; }, [draft]);
  useEffect(() => {
    if (!error || phase !== 'editing') return;
    requestAnimationFrame(() => errorFeedback.current?.scrollIntoView({ block: 'nearest' }));
  }, [error, phase, screen]);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      const checkId = activeCheckRef.current;
      if (checkId) void backend.cancelModelConnectionCheck(checkId).catch(() => undefined);
      releaseProtectedInput();
    };
  }, [backend, releaseProtectedInput]);
  useEffect(() => { void loadConnectionOptions(); }, [loadConnectionOptions]);

  const invalidate = useCallback((
    update: (current: ModelConnectionDraft) => ModelConnectionDraft,
    securityChanged = false,
    nextScreen?: Screen,
  ) => {
    const checkId = activeCheckRef.current;
    activeCheckRef.current = null;
    if (checkId) void backend.cancelModelConnectionCheck(checkId).catch(() => undefined);
    if (securityChanged) releaseProtectedInput();
    setDraft(current => {
      const nextRevision = current.edit_revision + 1;
      const updated = update(current);
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
      return versioned;
    });
    setResult(null);
    setSelected(new Set());
    setPhase('editing');
    setError('');
    if (nextScreen) setScreen(nextScreen);
  }, [backend, releaseProtectedInput]);

  function updateManualModel(model: ModelDeclaration) {
    invalidate(current => ({ ...current, models: current.models.map(value => value.client_id === model.client_id ? model : value) }), false, 'manual');
  }

  async function saveChecked(checked: ModelConnectionCheckView, selectedRefs: string[]) {
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
    setPhase('saving');
    try {
      const preview = await backend.previewComputeSave(buildSaveChange({
        result: checked,
        expectedRevisions: props.expectedRevisions,
        selectedModelRefs: selectedRefs,
        enable: true,
        protectedInput: protectedInputRef.current,
      }));
      if (!aliveRef.current) {
        releaseProtectedInput();
        return;
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
    const current = { ...base, inference_model_id: inferenceModelId, models: inferenceModelId && !base.models.some(model => model.upstream_model_id === inferenceModelId) ? [...base.models, blankModel(inferenceModelId)] : base.models };
    const passwordElement = password.current;
    const enteredSecret = passwordElement?.value ?? '';
    const connectionReady = custom
      ? validEndpoint(current.base_url)
      : Boolean(connectionOptions && registeredOption);
    if (!connectionReady) {
      setError(custom ? 'MODEL_CONNECTION_FIELDS_REQUIRED' : 'MODEL_CONNECTION_OPTION_UNAVAILABLE');
      return;
    }
    if (current.authentication.kind !== 'none' && !enteredSecret && !protectedInputRef.current && !current.existing_source_id) {
      setError('MODEL_CONNECTION_KEY_REQUIRED');
      return;
    }
    if (saveAfterCheck) {
      const validation = (new Set(current.models.map(model => model.upstream_model_id.trim())).size !== current.models.length ? 'MODEL_ID_DUPLICATE' : '') || current.models.map(manualModelError).find(Boolean) || (!current.models.length ? 'MODEL_ID_REQUIRED' : '');
      if (validation) { setError(validation); return; }
    }
    const checkId = clientOperationId('check');
    const checkingDraft = { ...current, display_name: current.display_name.trim() || `${zh ? '自定义 API' : 'Custom API'} · ${custom ? new URL(current.base_url).hostname : ''}`, candidate_ref: enteredSecret ? null : current.candidate_ref, check_id: checkId };
    draftRef.current = checkingDraft;
    setDraft(checkingDraft);
    activeCheckRef.current = checkId;
    // A manual capability declaration must be checked again before it can be
    // saved. Keep that check cancellable; only lock the dialog after the
    // checked candidate crosses into the save phase.
    setPhase('checking');
    let newlyRegistered: ComputeCandidateRef | null = null;
    try {
      if (checkingDraft.authentication.kind !== 'none' && enteredSecret) {
        newlyRegistered = (await backend.registerProtectedInput(enteredSecret)).input_candidate;
        if (passwordElement) passwordElement.value = '';
        if (activeCheckRef.current !== checkId) {
          await backend.releaseProtectedInput(newlyRegistered);
          return;
        }
        releaseProtectedInput();
        protectedInputRef.current = newlyRegistered;
      }
      const inputCandidate = protectedInputRef.current;
      const checked = custom
        ? await backend.checkModelConnection({
            draft: buildCheckDraft(checkingDraft),
            ...(inputCandidate ? { input_candidate: inputCandidate } : {}),
          })
        : await backend.checkRegisteredModelConnection({
            inference_model_id: inferenceModelId,
            models: checkingDraft.models.filter(model => model.upstream_model_id.trim()).map(({ client_id: _clientId, ...model }) => ({ ...model, display_name: model.display_name.trim() || model.upstream_model_id })),
            connection_option_id: registeredOption!.connection_option_id,
            expected_catalog: connectionOptions!.catalog,
            candidate_ref: checkingDraft.candidate_ref,
            lineage_ref: checkingDraft.lineage_ref,
            edit_revision: checkingDraft.edit_revision,
            check_id: checkingDraft.check_id,
            input_candidate: inputCandidate!,
            existing_source_id: checkingDraft.existing_source_id,
            expected_source_revision: null,
          });
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
      const checkFailure = checkFailureCode(checked);
      if (checkFailure) {
        setError(checkFailure);
        setPhase('editing');
        return;
      }
      const selectable = checked.candidate.models.filter(model => model.selectable);
      if (saveAfterCheck) {
        await saveChecked(checked, selectable.filter(model => checkingDraft.models.some(declared => declared.upstream_model_id === model.upstream_model_id)).map(model => model.model_ref));
        return;
      }
      if (custom && selectable.length === 0) {
        const observed = checked.candidate.models[0];
        const existing = acceptedDraft.models.find(model =>
          model.upstream_model_id === observed?.upstream_model_id
        );
        const manualDraft = {
          ...acceptedDraft,
          edit_revision: acceptedDraft.edit_revision + 1,
          check_id: '',
          models: acceptedDraft.models.length ? acceptedDraft.models : [existing ?? blankModel(observed?.upstream_model_id, observed?.display_name)],
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
      // The catalog is a review step: do not silently opt the user into every
      // model returned by an account-wide directory request.
      setSelected(new Set());
      setScreen('models');
      setPhase('editing');
    } catch (caught) {
      if (activeCheckRef.current === checkId) {
        activeCheckRef.current = null;
        const code = safeConnectionErrorCode(caught);
        if (!custom && code === 'compute.registered_catalog_changed') {
          try {
            const options = await backend.listConnectionOptions();
            if (aliveRef.current) setConnectionOptions(options);
          } catch {
            // Keep the catalog-changed action visible; the next explicit retry can reload again.
          }
        }
        setError(code);
        setPhase('editing');
      } else if (newlyRegistered) {
        await backend.releaseProtectedInput(newlyRegistered).catch(() => undefined);
      }
    }
  }

  function cancelActiveCheck() {
    const checkId = activeCheckRef.current;
    activeCheckRef.current = null;
    if (checkId) void backend.cancelModelConnectionCheck(checkId).catch(() => undefined);
    setPhase('editing');
    setError('');
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
    if (kind === 'back') props.onBack(); else props.onCancel();
    props.restoreFocus();
  }

  async function confirmReplacement() {
    if (dirty && !saveCompletedRef.current && !(await confirmDiscard(language))) return false;
    discardAndClose('cancel');
    return true;
  }

  async function openDocumentation(event: React.MouseEvent<HTMLAnchorElement>, url?: string) {
    if (!("__TAURI_INTERNALS__" in window)) return;
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
  const title = screen === 'models'
    ? (zh ? '选择模型' : 'Select models')
    : screen === 'manual'
      ? (zh ? '补充未知模型' : 'Complete unknown model details')
      : custom ? (zh ? '自定义 API' : 'Custom API') : (zh ? '连接 API' : 'Connect an API');
  const footer = busy
    ? <button className="btn" type="button" disabled={phase === 'saving'} onClick={() => discardAndClose('cancel')}>{phase === 'saving' ? (zh ? '正在保存…' : 'Saving…') : (zh ? '取消检查' : 'Cancel check')}</button>
    : screen === 'connection'
      ? <><button className="btn" type="button" onClick={() => discardAndClose('back')}>{zh ? '返回' : 'Back'}</button><button className="btn btn-primary" type="submit" form="mc-connection-form" disabled={!props.mutable || (!custom && (optionsLoading || !registeredOption))}>{zh ? '检查接入' : 'Check connection'}</button></>
      : screen === 'manual'
        ? <><button className="btn" type="button" onClick={backWithinFlow}>{zh ? '返回' : 'Back'}</button><button className="btn btn-primary" type="submit" form="mc-manual-form" disabled={!props.mutable}>{zh ? '保存接入' : 'Save connection'}</button></>
        : <><button className="btn" type="button" onClick={backWithinFlow}>{zh ? '返回' : 'Back'}</button><button className="btn btn-primary" type="button" disabled={!props.mutable || !selectedCount} onClick={() => void saveSelected()}>{zh ? '保存接入' : 'Save connection'}</button></>;

  return <Dialog open title={title} closeLabel={phase === 'saving' ? (zh ? '正在保存' : 'Saving') : (zh ? '取消添加' : 'Cancel')} closeDisabled={phase === 'saving'} onClose={() => discardAndClose('cancel')} footer={footer}>
    <section className="connection-flow" aria-label={zh ? '添加模型连接' : 'Add model connection'}>
      {!props.mutable && !busy && <div className="callout warn" role="status"><UiIcon name="warning" /><span>{zh ? '本机服务连接中断。当前输入已保留，重新连接后再保存。' : 'The local service disconnected. Your input is retained; reconnect before saving.'}</span></div>}
      {busy ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{phase === 'checking' ? (zh ? '正在检查接入' : 'Checking connection') : (zh ? '正在保存接入' : 'Saving connection')}</strong><p>{phase === 'checking' ? (zh ? '检查配置，并在提供目录时读取模型。' : 'Checking configuration and reading the model directory when configured.') : (zh ? '保存所选模型与凭据。' : 'Saving the selected models and credential.')}</p></div></div> : <>
        <form hidden={screen !== 'connection'} id="mc-connection-form" onSubmit={event => { event.preventDefault(); void runCheck(false); }}>
          <fieldset className="connection-fields" disabled={!props.mutable}>
            {!custom && !free && <button className="btn btn-quiet" type="button" onClick={() => { setCustom(true); invalidate(current => ({ ...current, entry_kind: 'custom_api' }), true, 'connection'); }}>{zh ? '自定义 API' : 'Custom API'}</button>}
            {custom ? <>
              <ProviderIcon optionId={draft.display_template_id} language={language} />
              {!optionsLoading && metadataPrefills.length > 0 && <div className="connection-fields metadata-prefill-fields">
                <label className="field"><span className="field-label">{zh ? '接入资料预填（可选）' : 'Connection prefill (optional)'}</span><select className="select" value={metadataPrefillKey} onChange={event => selectMetadataPrefill(event.target.value)}>
                  <option value="">{zh ? '不使用预填' : 'No prefill'}</option>
                  <optgroup label={zh ? '内置接入' : 'Built-in connections'}>
                    {metadataPrefills.filter(prefill => prefill.kind === 'template').map(prefill => <option key={prefill.key} value={prefill.key}>{connectionName(prefill.option.connection_option_id, language, prefill.option.display_name)}</option>)}
                  </optgroup>
                  {metadataProviders.length > 0 && <optgroup label={zh ? '供应商资料记录' : 'Provider metadata records'}>
                    {metadataPrefills.filter(prefill => prefill.kind === 'provider').map(prefill => <option key={prefill.key} value={prefill.key}>{prefill.provider.display_name}</option>)}
                  </optgroup>}
                </select></label>
                {selectedPrefill && <p className="field-help">{zh
                  ? `已收录 ${metadataCandidates.length} 个模型，检查接入后统一选择；未知能力保持空白，凭据不会从资料推断。`
                  : `${metadataCandidates.length} models in this catalog. Choose after checking the connection; unknown capabilities remain blank and credentials are never inferred.`}</p>}
              </div>}
              <label className="field"><span className="field-label">{zh ? '接入名称' : 'Connection name'}</span><input className="input" data-autofocus value={draft.display_name} placeholder={zh ? '例如：团队模型服务' : 'e.g. Team models'} onChange={event => invalidate(current => ({ ...current, display_name: event.target.value }), false, 'connection')} /></label>
              <label className="field"><span className="field-label">Base URL</span><input className="input" inputMode="url" value={draft.base_url} placeholder="https://example.com/v1" onChange={event => invalidate(current => ({ ...current, base_url: event.target.value }), true, 'connection')} /></label>
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
                    models: current.models.map(model => {
                      const previous = model.capabilities.native_reasoning.value;
                      const next = previous ? reasoningValue(previous.kind, protocol, previous) : null;
                      return { ...model, capabilities: { ...model.capabilities, native_reasoning: { value: next, basis: next ? 'user_declared' : 'unknown' } } };
                    }),
                  };
                }, true, 'connection');
              }}><option value="chat_completions">OpenAI Chat Completions</option><option value="responses">OpenAI Responses</option><option value="messages">Anthropic Messages</option></select></label>
            </> : optionsLoading
              ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{zh ? '正在读取接入方式' : 'Loading connection'}</strong><p>{zh ? '从本机可信目录确认当前支持的产品。' : 'Checking supported products in the local trusted catalog.'}</p></div></div>
              : registeredOption
                ? <label className="field"><span className="field-label">{zh ? '接入方式' : 'Connection'}</span><select className="select" value={registeredOption.connection_option_id} onChange={event => {
                  setRegisteredOptionId(event.target.value);
                  invalidate(current => ({ ...current, candidate_ref: null, models: [] }), true, 'connection');
                }}>{registeredOptions.map(option => <option key={option.connection_option_id} value={option.connection_option_id}>{connectionName(option.connection_option_id, language, option.display_name)}</option>)}</select><span className="field-help">{registeredLabel.detail}</span></label>
                : <div className="callout warn"><UiIcon name="warning" /><div><strong>{zh ? '当前没有可用的内置接入' : 'No built-in connection is available'}</strong><p>{connectionErrorMessage(error || 'MODEL_CONNECTION_OPTION_UNAVAILABLE', zh)}</p><button className="btn" type="button" onClick={() => void loadConnectionOptions()}>{zh ? '重新读取' : 'Reload'}</button></div></div>}
            {!custom && registeredOption && <section>
              <ProviderIcon optionId={registeredOption.connection_option_id} language={language} />
              <p>{connectionTemplate(registeredOption.connection_option_id)?.description[language]}</p>
              <a href={connectionTemplate(registeredOption.connection_option_id)?.documentation_url} target="_blank" rel="noreferrer" onClick={event => void openDocumentation(event, connectionTemplate(registeredOption.connection_option_id)?.documentation_url)}>{zh ? '官方说明与获取 Key' : 'Official guide and API keys'}</a>
              {externalLinkError && <div className="callout warn" role="alert"><UiIcon name="warning" /><span>{zh ? '无法打开默认浏览器；当前表单内容已保留。' : 'The default browser could not be opened. Your form input is retained.'}</span></div>}
              <p className="field-help">{zh
                ? `已收录 ${registeredCandidates.length} 个模型，填写 Key 后统一选择。具体可用范围取决于账号和套餐。`
                : `${registeredCandidates.length} models in this catalog. Enter your key, then choose models. Availability depends on your account and plan.`}</p>
              {registeredOption.endpoints?.length && <button className="btn btn-quiet" type="button" onClick={() => selectRegisteredTemplate(registeredOption, true)}>{zh ? '自定义此模板的连接配置' : 'Customize this template connection'}</button>}
            </section>}
            {custom && <>
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
                    <label className="field"><span className="field-label">Base URL</span><input className="input" inputMode="url" value={endpoint.base_url} onChange={event => updateEndpoint({ base_url: event.target.value })} /></label>
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
            </>}
            {draft.authentication.kind !== 'none' && <label className="field"><span className="field-label">API Key</span><input className="input" disabled={!custom && (optionsLoading || !registeredOption)} data-autofocus={!custom || undefined} ref={password} type="password" autoComplete="off" spellCheck={false} placeholder={protectedInputRef.current ? '••••••••••••••••' : undefined} onChange={() => invalidate(current => ({ ...current, candidate_ref: null }), true, 'connection')} /></label>}
            {!custom && <p className="field-help">{zh ? '检查连接配置；提供目录时读取模型，不发送推理请求。' : 'Checks connection settings and reads the model directory when available, without inference requests.'}</p>}
            <button className="btn" type="button" onClick={() => invalidate(current => ({ ...current, models: current.models.length ? current.models : [blankModel()] }), false, 'manual')}>{zh ? '手工维护模型' : 'Edit models manually'}</button>
          </fieldset>
        </form>

        {screen === 'manual' && <form id="mc-manual-form" onSubmit={event => { event.preventDefault(); void runCheck(true); }}>
          {draft.models.map(model => <div key={model.client_id}><ManualModelEditor model={model} protocol={draft.protocol} language={language} disabled={!props.mutable} catalogManaged={!custom && Object.hasOwn(registeredOption?.known_models ?? {}, model.upstream_model_id.trim())} onChange={updateManualModel} /><button className="btn btn-quiet" type="button" onClick={() => invalidate(current => ({ ...current, models: current.models.filter(value => value.client_id !== model.client_id) }), false, 'manual')}>{zh ? '移除模型' : 'Remove model'}</button></div>)}
          <button className="btn" type="button" onClick={() => invalidate(current => ({ ...current, models: [...current.models, blankModel()] }), false, 'manual')}>{zh ? '添加模型 ID' : 'Add model ID'}</button>
        </form>}

        {screen === 'models' && result && <section className="model-selection" aria-live="polite">
          <p className="v3-select-intro">{zh ? '选择需要接入的模型，已知能力会自动匹配。' : 'Choose models to connect. Known capabilities are matched automatically.'}</p>
          {result.reachability === 'transport_failed' && <p className="field-help">{zh ? '目录连接失败；保留已有模型声明，推理可用性仍未知。' : 'The directory connection failed. Model declarations are retained; inference availability remains unknown.'}</p>}
          {result.authentication === 'unknown' && <p className="field-help">{zh ? '凭据尚未验证。' : 'Credentials have not been verified.'}</p>}
          <p role="status">{result.inference === 'verified' ? (zh ? `${result.inference_model_id} 的函数工具调用已通过；不代表其他模型、联网搜索或客户端专用工具可用。` : `${result.inference_model_id} passed a function tool call; other models, web search and client-specific tools remain unverified.`) : (zh ? '工具调用尚未验证。保存不会自动发起推理。' : 'Tool calling has not been verified. Saving does not send inference requests.')}</p>
          <button className="btn" type="button" disabled={selectedCount !== 1} onClick={() => { const model = result.candidate.models.find(value => selected.has(value.model_ref)); if (model) void runCheck(false, model.upstream_model_id); }}>{zh ? '验证工具调用（仅所选模型，可能计费）' : 'Test tool calling for selected model (may incur cost)'}</button>
          <button className="btn btn-quiet" type="button" onClick={() => invalidate(current => ({ ...current, models: result.candidate.models.filter(model => selected.has(model.model_ref)).map(model => current.models.find(value => value.upstream_model_id === model.upstream_model_id) ?? blankModel(model.upstream_model_id, model.display_name)) }), false, 'manual')}>{zh ? '编辑所选模型 / 添加模型' : 'Edit selected models / add model'}</button>
          {['not_run', 'empty', 'unavailable', 'invalid'].includes(result.directory) && <p className="field-help">{zh ? '未取得模型目录，保留内置或手工声明的模型。' : 'No model directory was obtained. Built-in and manually declared models are retained.'}</p>}
          {result.directory === 'partial' && <div className="callout warn"><UiIcon name="warning" /><span>{zh ? '模型目录只读取到一部分。可以先保存已确认的模型，稍后再重新检查。' : 'Only part of the model catalog was read. You can save confirmed models and check again later.'}</span></div>}
          <fieldset className="v3-catalog model-result-list" disabled={!props.mutable}><legend className="sr-only">{zh ? '选择保存的模型' : 'Select models to save'}</legend>{result.candidate.models.map(model => {
            const canSelect = modelCanBeSelected(result, model, true);
            return <div className="model-result-row" key={model.model_ref}><label className="check-row"><input type="checkbox" disabled={!canSelect} checked={canSelect && selected.has(model.model_ref)} onChange={event => setSelected(current => { const next = new Set(current); if (event.target.checked) next.add(model.model_ref); else next.delete(model.model_ref); setError(''); return next; })} /><div><strong>{model.display_name}</strong><span>{custom ? draft.display_name : `${registeredLabel.title} · ${registeredLabel.detail}`}{model.fact_basis === 'runtime_fallback' ? ` · ${zh ? '运行时保守兜底' : 'Conservative runtime fallback'}` : ''}{!canSelect ? ` · ${modelAvailabilityMessage(model.reason, zh)}` : ''}</span></div></label>{custom && !canSelect && <button className="btn btn-quiet" type="button" onClick={() => {
              const prefilled = draftRef.current.models.find(value => value.upstream_model_id === model.upstream_model_id);
              const manualDraft = { ...draftRef.current, edit_revision: draftRef.current.edit_revision + 1, check_id: '', models: [prefilled ?? blankModel(model.upstream_model_id, model.display_name)] };
              draftRef.current = manualDraft; setDraft(manualDraft); setResult(null); setScreen('manual'); setError('');
            }}>{zh ? '补充能力' : 'Add capability details'}</button>}</div>;
          })}</fieldset>
        </section>}
      </>}
      {error && !busy && !optionUnavailable && <div ref={errorFeedback} role="alert" className="callout bad" data-error-code={error}><UiIcon name="warning" /><span>{connectionErrorMessage(error, zh)}</span></div>}
    </section>
  </Dialog>;
}
