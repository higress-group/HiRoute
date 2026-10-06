//! Pi sources use the same closed prepare/save transaction as other native API discoveries.
use super::*;
use hiroute_application_api::{ComputeDiscoveryRefV1, ComputeScanItemV1};
use hiroute_integrations::{DiscoveredAuthSource, PiApiSource};

pub(super) fn pi_discovery(source: &PiApiSource) -> ComputeDiscoveryRefV1 {
    ComputeDiscoveryRefV1 {
        discovery_ref: format!("discovery/{}", &source.evidence_digest.as_str()[7..]),
        discovery_revision: source.revision,
    }
}
impl LocalControlAdapter {
    pub(in crate::control::runtime) fn native_compute_scan_items(
        &self,
        dsh: bool,
    ) -> Result<Vec<ComputeScanItemV1>, hiroute_application::control::ControlReadError> {
        let agent_id = if dsh {
            "agent_dsh_default"
        } else {
            "agent_pi_default"
        };
        let sources = match if dsh {
            self.scanner.dsh_api_sources()
        } else {
            self.scanner.pi_api_sources()
        } {
            Ok(sources) => sources,
            Err(_) => {
                return Ok(vec![ComputeScanItemV1 {
                    agent_id: agent_id.into(),
                    supported: false,
                    native_provider_id: None,
                    configuration_state: "configuration_unavailable".into(),
                    discovered_source_ref: None,
                    connection_option_id: None,
                    endpoint_profile_id: None,
                    registered_base_url: None,
                    observed_model_id: None,
                    model_configuration_id: None,
                    inventory_eligible: false,
                    discovery: None,
                    actions_required: vec![
                        if dsh {
                            "check_dsh_configuration"
                        } else {
                            "check_pi_configuration"
                        }
                        .into(),
                    ],
                    permission_action: None,
                    credential_import: None,
                }]);
            }
        };
        sources
            .into_iter()
            .map(|source| {
                let ready = source.supported_auth
                    && source.protocol.is_some()
                    && source.credential.is_some();
                let reason = if !source.supported_auth || source.protocol.is_none() {
                    "native_configuration_not_importable"
                } else {
                    match source.authentication {
                        DiscoveredAuthSource::HelperNeedsInput => {
                            "credential_helper_requires_input"
                        }
                        DiscoveredAuthSource::NativeSessionNeedsConfirmation => {
                            "native_oauth_not_imported"
                        }
                        _ => "credential_import_required",
                    }
                };
                let discovery = ready.then(|| pi_discovery(&source));
                Ok(ComputeScanItemV1 {
                    agent_id: agent_id.into(),
                    supported: true,
                    native_provider_id: Some(source.provider_id),
                    configuration_state: if ready {
                        "static_api_with_protected_input"
                    } else {
                        reason
                    }
                    .into(),
                    discovered_source_ref: None,
                    connection_option_id: None,
                    endpoint_profile_id: None,
                    registered_base_url: None,
                    observed_model_id: Some(source.model_id),
                    model_configuration_id: None,
                    inventory_eligible: ready,
                    discovery,
                    actions_required: if ready {
                        Vec::new()
                    } else {
                        vec![reason.into()]
                    },
                    permission_action: None,
                    credential_import: None,
                })
            })
            .collect()
    }
    pub(super) fn prepare_native_api_candidate(
        &self,
        source: PiApiSource,
        prepare_id: &str,
    ) -> Result<ComputeCandidateViewV2, ComputeManagementControlError> {
        if !source.supported_auth || source.credential.is_none() {
            return Err(ComputeManagementControlError::DiscoveryNotImportable);
        }
        let protocol = source
            .protocol
            .ok_or(ComputeManagementControlError::DiscoveryNotImportable)?;
        let dsh = source.source_ref.starts_with("dsh-source/");
        let (base_url, secret) = (if dsh {
            self.scanner.read_dsh_api_source(&source)
        } else {
            self.scanner.read_pi_api_source(&source)
        })
        .map_err(|_| ComputeManagementControlError::DiscoveryChanged)?;
        let credential = source
            .credential
            .as_ref()
            .ok_or(ComputeManagementControlError::DiscoveryNotImportable)?;
        let input_slot = super::super::mutation::protected_input_slot(credential)
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let descriptor = ProtectedInputSourceDescriptorV1::DiscoveredConfig {
            scanner_id: credential.scanner_id.clone(),
            scanner_version: credential.scanner_version.clone(),
            source_ref: credential.discovered_source_ref.clone(),
            field_selector: credential.field_selector.clone(),
            observed_revision: credential.observed_revision,
        };
        let candidate_digest = hiroute_domain::CanonicalDigest::of(&(
            if dsh {
                "dsh-discovered-candidate/v1"
            } else {
                "pi-discovered-candidate/v1"
            },
            &source.evidence_digest,
            prepare_id,
        ))
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: format!(
                "candidate/native-discovery/{}",
                &candidate_digest.as_str()[7..]
            ),
            candidate_revision: 1,
        };
        if let Ok(view) = self
            .model_connections
            .candidate_port()
            .get_compute_candidate(&candidate)
        {
            return Ok(view);
        }
        self.model_connections
            .reserve_candidate_ref(&candidate)
            .map_err(|_| ComputeManagementControlError::Conflict)?;
        let observed = |v| NativeCandidateFactValueV1 {
            value: v,
            basis: if v.is_some() {
                NativeCandidateFactBasisV1::Observed
            } else {
                NativeCandidateFactBasisV1::Unknown
            },
        };
        let capability = NativeModelCapabilityDeclarationV1 {
            tool: NativeCandidateFactValueV1::unknown(),
            vision: observed(source.vision),
            streaming: NativeCandidateFactValueV1::unknown(),
            context_tokens: NativeCandidateFactValueV1 {
                value: source.context_tokens,
                basis: if source.context_tokens.is_some() {
                    NativeCandidateFactBasisV1::Observed
                } else {
                    NativeCandidateFactBasisV1::Unknown
                },
            },
            max_output_tokens: NativeCandidateFactValueV1 {
                value: source.max_output_tokens,
                basis: if source.max_output_tokens.is_some() {
                    NativeCandidateFactBasisV1::Observed
                } else {
                    NativeCandidateFactBasisV1::Unknown
                },
            },
            native_reasoning: NativeCandidateFactValueV1::unknown(),
        };
        let draft = NativeModelConnectionDraftV1 {
            display_template_id: None,
            inference_model_id: None,
            candidate_ref: Some(candidate.candidate_ref),
            lineage_ref: pi_discovery(&source).discovery_ref,
            trusted_lineage_digest: None,
            display_name: format!(
                "{} · {}",
                if dsh { "DSH" } else { "Pi" },
                source.provider_id
            ),
            existing_source_id: None,
            edit_revision: source.revision,
            check_id: prepare_id.into(),
            base_url: base_url.to_string(),
            base_kind: if protocol == UpstreamProtocol::Messages {
                ModelConnectionBaseKindV1::NativeMessagesBase
            } else {
                ModelConnectionBaseKindV1::NativeResponsesBase
            },
            request_path_override: None,
            inventory_path_override: None,
            protocol,
            protocol_profile_id: match protocol {
                UpstreamProtocol::Responses => "adapter.openai-responses.v1",
                UpstreamProtocol::Messages => "adapter.anthropic-messages.v1",
                UpstreamProtocol::ChatCompletions => "adapter.openai-chat-completions.v1",
            }
            .into(),
            protocol_profile_revision: 1,
            protocol_header_semantics: super::custom_endpoint_headers(protocol),
            authentication: if protocol == UpstreamProtocol::Messages {
                GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                    header: "x-api-key".into(),
                }
            } else {
                GatewayAuthenticationSemanticsV1::Bearer
            },
            additional_native_endpoints: Vec::new(),
            provenance: NativeConnectionProvenanceInputV1::UserConfigured {
                configuration_revision: source.revision,
            },
            qualification: NativeConnectionQualificationV1 {
                free_access: None,
                evidence_ref: None,
            },
            runtime_fallback_denied_model_ids: super::runtime_fallback_denied_model_ids(
                self.release_catalog.as_ref(),
            ),
            models: vec![NativeModelDeclarationV1 {
                upstream_model_id: source.model_id,
                display_name: source.display_name,
                catalog_configuration_id: None,
                membership: ComputeModelMembershipV2::Observed,
                capabilities: capability,
            }],
        };
        self.model_connections
            .prepare_observed_discovery(
                draft,
                NativeModelConnectionCredentialV1::Protected {
                    descriptor,
                    input_slot,
                    secret: &secret,
                },
                source.evidence_digest,
            )
            .map_err(|_| ComputeManagementControlError::Corrupt)
    }
}
