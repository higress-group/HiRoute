pub(super) mod defaults;
use hiroute_application::compute_management::{
    ComputeCandidatePort, ComputeManagementPlanner, ComputeManagementPlanningErrorV2,
    ComputeManagementPresentationFactsV1, ProtectedInputSourceDescriptorV1,
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
    PortError, PortErrorCode, ProtectedSecret, ProtocolEndpointV1, SecretStorePort,
    UpstreamProtocol, VerifiedSecretSubjectV1, WorkspaceId,
};
use hiroute_integrations::{
    ModelConnectionBaseKindV1, ModelConnectionProbeCancellationV1, NativeCandidateFactBasisV1,
    NativeCandidateFactValueV1, NativeConnectionProvenanceInputV1, NativeConnectionQualificationV1,
    NativeModelCapabilityDeclarationV1, NativeModelConnectionCredentialV1,
    NativeModelConnectionDraftV1, NativeModelDeclarationV1, TrustedReleaseCatalog,
};

use super::LocalControlAdapter;

mod discovered;
mod error;
mod feed_admission;
mod hermes_points;
mod key_inputs;
mod key_masks;
mod pi_discovered;
pub(in crate::control::runtime) mod point_catalog;
mod points;
mod presentation;
mod registered;
mod saved;
mod saved_prefill;

use error::{map_check_error, map_target_error};

pub(super) fn registered_source_matches_current_option(
    source: &hiroute_domain::ComputeManagementSourceV2,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
) -> bool {
    if resolved.connector.runtime_kind != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
        || resolved.option.origin == hiroute_domain::ConnectionOrigin::AgentSubscription
    {
        return false;
    }
    let primary_matches = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .any(|endpoint| registered_endpoint_matches_source(source, resolved, endpoint));
    primary_matches
        && source.additional_native_endpoints.iter().all(|saved| {
            resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .any(|endpoint| registered_endpoint_matches_saved_extra(saved, resolved, endpoint))
        })
}

pub(super) fn registered_endpoint_matches_saved_extra(
    saved: &hiroute_domain::ComputeNativeEndpointV3,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    endpoint: &ProtocolEndpointV1,
) -> bool {
    native_registered_authentication(resolved.connector.authentication, endpoint)
        == Some(saved.authentication.clone())
        && saved.recheck == registered_endpoint_recheck_descriptor(resolved, endpoint)
        && saved.target.scheme == "https"
        && saved.target.port == 443
        && endpoint.base_url.strip_prefix("https://") == Some(saved.target.authority.as_str())
        && endpoint.request_path == saved.target.request_path
        && endpoint.protocol == saved.target.upstream_protocol
        && registered_saved_adapter_matches(
            endpoint,
            &saved.target.protocol_profile_id,
            saved.target.protocol_profile_revision,
        )
}

/// Retained Token Plan sources keep their originally selected native Responses dialect.
/// This accepts an old choice; it never upgrades that choice to plaintext negotiation.
pub(super) fn registered_saved_adapter_matches(
    endpoint: &ProtocolEndpointV1,
    adapter: &str,
    revision: u64,
) -> bool {
    (endpoint.adapter_ref == adapter && endpoint.adapter_revision == revision)
        || (endpoint.protocol_endpoint_id == "endpoint.bailian.token-plan.cn.v1.responses"
            && endpoint.protocol == hiroute_domain::UpstreamProtocol::Responses
            && endpoint.base_url == "https://token-plan.cn-beijing.maas.aliyuncs.com"
            && endpoint.request_path == "/compatible-mode/v1/responses"
            && endpoint.adapter_ref == "adapter.standard-responses-json.v1"
            && endpoint.adapter_revision == 1
            && adapter == "adapter.openai-responses.v1"
            && revision == 1)
}

fn native_registered_authentication(
    connector: hiroute_domain::AuthenticationKind,
    endpoint: &ProtocolEndpointV1,
) -> Option<GatewayAuthenticationSemanticsV1> {
    let authentication = endpoint.authentication_semantics.clone()?;
    match (connector, &authentication) {
        (hiroute_domain::AuthenticationKind::None, GatewayAuthenticationSemanticsV1::None)
        | (
            hiroute_domain::AuthenticationKind::ProviderApiKey,
            GatewayAuthenticationSemanticsV1::Bearer
            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
        ) => Some(authentication),
        _ => None,
    }
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
    fn manage_subscription_login(
        &self,
        request: hiroute_application_api::ComputeSubscriptionLoginRequestV1,
    ) -> Result<
        hiroute_application_api::ComputeSubscriptionLoginResultV1,
        ComputeManagementControlError,
    > {
        self.subscription_login(request)
    }

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

    fn saved_subscription_candidate(
        &self,
        source: hiroute_application_api::ComputeSavedSubscriptionSourceV1,
    ) -> Result<ComputeSubscriptionCandidatesV2, ComputeManagementControlError> {
        self.saved_subscription_candidates(source)
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
            let expected_source_revision = request.draft.expected_source_revision;
            let mut draft = trusted_draft(
                request.draft,
                self.current_catalog()
                    .as_deref()
                    .map(|snapshot| &snapshot.catalog),
            )?;
            let saved_key = if let Some(source_id) = draft.existing_source_id.as_deref() {
                let expected_revision =
                    expected_source_revision.ok_or(ComputeManagementControlError::Invalid)?;
                let stores = self
                    .stores_lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                let source = stores
                    .control()
                    .compute_management_source(source_id)
                    .map_err(map_port)?
                    .ok_or(ComputeManagementControlError::NotFound)?;
                if source.revision != expected_revision
                    || !matches!(
                        source.provenance,
                        hiroute_domain::ComputeManagementProvenanceV2::UserConfigured { .. }
                    )
                {
                    return Err(ComputeManagementControlError::Conflict);
                }
                draft.trusted_lineage_digest = Some(source.lineage_digest.clone());
                if request.input_candidate.is_some()
                    || draft.authentication == GatewayAuthenticationSemanticsV1::None
                {
                    None
                } else if let Some(key) = source.enabled_credentials().next() {
                    let subject = VerifiedSecretSubjectV1::from_authenticated_transport(
                        "hirouted",
                        format!("source/{}", source.source_id),
                    )
                    .map_err(|_| ComputeManagementControlError::Corrupt)?;
                    let destination = source
                        .target
                        .credential_destination()
                        .map_err(|_| ComputeManagementControlError::Corrupt)?;
                    let secret = stores
                        .secrets()
                        .resolve_secret(
                            &subject,
                            &key.credential,
                            "provider-auth",
                            &destination,
                            key.credential.generation(),
                        )
                        .map_err(map_port)?;
                    Some((key.key_id.clone(), key.credential.generation(), secret))
                } else {
                    None
                }
            } else {
                if expected_source_revision.is_some() {
                    return Err(ComputeManagementControlError::Invalid);
                }
                None
            };
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
                (_, None) => match saved_key.as_ref() {
                    Some((credential_id, expected_generation, secret)) => {
                        NativeModelConnectionCredentialV1::Saved {
                            credential_id: credential_id.clone(),
                            expected_generation: *expected_generation,
                            secret,
                        }
                    }
                    None => NativeModelConnectionCredentialV1::PendingInput,
                },
            };
            self.model_connections
                .check(draft, credential, &cancellation)
                .map_err(map_check_error)
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
                .map_err(map_check_error)
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
                .map_err(map_check_error)
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
        self.compute_management_snapshot_v3(query)
            .map(|view| view.into_v2())
    }

    fn compute_key_masks(
        &self,
        query: &hiroute_application_api::ComputeKeyMasksQueryV1,
    ) -> Result<hiroute_application_api::ComputeKeyMasksViewV1, ComputeManagementControlError> {
        self.saved_key_masks(query)
    }

    fn compute_management_snapshot_v3(
        &self,
        query: &ComputeManagementQueryV2,
    ) -> Result<hiroute_application_api::ComputeManagementSnapshotV3, ComputeManagementControlError>
    {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::PreviewStale)?;
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
        let mut result = hiroute_application::compute_management::query_compute_management_v3(
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
        })?;
        for view in &mut result.sources {
            let saved = stores
                .control()
                .compute_management_source(&view.source_id)
                .map_err(map_port)?
                .ok_or(ComputeManagementControlError::NotFound)?;
            let preferences = stores
                .control()
                .source_default_preferences(&view.source_id)
                .map_err(map_port)?;
            let effective = defaults::project_source(&catalog, &saved, &preferences);
            for model in &mut view.models {
                if let Some(projected) = effective
                    .models
                    .iter()
                    .find(|m| m.binding_id == model.binding_id)
                {
                    model.capabilities = projected.capabilities.clone();
                    model.native_reasoning = projected.capabilities.native_reasoning.value.clone();
                }
            }
        }
        drop(stores);
        if self
            .current_catalog()
            .is_none_or(|c| c.generation != catalog.generation)
        {
            return Err(ComputeManagementControlError::PreviewStale);
        }
        Ok(result)
    }

    fn preview_compute_save(
        &self,
        change: ComputeManagementChangeV2,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        self.validate_subscription_save_change(&change)?;
        self.bind_saved_key_inputs(&change)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let preview = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview(change)
        .map_err(map_planning)?;
        self.validate_feed_save_plan(preview.plan(), &catalog)?;
        if self.model_feed.as_ref().is_some_and(|feed| {
            feed.available_catalog()
                .is_none_or(|current| current.generation != catalog.generation)
        }) {
            return Err(ComputeManagementControlError::Conflict);
        }
        Ok(preview.result)
    }

    fn compute_defaults(
        &self,
        query: hiroute_application_api::ComputeDefaultsQueryV1,
    ) -> Result<hiroute_application_api::ComputeDefaultsResultV1, ComputeManagementControlError>
    {
        LocalControlAdapter::compute_defaults(self, query)
    }
    fn preview_compute_save_v3(
        &self,
        mut change: hiroute_application_api::ComputeManagementChangeV3,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        self.validate_subscription_save_change(&change.change)?;
        self.bind_saved_key_inputs(&change.change)?;
        self.establish_save_defaults(&mut change, &catalog)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let preview = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview_v3(change)
        .map_err(map_planning)?;
        self.validate_feed_save_plan(preview.plan(), &catalog)?;
        if self
            .current_catalog()
            .is_none_or(|current| current.generation != catalog.generation)
        {
            return Err(ComputeManagementControlError::Conflict);
        }
        Ok(preview.result)
    }

    fn prepare_compute_save(
        &self,
        request: ComputeConnectionApplyRequestV1,
    ) -> Result<hiroute_application::PreparedTransactionV1, ComputeManagementControlError> {
        let change =
            hiroute_application_api::saved_compute_change(request.spec.desired_state.clone())
                .map_err(|_| ComputeManagementControlError::Invalid)?;
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        self.validate_subscription_save_change(&change)?;
        self.bind_saved_key_inputs(&change)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let planner = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        );
        let preview = planner
            .preview_saved_state(request.spec.desired_state.clone())
            .map_err(map_planning)?;
        self.validate_feed_save_plan(preview.plan(), &catalog)?;
        let prepared = planner.prepare_apply(request).map_err(map_planning)?;
        let feed = self.model_feed.clone();
        Ok(prepared.with_current_facts_check(move || {
            if feed.as_ref().is_some_and(|feed| {
                feed.available_catalog()
                    .is_none_or(|current| current.generation != catalog.generation)
            }) {
                return Err(hiroute_application::TransactionError::ChangePreviewStale);
            }
            Ok(())
        }))
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
        let change = hiroute_application_api::saved_compute_change(
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
            source_id: source
                .as_ref()
                .map(|source| source.source_id.clone())
                .or_else(|| {
                    (operation.state == OperationState::Succeeded
                        && matches!(
                            change.edit,
                            Some(hiroute_application_api::ComputeManagementEditV1::Delete)
                        ))
                    .then(|| operation.plan.spec().resource_id.clone())
                    .flatten()
                }),
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
    let header_semantics = custom_endpoint_headers(draft.protocol);
    let additional_native_endpoints = draft
        .additional_endpoints
        .iter()
        .map(|endpoint| {
            let headers = custom_endpoint_headers(endpoint.protocol);
            let normalized = hiroute_integrations::normalize_model_connection_target(
                hiroute_integrations::ModelConnectionTargetInputV1 {
                    base_url: &endpoint.base_url,
                    base_kind: match endpoint.base_kind {
                        NativeModelConnectionBaseKindV1::ApiRoot => {
                            ModelConnectionBaseKindV1::ApiRoot
                        }
                        NativeModelConnectionBaseKindV1::NativeMessagesBase => {
                            ModelConnectionBaseKindV1::NativeMessagesBase
                        }
                        NativeModelConnectionBaseKindV1::NativeResponsesBase => {
                            ModelConnectionBaseKindV1::NativeResponsesBase
                        }
                    },
                    protocol: endpoint.protocol,
                    request_path_override: endpoint.request_path_override.as_deref(),
                    inventory_path_override: endpoint.inventory_path_override.as_deref(),
                    protocol_profile_id: &endpoint.protocol_profile_id,
                    protocol_profile_revision: endpoint.protocol_profile_revision,
                    protocol_header_semantics: &headers,
                    authentication: &endpoint.authentication,
                },
            )
            .map_err(map_target_error)?;
            let target = normalized.candidate_target;
            Ok(hiroute_domain::ComputeNativeEndpointV3 {
                target: hiroute_domain::ComputeManagementTargetV2 {
                    scheme: target.scheme,
                    authority: target.authority,
                    port: target.port,
                    request_path: target.request_path,
                    upstream_protocol: target.upstream_protocol,
                    protocol_profile_id: target.protocol_profile_id,
                    protocol_profile_revision: target.protocol_profile_revision,
                },
                authentication: endpoint.authentication.clone(),
                recheck: Some(ComputeNativeRecheckDescriptorV2 {
                    display_template_id: None,
                    inventory_path: normalized.inventory_path,
                    protocol_header_semantics: headers,
                }),
            })
        })
        .collect::<Result<Vec<_>, ComputeManagementControlError>>()?;
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
        additional_native_endpoints,
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

fn custom_endpoint_headers(protocol: UpstreamProtocol) -> GatewayHeaderSemanticsV1 {
    GatewayHeaderSemanticsV1 {
        content_type: "application/json".into(),
        required_headers: if protocol == UpstreamProtocol::Messages {
            vec![("anthropic-version".into(), "2023-06-01".into())]
        } else {
            Vec::new()
        },
        forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
    }
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
