//! Keep legacy check carriers while binding all Native checks to a pinned network policy.
use super::*;
use hiroute_application_api::{ModelConnectionCheckViewV2, NativeNetworkEditV1};
use hiroute_domain::NativeNetworkPolicyV1;
use hiroute_integrations::NativeModelConnectionNetworkContextV1;
impl LocalControlAdapter {
    pub(super) fn native_network_check_context(
        &self,
        source: Option<&str>,
        edit: Option<NativeNetworkEditV1>,
    ) -> Result<NativeModelConnectionNetworkContextV1, ComputeManagementControlError> {
        let snapshot = self
            .native_network
            .pin()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let current = source
            .map(|id| snapshot.source(id))
            .transpose()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let revision = current.map_or(0, |r| r.network_revision);
        let mut edit = edit.unwrap_or_else(|| NativeNetworkEditV1 {
            expected_network_revision: revision,
            policy: current.map_or(NativeNetworkPolicyV1::Direct, |r| r.policy.clone()),
        });
        if edit.expected_network_revision != revision {
            return Err(ComputeManagementControlError::Conflict);
        }
        edit.policy = edit
            .policy
            .normalized()
            .map_err(|_| ComputeManagementControlError::Invalid)?;
        Ok(NativeModelConnectionNetworkContextV1 { snapshot, edit })
    }
    pub(super) fn check_native_model_connection_network(
        &self,
        request: NativeModelConnectionCheckRequestV1,
        network_edit: Option<NativeNetworkEditV1>,
    ) -> Result<ModelConnectionCheckViewV2, ComputeManagementControlError> {
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
            let context = self.native_network_check_context(
                draft.existing_source_id.as_deref(),
                network_edit.clone(),
            )?;
            self.model_connections
                .check_with_network(draft, credential, &cancellation, context)
                .map_err(map_check_error)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }
    pub(super) fn check_registered_model_connection_network(
        &self,
        request: RegisteredModelConnectionCheckRequestV1,
        network_edit: Option<NativeNetworkEditV1>,
    ) -> Result<ModelConnectionCheckViewV2, ComputeManagementControlError> {
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
            let context = self.native_network_check_context(
                draft.existing_source_id.as_deref(),
                network_edit.clone(),
            )?;
            self.model_connections
                .check_with_network(
                    draft,
                    NativeModelConnectionCredentialV1::Protected {
                        descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                        input_slot: candidate.candidate_ref.clone(),
                        secret: &secret,
                    },
                    &cancellation,
                    context,
                )
                .map_err(map_check_error)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }
    pub(super) fn check_saved_model_connection_network(
        &self,
        request: SavedModelConnectionCheckRequestV1,
        network_edit: Option<NativeNetworkEditV1>,
    ) -> Result<ModelConnectionCheckViewV2, ComputeManagementControlError> {
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
            let context =
                self.native_network_check_context(Some(&request.source_id), network_edit.clone())?;
            self.model_connections
                .check_with_network(prepared.draft, credential, &cancellation, context)
                .map_err(map_check_error)
        })();
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&check_id);
        checked
    }
}
