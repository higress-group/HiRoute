//! Grouped discovery is a V2 projection; legacy one-model references retain their meaning.
use super::*;
use hiroute_application_api::{
    COMPUTE_SCAN_RESULT_SCHEMA_V2, ComputeConnectionPointV2, ComputeDiscoveryRefV1,
    ComputeScanDiagnosticV2, ComputeScanRequestV2, ComputeScanResultV2,
};
use hiroute_integrations::{NativeApiConnectionPointV2, native_api_connection_points};

pub(super) fn point_discovery(
    point: &NativeApiConnectionPointV2,
    catalog: &TrustedReleaseCatalog,
) -> ComputeDiscoveryRefV1 {
    let evidence = super::point_catalog::point_evidence(catalog, point)
        .expect("validated catalog and source facts");
    ComputeDiscoveryRefV1 {
        discovery_ref: format!("discovery/{}", &point.key_digest.as_str()[7..]),
        discovery_revision: u64::from_str_radix(&evidence.as_str()[7..23], 16)
            .expect("digest revision")
            .max(1),
    }
}

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn claude_api_point_sources(
        &self,
    ) -> Result<
        Vec<hiroute_integrations::NativeApiSource>,
        hiroute_application::control::ControlReadError,
    > {
        use super::super::compute_routing::authorized_compute_discovery;
        use hiroute_application::control::ControlReadError;
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let mut sources = Vec::new();
        for native in self.scanner.scan() {
            let Some(discovery) = authorized_compute_discovery(catalog, &native)? else {
                continue;
            };
            if self.validate_discovered_allowlist(&discovery).is_err() {
                continue;
            }
            let credential = discovery.credential;
            let scope = hiroute_domain::CanonicalDigest::of(&(
                "claude-native-point-scope/v2",
                &credential.discovered_source_ref,
            ))
            .map_err(|_| ControlReadError::Corrupt)?;
            sources.push(hiroute_integrations::NativeApiSource {
                point_binding: Some(hiroute_integrations::NativeApiPointBindingV1 {
                    native_scope: scope,
                    configured_default_model_id: Some(discovery.fact.observed_model_id.clone()),
                    provider_definition: discovery.fact.connection_option_id.clone(),
                    endpoint: discovery.fact.registered_base_url,
                    authentication: GatewayAuthenticationSemanticsV1::Bearer,
                    endpoint_selector: "env.ANTHROPIC_BASE_URL".into(),
                    protocol_selector: "native-messages".into(),
                    model_override_scope: None,
                    credential_binding: format!(
                        "{}:{}",
                        credential.discovered_source_ref, credential.field_selector
                    ),
                }),
                provider_id: discovery.fact.connection_option_id,
                model_id: discovery.fact.observed_model_id.clone(),
                display_name: discovery.fact.observed_model_id,
                protocol: Some(UpstreamProtocol::Messages),
                context_tokens: None,
                max_output_tokens: None,
                vision: None,
                reasoning: None,
                authentication: hiroute_integrations::DiscoveredAuthSource::InlineToken,
                source_ref: credential.discovered_source_ref.clone(),
                evidence_digest: discovery.evidence_digest,
                revision: discovery.fact.configuration_revision,
                credential: Some(credential),
                supported_auth: true,
            });
        }
        Ok(sources)
    }

    pub(in crate::control::runtime) fn compute_point_scan(
        &self,
        request: &ComputeScanRequestV2,
    ) -> Result<ComputeScanResultV2, hiroute_application::control::ControlReadError> {
        use hiroute_application::control::ControlReadError;
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ControlReadError::Corrupt)?;
        let mut connections = Vec::new();
        let mut diagnostics = Vec::new();
        let mut selected_target = request
            .native_target
            .as_ref()
            .map(hiroute_integrations::canonical_hermes_target);
        if let Some(target) = &request.native_target
            && self.remember_hermes_scan_target(target).is_err()
        {
            selected_target = Some(Err(hiroute_integrations::HermesNativeError {
                stage: "selected profile unavailable",
            }));
            diagnostics.push(ComputeScanDiagnosticV2 {
                agent_id: "agent_hermes_default".into(),
                reason: "selected_profile_unavailable".into(),
                native_provider_id: None,
                model_id: None,
            });
        }
        let hermes = match selected_target {
            Some(Ok(target)) => hiroute_integrations::hermes_api_sources(&target).map_err(|_| ()),
            Some(Err(_)) => Ok(hiroute_integrations::HermesApiDiscoveryV1 {
                sources: Vec::new(),
                diagnostics: Vec::new(),
            }),
            None => self.hermes_source_facts().map_err(|_| ()),
        };
        let hermes_sources =
            hermes.map(|facts| {
                diagnostics.extend(facts.diagnostics.into_iter().map(|d| {
                    ComputeScanDiagnosticV2 {
                        agent_id: "agent_hermes_default".into(),
                        reason: d.reason.into(),
                        native_provider_id: d.provider_id,
                        model_id: d.model_id,
                    }
                }));
                facts.sources
            });
        for (agent, sources) in [
            (
                "agent_pi_default",
                self.scanner.pi_api_sources().map_err(|_| ()),
            ),
            (
                "agent_dsh_default",
                self.scanner.dsh_api_sources().map_err(|_| ()),
            ),
            (
                "agent_claude_default",
                self.claude_api_point_sources().map_err(|_| ()),
            ),
            ("agent_hermes_default", hermes_sources),
        ] {
            let sources = match sources {
                Ok(sources) => sources,
                Err(_) => {
                    diagnostics.push(ComputeScanDiagnosticV2 {
                        agent_id: agent.into(),
                        reason: "configuration_unavailable".into(),
                        native_provider_id: None,
                        model_id: None,
                    });
                    continue;
                }
            };
            for source in &sources {
                if source.point_binding.is_none() {
                    diagnostics.push(ComputeScanDiagnosticV2 {
                        agent_id: agent.into(),
                        reason: "native_endpoint_unavailable".into(),
                        native_provider_id: Some(source.provider_id.clone()),
                        model_id: Some(source.model_id.clone()),
                    });
                }
            }
            for point in native_api_connection_points(sources) {
                let first = point.sources.first().ok_or(ControlReadError::Corrupt)?;
                let target = super::point_catalog::native_target(first);
                let importable = target.is_some()
                    && point.sources.iter().all(|s| {
                        s.supported_auth && s.protocol.is_some() && s.credential.is_some()
                    });
                connections.push(ComputeConnectionPointV2 {
                    context_ref: format!(
                        "native-scope/{}",
                        &first
                            .point_binding
                            .as_ref()
                            .ok_or(ControlReadError::Corrupt)?
                            .native_scope
                            .as_str()[7..]
                    ),
                    connection_option_id: super::point_catalog::qualified_point(catalog, first)
                        .map(|(r, _)| r.option.connection_option_id),
                    endpoint_profile_id: super::point_catalog::qualified_point(catalog, first)
                        .map(|(r, _)| r.endpoint_profile.endpoint_profile_id),
                    target,
                    connection_ref: format!("connection-point/{}", &point.key_digest.as_str()[7..]),
                    agent_id: agent.into(),
                    display_name: first.provider_id.clone(),
                    native_provider_id: Some(first.provider_id.clone()),
                    protocol: point.protocol(),
                    declared_model_ids: point.sources.iter().map(|s| s.model_id.clone()).collect(),
                    default_model_id: first
                        .point_binding
                        .as_ref()
                        .and_then(|b| b.configured_default_model_id.clone()),
                    configuration_state: if importable {
                        "static_api_with_protected_input"
                    } else {
                        "native_configuration_not_importable"
                    }
                    .into(),
                    discovery: importable.then(|| point_discovery(&point, catalog)),
                    actions_required: if importable {
                        vec![]
                    } else {
                        vec!["review_native_configuration".into()]
                    },
                });
            }
        }
        connections.sort_by(|a, b| a.connection_ref.cmp(&b.connection_ref));
        Ok(ComputeScanResultV2 {
            schema: COMPUTE_SCAN_RESULT_SCHEMA_V2.into(),
            catalog: super::super::compute_routing::public_provenance(&provenance),
            connections,
            diagnostics,
        })
    }
}
