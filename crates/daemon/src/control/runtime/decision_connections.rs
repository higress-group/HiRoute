//! Safe local read projections for decision connections; no provider calls or Secret plaintext.
use std::collections::BTreeSet;

use hiroute_application::control::ControlReadError;
use hiroute_application_api::{
    DecisionConnectionSourcesV1, DecisionServiceViewV2, DecisionServicesV2,
    DecisionSourceAuthKindV1, DecisionSourceStateV1, DecisionSourceSuggestionV1,
    DecisionSourceSummaryV1,
};
use hiroute_domain::{
    CanonicalDigest, ComputeManagementRepositoryPort, ComputeManagementSourceV2,
    DecisionCredentialBindingV1, DecisionServiceRecord, DecisionWireProtocolV2,
    GatewayAuthenticationSemanticsV1, MaterializationState, WorkspaceId,
};

use super::LocalControlAdapter;

impl LocalControlAdapter {
    pub(super) fn decision_source_view(
        &self,
        id: &str,
    ) -> Result<DecisionSourceSummaryV1, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        match stores
            .control()
            .compute_management_source(id)
            .map_err(super::map_port)?
        {
            Some(source) if eligible(&source) => summary(&source),
            Some(_) => Ok(unavailable(id)),
            None => Ok(missing(id)),
        }
    }
    pub(super) fn decision_connection_sources_view(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<DecisionConnectionSourcesV1, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        let snapshot = stores
            .control()
            .compute_management_snapshot(workspace)
            .map_err(super::map_port)?;
        let mut sources = snapshot
            .sources
            .iter()
            .filter(|source| eligible(source))
            .map(summary)
            .collect::<Result<Vec<_>, _>>()?;
        sources.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        Ok(DecisionConnectionSourcesV1 { sources })
    }

    pub(super) fn decision_services_view(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<DecisionServicesV2, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        let records = stores
            .control()
            .decision_service_records(workspace)
            .map_err(super::map_port)?;
        let mut services = Vec::with_capacity(records.len());
        for service in records {
            let versions = stores
                .control()
                .decision_service_versions(workspace, service.id())
                .map_err(super::map_port)?;
            let mut ids = BTreeSet::new();
            for version in versions {
                if let DecisionServiceRecord::V2(version) = version
                    && let Some(binding) = DecisionCredentialBindingV1::for_service(&version)
                        .map_err(|_| ControlReadError::Corrupt)?
                {
                    ids.insert(binding.source_id);
                }
            }
            let binding = match &service {
                DecisionServiceRecord::V2(service) => {
                    DecisionCredentialBindingV1::for_service(service)
                        .map_err(|_| ControlReadError::Corrupt)?
                }
                _ => None,
            };
            if let Some(binding) = &binding {
                ids.insert(binding.source_id.clone());
            }
            let mut related_sources = Vec::new();
            for id in ids {
                let current = stores
                    .control()
                    .compute_management_source(&id)
                    .map_err(super::map_port)?;
                let value = match current {
                    Some(source) if eligible(&source) => summary(&source)?,
                    Some(_) => unavailable(&id),
                    None => missing(&id),
                };
                related_sources.push(value);
            }
            let mut credential_state_revision = None;
            if let Some(binding) = &binding {
                if let Some(source) = related_sources
                    .iter_mut()
                    .find(|source| source.source_id == binding.source_id)
                    && source.state != DecisionSourceStateV1::Missing
                {
                    match stores.admit_decision_credentials(binding) {
                        Ok(admission) => {
                            credential_state_revision =
                                Some(admission.credential_state_revision.clone());
                            source.credential_state_revision = credential_state_revision.clone();
                        }
                        Err(_) => {
                            source.state = DecisionSourceStateV1::Unavailable;
                            source.credential_state_revision = None;
                        }
                    }
                }
            } else if let Some(header) = service.auth_header()
                && let Ok(Some(generation)) = stores
                    .secrets()
                    .decision_header_generation(&header.value_secret_ref)
            {
                credential_state_revision = hiroute_domain::decision_header_state_revision(
                    &header.value_secret_ref,
                    generation,
                )
                .ok();
            }
            let source = binding.as_ref().and_then(|binding| {
                related_sources
                    .iter()
                    .find(|source| source.source_id == binding.source_id)
                    .cloned()
            });
            let configuration_digest =
                CanonicalDigest::of(&service).map_err(|_| ControlReadError::Corrupt)?;
            services.push(DecisionServiceViewV2 {
                service,
                configuration_digest,
                credential_state_revision,
                source,
                related_sources,
            });
        }
        Ok(DecisionServicesV2 { services })
    }
}

fn eligible(source: &ComputeManagementSourceV2) -> bool {
    source.provenance.is_native()
        && (source.authentication != GatewayAuthenticationSemanticsV1::None
            || source
                .additional_native_endpoints
                .iter()
                .any(|target| target.authentication != GatewayAuthenticationSemanticsV1::None))
}

fn summary(
    source: &ComputeManagementSourceV2,
) -> Result<DecisionSourceSummaryV1, ControlReadError> {
    source.validate().map_err(|_| ControlReadError::Corrupt)?;
    let keys = source
        .credentials
        .iter()
        .map(|key| {
            (
                &key.key_id,
                key.credential.generation(),
                key.ordinal,
                key.enabled,
            )
        })
        .collect::<Vec<_>>();
    let revision = CanonicalDigest::of(&(
        "hiroute.decision-source-state/v1",
        &source.source_id,
        &source.lineage_digest,
        source.state,
        keys,
    ))
    .map_err(|_| ControlReadError::Corrupt)?;
    let state = match source.state {
        MaterializationState::Disabled => DecisionSourceStateV1::Disabled,
        MaterializationState::Ready if source.credentials.iter().any(|key| key.enabled) => {
            DecisionSourceStateV1::Ready
        }
        MaterializationState::NeedsCredential => DecisionSourceStateV1::MissingCredentials,
        MaterializationState::Ready => DecisionSourceStateV1::MissingCredentials,
        MaterializationState::NeedsAuthorization => DecisionSourceStateV1::Unavailable,
    };
    let host = source
        .target
        .authority
        .parse::<http::uri::Authority>()
        .ok()
        .map(|value| value.host().to_ascii_lowercase());
    let preset = host.as_deref().and_then(preset_for_host);
    Ok(DecisionSourceSummaryV1 {
        source_id: source.source_id.clone(),
        lineage_digest: Some(source.lineage_digest.clone()),
        revision: Some(source.revision),
        display_name: Some(source.display_name.clone()),
        provider_label: preset.as_ref().map(|(_, name)| name.clone()),
        endpoint_host: host,
        auth_kind: Some(DecisionSourceAuthKindV1::NativeApiKey),
        state,
        credential_state_revision: Some(revision),
        suggestion: preset.map(|(suggestion, _)| suggestion),
    })
}

fn preset_for_host(host: &str) -> Option<(DecisionSourceSuggestionV1, String)> {
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../assets/decision-services/providers.v1.json"
    ))
    .ok()?;
    let provider = catalog
        .get("providers")?
        .as_array()?
        .iter()
        .find(|provider| {
            provider
                .get("endpoint")
                .and_then(serde_json::Value::as_str)
                .and_then(|endpoint| endpoint.parse::<http::Uri>().ok())
                .and_then(|uri| uri.host().map(str::to_ascii_lowercase))
                .as_deref()
                == Some(host)
        })?;
    let text = |name: &str| {
        provider
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let wire_protocol = match provider
        .get("wire_protocol")
        .and_then(serde_json::Value::as_str)
    {
        None | Some("system_one") => Some(DecisionWireProtocolV2::SystemOne),
        Some("openai_decisions") => Some(DecisionWireProtocolV2::OpenaiDecisions),
        Some(_) => None,
    };
    Some((
        DecisionSourceSuggestionV1 {
            model: text("model"),
            endpoint: text("endpoint"),
            wire_protocol,
        },
        text("name")?,
    ))
}

fn missing(id: &str) -> DecisionSourceSummaryV1 {
    empty(id, DecisionSourceStateV1::Missing)
}
fn unavailable(id: &str) -> DecisionSourceSummaryV1 {
    empty(id, DecisionSourceStateV1::Unavailable)
}
fn empty(id: &str, state: DecisionSourceStateV1) -> DecisionSourceSummaryV1 {
    DecisionSourceSummaryV1 {
        source_id: id.into(),
        lineage_digest: None,
        revision: None,
        display_name: None,
        provider_label: None,
        endpoint_host: None,
        auth_kind: None,
        state,
        credential_state_revision: None,
        suggestion: None,
    }
}
