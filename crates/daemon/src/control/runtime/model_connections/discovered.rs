//! Protected candidateization of one exact local registered configuration.

use hiroute_application::ProtectedInputPort;
use hiroute_application::compute_management::{
    ComputeCandidatePort, ProtectedInputSourceDescriptorV1,
};
use hiroute_application::control::{ComputeManagementControlError, ControlReadError};
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeCandidateViewV2, ComputeModelMembershipV2,
    PrepareDiscoveredModelConnectionRequestV1,
};
use hiroute_domain::{
    AuthenticationKind, BillingClass, CanonicalDigest, ConnectionOrigin,
    GatewayAuthenticationSemanticsV1, UpstreamProtocol,
};
use hiroute_integrations::{
    AgentDiscoveryOutcomeV1, ModelConnectionBaseKindV1, ModelConnectionProbeCancellationV1,
    NativeCandidateFactBasisV1, NativeCandidateFactValueV1, NativeConnectionProvenanceInputV1,
    NativeConnectionQualificationV1, NativeModelCapabilityDeclarationV1,
    NativeModelConnectionCredentialV1, NativeModelConnectionDraftV1, NativeModelDeclarationV1,
};

use super::super::compute_routing::{AuthorizedComputeDiscoveryV1, authorized_compute_discovery};
use super::LocalControlAdapter;

const CLAUDE_AGENT_ID: &str = "agent_claude_default";
const CONNECTION_OPTION_ID: &str = "zhipu.coding-plan.cn.v1";
const ENDPOINT_PROFILE_ID: &str = "endpoint.zhipu.coding-plan.cn.v1";
const DISCOVERED_PROTOCOL_ENDPOINT_ID: &str = "endpoint.zhipu.coding-plan.cn.v1.messages";
const ROUTING_PROTOCOL_ENDPOINT_ID: &str = "endpoint.zhipu.coding-plan.cn.v1.responses";
const MODEL_CONFIGURATION_ID: &str = "model.zhipu.glm-5.3";
const UPSTREAM_MODEL_ID: &str = "glm-5.3";
const MESSAGES_ADAPTER_ID: &str = "adapter.anthropic-messages.v1";
const RESPONSES_ADAPTER_ID: &str = "adapter.openai-responses.v1";

impl LocalControlAdapter {
    pub(super) fn prepare_discovered_candidate(
        &self,
        request: PrepareDiscoveredModelConnectionRequestV1,
    ) -> Result<ComputeCandidateViewV2, ComputeManagementControlError> {
        if !request.valid() {
            return Err(ComputeManagementControlError::Invalid);
        }
        {
            let preparations = self
                .prepared_discoveries
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            if let Some(bound) = preparations.get(&request.prepare_id) {
                if bound != &request {
                    return Err(ComputeManagementControlError::Conflict);
                }
            } else if preparations.len() >= 256 {
                return Err(ComputeManagementControlError::Unavailable);
            }
        }
        let lock_key = format!("prepare-discovery/{}", request.prepare_id);
        {
            let mut preparations = self
                .model_connection_cancellations
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            if preparations.len() >= 256 || preparations.contains_key(&lock_key) {
                return Err(ComputeManagementControlError::Conflict);
            }
            preparations.insert(
                lock_key.clone(),
                ModelConnectionProbeCancellationV1::default(),
            );
        }
        let prepared = self.prepare_discovered_candidate_inner(request.clone());
        self.model_connection_cancellations
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&lock_key);
        if prepared.is_ok() {
            let mut preparations = self
                .prepared_discoveries
                .lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            match preparations.get(&request.prepare_id) {
                Some(bound) if bound != &request => {
                    return Err(ComputeManagementControlError::Conflict);
                }
                Some(_) => {}
                None if preparations.len() < 256 => {
                    preparations.insert(request.prepare_id.clone(), request);
                }
                None => return Err(ComputeManagementControlError::Unavailable),
            }
        }
        prepared
    }

    fn prepare_discovered_candidate_inner(
        &self,
        request: PrepareDiscoveredModelConnectionRequestV1,
    ) -> Result<ComputeCandidateViewV2, ComputeManagementControlError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let discoveries = self
            .refresh_discovery()
            .map_err(|_| ComputeManagementControlError::DiscoveryUnavailable)?;
        let mut authorized = Vec::new();
        let mut recognized_configuration = false;
        for discovery in &discoveries {
            let is_claude = match &discovery.outcome {
                AgentDiscoveryOutcomeV1::Supported { installation } => {
                    installation.agent_id == CLAUDE_AGENT_ID
                }
                AgentDiscoveryOutcomeV1::ReportOnly { agent_id, .. } => agent_id == CLAUDE_AGENT_ID,
            };
            recognized_configuration |= is_claude
                && (discovery.claude_configuration.is_some()
                    || discovery.configuration_issue.is_some()
                    || discovery.discovered_credential.is_some());
            match authorized_compute_discovery(catalog, discovery) {
                Ok(Some(candidate)) => authorized.push(candidate),
                Ok(None) => {}
                Err(ControlReadError::Corrupt) => {
                    return Err(ComputeManagementControlError::Corrupt);
                }
                Err(_) => return Err(ComputeManagementControlError::Unavailable),
            }
        }
        let current_exists = !authorized.is_empty();
        let Some(discovery) = authorized.into_iter().find(|candidate| {
            candidate.discovery.discovery_ref == request.discovery.discovery_ref
                && candidate.discovery.discovery_revision == request.discovery.discovery_revision
        }) else {
            return Err(if current_exists {
                ComputeManagementControlError::DiscoveryChanged
            } else if recognized_configuration {
                ComputeManagementControlError::DiscoveryNotImportable
            } else {
                ComputeManagementControlError::DiscoveryUnavailable
            });
        };
        self.validate_discovered_allowlist(&discovery)?;

        let descriptor = ProtectedInputSourceDescriptorV1::DiscoveredConfig {
            scanner_id: discovery.credential.scanner_id.clone(),
            scanner_version: discovery.credential.scanner_version.clone(),
            source_ref: discovery.credential.discovered_source_ref.clone(),
            field_selector: discovery.credential.field_selector.clone(),
            observed_revision: discovery.credential.observed_revision,
        };
        let input_slot = super::super::mutation::protected_input_slot(&discovery.credential)
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        // This is the only prepare-time credential access. It reopens the exact no-follow source,
        // checks its revision, and immediately drops the bytes after candidate registration.
        let secret = self
            .read_secret(&input_slot)
            .map_err(|_| ComputeManagementControlError::DiscoveryChanged)?;
        let candidate_digest = CanonicalDigest::of(&(
            "hiroute.discovered-model-candidate/v1",
            &discovery.evidence_digest,
            &request.prepare_id,
        ))
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: format!(
                "candidate/native-discovery/{}",
                candidate_digest.as_str().trim_start_matches("sha256:")
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
        let draft =
            self.discovered_registered_draft(&discovery, &candidate, &request.prepare_id)?;
        self.model_connections
            .prepare_registered_discovery(
                draft,
                NativeModelConnectionCredentialV1::Protected {
                    descriptor,
                    input_slot,
                    secret: &secret,
                },
                discovery.evidence_digest,
            )
            .map_err(|_| ComputeManagementControlError::Corrupt)
    }

    fn validate_discovered_allowlist(
        &self,
        discovery: &AuthorizedComputeDiscoveryV1,
    ) -> Result<(), ComputeManagementControlError> {
        let fact = &discovery.fact;
        if fact.agent_id != CLAUDE_AGENT_ID
            || fact.connection_option_id != CONNECTION_OPTION_ID
            || fact.endpoint_profile_id != ENDPOINT_PROFILE_ID
            || fact.model_configuration_id != MODEL_CONFIGURATION_ID
            || fact.observed_model_id != UPSTREAM_MODEL_ID
            || discovery.credential.field_selector != "env.ANTHROPIC_AUTH_TOKEN"
        {
            return Err(ComputeManagementControlError::DiscoveryNotImportable);
        }
        Ok(())
    }

    fn discovered_registered_draft(
        &self,
        discovery: &AuthorizedComputeDiscoveryV1,
        candidate: &ComputeCandidateRefV2,
        prepare_id: &str,
    ) -> Result<NativeModelConnectionDraftV1, ComputeManagementControlError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let resolved = catalog
            .resolve_connection_option(CONNECTION_OPTION_ID)
            .map_err(|_| ComputeManagementControlError::RegisteredOptionUnavailable)?;
        if resolved.option.origin != ConnectionOrigin::NativeApi
            || resolved.option.billing_class != BillingClass::Subscription
            || resolved.connector.connector_id != "connector.zhipu.p0"
            || resolved.connector.authentication != AuthenticationKind::ProviderApiKey
            || resolved.connector.catalog_adapter_ref != "catalog.zhipu.p0"
            || resolved.connector.catalog_adapter_revision != 1
            || resolved.endpoint_profile.endpoint_profile_id != ENDPOINT_PROFILE_ID
        {
            return Err(ComputeManagementControlError::RegisteredOptionUnavailable);
        }
        let discovered_endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| {
                endpoint.protocol_endpoint_id == DISCOVERED_PROTOCOL_ENDPOINT_ID
                    && endpoint.protocol == UpstreamProtocol::Messages
                    && endpoint.adapter_ref == MESSAGES_ADAPTER_ID
                    && endpoint.adapter_revision == 1
            })
            .ok_or(ComputeManagementControlError::RegisteredOptionUnavailable)?;
        let endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| {
                endpoint.protocol_endpoint_id == ROUTING_PROTOCOL_ENDPOINT_ID
                    && endpoint.protocol == UpstreamProtocol::Responses
                    && endpoint.adapter_ref == RESPONSES_ADAPTER_ID
                    && endpoint.adapter_revision == 1
                    && endpoint.authentication_semantics
                        == Some(GatewayAuthenticationSemanticsV1::Bearer)
            })
            .ok_or(ComputeManagementControlError::RegisteredOptionUnavailable)?;
        let capability = catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|capability| {
                capability.model_configuration_id == MODEL_CONFIGURATION_ID
                    && capability.upstream_model_id == UPSTREAM_MODEL_ID
                    && capability.connector_id == resolved.connector.connector_id
                    && capability.connector_revision == resolved.connector.revision
                    && capability.endpoint_profile_id == ENDPOINT_PROFILE_ID
                    && capability.endpoint_profile_revision == resolved.endpoint_profile.revision
                    && capability.protocol_endpoint_id == ROUTING_PROTOCOL_ENDPOINT_ID
                    && capability.upstream_protocol == UpstreamProtocol::Responses
                    && capability.required_adapter_ref == RESPONSES_ADAPTER_ID
                    && capability.required_adapter_revision == 1
            })
            .ok_or(ComputeManagementControlError::RegisteredOptionUnavailable)?;
        let model = catalog
            .model_data()
            .model(MODEL_CONFIGURATION_ID)
            .ok_or(ComputeManagementControlError::Corrupt)?;
        let reasoning = catalog
            .native_reasoning()
            .iter()
            .find(|value| value.model_configuration_id == MODEL_CONFIGURATION_ID)
            .ok_or(ComputeManagementControlError::Corrupt)?;
        let registered_base_url = format!(
            "{}{}",
            discovered_endpoint.base_url, discovered_endpoint.request_path
        )
        .strip_suffix("/v1/messages")
        .ok_or(ComputeManagementControlError::Corrupt)?
        .to_owned();
        if discovery.fact.endpoint_profile_revision != resolved.endpoint_profile.revision
            || discovery.fact.registered_base_url != registered_base_url
            || discovery.fact.configuration_revision == 0
            || capability.evidence_digest == CanonicalDigest::of_bytes(&[])
        {
            return Err(ComputeManagementControlError::DiscoveryChanged);
        }
        Ok(NativeModelConnectionDraftV1 {
            display_template_id: None,
            inference_model_id: None,
            candidate_ref: Some(candidate.candidate_ref.clone()),
            lineage_ref: discovery.discovery.discovery_ref.clone(),
            trusted_lineage_digest: None,
            display_name: resolved.option.display_name.clone(),
            existing_source_id: None,
            edit_revision: discovery.discovery.discovery_revision,
            check_id: prepare_id.to_owned(),
            base_url: endpoint.base_url.clone(),
            base_kind: ModelConnectionBaseKindV1::ApiRoot,
            request_path_override: Some(endpoint.request_path.clone()),
            inventory_path_override: endpoint.inventory_path.clone(),
            protocol: UpstreamProtocol::Responses,
            protocol_profile_id: endpoint.adapter_ref.clone(),
            protocol_profile_revision: endpoint.adapter_revision,
            protocol_header_semantics: super::registered_endpoint_header_semantics(endpoint),
            authentication: GatewayAuthenticationSemanticsV1::Bearer,
            provenance: NativeConnectionProvenanceInputV1::Registered {
                connection_option_id: CONNECTION_OPTION_ID.into(),
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
            models: vec![NativeModelDeclarationV1 {
                upstream_model_id: UPSTREAM_MODEL_ID.into(),
                display_name: model.display_name.clone(),
                catalog_configuration_id: Some(MODEL_CONFIGURATION_ID.into()),
                membership: ComputeModelMembershipV2::Catalog,
                capabilities: NativeModelCapabilityDeclarationV1 {
                    tool: registered_fact(model.capabilities.tool),
                    vision: registered_fact(model.capabilities.vision),
                    streaming: registered_fact(model.capabilities.streaming),
                    context_tokens: registered_fact(model.capabilities.context_tokens),
                    max_output_tokens: registered_fact(model.capabilities.max_output_tokens),
                    native_reasoning: registered_fact(reasoning.capability.clone()),
                },
            }],
        })
    }
}

fn registered_fact<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}
