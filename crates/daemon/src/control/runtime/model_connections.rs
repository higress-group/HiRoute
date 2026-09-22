use hiroute_application::compute_management::{
    ComputeCandidatePort, ComputeManagementPlanner, ComputeManagementPlanningErrorV2,
    ComputeManagementPresentationFactsV1, ProtectedInputSourceDescriptorV1,
    query_compute_management_with_presentation,
};
use hiroute_application::control::{ComputeManagementControlError, ComputeManagementControlPort};
use hiroute_application::subscriptions::{
    ComputeSubscriptionPlanner, SubscriptionPreparationError,
};
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeCandidateViewV2, ComputeConnectionApplyRequestV1,
    ComputeManagementChangeV2, ComputeManagementQueryV2, ComputeManagementSnapshotV2,
    ComputeManagementSubjectV2, ComputeModelMembershipV2, ComputeSaveDispositionV2,
    ComputeSavePreviewV2, ComputeSaveResultV2, ComputeSavedBindingV2,
    ComputeSubscriptionCandidatesV2, ComputeSubscriptionCheckPreviewV2,
    ComputeSubscriptionCheckResultV2, ComputeSubscriptionDiscoveryStateV2,
    ModelConnectionCheckViewV1, NativeModelConnectionBaseKindV1,
    NativeModelConnectionCheckRequestV1, NativeUserFactBasisV1, OperationReferenceV1,
    RegisteredModelConnectionCheckRequestV1, SavedModelConnectionCheckRequestV1,
};
use hiroute_domain::{
    ComputeManagementRepositoryPort, ComputeNativeRecheckDescriptorV2, ControlRepositoryPort,
    GatewayAuthenticationSemanticsV1, GatewayHeaderSemanticsV1, OperationId, OperationState,
    PortError, PortErrorCode, ProtectedSecret, ProtocolEndpointV1, UpstreamProtocol, WorkspaceId,
};
use hiroute_integrations::{
    ModelConnectionBaseKindV1, ModelConnectionProbeCancellationV1, NativeCandidateFactBasisV1,
    NativeCandidateFactValueV1, NativeConnectionProvenanceInputV1, NativeConnectionQualificationV1,
    NativeModelCapabilityDeclarationV1, NativeModelConnectionCredentialV1,
    NativeModelConnectionDraftV1, NativeModelDeclarationV1, TrustedReleaseCatalog,
};

use super::LocalControlAdapter;

mod discovered;
mod key_inputs;
mod presentation;
mod registered;
mod saved;

pub(super) fn registered_source_matches_current_option(
    source: &hiroute_domain::ComputeManagementSourceV2,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
) -> bool {
    if resolved.connector.runtime_kind != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
        || resolved.option.origin == hiroute_domain::ConnectionOrigin::AgentSubscription
    {
        return false;
    }
    resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .any(|endpoint| registered_endpoint_matches_source(source, resolved, endpoint))
}

pub(super) fn registered_endpoint_header_semantics(
    endpoint: &ProtocolEndpointV1,
) -> GatewayHeaderSemanticsV1 {
    GatewayHeaderSemanticsV1 {
        content_type: "application/json".into(),
        required_headers: endpoint.required_headers.clone(),
        forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
    }
}

pub(super) fn registered_endpoint_recheck_descriptor(
    _resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    endpoint: &ProtocolEndpointV1,
) -> Option<ComputeNativeRecheckDescriptorV2> {
    Some(ComputeNativeRecheckDescriptorV2 {
        display_template_id: None,
        inventory_path: endpoint.inventory_path.clone(),
        protocol_header_semantics: registered_endpoint_header_semantics(endpoint),
    })
}

pub(super) fn registered_endpoint_matches_source(
    source: &hiroute_domain::ComputeManagementSourceV2,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    endpoint: &ProtocolEndpointV1,
) -> bool {
    let authentication_matches = match (
        resolved.connector.authentication,
        endpoint.authentication_semantics.as_ref(),
    ) {
        (
            hiroute_domain::AuthenticationKind::ProviderApiKey,
            Some(
                hiroute_domain::GatewayAuthenticationSemanticsV1::Bearer
                | hiroute_domain::GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
            ),
        )
        | (
            hiroute_domain::AuthenticationKind::None,
            Some(hiroute_domain::GatewayAuthenticationSemanticsV1::None),
        ) => endpoint.authentication_semantics.as_ref() == Some(&source.authentication),
        _ => false,
    };
    authentication_matches
        && source.native_recheck.as_ref().is_some_and(|descriptor| {
            registered_endpoint_recheck_descriptor(resolved, endpoint).as_ref() == Some(descriptor)
        })
        && source.target.scheme == "https"
        && source.target.port == 443
        && endpoint.base_url.strip_prefix("https://") == Some(source.target.authority.as_str())
        && endpoint.request_path == source.target.request_path
        && endpoint.protocol == source.target.upstream_protocol
        && endpoint.adapter_ref == source.target.protocol_profile_id
        && endpoint.adapter_revision == source.target.protocol_profile_revision
}

impl ComputeManagementControlPort for LocalControlAdapter {
    fn compute_subscriptions(
        &self,
    ) -> Result<ComputeSubscriptionCandidatesV2, ComputeManagementControlError> {
        let Some(_) = self.cpa_runtime.as_ref() else {
            return Ok(ComputeSubscriptionCandidatesV2 {
                candidates: Vec::new(),
                discovery_state: ComputeSubscriptionDiscoveryStateV2::RuntimeUnavailable,
                reason_code: Some("subscription_runtime_unavailable".into()),
            });
        };
        Ok(ComputeSubscriptionCandidatesV2 {
            candidates: self.refresh_subscription_candidates()?,
            discovery_state: ComputeSubscriptionDiscoveryStateV2::Complete,
            reason_code: None,
        })
    }

    fn preview_subscription_check(
        &self,
        candidate: ComputeCandidateRefV2,
    ) -> Result<ComputeSubscriptionCheckPreviewV2, ComputeManagementControlError> {
        let prepared = {
            let stores = self
                .stores_lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            ComputeSubscriptionPlanner::new(
                self.model_connections.candidate_port(),
                stores.control(),
            )
            .preview(candidate)
            .map_err(map_subscription_preparation)?
        };
        self.remember_subscription_preview(&prepared)?;
        Ok(prepared.result)
    }

    fn prepare_subscription_check(
        &self,
        request: ComputeConnectionApplyRequestV1,
        apply_capability: Option<String>,
    ) -> Result<hiroute_application::PreparedTransactionV1, ComputeManagementControlError> {
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        ComputeSubscriptionPlanner::new(self.model_connections.candidate_port(), stores.control())
            .prepare_apply(request, apply_capability)
            .map_err(map_subscription_preparation)
    }

    fn compute_subscription_check_result(
        &self,
        operation: &OperationReferenceV1,
    ) -> Result<ComputeSubscriptionCheckResultV2, ComputeManagementControlError> {
        self.subscription_check_result(operation)
    }

    fn release_subscription_check(
        &self,
        validation: &hiroute_application_api::ComputeValidationRefV2,
    ) -> Result<ComputeSubscriptionCheckResultV2, ComputeManagementControlError> {
        self.release_subscription_validation(validation)
    }

    fn check_native_model_connection(
        &self,
        request: NativeModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError> {
        let check_id = request.draft.check_id.clone();
        if check_id.trim().is_empty() {
            return Err(ComputeManagementControlError::Invalid);
        }
        let cancellation = ModelConnectionProbeCancellationV1::default();
        {
            let mut cancellations = self
                .model_connection_cancellations
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            if cancellations.len() >= 256 || cancellations.contains_key(&check_id) {
                return Err(ComputeManagementControlError::Conflict);
            }
            cancellations.insert(check_id.clone(), cancellation.clone());
        }

        let checked = (|| {
            let mut draft = trusted_draft(request.draft, self.release_catalog.as_ref())?;
            let secret = match (&draft.authentication, request.input_candidate) {
                (GatewayAuthenticationSemanticsV1::None, None) => None,
                (GatewayAuthenticationSemanticsV1::None, Some(_)) => {
                    return Err(ComputeManagementControlError::Invalid);
                }
                (
                    GatewayAuthenticationSemanticsV1::Bearer
                    | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                    Some(candidate),
                ) => {
                    candidate
                        .validate_shape()
                        .map_err(|_| ComputeManagementControlError::Invalid)?;
                    if draft
                        .candidate_ref
                        .as_ref()
                        .is_some_and(|current| current != &candidate.candidate_ref)
                    {
                        return Err(ComputeManagementControlError::Conflict);
                    }
                    draft.candidate_ref = Some(candidate.candidate_ref.clone());
                    let inputs = self
                        .manual_protected_inputs
                        .lock()
                        .map_err(|_| ComputeManagementControlError::Unavailable)?;
                    let source = inputs
                        .get(&candidate.candidate_ref)
                        .ok_or(ComputeManagementControlError::NotFound)?;
                    Some(
                        ProtectedSecret::new(source.expose().to_vec())
                            .map_err(|_| ComputeManagementControlError::Corrupt)?,
                    )
                }
                (
                    GatewayAuthenticationSemanticsV1::Bearer
                    | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                    None,
                ) => None,
            };
            let credential = match (&draft.authentication, secret.as_ref()) {
                (GatewayAuthenticationSemanticsV1::None, _) => {
                    NativeModelConnectionCredentialV1::NotRequired
                }
                (_, Some(secret)) => NativeModelConnectionCredentialV1::Protected {
                    descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                    input_slot: draft
                        .candidate_ref
                        .clone()
                        .ok_or(ComputeManagementControlError::Invalid)?,
                    secret,
                },
                (_, None) => NativeModelConnectionCredentialV1::PendingInput,
            };
            self.model_connections
                .check(draft, credential, &cancellation)
                .map_err(|_| ComputeManagementControlError::Invalid)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }

    fn check_registered_model_connection(
        &self,
        request: RegisteredModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError> {
        if !request.valid() {
            return Err(ComputeManagementControlError::Invalid);
        }
        let check_id = request.check_id.clone();
        let cancellation = ModelConnectionProbeCancellationV1::default();
        {
            let mut cancellations = self
                .model_connection_cancellations
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            if cancellations.len() >= 256 || cancellations.contains_key(&check_id) {
                return Err(ComputeManagementControlError::Conflict);
            }
            cancellations.insert(check_id.clone(), cancellation.clone());
        }
        let checked = (|| {
            let mut draft = self.trusted_registered_draft(&request)?;
            self.validate_registered_source_edit(&request, &draft)?;
            let candidate = &request.input_candidate;
            if draft
                .candidate_ref
                .as_ref()
                .is_some_and(|current| current != &candidate.candidate_ref)
            {
                return Err(ComputeManagementControlError::Conflict);
            }
            draft.candidate_ref = Some(candidate.candidate_ref.clone());
            let secret = {
                let inputs = self
                    .manual_protected_inputs
                    .lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                let source = inputs
                    .get(&candidate.candidate_ref)
                    .ok_or(ComputeManagementControlError::NotFound)?;
                ProtectedSecret::new(source.expose().to_vec())
                    .map_err(|_| ComputeManagementControlError::Corrupt)?
            };
            self.model_connections
                .check(
                    draft,
                    NativeModelConnectionCredentialV1::Protected {
                        descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                        input_slot: candidate.candidate_ref.clone(),
                        secret: &secret,
                    },
                    &cancellation,
                )
                .map_err(|_| ComputeManagementControlError::Invalid)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }

    fn prepare_discovered_model_connection(
        &self,
        request: hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1,
    ) -> Result<ComputeCandidateViewV2, ComputeManagementControlError> {
        self.prepare_discovered_candidate(request)
    }

    fn check_saved_model_connection(
        &self,
        request: SavedModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError> {
        if !request.valid() {
            return Err(ComputeManagementControlError::Invalid);
        }
        let check_id = request.check_id.clone();
        let cancellation = ModelConnectionProbeCancellationV1::default();
        {
            let mut cancellations = self
                .model_connection_cancellations
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            if cancellations.len() >= 256 || cancellations.contains_key(&check_id) {
                return Err(ComputeManagementControlError::Conflict);
            }
            cancellations.insert(check_id.clone(), cancellation.clone());
        }
        let checked = (|| {
            let prepared = self.prepare_saved_model_connection(&request)?;
            self.model_connections
                .restore_saved_candidate_ref(&prepared.candidate)
                .map_err(|_| ComputeManagementControlError::Corrupt)?;
            let credential = match prepared.credential.as_ref() {
                Some(credential) => NativeModelConnectionCredentialV1::Saved {
                    credential_id: credential.credential_id.clone(),
                    expected_generation: credential.generation,
                    secret: &credential.secret,
                },
                None => NativeModelConnectionCredentialV1::NotRequired,
            };
            self.model_connections
                .check(prepared.draft, credential, &cancellation)
                .map_err(|_| ComputeManagementControlError::Invalid)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }

    fn cancel_native_model_connection_check(
        &self,
        check_id: &str,
    ) -> Result<(), ComputeManagementControlError> {
        if check_id.trim().is_empty() {
            return Err(ComputeManagementControlError::Invalid);
        }
        if let Some(cancellation) = self
            .model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .get(check_id)
        {
            cancellation.cancel();
        }
        Ok(())
    }

    fn get_compute_candidate(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<ComputeCandidateViewV2, ComputeManagementControlError> {
        self.model_connections
            .candidate_port()
            .get_compute_candidate(candidate)
            .map_err(map_port)
    }

    fn compute_management_snapshot(
        &self,
        query: &ComputeManagementQueryV2,
    ) -> Result<ComputeManagementSnapshotV2, ComputeManagementControlError> {
        // Price/catalog collection takes the same storage mutex internally. Capture those safe
        // facts first, then take one repository snapshot without nesting the writer lock.
        let presentation = match self.compute_management_presentation_facts() {
            Ok(presentation) => presentation,
            Err(_) => ComputeManagementPresentationFactsV1 {
                evaluated_at_ms: current_unix_ms()?,
                complete: false,
                ..Default::default()
            },
        };
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        query_compute_management_with_presentation(
            stores.control(),
            stores.runtime(),
            &WorkspaceId::default(),
            query,
            Some(&presentation),
        )
        .map_err(|error| match error {
            hiroute_application::compute_management::ComputeManagementQueryErrorV2::NotFound => {
                ComputeManagementControlError::NotFound
            }
            hiroute_application::compute_management::ComputeManagementQueryErrorV2::Port(port) => {
                map_port(port)
            }
            hiroute_application::compute_management::ComputeManagementQueryErrorV2::Corrupt => {
                ComputeManagementControlError::Corrupt
            }
        })
    }

    fn preview_compute_save(
        &self,
        change: ComputeManagementChangeV2,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        self.validate_subscription_save_change(&change)?;
        self.bind_saved_key_inputs(&change)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview(change)
        .map(|preview| preview.result)
        .map_err(map_planning)
    }

    fn prepare_compute_save(
        &self,
        request: ComputeConnectionApplyRequestV1,
    ) -> Result<hiroute_application::PreparedTransactionV1, ComputeManagementControlError> {
        let change =
            serde_json::from_value::<ComputeManagementChangeV2>(request.spec.desired_state.clone())
                .map_err(|_| ComputeManagementControlError::Invalid)?;
        self.validate_subscription_save_change(&change)?;
        self.bind_saved_key_inputs(&change)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .prepare_apply(request)
        .map_err(map_planning)
    }

    fn compute_save_result(
        &self,
        requested: &OperationReferenceV1,
    ) -> Result<ComputeSaveResultV2, ComputeManagementControlError> {
        let operation_id = OperationId::parse(&requested.operation_id)
            .map_err(|_| ComputeManagementControlError::Invalid)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let operation = stores
            .control()
            .load_operation(&operation_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        let change = serde_json::from_value::<ComputeManagementChangeV2>(
            operation.plan.spec().desired_state.clone(),
        )
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let (candidate, validation) = match change.subject {
            ComputeManagementSubjectV2::Candidate { candidate } => {
                (Some(candidate), change.validation)
            }
            ComputeManagementSubjectV2::SavedSource { .. } => (None, change.validation),
        };
        let source = if operation.state == OperationState::Succeeded {
            operation
                .plan
                .spec()
                .resource_id
                .as_deref()
                .map(|source_id| stores.control().compute_management_source(source_id))
                .transpose()
                .map_err(map_port)?
                .flatten()
        } else {
            None
        };
        let disposition = save_disposition(operation.state);
        let reference = OperationReferenceV1 {
            operation_id: operation.operation_id.to_string(),
            state: operation.state.as_str().into(),
            sequence: operation.generation,
            cancellable: !operation.state.is_terminal(),
        };
        Ok(ComputeSaveResultV2 {
            candidate,
            validation,
            disposition,
            source_id: source.as_ref().map(|source| source.source_id.clone()),
            bindings: source
                .as_ref()
                .map(|source| {
                    source
                        .models
                        .iter()
                        .map(|model| ComputeSavedBindingV2 {
                            model_ref: model.model_ref.clone(),
                            binding_id: model.binding_id.clone(),
                            revision: model.revision,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            saved_revision: source.as_ref().map(|source| source.revision),
            management_state: source.as_ref().map(|source| source.state),
            operation: Some(reference),
            reason: operation.safe_error_code,
        })
    }
}

fn save_disposition(state: OperationState) -> ComputeSaveDispositionV2 {
    match state {
        OperationState::Succeeded => ComputeSaveDispositionV2::Saved,
        OperationState::NeedsAttention => ComputeSaveDispositionV2::NeedsInput,
        OperationState::RolledBack => ComputeSaveDispositionV2::Failed,
        _ => ComputeSaveDispositionV2::Pending,
    }
}

fn current_unix_ms() -> Result<i64, ComputeManagementControlError> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ComputeManagementControlError::Unavailable)?
        .as_millis();
    i64::try_from(millis).map_err(|_| ComputeManagementControlError::Unavailable)
}

fn trusted_draft(
    draft: hiroute_application_api::NativeUserModelConnectionDraftV1,
    catalog: Option<&TrustedReleaseCatalog>,
) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
    if draft.configuration_revision == 0
        || draft.models.iter().any(|model| {
            model.catalog_configuration_id.is_some()
                || model.membership != ComputeModelMembershipV2::UserDeclared
        })
    {
        return Err(ComputeManagementControlError::Invalid);
    }
    let header_semantics = GatewayHeaderSemanticsV1 {
        content_type: "application/json".into(),
        required_headers: if draft.protocol == UpstreamProtocol::Messages {
            vec![("anthropic-version".into(), "2023-06-01".into())]
        } else {
            Vec::new()
        },
        forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
    };
    Ok(NativeModelConnectionDraftV1 {
        display_template_id: draft.display_template_id,
        inference_model_id: draft.inference_model_id,
        candidate_ref: draft.candidate_ref,
        lineage_ref: draft.lineage_ref,
        trusted_lineage_digest: None,
        display_name: draft.display_name,
        existing_source_id: draft.existing_source_id,
        edit_revision: draft.edit_revision,
        check_id: draft.check_id,
        base_url: draft.base_url,
        base_kind: match draft.base_kind {
            NativeModelConnectionBaseKindV1::ApiRoot => ModelConnectionBaseKindV1::ApiRoot,
            NativeModelConnectionBaseKindV1::NativeMessagesBase => {
                ModelConnectionBaseKindV1::NativeMessagesBase
            }
            NativeModelConnectionBaseKindV1::NativeResponsesBase => {
                ModelConnectionBaseKindV1::NativeResponsesBase
            }
        },
        request_path_override: draft.request_path_override,
        inventory_path_override: draft.inventory_path_override,
        protocol: draft.protocol,
        protocol_profile_id: draft.protocol_profile_id,
        protocol_profile_revision: draft.protocol_profile_revision,
        protocol_header_semantics: header_semantics,
        authentication: draft.authentication,
        provenance: NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision: draft.configuration_revision,
        },
        qualification: NativeConnectionQualificationV1 {
            free_access: None,
            evidence_ref: None,
        },
        runtime_fallback_denied_model_ids: runtime_fallback_denied_model_ids(catalog),
        models: draft.models.into_iter().map(user_model).collect(),
    })
}

fn user_model(
    model: hiroute_application_api::NativeUserModelDeclarationV1,
) -> NativeModelDeclarationV1 {
    NativeModelDeclarationV1 {
        upstream_model_id: model.upstream_model_id,
        display_name: model.display_name,
        catalog_configuration_id: None,
        membership: ComputeModelMembershipV2::UserDeclared,
        capabilities: NativeModelCapabilityDeclarationV1 {
            tool: fact(model.capabilities.tool),
            vision: fact(model.capabilities.vision),
            streaming: fact(model.capabilities.streaming),
            context_tokens: fact(model.capabilities.context_tokens),
            max_output_tokens: fact(model.capabilities.max_output_tokens),
            native_reasoning: fact(model.capabilities.native_reasoning),
        },
    }
}

fn runtime_fallback_denied_model_ids(
    catalog: Option<&TrustedReleaseCatalog>,
) -> std::collections::BTreeSet<String> {
    catalog
        .map(TrustedReleaseCatalog::runtime_fallback_denied_model_ids)
        .unwrap_or_default()
}

fn fact<T>(
    value: hiroute_application_api::NativeUserFactValueV1<T>,
) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: value.value,
        basis: match value.basis {
            NativeUserFactBasisV1::UserDeclared => NativeCandidateFactBasisV1::UserDeclared,
            NativeUserFactBasisV1::Unknown => NativeCandidateFactBasisV1::Unknown,
        },
    }
}

fn map_planning(error: ComputeManagementPlanningErrorV2) -> ComputeManagementControlError {
    match error {
        ComputeManagementPlanningErrorV2::InvalidChange
        | ComputeManagementPlanningErrorV2::InvalidCandidate
        | ComputeManagementPlanningErrorV2::LineageConflict
        | ComputeManagementPlanningErrorV2::ValidationConflict
        | ComputeManagementPlanningErrorV2::ModelNotSelectable
        | ComputeManagementPlanningErrorV2::CredentialRequired
        | ComputeManagementPlanningErrorV2::CredentialConflict
        | ComputeManagementPlanningErrorV2::InvalidKeyEdit => {
            ComputeManagementControlError::Invalid
        }
        ComputeManagementPlanningErrorV2::SourceNotFound => ComputeManagementControlError::NotFound,
        ComputeManagementPlanningErrorV2::ApprovalRequired => {
            ComputeManagementControlError::ActionRequired
        }
        ComputeManagementPlanningErrorV2::RevisionConflict => {
            ComputeManagementControlError::Conflict
        }
        ComputeManagementPlanningErrorV2::PreviewStale => {
            ComputeManagementControlError::PreviewStale
        }
        ComputeManagementPlanningErrorV2::Port(port) => map_port(port),
    }
}

fn map_subscription_preparation(
    error: SubscriptionPreparationError,
) -> ComputeManagementControlError {
    match error {
        SubscriptionPreparationError::NeedsApproval => {
            ComputeManagementControlError::ActionRequired
        }
        SubscriptionPreparationError::InvalidRequest
        | SubscriptionPreparationError::NeedsCredential
        | SubscriptionPreparationError::InvalidModelSelection => {
            ComputeManagementControlError::Invalid
        }
    }
}

fn map_port(error: PortError) -> ComputeManagementControlError {
    match error.code {
        PortErrorCode::NotFound => ComputeManagementControlError::NotFound,
        PortErrorCode::Conflict => ComputeManagementControlError::Conflict,
        PortErrorCode::Unavailable => ComputeManagementControlError::Unavailable,
        PortErrorCode::InvalidData | PortErrorCode::PermissionDenied => {
            ComputeManagementControlError::Invalid
        }
        PortErrorCode::Corrupt => ComputeManagementControlError::Corrupt,
        _ => ComputeManagementControlError::Unavailable,
    }
}

#[cfg(test)]
mod save_disposition_tests {
    use super::*;

    #[test]
    fn unfinished_save_is_never_reported_as_saved_or_unchanged() {
        for state in [
            OperationState::Accepted,
            OperationState::Activating,
            OperationState::RollingBack,
        ] {
            assert_eq!(save_disposition(state), ComputeSaveDispositionV2::Pending);
        }
        assert_eq!(
            save_disposition(OperationState::Succeeded),
            ComputeSaveDispositionV2::Saved
        );
        assert_eq!(
            save_disposition(OperationState::RolledBack),
            ComputeSaveDispositionV2::Failed
        );
        assert_eq!(
            save_disposition(OperationState::NeedsAttention),
            ComputeSaveDispositionV2::NeedsInput
        );
    }
}
