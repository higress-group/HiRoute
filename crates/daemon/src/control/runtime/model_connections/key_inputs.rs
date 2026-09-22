//! Bind manually entered replacement/additional keys to the persisted source being edited.

use super::{LocalControlAdapter, map_port};
use hiroute_application::compute_management::{
    ComputeCandidateFactsV2, ComputeCandidatePort, ComputeCandidateProvenanceV2,
    ComputeCredentialBindingV2, ProtectedInputSourceDescriptorV1,
};
use hiroute_application::control::ComputeManagementControlError;
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateTargetV2, ComputeCheckCorrelationV2,
    ComputeKeyEditV2, ComputeManagementChangeV2, ComputeManagementSubjectV2,
};
use hiroute_domain::{
    CanonicalDigest, ComputeManagementProvenanceV2, ComputeManagementRepositoryPort,
    GatewayAuthenticationSemanticsV1,
};

impl LocalControlAdapter {
    pub(super) fn bind_saved_key_inputs(
        &self,
        change: &ComputeManagementChangeV2,
    ) -> Result<(), ComputeManagementControlError> {
        let ComputeManagementSubjectV2::SavedSource { source_id } = &change.subject else {
            return Ok(());
        };
        let inputs = change
            .key_edits
            .iter()
            .filter_map(|edit| match edit {
                ComputeKeyEditV2::Add { input_candidate }
                | ComputeKeyEditV2::Replace {
                    input_candidate, ..
                } => Some(input_candidate),
                _ => None,
            })
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            return Ok(());
        }
        let source = self
            .stores_lock()
            .map_err(map_port)?
            .control()
            .compute_management_source(source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        source
            .validate()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        if source.authentication == GatewayAuthenticationSemanticsV1::None {
            return Err(ComputeManagementControlError::Invalid);
        }
        let provenance = match &source.provenance {
            ComputeManagementProvenanceV2::Registered {
                connection_option_id,
                registry_version,
                catalog_digest,
            } => ComputeCandidateProvenanceV2::Registered {
                connection_option_id: connection_option_id.clone(),
                registry_version: registry_version.clone(),
                catalog_digest: catalog_digest.clone(),
            },
            ComputeManagementProvenanceV2::UserConfigured {
                configuration_revision,
                evidence_digest,
            } => ComputeCandidateProvenanceV2::UserConfigured {
                configuration_revision: *configuration_revision,
                evidence_digest: evidence_digest.clone(),
            },
            ComputeManagementProvenanceV2::ConnectorOwned { .. } => {
                return Err(ComputeManagementControlError::Invalid);
            }
        };
        let target = ComputeCandidateTargetV2 {
            scheme: source.target.scheme.clone(),
            authority: source.target.authority.clone(),
            port: source.target.port,
            request_path: source.target.request_path.clone(),
            upstream_protocol: source.target.upstream_protocol,
            protocol_profile_id: source.target.protocol_profile_id.clone(),
            protocol_profile_revision: source.target.protocol_profile_revision,
        };
        for candidate in inputs {
            // These handles are issued only by the protected input registrar. They are not
            // connection-check results, and saving a key must not trigger a network probe.
            if candidate.candidate_revision != 1 {
                return Err(ComputeManagementControlError::Invalid);
            }
            candidate
                .validate_shape()
                .map_err(|_| ComputeManagementControlError::Invalid)?;
            if !self
                .manual_protected_inputs
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?
                .contains_key(&candidate.candidate_ref)
            {
                return Err(ComputeManagementControlError::NotFound);
            }
            let digest = CanonicalDigest::of(&(
                "saved-source-key-input/v1",
                source_id,
                &source.lineage_digest,
                &target,
                &source.authentication,
                candidate,
            ))
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
            // Registration is immutable and idempotent: retrying the same source is safe,
            // while using this input with a different source cannot replace its binding.
            self.model_connections
                .candidate_port()
                .register_compute_candidate(ComputeCandidateFactsV2 {
                    candidate: candidate.clone(),
                    correlation: ComputeCheckCorrelationV2 {
                        candidate_ref: candidate.candidate_ref.clone(),
                        edit_revision: 1,
                        check_id: candidate.candidate_ref.clone(),
                        input_digest: digest.clone(),
                    },
                    producer: ComputeCandidateProducerV2::Native,
                    lineage_ref: format!("saved-source/{source_id}"),
                    trusted_lineage_digest: Some(source.lineage_digest.clone()),
                    display_name: source.display_name.clone(),
                    existing_source_id: Some(source_id.clone()),
                    evidence_digest: digest,
                    provenance: provenance.clone(),
                    target: Some(target.clone()),
                    authentication: Some(source.authentication.clone()),
                    models: Vec::new(),
                    native_recheck: None,
                    discovery_guard: None,
                    credential_binding: ComputeCredentialBindingV2::NativeProtected {
                        descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                        input_slot: candidate.candidate_ref.clone(),
                    },
                    validation: None,
                })
                .map_err(map_port)?;
        }
        Ok(())
    }
}
