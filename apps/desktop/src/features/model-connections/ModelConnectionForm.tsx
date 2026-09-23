import { connectionName, connectionTemplate } from '../../ui/provider-identity';
import { ProviderIcon } from '../../ui/ProviderIcon';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { Dialog } from '../../ui/Dialog';
import { UiIcon } from '../../ui/UiIcon';
import { confirmDiscard, useDiscardGuard } from '../../ui/discard-guard';
import { saveFailureDefinitelyPreAdmission } from '../subscriptions/model';
import {
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
  ModelConnectionFormProps,
  ModelDeclaration,
  ModelMetadataRecord,
  ProviderMetadataRecord,
  UpstreamProtocol,
} from './types';
import { checkFailureCode, modelAvailabilityMessage } from './copy';
import {
  metadataCostHint,
  metadataModelPrefill,
  metadataReasoningHint,
} from './metadata-prefill';
import { registeredModelCandidates, templateEndpointForProtocol } from './registered-metadata';
import {
  blankModel,
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
  const [metadataProviderKey, setMetadataProviderKey] = useState('');
  const [metadataModelKey, setMetadataModelKey] = useState('');
  const password = useRef<HTMLInputElement>(null);
  const errorFeedback = useRef<HTMLDivElement>(null);
  const draftRef = useRef(draft);
  const protectedInputRef = useRef<ComputeCandidateRef | null>(null);
  const activeCheckRef = useRef<string | null>(null);
  const applyDispatchedRef = useRef(false);
  const aliveRef = useRef(true);
  const saveCompletedRef = useRef(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(props.initialDraft) || Boolean(protectedInputRef.current);
  const registeredOptions = useMemo(() => connectionOptions?.options.filter(option =>
    option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free')
  ) ?? [], [connectionOptions, free]);
  const registeredOption = registeredOptions.find(option => option.connection_option_id === registeredOptionId)
    ?? registeredOptions[0]
    ?? null;
  const registeredLabel = registeredConnectionLabel(registeredOption, zh);
  const registeredCandidates = useMemo(() => registeredOption
    ? registeredModelCandidates(registeredOption, connectionOptions?.metadata_catalog)
    : [], [registeredOption, connectionOptions]);
  const metadataProviders = useMemo(() => connectionOptions?.metadata_catalog?.provider_records.filter(provider =>
    provider.usable_for.includes('custom-api-endpoint-prefill')
    && provider.base_url_candidates.length > 0
    && provider.protocol_candidates.length > 0
  ) ?? [], [connectionOptions]);
  const metadataProvider = metadataProviders.find(provider => provider.provider_record_key === metadataProviderKey) ?? null;
  const metadataModels = useMemo(() => connectionOptions?.metadata_catalog?.model_records.filter(model =>
    model.provider_record_key === metadataProviderKey
    && model.usable_for.includes('custom-api-model-prefill')
    && model.execution_fit.state === 'native_text_representable'
  ) ?? [], [connectionOptions, metadataProviderKey]);
  const metadataModel = metadataModels.find(model => model.model_record_key === metadataModelKey) ?? null;
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
      const executable = options.options.filter(option => option.origin !== 'agent_subscription' && (!free || option.billing_class === 'free'));
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
    setMetadataProviderKey(provider?.provider_record_key ?? '');
    setMetadataModelKey('');
    if (!provider) return;
    const protocol = provider.protocol_candidates.includes(draftRef.current.protocol)
      ? draftRef.current.protocol
      : provider.protocol_candidates[0];
    invalidate(current => ({
      ...current,
      display_name: provider.display_name,
      display_template_id: null,
      base_url: provider.base_url_candidates[0],
      protocol,
      protocol_profile_id: `profile/custom/${protocol}`,
      protocol_profile_revision: current.protocol_profile_revision + 1,
      models: [],
    }), true, 'connection');
  }

  function selectMetadataModel(model: ModelMetadataRecord | null) {
    setMetadataModelKey(model?.model_record_key ?? '');
    if (!model) {
      invalidate(current => ({ ...current, models: [] }), false, 'connection');
      return;
    }
    invalidate(current => ({
      ...current,
      models: [...current.models.filter(value => value.upstream_model_id !== model.upstream_model_id), metadataModelPrefill(model, clientOperationId('model'))],
    }), false, 'connection');
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
    const base = draftRef.current;
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
    if (current.authentication.kind !== 'none' && !enteredSecret && !protectedInputRef.current) {
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
              {!optionsLoading && metadataProviders.length > 0 && <div className="connection-fields metadata-prefill-fields">
                <label className="field"><span className="field-label">{zh ? '供应商元数据预填（可选）' : 'Provider metadata prefill (optional)'}</span><select className="select" value={metadataProviderKey} onChange={event => selectMetadataProvider(metadataProviders.find(provider => provider.provider_record_key === event.target.value) ?? null)}>
                  <option value="">{zh ? '不使用预填' : 'No prefill'}</option>
                  {metadataProviders.map(provider => <option key={provider.provider_record_key} value={provider.provider_record_key}>{provider.display_name}</option>)}
                </select></label>
                {metadataProvider && <label className="field"><span className="field-label">{zh ? '模型元数据预填（可选）' : 'Model metadata prefill (optional)'}</span><select className="select" value={metadataModelKey} onChange={event => selectMetadataModel(metadataModels.find(model => model.model_record_key === event.target.value) ?? null)}>
                  <option value="">{zh ? '不预填模型' : 'No model prefill'}</option>
                  {metadataModels.map(model => <option key={model.model_record_key} value={model.model_record_key}>{model.display_name} · {model.upstream_model_id}</option>)}
                </select></label>}
                <p className="field-help">{zh
                  ? `来自客户端内置的 ${connectionOptions?.metadata_catalog?.as_of ?? ''} 当前元数据快照；选择即表示把已知字段复制到草稿。未知字段保持空白，凭据不会从元数据推断。`
                  : `From the client-bundled ${connectionOptions?.metadata_catalog?.as_of ?? ''} current metadata snapshot. Selecting copies known fields into this draft; unknown fields stay blank and credentials are never inferred.`}</p>
                {metadataModel?.usable_for.includes('lifecycle-warning') && metadataModel.lifecycle !== 'active' && metadataModel.lifecycle !== 'unknown' && <div className="callout warn"><UiIcon name="warning" /><span>{zh ? `生命周期：${metadataModel.lifecycle}` : `Lifecycle: ${metadataModel.lifecycle}`}{metadataModel.replacement_upstream_ids.length ? ` · ${zh ? '替代项' : 'Replacements'}: ${metadataModel.replacement_upstream_ids.join(', ')}` : ''}</span></div>}
                {metadataModel && metadataReasoningHint(metadataModel, zh) && <p className="field-help">{metadataReasoningHint(metadataModel, zh)}</p>}
                {metadataModel && metadataCostHint(metadataModel, zh) && <p className="field-help">{metadataCostHint(metadataModel, zh)}</p>}
              </div>}
              <label className="field"><span className="field-label">{zh ? '接入名称' : 'Connection name'}</span><input className="input" data-autofocus value={draft.display_name} placeholder={zh ? '例如：团队模型服务' : 'e.g. Team models'} onChange={event => invalidate(current => ({ ...current, display_name: event.target.value }), false, 'connection')} /></label>
              <label className="field"><span className="field-label">Base URL</span><input className="input" inputMode="url" value={draft.base_url} placeholder="https://example.com/v1" onChange={event => invalidate(current => ({ ...current, base_url: event.target.value }), true, 'connection')} /></label>
              <label className="field"><span className="field-label">{zh ? '协议' : 'Protocol'}</span><select className="select" value={draft.protocol} onChange={event => {
                const protocol = event.target.value as UpstreamProtocol;
                invalidate(current => {
                  const template = registeredOptions.find(option => option.connection_option_id === current.display_template_id) ?? null;
                  const endpoint = templateEndpointForProtocol(current, template, protocol);
                  return {
                    ...current,
                    base_url: endpoint?.base_url ?? current.base_url,
                    request_path_override: endpoint?.request_path ?? current.request_path_override,
                    inventory_path_override: endpoint ? endpoint.inventory_path ?? null : current.inventory_path_override,
                    protocol,
                    protocol_profile_id: `profile/custom/${protocol}`,
                    protocol_profile_revision: current.protocol_profile_revision + 1,
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
              <ul>{registeredCandidates.map(candidate => <li key={candidate.upstream_model_id}>
                {candidate.display_name} · {candidate.upstream_model_id}
                {candidate.source === 'product_metadata' && <>
                  {' · '}<span className="field-help">{zh ? '产品资料候选' : 'Product metadata candidate'}</span>{' '}
                  <button className="btn btn-quiet" type="button" disabled={!props.mutable || draft.models.some(model => model.upstream_model_id === candidate.upstream_model_id)} onClick={() => invalidate(current => ({ ...current, models: [...current.models, blankModel(candidate.upstream_model_id, candidate.display_name)] }), false, 'connection')}>
                    {draft.models.some(model => model.upstream_model_id === candidate.upstream_model_id) ? (zh ? '已加入' : 'Added') : (zh ? '导入模型 ID' : 'Import model ID')}
                  </button>
                </>}
              </li>)}</ul>
              <p className="field-help">{zh ? '内置模型有已资格化能力资料；产品资料候选仅复制模型 ID，账号可见性、协议与推理能力仍需检查。' : 'Built-in models have qualified capability data. Product metadata candidates only copy the model ID; account access, protocol, and inference still need checking.'}</p>
              {registeredOption.endpoints?.length && <button className="btn btn-quiet" type="button" onClick={() => {
                const endpoint = [...registeredOption.endpoints!].sort((a, b) => a.stable_preference - b.stable_preference)[0];
                setCustom(true);
                invalidate(current => ({ ...current, entry_kind: 'custom_api', display_template_id: registeredOption.connection_option_id, display_name: connectionName(registeredOption.connection_option_id, language, registeredOption.display_name), base_url: endpoint.base_url, request_path_override: endpoint.request_path, inventory_path_override: endpoint.inventory_path ?? null, protocol: endpoint.protocol, protocol_profile_id: `profile/custom/${endpoint.protocol}`, authentication: endpoint.authentication_semantics ?? { kind: 'bearer' }, models: [...Object.entries(registeredOption.known_models ?? {}).map(([id, name]) => blankModel(id, name)), ...current.models.filter(model => !Object.hasOwn(registeredOption.known_models ?? {}, model.upstream_model_id))] }), true, 'connection');
              }}>{zh ? '自定义此模板的连接配置' : 'Customize this template connection'}</button>}
            </section>}
            {custom && <>
              <label className="field"><span className="field-label">{zh ? '请求路径（可选覆盖）' : 'Request path (optional override)'}</span><input className="input" value={draft.request_path_override ?? ''} onChange={event => invalidate(current => ({ ...current, request_path_override: event.target.value || null }), true, 'connection')} /></label>
              <label className="field"><span className="field-label">{zh ? '模型目录路径（可选）' : 'Model directory path (optional)'}</span><input className="input" placeholder="/v1/models" value={draft.inventory_path_override ?? ''} onChange={event => invalidate(current => ({ ...current, inventory_path_override: event.target.value || null }), true, 'connection')} /></label>
              <label className="field"><span className="field-label">{zh ? '认证方式' : 'Authentication'}</span><select className="select" value={draft.authentication.kind} onChange={event => invalidate(current => ({ ...current, authentication: event.target.value === 'api_key_header' ? { kind: 'api_key_header', header: 'x-api-key' } : event.target.value === 'none' ? { kind: 'none' } : { kind: 'bearer' } }), true, 'connection')}><option value="none">{zh ? '无认证' : 'None'}</option><option value="bearer">Bearer Token</option><option value="api_key_header">{zh ? '自定义 Header' : 'Custom header'}</option></select></label>
              {draft.authentication.kind === 'api_key_header' && <label className="field"><span className="field-label">Header</span><input className="input" value={draft.authentication.header} onChange={event => invalidate(current => ({ ...current, authentication: { kind: 'api_key_header', header: event.target.value } }), true, 'connection')} /></label>}
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
