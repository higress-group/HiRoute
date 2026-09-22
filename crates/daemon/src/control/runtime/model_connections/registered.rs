//! Trusted synthesis for catalog-registered model checks and same-source edit fencing.

use super::{LocalControlAdapter, map_port};
use hiroute_application::control::ComputeManagementControlError;
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateTargetV2, ComputeCatalogProvenanceViewV1,
    ComputeModelMembershipV2, RegisteredModelConnectionCheckRequestV1,
};
use hiroute_domain::{
    AuthenticationKind, CanonicalDigest, ComputeManagementProvenanceV2,
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, ConnectionOrigin,
    ProtocolEndpointV1, ResolvedConnectionOptionV1,
};
use hiroute_integrations::{
    ModelConnectionBaseKindV1, ModelConnectionTargetInputV1, NativeCandidateFactBasisV1,
    NativeCandidateFactValueV1, NativeConnectionProvenanceInputV1, NativeConnectionQualificationV1,
    NativeModelCapabilityDeclarationV1, NativeModelConnectionDraftV1, NativeModelDeclarationV1,
    normalize_model_connection_target,
};

impl LocalControlAdapter {
    pub(super) fn trusted_registered_draft(
        &self,
        request: &RegisteredModelConnectionCheckRequestV1,
    ) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
        let source = if let (Some(source_id), Some(expected_revision)) = (
            request.existing_source_id.as_deref(),
            request.expected_source_revision,
        ) {
            let source = self
                .stores_lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?
                .control()
                .compute_management_source(source_id)
                .map_err(map_port)?
                .ok_or(ComputeManagementControlError::NotFound)?;
            source
                .validate()
                .map_err(|_| ComputeManagementControlError::Corrupt)?;
            if source.revision != expected_revision {
                return Err(ComputeManagementControlError::Conflict);
            }
            Some(source)
        } else {
            None
        };
        self.trusted_registered_draft_for_source(request, source.as_ref())
    }

    pub(super) fn trusted_registered_draft_for_saved_source(
        &self,
        request: &RegisteredModelConnectionCheckRequestV1,
        source: &ComputeManagementSourceV2,
    ) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
        self.trusted_registered_draft_for_source(request, Some(source))
    }

    fn trusted_registered_draft_for_source(
        &self,
        request: &RegisteredModelConnectionCheckRequestV1,
        source: Option<&ComputeManagementSourceV2>,
    ) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let public = ComputeCatalogProvenanceViewV1 {
            product_release: provenance.product_release.clone(),
            catalog_binding_id: provenance.catalog_binding_id.clone(),
            release_sequence: provenance.release_sequence,
            connector_registry_digest: provenance.connector_registry_digest.clone(),
            model_data_digest: provenance.model_data_digest.clone(),
            cross_reference_digest: provenance.cross_reference_digest.clone(),
        };
        if public != request.expected_catalog {
            return Err(ComputeManagementControlError::RegisteredCatalogChanged);
        }
        let resolved = catalog
            .resolve_connection_option(&request.connection_option_id)
            .map_err(|_| ComputeManagementControlError::RegisteredOptionUnavailable)?;
        if resolved.option.origin == ConnectionOrigin::AgentSubscription
            || resolved.connector.runtime_kind
                != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
            || resolved.connector.authentication != AuthenticationKind::ProviderApiKey
        {
            return Err(ComputeManagementControlError::RegisteredOptionUnavailable);
        }
        let endpoint = select_registered_endpoint(&resolved, source)?;
        let inventory_path = endpoint.inventory_path.clone();
        let authentication = endpoint
            .authentication_semantics
            .clone()
            .ok_or(ComputeManagementControlError::RegisteredOptionUnavailable)?;
        let model_data = catalog.model_data();
        let native_reasoning = catalog.native_reasoning();
        let mut models = model_data
            .model_endpoint_capabilities
            .iter()
            .filter(|capability| {
                capability.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                    && capability.endpoint_profile_revision == resolved.endpoint_profile.revision
                    && capability.protocol_endpoint_id == endpoint.protocol_endpoint_id
                    && capability.upstream_protocol == endpoint.protocol
                    && capability.connector_id == resolved.connector.connector_id
                    && capability.connector_revision == resolved.connector.revision
            })
            .map(|capability| {
                let model = model_data
                    .model(&capability.model_configuration_id)
                    .ok_or(ComputeManagementControlError::Corrupt)?;
                let reasoning = native_reasoning
                    .iter()
                    .find(|value| value.model_configuration_id == capability.model_configuration_id)
                    .ok_or(ComputeManagementControlError::Corrupt)?;
                Ok(NativeModelDeclarationV1 {
                    upstream_model_id: capability.upstream_model_id.clone(),
                    display_name: model.display_name.clone(),
                    catalog_configuration_id: Some(model.model_configuration_id.clone()),
                    membership: ComputeModelMembershipV2::Catalog,
                    capabilities: NativeModelCapabilityDeclarationV1 {
                        tool: registered_fact(model.capabilities.tool),
                        vision: registered_fact(model.capabilities.vision),
                        streaming: registered_fact(model.capabilities.streaming),
                        context_tokens: registered_fact(model.capabilities.context_tokens),
                        max_output_tokens: registered_fact(model.capabilities.max_output_tokens),
                        native_reasoning: registered_fact(reasoning.capability.clone()),
                    },
                })
            })
            .collect::<Result<Vec<_>, ComputeManagementControlError>>()?;
        for declaration in &request.models {
            if declaration.catalog_configuration_id.is_some()
                || declaration.membership != ComputeModelMembershipV2::UserDeclared
            {
                return Err(ComputeManagementControlError::Invalid);
            }
            // Exact built-in models retain server-owned facts. Additional IDs carry only
            // source-local declarations and can never inherit prices or free qualification.
            if !models
                .iter()
                .any(|model| model.upstream_model_id == declaration.upstream_model_id)
            {
                models.push(super::user_model(declaration.clone()));
            }
        }
        models.sort_by(|left, right| left.upstream_model_id.cmp(&right.upstream_model_id));
        Ok(NativeModelConnectionDraftV1 {
            display_template_id: None,
            inference_model_id: request.inference_model_id.clone(),
            candidate_ref: request.candidate_ref.clone(),
            lineage_ref: request.lineage_ref.clone(),
            trusted_lineage_digest: None,
            display_name: resolved.option.display_name.clone(),
            existing_source_id: request.existing_source_id.clone(),
            edit_revision: request.edit_revision,
            check_id: request.check_id.clone(),
            base_url: endpoint.base_url.clone(),
            base_kind: ModelConnectionBaseKindV1::ApiRoot,
            request_path_override: Some(endpoint.request_path.clone()),
            inventory_path_override: inventory_path,
            protocol: endpoint.protocol,
            protocol_profile_id: endpoint.adapter_ref.clone(),
            protocol_profile_revision: endpoint.adapter_revision,
            protocol_header_semantics: super::registered_endpoint_header_semantics(endpoint),
            authentication,
            provenance: NativeConnectionProvenanceInputV1::Registered {
                connection_option_id: request.connection_option_id.clone(),
                registry_version: catalog.registry().registry_version.clone(),
                catalog_digest: provenance.cross_reference_digest,
            },
            qualification: NativeConnectionQualificationV1 {
                free_access: None,
                evidence_ref: None,
            },
            runtime_fallback_denied_model_ids: super::runtime_fallback_denied_model_ids(Some(
                catalog,
            )),
            models,
        })
    }

    pub(super) fn validate_registered_source_edit(
        &self,
        request: &RegisteredModelConnectionCheckRequestV1,
        draft: &NativeModelConnectionDraftV1,
    ) -> Result<(), ComputeManagementControlError> {
        let (Some(source_id), Some(expected_revision)) = (
            request.existing_source_id.as_deref(),
            request.expected_source_revision,
        ) else {
            return Ok(());
        };
        let source = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .control()
            .compute_management_source(source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        if source.revision != expected_revision {
            return Err(ComputeManagementControlError::Conflict);
        }
        let expected_target = self.validate_registered_source_identity(&source, draft)?;
        let NativeConnectionProvenanceInputV1::Registered {
            connection_option_id,
            ..
        } = &draft.provenance
        else {
            return Err(ComputeManagementControlError::Corrupt);
        };
        let stable_provenance = serde_json::json!({
            "kind": "registered",
            "connection_option_id": connection_option_id,
        });
        let lineage = CanonicalDigest::of(&(
            "hiroute.compute-management-lineage/v2",
            ComputeCandidateProducerV2::Native,
            &draft.lineage_ref,
            stable_provenance,
            &Some(&expected_target),
            &Some(&draft.authentication),
        ))
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        if source.lineage_digest != lineage {
            return Err(ComputeManagementControlError::RegisteredSourceMismatch);
        }
        Ok(())
    }

    pub(super) fn validate_registered_source_identity(
        &self,
        source: &hiroute_domain::ComputeManagementSourceV2,
        draft: &NativeModelConnectionDraftV1,
    ) -> Result<ComputeCandidateTargetV2, ComputeManagementControlError> {
        let NativeConnectionProvenanceInputV1::Registered {
            connection_option_id,
            ..
        } = &draft.provenance
        else {
            return Err(ComputeManagementControlError::Corrupt);
        };
        if !matches!(
            &source.provenance,
            ComputeManagementProvenanceV2::Registered {
                connection_option_id: stored_option,
                ..
            } if stored_option == connection_option_id
        ) {
            return Err(ComputeManagementControlError::RegisteredSourceMismatch);
        }
        let target = normalize_model_connection_target(ModelConnectionTargetInputV1 {
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
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let expected_target = &target.candidate_target;
        if source.target.scheme != expected_target.scheme
            || source.target.authority != expected_target.authority
            || source.target.port != expected_target.port
            || source.target.request_path != expected_target.request_path
            || source.target.upstream_protocol != expected_target.upstream_protocol
            || source.target.protocol_profile_id != expected_target.protocol_profile_id
            || source.target.protocol_profile_revision != expected_target.protocol_profile_revision
            || source.authentication != draft.authentication
            || source.native_recheck.as_ref()
                != Some(&hiroute_domain::ComputeNativeRecheckDescriptorV2 {
                    display_template_id: None,
                    inventory_path: target.inventory_path.clone(),
                    protocol_header_semantics: draft.protocol_header_semantics.clone(),
                })
        {
            return Err(ComputeManagementControlError::RegisteredSourceMismatch);
        }
        Ok(expected_target.clone())
    }
}

fn select_registered_endpoint<'a>(
    resolved: &'a ResolvedConnectionOptionV1,
    source: Option<&ComputeManagementSourceV2>,
) -> Result<&'a ProtocolEndpointV1, ComputeManagementControlError> {
    let endpoints = &resolved.endpoint_profile.protocol_endpoints;
    if let Some(source) = source {
        let mut compatible = endpoints.iter().filter(|endpoint| {
            super::registered_endpoint_matches_source(source, resolved, endpoint)
        });
        let endpoint = compatible
            .next()
            .ok_or(ComputeManagementControlError::RegisteredSourceMismatch)?;
        if compatible.next().is_some() {
            return Err(ComputeManagementControlError::RegisteredSourceMismatch);
        }
        return Ok(endpoint);
    }
    endpoints
        .iter()
        .filter(|endpoint| endpoint.authentication_semantics.is_some())
        .min_by_key(|endpoint| endpoint.stable_preference)
        .ok_or(ComputeManagementControlError::RegisteredOptionUnavailable)
}

fn registered_fact<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{
        GatewayAuthenticationSemanticsV1, NativeReasoningCapabilityV1, UpstreamProtocol,
    };

    const TEST_NAME: &str = "control::runtime::model_connections::registered::tests::registered_drafts_use_only_verified_catalog_facts";

    fn request(
        catalog: &hiroute_integrations::TrustedReleaseCatalog,
        connection_option_id: &str,
    ) -> RegisteredModelConnectionCheckRequestV1 {
        let provenance = catalog.compute_catalog_provenance().unwrap();
        RegisteredModelConnectionCheckRequestV1 {
            inference_model_id: None,
            models: Vec::new(),
            connection_option_id: connection_option_id.into(),
            expected_catalog: ComputeCatalogProvenanceViewV1 {
                product_release: provenance.product_release,
                catalog_binding_id: provenance.catalog_binding_id,
                release_sequence: provenance.release_sequence,
                connector_registry_digest: provenance.connector_registry_digest,
                model_data_digest: provenance.model_data_digest,
                cross_reference_digest: provenance.cross_reference_digest,
            },
            candidate_ref: None,
            lineage_ref: format!("lineage/{connection_option_id}"),
            edit_revision: 1,
            check_id: format!("check/{connection_option_id}/1"),
            input_candidate: hiroute_application_api::ComputeCandidateRefV2 {
                candidate_ref: format!("candidate/native/{connection_option_id}"),
                candidate_revision: 1,
            },
            existing_source_id: None,
            expected_source_revision: None,
        }
    }

    #[test]
    fn registered_drafts_use_only_verified_catalog_facts() {
        if crate::test_support::isolated_agent_home(TEST_NAME) {
            return;
        }
        let catalog = crate::release_catalog::current_fixture_catalog();
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog.clone(),
        )
        .unwrap();
        let listed = hiroute_application::control::ComputeFactsPort::connection_options(
            runtime.adapter.as_ref(),
        )
        .unwrap();
        let listed_bytes = serde_json::to_vec(&listed).unwrap();
        assert!(
            listed_bytes.len() <= hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES - 8 * 1024,
            "current metadata options exceed the bounded Local Control frame budget: {} bytes",
            listed_bytes.len()
        );
        let metadata = listed.metadata_catalog.as_ref().unwrap();
        // Connection clients receive the complete verified catalog. Inference provenance and
        // rule explanations must not disappear at the transport boundary.
        assert_eq!(metadata.provider_records.len(), 103);
        assert_eq!(metadata.model_records.len(), 761);
        assert_eq!(metadata.inference_rules.len(), 187);
        assert_eq!(metadata.evidence_sources.len(), 125);
        assert_eq!(metadata.endpoint_bindings.len(), 25);
        assert!(
            metadata
                .model_records
                .iter()
                .any(|model| !model.field_provenance.is_empty())
        );
        assert!(
            listed
                .options
                .iter()
                .filter(|option| option.origin != ConnectionOrigin::AgentSubscription)
                .all(|option| option.registered_check_available)
        );
        let bailian = runtime
            .adapter
            .trusted_registered_draft(&request(&catalog, "bailian.payg.cn.v1"))
            .unwrap();
        assert_eq!(bailian.base_url, "https://dashscope.aliyuncs.com");
        assert_eq!(
            bailian.request_path_override.as_deref(),
            Some("/compatible-mode/v1/chat/completions")
        );
        assert_eq!(
            bailian.inventory_path_override.as_deref(),
            Some("/api/v1/models")
        );
        assert_eq!(bailian.protocol, UpstreamProtocol::ChatCompletions);
        assert_eq!(
            bailian.authentication,
            GatewayAuthenticationSemanticsV1::Bearer
        );
        assert!(
            bailian
                .protocol_header_semantics
                .required_headers
                .is_empty()
        );

        let anthropic = runtime
            .adapter
            .trusted_registered_draft(&request(&catalog, "anthropic.platform.global.v1"))
            .unwrap();
        assert_eq!(
            anthropic.authentication,
            GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "x-api-key".into(),
            }
        );
        assert_eq!(
            anthropic.protocol_header_semantics.required_headers,
            [("anthropic-version".into(), "2023-06-01".into())]
        );
        assert_eq!(
            bailian
                .models
                .iter()
                .map(|model| model.upstream_model_id.as_str())
                .collect::<Vec<_>>(),
            ["qwen3-max-2026-01-23"]
        );
        assert!(matches!(
            &bailian.models[0].capabilities.native_reasoning,
            NativeCandidateFactValueV1 {
                value: Some(NativeReasoningCapabilityV1::Toggle { parameter }),
                basis: NativeCandidateFactBasisV1::RegisteredCatalog,
            } if parameter == "enable_thinking"
        ));

        let deepseek = runtime
            .adapter
            .trusted_registered_draft(&request(&catalog, "deepseek.official.global.v1"))
            .unwrap();
        assert_eq!(deepseek.base_url, "https://api.deepseek.com");
        assert_eq!(deepseek.inventory_path_override.as_deref(), Some("/models"));
        assert_eq!(deepseek.protocol, UpstreamProtocol::Responses);
        assert!(deepseek.models.is_empty());
        assert!(
            !deepseek
                .runtime_fallback_denied_model_ids
                .contains("deepseek-future-text-model")
        );

        let coding_plan = runtime
            .adapter
            .trusted_registered_draft(&request(&catalog, "zhipu.coding-plan.cn.v1"))
            .unwrap();
        assert_eq!(coding_plan.protocol, UpstreamProtocol::Responses);
        assert_eq!(coding_plan.base_url, "https://open.bigmodel.cn");
        assert_eq!(
            coding_plan.request_path_override.as_deref(),
            Some("/api/v1/responses")
        );
        assert!(
            coding_plan
                .protocol_header_semantics
                .required_headers
                .is_empty()
        );
        assert_eq!(coding_plan.models[0].upstream_model_id, "glm-5.3");

        let mut stale = request(&catalog, "bailian.payg.cn.v1");
        stale.expected_catalog.release_sequence += 1;
        assert!(matches!(
            runtime.adapter.trusted_registered_draft(&stale),
            Err(ComputeManagementControlError::RegisteredCatalogChanged)
        ));
        for supported in [
            "bailian.coding-plan.cn.v1",
            "bailian.token-plan.cn.v1",
            "kimi.code.cn.v1",
            "kimi.open-platform.cn.v1",
            "zhipu.coding-plan.cn.v1",
            "zhipu.general.cn.v1",
        ] {
            let draft = runtime
                .adapter
                .trusted_registered_draft(&request(&catalog, supported))
                .unwrap();
            assert_eq!(draft.inventory_path_override, None);
            assert_eq!(
                draft.authentication,
                GatewayAuthenticationSemanticsV1::Bearer
            );
        }
    }
}
