//! Service-owned reconstruction for an exact durable Native source recheck.

use super::{LocalControlAdapter, map_port};
use hiroute_application::control::ComputeManagementControlError;
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeCatalogProvenanceViewV1, ComputeModelMembershipV2,
    RegisteredModelConnectionCheckRequestV1, SavedModelConnectionCheckRequestV1,
};
use hiroute_domain::{
    ComputeManagementFactBasisV2, ComputeManagementMembershipV2, ComputeManagementProvenanceV2,
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, GatewayAuthenticationSemanticsV1,
    PortErrorCode, ProtectedSecret, SecretStorePort, VerifiedSecretSubjectV1,
};
use hiroute_integrations::{
    ModelConnectionBaseKindV1, NativeCandidateFactBasisV1, NativeCandidateFactValueV1,
    NativeConnectionProvenanceInputV1, NativeConnectionQualificationV1,
    NativeModelCapabilityDeclarationV1, NativeModelConnectionDraftV1, NativeModelDeclarationV1,
};

pub(super) struct SavedCredentialMaterial {
    pub(super) credential_id: String,
    pub(super) generation: u64,
    pub(super) secret: ProtectedSecret,
}

pub(super) struct SavedModelConnectionPreparation {
    pub(super) candidate: ComputeCandidateRefV2,
    pub(super) draft: NativeModelConnectionDraftV1,
    pub(super) credential: Option<SavedCredentialMaterial>,
}

impl LocalControlAdapter {
    pub(super) fn prepare_saved_model_connection(
        &self,
        request: &SavedModelConnectionCheckRequestV1,
    ) -> Result<SavedModelConnectionPreparation, ComputeManagementControlError> {
        let source = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .control()
            .compute_management_source(&request.source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        source
            .validate()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        if source.revision != request.expected_source_revision {
            return Err(ComputeManagementControlError::Conflict);
        }
        if request
            .candidate_ref
            .as_ref()
            .is_some_and(|candidate_ref| candidate_ref != &source.last_candidate_ref)
        {
            return Err(ComputeManagementControlError::SavedSourceMismatch);
        }
        if !source.provenance.is_native() {
            return Err(ComputeManagementControlError::Invalid);
        }

        let candidate = ComputeCandidateRefV2 {
            candidate_ref: source.last_candidate_ref.clone(),
            candidate_revision: source.last_candidate_revision,
        };
        candidate
            .validate_shape()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let draft = match &source.provenance {
            ComputeManagementProvenanceV2::Registered { .. } => {
                self.saved_registered_draft(request, &source, &candidate)?
            }
            ComputeManagementProvenanceV2::UserConfigured { .. } => {
                saved_user_draft(request, &source, self.release_catalog.as_ref())?
            }
            ComputeManagementProvenanceV2::ConnectorOwned { .. } => {
                return Err(ComputeManagementControlError::Invalid);
            }
        };
        let credential = self.resolve_saved_model_credential(&source)?;
        Ok(SavedModelConnectionPreparation {
            candidate,
            draft,
            credential,
        })
    }

    fn saved_registered_draft(
        &self,
        request: &SavedModelConnectionCheckRequestV1,
        source: &ComputeManagementSourceV2,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
        let ComputeManagementProvenanceV2::Registered {
            connection_option_id,
            ..
        } = &source.provenance
        else {
            return Err(ComputeManagementControlError::Corrupt);
        };
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let registered = RegisteredModelConnectionCheckRequestV1 {
            inference_model_id: None,
            models: Vec::new(),
            connection_option_id: connection_option_id.clone(),
            expected_catalog: ComputeCatalogProvenanceViewV1 {
                product_release: provenance.product_release,
                catalog_binding_id: provenance.catalog_binding_id,
                release_sequence: provenance.release_sequence,
                connector_registry_digest: provenance.connector_registry_digest,
                model_data_digest: provenance.model_data_digest,
                cross_reference_digest: provenance.cross_reference_digest,
            },
            candidate_ref: Some(candidate.candidate_ref.clone()),
            lineage_ref: format!("saved-source/{}", source.source_id),
            edit_revision: request.edit_revision,
            check_id: request.check_id.clone(),
            input_candidate: candidate.clone(),
            existing_source_id: Some(source.source_id.clone()),
            expected_source_revision: Some(source.revision),
        };
        let mut draft = self
            .trusted_registered_draft_for_saved_source(&registered, source)
            .map_err(|error| match error {
                ComputeManagementControlError::RegisteredSourceMismatch => {
                    ComputeManagementControlError::SavedSourceMismatch
                }
                other => other,
            })?;
        let NativeConnectionProvenanceInputV1::Registered {
            connection_option_id: current_option,
            ..
        } = &draft.provenance
        else {
            return Err(ComputeManagementControlError::Corrupt);
        };
        if current_option != connection_option_id {
            return Err(ComputeManagementControlError::SavedSourceMismatch);
        }
        self.validate_registered_source_identity(source, &draft)
            .map_err(|error| match error {
                ComputeManagementControlError::RegisteredSourceMismatch => {
                    ComputeManagementControlError::SavedSourceMismatch
                }
                other => other,
            })?;
        for model in &source.models {
            if model.catalog_configuration_id.is_none()
                && !draft
                    .models
                    .iter()
                    .any(|declared| declared.upstream_model_id == model.upstream_model_id)
            {
                draft.models.push(saved_model(model)?);
            }
        }
        draft.trusted_lineage_digest = Some(source.lineage_digest.clone());
        Ok(draft)
    }

    fn resolve_saved_model_credential(
        &self,
        source: &ComputeManagementSourceV2,
    ) -> Result<Option<SavedCredentialMaterial>, ComputeManagementControlError> {
        if matches!(
            source.authentication,
            GatewayAuthenticationSemanticsV1::None
        ) {
            return Ok(None);
        }
        let credential = source
            .credentials
            .iter()
            .filter(|credential| credential.enabled)
            .min_by_key(|credential| credential.ordinal)
            .ok_or(ComputeManagementControlError::ActionRequired)?;
        let destination = source
            .target
            .credential_destination()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let subject = VerifiedSecretSubjectV1::from_authenticated_transport(
            "hirouted",
            format!("source/{}", source.source_id),
        )
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let generation = credential.credential.generation();
        let secret = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .secrets()
            .resolve_secret(
                &subject,
                &credential.credential,
                "provider-auth",
                &destination,
                generation,
            )
            .map_err(|error| match error.code {
                PortErrorCode::Conflict => ComputeManagementControlError::Conflict,
                PortErrorCode::NotFound => ComputeManagementControlError::ActionRequired,
                PortErrorCode::PermissionDenied | PortErrorCode::InvalidData => {
                    ComputeManagementControlError::SavedSourceMismatch
                }
                PortErrorCode::Corrupt => ComputeManagementControlError::Corrupt,
                _ => ComputeManagementControlError::Unavailable,
            })?;
        Ok(Some(SavedCredentialMaterial {
            credential_id: credential.credential.credential_id().to_owned(),
            generation,
            secret,
        }))
    }
}

fn saved_user_draft(
    request: &SavedModelConnectionCheckRequestV1,
    source: &ComputeManagementSourceV2,
    catalog: Option<&hiroute_integrations::TrustedReleaseCatalog>,
) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
    let descriptor = source
        .native_recheck
        .as_ref()
        .ok_or(ComputeManagementControlError::RecheckContextUnavailable)?;
    descriptor
        .validate()
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
    let ComputeManagementProvenanceV2::UserConfigured {
        configuration_revision,
        ..
    } = &source.provenance
    else {
        return Err(ComputeManagementControlError::Corrupt);
    };
    Ok(NativeModelConnectionDraftV1 {
        display_template_id: descriptor.display_template_id.clone(),
        inference_model_id: None,
        candidate_ref: Some(source.last_candidate_ref.clone()),
        lineage_ref: format!("saved-source/{}", source.source_id),
        trusted_lineage_digest: Some(source.lineage_digest.clone()),
        display_name: source.display_name.clone(),
        existing_source_id: Some(source.source_id.clone()),
        edit_revision: request.edit_revision,
        check_id: request.check_id.clone(),
        base_url: source_origin(source),
        base_kind: ModelConnectionBaseKindV1::ApiRoot,
        request_path_override: Some(source.target.request_path.clone()),
        inventory_path_override: descriptor.inventory_path.clone(),
        protocol: source.target.upstream_protocol,
        protocol_profile_id: source.target.protocol_profile_id.clone(),
        protocol_profile_revision: source.target.protocol_profile_revision,
        protocol_header_semantics: descriptor.protocol_header_semantics.clone(),
        authentication: source.authentication.clone(),
        additional_native_endpoints: source.additional_native_endpoints.clone(),
        provenance: NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision: *configuration_revision,
        },
        qualification: NativeConnectionQualificationV1 {
            free_access: None,
            evidence_ref: None,
        },
        runtime_fallback_denied_model_ids: super::runtime_fallback_denied_model_ids(catalog),
        models: source
            .models
            .iter()
            .map(saved_model)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn source_origin(source: &ComputeManagementSourceV2) -> String {
    let authority =
        if source.target.authority.contains(':') && !source.target.authority.starts_with('[') {
            format!("[{}]", source.target.authority)
        } else {
            source.target.authority.clone()
        };
    format!(
        "{}://{}:{}",
        source.target.scheme, authority, source.target.port
    )
}

fn saved_model(
    model: &hiroute_domain::ComputeManagedModelV2,
) -> Result<NativeModelDeclarationV1, ComputeManagementControlError> {
    Ok(NativeModelDeclarationV1 {
        upstream_model_id: model.upstream_model_id.clone(),
        display_name: model.display_name.clone(),
        catalog_configuration_id: model.catalog_configuration_id.clone(),
        membership: match model.membership {
            ComputeManagementMembershipV2::Catalog => ComputeModelMembershipV2::Catalog,
            ComputeManagementMembershipV2::Observed => ComputeModelMembershipV2::Observed,
            ComputeManagementMembershipV2::UserDeclared => ComputeModelMembershipV2::UserDeclared,
        },
        capabilities: NativeModelCapabilityDeclarationV1 {
            tool: saved_fact(&model.capabilities.tool)?,
            vision: saved_fact(&model.capabilities.vision)?,
            streaming: saved_fact(&model.capabilities.streaming)?,
            context_tokens: saved_fact(&model.capabilities.context_tokens)?,
            max_output_tokens: saved_fact(&model.capabilities.max_output_tokens)?,
            native_reasoning: saved_fact(&model.capabilities.native_reasoning)?,
        },
    })
}

fn saved_fact<T: Clone>(
    fact: &hiroute_domain::ComputeManagementFactValueV2<T>,
) -> Result<NativeCandidateFactValueV1<T>, ComputeManagementControlError> {
    let basis = match fact.basis {
        ComputeManagementFactBasisV2::RegisteredCatalog => {
            NativeCandidateFactBasisV1::RegisteredCatalog
        }
        ComputeManagementFactBasisV2::RuntimeFallback => {
            NativeCandidateFactBasisV1::RuntimeFallback
        }
        ComputeManagementFactBasisV2::Observed => NativeCandidateFactBasisV1::Observed,
        ComputeManagementFactBasisV2::UserDeclared => NativeCandidateFactBasisV1::UserDeclared,
        ComputeManagementFactBasisV2::Unknown => NativeCandidateFactBasisV1::Unknown,
        ComputeManagementFactBasisV2::ConnectorVerified => {
            return Err(ComputeManagementControlError::Corrupt);
        }
    };
    Ok(NativeCandidateFactValueV1 {
        value: fact.value.clone(),
        basis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use hiroute_application::control::ComputeManagementControlPort;
    use hiroute_domain::{
        COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2, CanonicalDigest, ComputeManagedCapabilitiesV2,
        ComputeManagedCredentialV2, ComputeManagedModelV2, ComputeManagementFactBasisV2,
        ComputeManagementFactValueV2, ComputeManagementMembershipV2, ComputeManagementTargetV2,
        ConnectorRegistryBundleV1, CredentialRefV1, MaterializationState, OperationId,
        ReleaseFactsManifestV2, ReleaseModelDataBundleV2, SecretMutationV1, SecretStorePort,
        WorkspaceId,
    };
    use hiroute_integrations::{
        ModelConnectionProbeCredentialV1, ModelConnectionTargetInputV1,
        ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1, ModelDirectoryTransportV1,
        NormalizedModelConnectionTargetV1, normalize_model_connection_target,
    };

    const TEST_NAME: &str = "control::runtime::model_connections::saved::tests::registered_source_recheck_reconstructs_current_identity_after_restart";

    #[derive(Clone, Default)]
    struct CountingTransport(Arc<AtomicUsize>);

    impl ModelDirectoryTransportV1 for CountingTransport {
        fn get(
            &self,
            target: &NormalizedModelConnectionTargetV1,
            _query: Option<&str>,
            _credential: Option<ModelConnectionProbeCredentialV1<'_>>,
            _timeout: std::time::Duration,
            _response_limit: usize,
        ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
            assert_eq!(target.candidate_target.authority, "dashscope.aliyuncs.com");
            assert_eq!(target.inventory_path.as_deref(), Some("/api/v1/models"));
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ModelDirectoryHttpResponseV1 {
                status: 200,
                body: br#"{"data":[]}"#.to_vec(),
                truncated: false,
            })
        }
    }

    fn registered_request(
        catalog: &hiroute_integrations::TrustedReleaseCatalog,
    ) -> RegisteredModelConnectionCheckRequestV1 {
        let provenance = catalog.compute_catalog_provenance().unwrap();
        RegisteredModelConnectionCheckRequestV1 {
            inference_model_id: None,
            models: Vec::new(),
            connection_option_id: "bailian.payg.cn.v1".into(),
            expected_catalog: ComputeCatalogProvenanceViewV1 {
                product_release: provenance.product_release,
                catalog_binding_id: provenance.catalog_binding_id,
                release_sequence: provenance.release_sequence,
                connector_registry_digest: provenance.connector_registry_digest,
                model_data_digest: provenance.model_data_digest,
                cross_reference_digest: provenance.cross_reference_digest,
            },
            candidate_ref: Some("candidate/native/bailian-saved".into()),
            lineage_ref: "lineage/bailian-saved".into(),
            edit_revision: 1,
            check_id: "check/bailian-saved/1".into(),
            input_candidate: ComputeCandidateRefV2 {
                candidate_ref: "candidate/native/bailian-saved".into(),
                candidate_revision: 3,
            },
            existing_source_id: None,
            expected_source_revision: None,
        }
    }

    fn upgraded_catalog_with_new_preferred_bailian_endpoint()
    -> hiroute_integrations::TrustedReleaseCatalog {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut registry: ConnectorRegistryBundleV1 = serde_json::from_slice(
            &std::fs::read(
                root.join("assets/release-facts/current/bundle/connector-registry.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let mut release: ReleaseModelDataBundleV2 = serde_json::from_slice(
            &std::fs::read(root.join("assets/release-facts/current/bundle/model-data.json"))
                .unwrap(),
        )
        .unwrap();
        let mut manifest: ReleaseFactsManifestV2 = serde_json::from_slice(
            &std::fs::read(root.join("assets/release-facts/current/bundle/manifest.json")).unwrap(),
        )
        .unwrap();
        let profile = registry
            .endpoint_profiles
            .iter_mut()
            .find(|profile| profile.endpoint_profile_id == "endpoint.bailian.payg.cn.v1")
            .unwrap();
        let current = profile
            .protocol_endpoints
            .iter_mut()
            .find(|endpoint| endpoint.protocol_endpoint_id == "endpoint.bailian.payg.cn.v1.chat")
            .unwrap();
        current.stable_preference = 10;
        let mut preferred = current.clone();
        preferred.protocol_endpoint_id = "endpoint.bailian.payg.cn.v1.chat-preferred".into();
        preferred.base_url = "https://preferred-dashscope.example.test".into();
        preferred.stable_preference = 0;
        profile.protocol_endpoints.push(preferred);

        let old_capabilities = release
            .data
            .model_endpoint_capabilities
            .iter()
            .filter(|capability| {
                capability.protocol_endpoint_id == "endpoint.bailian.payg.cn.v1.chat"
            })
            .cloned()
            .collect::<Vec<_>>();
        for mut capability in old_capabilities {
            capability.capability_id.push_str(".preferred");
            capability.protocol_endpoint_id = "endpoint.bailian.payg.cn.v1.chat-preferred".into();
            release.data.model_endpoint_capabilities.push(capability);
        }

        manifest.catalog_id = "hiroute-mvp-current-upgraded".into();
        manifest.sequence += 1;
        let registry_bytes = serde_json::to_vec(&registry).unwrap();
        let model_data_bytes = serde_json::to_vec(&release).unwrap();
        manifest.connector_registry_digest = CanonicalDigest::of_bytes(&registry_bytes);
        manifest.model_data_digest = CanonicalDigest::of_bytes(&model_data_bytes);
        manifest.cross_reference_digest = release.cross_reference_digest(&registry).unwrap();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        hiroute_integrations::TrustedReleaseCatalog::load_release_facts(
            &manifest_bytes,
            &registry_bytes,
            &model_data_bytes,
        )
        .unwrap()
    }

    fn registered_fact<T: Clone>(
        fact: &NativeCandidateFactValueV1<T>,
    ) -> ComputeManagementFactValueV2<T> {
        assert_eq!(fact.basis, NativeCandidateFactBasisV1::RegisteredCatalog);
        ComputeManagementFactValueV2 {
            value: fact.value.clone(),
            basis: ComputeManagementFactBasisV2::RegisteredCatalog,
        }
    }

    fn source_from_registered_draft(
        draft: &NativeModelConnectionDraftV1,
    ) -> ComputeManagementSourceV2 {
        let normalized = normalize_model_connection_target(ModelConnectionTargetInputV1 {
            base_url: &draft.base_url,
            base_kind: draft.base_kind,
            protocol: draft.protocol,
            request_path_override: draft.request_path_override.as_deref(),
            inventory_path_override: draft.inventory_path_override.as_deref(),
            protocol_profile_id: &draft.protocol_profile_id,
            protocol_profile_revision: draft.protocol_profile_revision,
            protocol_header_semantics: &draft.protocol_header_semantics,
            authentication: &draft.authentication,
        })
        .unwrap();
        let NativeConnectionProvenanceInputV1::Registered {
            connection_option_id,
            registry_version,
            catalog_digest,
        } = &draft.provenance
        else {
            panic!("fixture draft must remain registered");
        };
        let model = &draft.models[0];
        let source = ComputeManagementSourceV2 {
            schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
            source_id: "source/registered-bailian".into(),
            revision: 5,
            lineage_digest: CanonicalDigest::of_bytes(b"saved-registered-lineage"),
            display_name: draft.display_name.clone(),
            provenance: ComputeManagementProvenanceV2::Registered {
                connection_option_id: connection_option_id.clone(),
                registry_version: registry_version.clone(),
                catalog_digest: catalog_digest.clone(),
            },
            target: ComputeManagementTargetV2 {
                scheme: normalized.candidate_target.scheme,
                authority: normalized.candidate_target.authority,
                port: normalized.candidate_target.port,
                request_path: normalized.candidate_target.request_path,
                upstream_protocol: normalized.candidate_target.upstream_protocol,
                protocol_profile_id: normalized.candidate_target.protocol_profile_id,
                protocol_profile_revision: normalized.candidate_target.protocol_profile_revision,
            },
            authentication: draft.authentication.clone(),
            state: MaterializationState::NeedsCredential,
            models: vec![ComputeManagedModelV2 {
                model_ref: "model/registered-bailian".into(),
                binding_id: "binding/registered-bailian".into(),
                revision: 1,
                upstream_model_id: model.upstream_model_id.clone(),
                display_name: model.display_name.clone(),
                catalog_configuration_id: model.catalog_configuration_id.clone(),
                membership: ComputeManagementMembershipV2::Catalog,
                execution_eligible: true,
                capabilities: ComputeManagedCapabilitiesV2 {
                    tool: registered_fact(&model.capabilities.tool),
                    vision: registered_fact(&model.capabilities.vision),
                    streaming: registered_fact(&model.capabilities.streaming),
                    context_tokens: registered_fact(&model.capabilities.context_tokens),
                    max_output_tokens: registered_fact(&model.capabilities.max_output_tokens),
                    native_reasoning: registered_fact(&model.capabilities.native_reasoning),
                },
                capability_evidence_digest: CanonicalDigest::of_bytes(
                    b"saved-registered-capabilities",
                ),
            }],
            native_recheck: Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
                display_template_id: None,
                inventory_path: draft.inventory_path_override.clone(),
                protocol_header_semantics: draft.protocol_header_semantics.clone(),
            }),
            additional_native_endpoints: Vec::new(),
            credentials: Vec::new(),
            validation: None,
            last_candidate_ref: "candidate/native/bailian-saved".into(),
            last_candidate_revision: 3,
        };
        source.validate().unwrap();
        source
    }

    fn attach_saved_credential(
        runtime: &super::super::super::ProductionControlRuntime,
        source: &mut ComputeManagementSourceV2,
    ) {
        let destination = source.target.credential_destination().unwrap();
        let key_id = "credential/registered-bailian-primary";
        let secret = ProtectedSecret::new(b"saved-registered-test-secret".to_vec()).unwrap();
        let pending = CredentialRefV1::new(
            key_id,
            format!("source/{}", source.source_id),
            "hirouted",
            "provider-auth",
            [destination.clone()],
            0,
        )
        .unwrap();
        let stores = runtime.adapter.stores_lock().unwrap();
        let fingerprint = stores.secrets().fingerprint(&secret).unwrap();
        let mutation = SecretMutationV1::upsert(
            pending,
            0,
            "registered-bailian-primary",
            Some(fingerprint.clone()),
        )
        .unwrap();
        let effect = stores
            .secrets()
            .apply_secret(
                &OperationId::parse("op_60606060606060606060606060606060").unwrap(),
                &mutation,
                Some(&secret),
            )
            .unwrap();
        stores.secrets().activate_secret(&effect).unwrap();
        source.credentials = vec![ComputeManagedCredentialV2 {
            key_id: key_id.into(),
            credential: CredentialRefV1::new(
                key_id,
                format!("source/{}", source.source_id),
                "hirouted",
                "provider-auth",
                [destination],
                1,
            )
            .unwrap(),
            fingerprint,
            ordinal: 0,
            enabled: true,
        }];
        source.state = MaterializationState::Ready;
        source.validate().unwrap();
    }

    fn user_fact<T>(value: T) -> ComputeManagementFactValueV2<T> {
        ComputeManagementFactValueV2 {
            value: Some(value),
            basis: ComputeManagementFactBasisV2::UserDeclared,
        }
    }

    fn user_source() -> ComputeManagementSourceV2 {
        let source = ComputeManagementSourceV2 {
            schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
            source_id: "source/user-configured".into(),
            revision: 4,
            lineage_digest: CanonicalDigest::of_bytes(b"saved-user-lineage"),
            display_name: "Saved custom API".into(),
            provenance: ComputeManagementProvenanceV2::UserConfigured {
                configuration_revision: 6,
                evidence_digest: CanonicalDigest::of_bytes(b"saved-user-evidence"),
            },
            target: ComputeManagementTargetV2 {
                scheme: "http".into(),
                authority: "127.0.0.1".into(),
                port: 8_812,
                request_path: "/custom/responses".into(),
                upstream_protocol: hiroute_domain::UpstreamProtocol::Responses,
                protocol_profile_id: "profile/custom-responses".into(),
                protocol_profile_revision: 3,
            },
            authentication: GatewayAuthenticationSemanticsV1::None,
            state: MaterializationState::Ready,
            models: vec![ComputeManagedModelV2 {
                model_ref: "model/custom".into(),
                binding_id: "binding/custom".into(),
                revision: 2,
                upstream_model_id: "custom-model-v2".into(),
                display_name: "Custom Model V2".into(),
                catalog_configuration_id: None,
                membership: ComputeManagementMembershipV2::UserDeclared,
                execution_eligible: true,
                capabilities: ComputeManagedCapabilitiesV2 {
                    tool: user_fact(false),
                    vision: user_fact(true),
                    streaming: user_fact(true),
                    context_tokens: user_fact(65_536),
                    max_output_tokens: user_fact(8_192),
                    native_reasoning: user_fact(
                        hiroute_domain::NativeReasoningCapabilityV1::Fixed {
                            profile: "provider-default".into(),
                        },
                    ),
                },
                capability_evidence_digest: CanonicalDigest::of_bytes(b"saved-user-capabilities"),
            }],
            native_recheck: Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
                display_template_id: None,
                inventory_path: Some("/custom/models".into()),
                protocol_header_semantics: hiroute_domain::GatewayHeaderSemanticsV1 {
                    content_type: "application/json".into(),
                    required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
                    forbidden_forward_headers: vec!["authorization".into()],
                },
            }),
            additional_native_endpoints: Vec::new(),
            credentials: Vec::new(),
            validation: None,
            last_candidate_ref: "candidate/native/custom-saved".into(),
            last_candidate_revision: 8,
        };
        source.validate().unwrap();
        source
    }

    #[test]
    fn user_configured_recheck_restores_exact_saved_target_and_context() {
        let source = user_source();
        let request = SavedModelConnectionCheckRequestV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
            candidate_ref: Some(source.last_candidate_ref.clone()),
            edit_revision: 11,
            check_id: "check/custom-saved/11".into(),
        };
        let draft = saved_user_draft(&request, &source, None).unwrap();
        assert_eq!(draft.base_url, "http://127.0.0.1:8812");
        assert_eq!(
            draft.request_path_override.as_deref(),
            Some("/custom/responses")
        );
        assert_eq!(
            draft.inventory_path_override.as_deref(),
            Some("/custom/models")
        );
        assert_eq!(
            draft.protocol_header_semantics,
            source
                .native_recheck
                .as_ref()
                .unwrap()
                .protocol_header_semantics
        );
        assert_eq!(draft.models.len(), 1);
        assert_eq!(draft.models[0].upstream_model_id, "custom-model-v2");
        assert_eq!(
            draft.trusted_lineage_digest,
            Some(source.lineage_digest.clone())
        );
        assert_eq!(draft.existing_source_id, Some(source.source_id.clone()));

        let mut legacy = source;
        legacy.native_recheck = None;
        legacy.validate().unwrap();
        assert!(matches!(
            saved_user_draft(&request, &legacy, None),
            Err(ComputeManagementControlError::RecheckContextUnavailable)
        ));
    }

    #[test]
    fn registered_source_recheck_reconstructs_current_identity_after_restart() {
        if crate::test_support::isolated_agent_home(TEST_NAME) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let catalog = crate::release_catalog::current_fixture_catalog();
        let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog.clone(),
        )
        .unwrap();
        let registered = registered_request(&catalog);
        let initial = runtime
            .adapter
            .trusted_registered_draft(&registered)
            .unwrap();
        let source = source_from_registered_draft(&initial);
        let saved = SavedModelConnectionCheckRequestV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
            candidate_ref: Some(source.last_candidate_ref.clone()),
            edit_revision: 7,
            check_id: "check/bailian-saved/recheck".into(),
        };
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: source.last_candidate_ref.clone(),
            candidate_revision: source.last_candidate_revision,
        };
        let before_restart = runtime
            .adapter
            .saved_registered_draft(&saved, &source, &candidate)
            .unwrap();
        assert_eq!(
            before_restart.trusted_lineage_digest,
            Some(source.lineage_digest.clone())
        );
        assert_eq!(
            source
                .native_recheck
                .as_ref()
                .unwrap()
                .inventory_path
                .as_deref(),
            Some("/api/v1/models")
        );
        assert!(matches!(
            &before_restart.provenance,
            NativeConnectionProvenanceInputV1::Registered { .. }
        ));
        drop(runtime);

        let reopened = super::super::super::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog,
        )
        .unwrap();
        let after_restart = reopened
            .adapter
            .saved_registered_draft(&saved, &source, &candidate)
            .unwrap();
        assert_eq!(after_restart.base_url, before_restart.base_url);
        assert_eq!(after_restart.models, before_restart.models);
        assert_eq!(
            after_restart.existing_source_id,
            Some(source.source_id.clone())
        );

        let mut cross_source = source.clone();
        cross_source.target.request_path = "/v1/substituted".into();
        cross_source.validate().unwrap();
        assert!(matches!(
            reopened
                .adapter
                .saved_registered_draft(&saved, &cross_source, &candidate),
            Err(ComputeManagementControlError::SavedSourceMismatch)
        ));

        let mut catalog_a_source = source;
        let ComputeManagementProvenanceV2::Registered {
            registry_version,
            catalog_digest,
            ..
        } = &mut catalog_a_source.provenance
        else {
            unreachable!();
        };
        registry_version.push_str("-catalog-a");
        *catalog_digest = CanonicalDigest::of_bytes(b"catalog-a");
        catalog_a_source.validate().unwrap();
        let upgraded = reopened
            .adapter
            .saved_registered_draft(&saved, &catalog_a_source, &candidate)
            .unwrap();
        assert_eq!(upgraded.base_url, before_restart.base_url);
        assert!(matches!(
            upgraded.provenance,
            NativeConnectionProvenanceInputV1::Registered {
                registry_version,
                ..
            } if registry_version == reopened
                .adapter
                .release_catalog
                .as_ref()
                .unwrap()
                .registry()
                .registry_version
        ));
    }

    #[test]
    fn saved_registered_recheck_keeps_exact_endpoint_when_upgrade_adds_a_new_preference() {
        if crate::test_support::isolated_agent_home(
            "control::runtime::model_connections::saved::tests::saved_registered_recheck_keeps_exact_endpoint_when_upgrade_adds_a_new_preference",
        ) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let initial_catalog = crate::release_catalog::current_fixture_catalog();
        let initial_runtime =
            super::super::super::ProductionControlRuntime::open_with_release_catalog(
                root.path(),
                initial_catalog.clone(),
            )
            .unwrap();
        let source = source_from_registered_draft(
            &initial_runtime
                .adapter
                .trusted_registered_draft(&registered_request(&initial_catalog))
                .unwrap(),
        );
        drop(initial_runtime);

        let upgraded_catalog = upgraded_catalog_with_new_preferred_bailian_endpoint();
        let upgraded_runtime =
            super::super::super::ProductionControlRuntime::open_with_release_catalog(
                root.path(),
                upgraded_catalog.clone(),
            )
            .unwrap();
        let new_default = upgraded_runtime
            .adapter
            .trusted_registered_draft(&registered_request(&upgraded_catalog))
            .unwrap();
        assert_eq!(
            new_default.base_url,
            "https://preferred-dashscope.example.test"
        );

        let request = SavedModelConnectionCheckRequestV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
            candidate_ref: Some(source.last_candidate_ref.clone()),
            edit_revision: 8,
            check_id: "check/bailian-saved/upgraded".into(),
        };
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: source.last_candidate_ref.clone(),
            candidate_revision: source.last_candidate_revision,
        };
        let recheck = upgraded_runtime
            .adapter
            .saved_registered_draft(&request, &source, &candidate)
            .unwrap();
        assert_eq!(recheck.base_url, "https://dashscope.aliyuncs.com");
        assert_eq!(
            recheck.request_path_override.as_deref(),
            Some("/compatible-mode/v1/chat/completions")
        );
        assert_eq!(
            recheck.inventory_path_override.as_deref(),
            Some("/api/v1/models")
        );
    }

    #[test]
    fn saved_registered_recheck_rejects_changed_inventory_or_static_headers() {
        if crate::test_support::isolated_agent_home(
            "control::runtime::model_connections::saved::tests::saved_registered_recheck_rejects_changed_inventory_or_static_headers",
        ) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let catalog = crate::release_catalog::current_fixture_catalog();
        let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog.clone(),
        )
        .unwrap();
        let source = source_from_registered_draft(
            &runtime
                .adapter
                .trusted_registered_draft(&registered_request(&catalog))
                .unwrap(),
        );
        let request = SavedModelConnectionCheckRequestV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
            candidate_ref: Some(source.last_candidate_ref.clone()),
            edit_revision: 9,
            check_id: "check/bailian-saved/incompatible".into(),
        };
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: source.last_candidate_ref.clone(),
            candidate_revision: source.last_candidate_revision,
        };

        let mut changed_inventory = source.clone();
        changed_inventory
            .native_recheck
            .as_mut()
            .unwrap()
            .inventory_path = Some("/api/v2/models".into());
        changed_inventory.validate().unwrap();
        assert!(matches!(
            runtime
                .adapter
                .saved_registered_draft(&request, &changed_inventory, &candidate),
            Err(ComputeManagementControlError::SavedSourceMismatch)
        ));

        let mut changed_headers = source;
        changed_headers
            .native_recheck
            .as_mut()
            .unwrap()
            .protocol_header_semantics
            .required_headers
            .push(("anthropic-version".into(), "2023-06-01".into()));
        changed_headers.validate().unwrap();
        assert!(matches!(
            runtime
                .adapter
                .saved_registered_draft(&request, &changed_headers, &candidate),
            Err(ComputeManagementControlError::SavedSourceMismatch)
        ));
    }

    #[test]
    fn saved_registered_recheck_fences_descriptors_then_keeps_endpoint_after_upgrade() {
        if crate::test_support::isolated_agent_home(
            "control::runtime::model_connections::saved::tests::saved_registered_recheck_fences_descriptors_then_keeps_endpoint_after_upgrade",
        ) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let storage = root.path().join("storage");
        let catalog = crate::release_catalog::current_fixture_catalog();
        let transport = Arc::new(CountingTransport::default());
        let calls = Arc::clone(&transport.0);
        let runtime =
            super::super::super::ProductionControlRuntime::open_with_release_catalog_and_model_transport(
                &storage,
                catalog.clone(),
                transport.clone(),
            )
            .unwrap();
        let mut source = source_from_registered_draft(
            &runtime
                .adapter
                .trusted_registered_draft(&registered_request(&catalog))
                .unwrap(),
        );
        attach_saved_credential(&runtime, &mut source);
        source.native_recheck.as_mut().unwrap().inventory_path = Some("/api/v2/models".into());
        source.validate().unwrap();
        let connection = rusqlite::Connection::open(storage.join("live/control.db")).unwrap();
        let owner = "op_50505050505050505050505050505050";
        let owner_digest = CanonicalDigest::of_bytes(b"descriptor-mismatch-owner");
        connection
            .execute(
                "INSERT INTO operations(
                    operation_id,workspace_id,principal,operation_kind,idempotency_key,
                    request_digest,accepted_change_digest,state,generation,operation_json,
                    created_at,updated_at
                 ) VALUES (?1,?2,'desktop','fixture.management','descriptor-mismatch',
                           ?3,?3,'succeeded',0,'{}',0,0)",
                rusqlite::params![
                    owner,
                    WorkspaceId::default().as_str(),
                    owner_digest.as_str()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO compute_management_sources(
                    workspace_id,source_id,lineage_digest,revision,source_json,
                    owner_operation_id,updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,unixepoch())",
                rusqlite::params![
                    WorkspaceId::default().as_str(),
                    source.source_id,
                    source.lineage_digest.as_str(),
                    source.revision,
                    serde_json::to_string(&source).unwrap(),
                    owner,
                ],
            )
            .unwrap();

        let request = SavedModelConnectionCheckRequestV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
            candidate_ref: Some(source.last_candidate_ref.clone()),
            edit_revision: 10,
            check_id: "check/bailian-saved/pre-network".into(),
        };
        assert!(matches!(
            ComputeManagementControlPort::check_saved_model_connection(
                runtime.adapter.as_ref(),
                request.clone()
            ),
            Err(ComputeManagementControlError::SavedSourceMismatch)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let descriptor = source.native_recheck.as_mut().unwrap();
        descriptor.inventory_path = Some("/api/v1/models".into());
        descriptor
            .protocol_header_semantics
            .required_headers
            .push(("anthropic-version".into(), "2023-06-01".into()));
        source.validate().unwrap();
        connection
            .execute(
                "UPDATE compute_management_sources SET source_json=?1 WHERE source_id=?2",
                rusqlite::params![serde_json::to_string(&source).unwrap(), source.source_id],
            )
            .unwrap();
        let mut header_request = request.clone();
        header_request.edit_revision += 1;
        header_request.check_id = "check/bailian-saved/pre-network-header".into();
        assert!(matches!(
            ComputeManagementControlPort::check_saved_model_connection(
                runtime.adapter.as_ref(),
                header_request
            ),
            Err(ComputeManagementControlError::SavedSourceMismatch)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        source
            .native_recheck
            .as_mut()
            .unwrap()
            .protocol_header_semantics
            .required_headers
            .clear();
        source.validate().unwrap();
        connection
            .execute(
                "UPDATE compute_management_sources SET source_json=?1 WHERE source_id=?2",
                rusqlite::params![serde_json::to_string(&source).unwrap(), source.source_id],
            )
            .unwrap();
        drop(connection);
        drop(runtime);

        let upgraded =
            super::super::super::ProductionControlRuntime::open_with_release_catalog_and_model_transport(
                &storage,
                upgraded_catalog_with_new_preferred_bailian_endpoint(),
                transport,
            )
            .unwrap();
        let checked = ComputeManagementControlPort::check_saved_model_connection(
            upgraded.adapter.as_ref(),
            SavedModelConnectionCheckRequestV1 {
                edit_revision: 12,
                check_id: "check/bailian-saved/upgraded-network".into(),
                ..request
            },
        )
        .unwrap();
        assert_eq!(checked.target.authority, "dashscope.aliyuncs.com");
        assert_eq!(
            checked.target.request_path,
            "/compatible-mode/v1/chat/completions"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
