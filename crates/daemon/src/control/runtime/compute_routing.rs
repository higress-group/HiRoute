use hiroute_application::control::{
    ComputeFactsPort, ComputeProjectionReadError, ControlReadError,
};
use hiroute_application_api::{
    COMPUTE_CONNECTION_OPTIONS_SCHEMA_V1, COMPUTE_SCAN_RESULT_SCHEMA_V1,
    ComputeCatalogProvenanceViewV1, ComputeConnectionChangeV1, ComputeConnectionOptionV1,
    ComputeConnectionOptionsResultV1, ComputeDiscoveredCredentialImportV1, ComputeDiscoveryRefV1,
    ComputePermissionActionV1, ComputeScanItemV1, ComputeScanResultV1,
};
use hiroute_domain::{CanonicalDigest, PreparedComputeProjectionV1};
use hiroute_integrations::{
    AgentDiscoveryOutcomeV1, ComputeProjectionIdsV1, CpaRegisteredSourceV1,
    DiscoveredCredentialRefV1, FilesystemAgentDiscoveryV1, RegisteredComputeCandidateV1,
    RegisteredComputeDiscoveryFactV1,
};

use super::LocalControlAdapter;

const PROTECTED_PREPARE_CONNECTION_OPTION_ID: &str = "zhipu.coding-plan.cn.v1";
const PROTECTED_PREPARE_MODEL_CONFIGURATION_ID: &str = "model.zhipu.glm-5.3";
const PROTECTED_PREPARE_FIELD_SELECTOR: &str = "env.ANTHROPIC_AUTH_TOKEN";

impl LocalControlAdapter {
    pub(super) fn registered_compute_candidates(
        &self,
    ) -> Result<Vec<RegisteredComputeCandidateV1>, ControlReadError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let discoveries = self
            .refresh_discovery()
            .map_err(|_| ControlReadError::Unavailable)?;
        let mut candidates = Vec::new();
        for discovery in discoveries {
            if let Some(fact) = discovery_fact(&discovery) {
                let candidate = catalog
                    .authorize_compute_discovery(fact)
                    .map_err(|_| ControlReadError::Corrupt)?;
                candidates.push(candidate);
            }
        }
        candidates.sort_by(|left, right| {
            left.fact
                .discovered_source_ref
                .cmp(&right.fact.discovered_source_ref)
        });
        Ok(candidates)
    }

    /// Recomputes the exact discovery/config/catalog evidence for a protected candidate slot.
    /// This never calls a model endpoint and never updates the public discovery projection.
    pub(super) fn validate_compute_discovery_evidence(
        &self,
        input_slot: &str,
        expected_evidence: &CanonicalDigest,
    ) -> Result<(), ControlReadError> {
        let descriptor = self
            .protected_inputs
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .get(input_slot)
            .cloned()
            .ok_or(ControlReadError::NotFound)?;
        if descriptor.field_selector == "pi.api-key" {
            return self
                .scanner
                .pi_api_sources()
                .map_err(|_| ControlReadError::SnapshotChanged)?
                .iter()
                .any(|source| {
                    source.credential.as_ref() == Some(&descriptor)
                        && &source.evidence_digest == expected_evidence
                })
                .then_some(())
                .ok_or(ControlReadError::SnapshotChanged);
        }
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let mut matched = false;
        for discovery in self.scanner.scan() {
            let Some(authorized) = authorized_compute_discovery(catalog, &discovery)? else {
                continue;
            };
            if authorized.credential == descriptor
                && &authorized.evidence_digest == expected_evidence
            {
                matched = true;
                break;
            }
        }
        if matched {
            Ok(())
        } else {
            Err(ControlReadError::SnapshotChanged)
        }
    }

    pub(super) fn registered_cpa_candidates(
        &self,
    ) -> Result<Vec<(CpaRegisteredSourceV1, String)>, ControlReadError> {
        let Some(authority) = &self.cpa_sources else {
            return Ok(Vec::new());
        };
        let sources = authority
            .discover_registered_sources()
            .map_err(|_| ControlReadError::Unavailable)?;
        self.cpa_candidates_from_sources(sources)
    }

    pub(super) fn cpa_candidates_from_sources(
        &self,
        sources: Vec<CpaRegisteredSourceV1>,
    ) -> Result<Vec<(CpaRegisteredSourceV1, String)>, ControlReadError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let mut candidates = sources
            .into_iter()
            .flat_map(|source| {
                let model_ids = source
                    .inventory
                    .iter()
                    .filter_map(|model| model.model_configuration_id.clone())
                    .collect::<Vec<_>>();
                model_ids
                    .into_iter()
                    .map(move |model_id| (source.clone(), model_id))
            })
            .filter(|(source, model_id)| {
                catalog.cpa_compute_projection_ids(source, model_id).is_ok()
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.0
                .source
                .source_id
                .cmp(&right.0.source.source_id)
                .then_with(|| left.1.cmp(&right.1))
        });
        Ok(candidates)
    }

    fn projection_for_change(
        &self,
        change: &ComputeConnectionChangeV1,
    ) -> Result<PreparedComputeProjectionV1, ComputeProjectionReadError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeProjectionReadError::Control(
                ControlReadError::Unavailable,
            ))?;
        let filesystem_candidate = self
            .registered_compute_candidates()
            .map_err(ComputeProjectionReadError::Control)?
            .into_iter()
            .find(|candidate| {
                candidate.fact.discovered_source_ref == change.discovered_source_ref
                    && candidate.fact.connection_option_id == change.connection_option_id
                    && candidate.fact.model_configuration_id == change.model_configuration_id
            });
        let cpa_candidate = if filesystem_candidate.is_none() {
            self.registered_cpa_candidates()
                .map_err(ComputeProjectionReadError::Control)?
                .into_iter()
                .find(|(source, model_id)| {
                    source.source.source_id == change.discovered_source_ref
                        && source.source.connection_option_id == change.connection_option_id
                        && model_id == &change.model_configuration_id
                })
        } else {
            None
        };
        let ids = if let Some(candidate) = &filesystem_candidate {
            catalog
                .compute_projection_ids(candidate)
                .map_err(|_| ComputeProjectionReadError::Control(ControlReadError::Corrupt))?
        } else if let Some((source, model_id)) = &cpa_candidate {
            catalog
                .cpa_compute_projection_ids(source, model_id)
                .map_err(|_| ComputeProjectionReadError::Control(ControlReadError::Corrupt))?
        } else {
            return Err(ComputeProjectionReadError::InvalidSelection);
        };
        let expected = {
            let stores = self
                .stores_lock()
                .map_err(super::map_port)
                .map_err(ComputeProjectionReadError::Control)?;
            projection_expectation_for_ids(stores.control(), &ids)
                .map_err(super::map_port)
                .map_err(ComputeProjectionReadError::Control)?
        };
        if expected.source_revision != change.expected_source_revision
            || expected.binding_revision != change.expected_binding_revision
            || expected.inventory_revision != change.expected_inventory_revision
        {
            return Err(ComputeProjectionReadError::RevisionConflict);
        }
        let prepared = if let Some(candidate) = filesystem_candidate {
            catalog.prepare_compute_projection(candidate, expected, change.explicit_materialization)
        } else {
            let (source, model_id) = cpa_candidate
                .as_ref()
                .ok_or(ComputeProjectionReadError::InvalidSelection)?;
            catalog.prepare_cpa_compute_projection(
                source,
                model_id,
                expected,
                change.explicit_materialization,
            )
        };
        prepared.map_err(|error| match error {
            hiroute_integrations::ComputeControlPlaneError::ExplicitMaterializationRequired => {
                ComputeProjectionReadError::ActionRequired
            }
            hiroute_integrations::ComputeControlPlaneError::DiscoveryMismatch => {
                ComputeProjectionReadError::InvalidSelection
            }
            _ => ComputeProjectionReadError::Control(ControlReadError::Corrupt),
        })
    }
}

fn projection_expectation_for_ids(
    control: &hiroute_local_storage::ControlStore,
    ids: &ComputeProjectionIdsV1,
) -> hiroute_domain::PortResult<hiroute_domain::ComputeProjectionExpectationV1> {
    control.compute_projection_expectation_with_legacy_lineage(
        ids.source_id(),
        ids.binding_id(),
        ids.endpoint_profile_id(),
        ids.legacy_v7_source_id(),
    )
}

impl ComputeFactsPort for LocalControlAdapter {
    fn scan_compute(&self) -> Result<ComputeScanResultV1, ControlReadError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ControlReadError::Corrupt)?;
        let discoveries = self
            .refresh_discovery()
            .map_err(|_| ControlReadError::Unavailable)?;
        let mut items = discoveries
            .iter()
            .map(|discovery| scan_item(catalog, discovery))
            .collect::<Result<Vec<_>, _>>()?;
        items.extend(self.pi_compute_scan_items()?);
        // Subscription discovery has its own metadata-only API. Generic ScanCompute must not
        // start CPA or materialize OAuth state merely because a screen was opened.
        items.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
        Ok(ComputeScanResultV1 {
            schema: COMPUTE_SCAN_RESULT_SCHEMA_V1.into(),
            catalog: public_provenance(&provenance),
            items,
        })
    }

    fn connection_options(&self) -> Result<ComputeConnectionOptionsResultV1, ControlReadError> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ControlReadError::Unavailable)?;
        let provenance = catalog
            .compute_catalog_provenance()
            .map_err(|_| ControlReadError::Corrupt)?;
        let options = catalog
            .registered_connection_options()
            .map_err(|_| ControlReadError::Corrupt)?
            .into_iter()
            .map(|option| {
                let authentication = serde_json::to_value(option.authentication)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(ControlReadError::Corrupt)?;
                let resolved = catalog
                    .resolve_connection_option(&option.connection_option_id)
                    .map_err(|_| ControlReadError::Corrupt)?;
                let known_models = catalog
                    .model_data()
                    .model_endpoint_capabilities
                    .iter()
                    .filter(|capability| {
                        capability.endpoint_profile_id == option.endpoint_profile_id
                    })
                    .filter_map(|capability| {
                        catalog
                            .model_data()
                            .model(&capability.model_configuration_id)
                            .map(|model| {
                                (
                                    capability.upstream_model_id.clone(),
                                    model.display_name.clone(),
                                )
                            })
                    })
                    .collect();
                Ok(ComputeConnectionOptionV1 {
                    endpoints: resolved.endpoint_profile.protocol_endpoints.clone(),
                    known_models,
                    connection_option_id: option.connection_option_id,
                    display_name: option.display_name,
                    connector_id: option.connector_id,
                    connector_revision: option.connector_revision,
                    endpoint_profile_id: option.endpoint_profile_id,
                    endpoint_profile_revision: option.endpoint_profile_revision,
                    origin: option.origin,
                    billing_class: option.billing_class,
                    authentication,
                    model_configuration_ids: option.model_configuration_ids,
                    registered_check_available: option.registered_check_available,
                })
            })
            .collect::<Result<Vec<_>, ControlReadError>>()?;
        Ok(ComputeConnectionOptionsResultV1 {
            schema: COMPUTE_CONNECTION_OPTIONS_SCHEMA_V1.into(),
            catalog: public_provenance(&provenance),
            options,
            subscriptions: None,
            metadata_catalog: Some(catalog.model_metadata().clone()),
        })
    }

    fn prepare_compute_projection(
        &self,
        change: &ComputeConnectionChangeV1,
    ) -> Result<PreparedComputeProjectionV1, ComputeProjectionReadError> {
        let first = self.projection_for_change(change)?;
        let second = self.projection_for_change(change)?;
        if first != second {
            return Err(ComputeProjectionReadError::RevisionConflict);
        }
        Ok(second)
    }
}

#[derive(Clone)]
pub(super) struct AuthorizedComputeDiscoveryV1 {
    pub(super) discovery: ComputeDiscoveryRefV1,
    pub(super) fact: RegisteredComputeDiscoveryFactV1,
    pub(super) credential: DiscoveredCredentialRefV1,
    pub(super) evidence_digest: CanonicalDigest,
}

pub(super) fn authorized_compute_discovery(
    catalog: &hiroute_integrations::TrustedReleaseCatalog,
    discovery: &FilesystemAgentDiscoveryV1,
) -> Result<Option<AuthorizedComputeDiscoveryV1>, ControlReadError> {
    let Some(fact) = discovery_fact(discovery) else {
        return Ok(None);
    };
    if catalog.authorize_compute_discovery(fact.clone()).is_err() {
        return Ok(None);
    }
    let credential = discovery
        .discovered_credential
        .clone()
        .ok_or(ControlReadError::Corrupt)?;
    let provenance = catalog
        .compute_catalog_provenance()
        .map_err(|_| ControlReadError::Corrupt)?;
    let fact_digest = CanonicalDigest::of(&(
        "hiroute.registered-compute-discovery-fact/v1",
        (
            &fact.agent_id,
            &fact.scanner_id,
            &fact.scanner_version,
            &fact.discovered_source_ref,
            fact.configuration_revision,
            &fact.connection_option_id,
            &fact.endpoint_profile_id,
        ),
        (
            fact.endpoint_profile_revision,
            &fact.registered_base_url,
            &fact.observed_model_id,
            &fact.model_configuration_id,
            fact.protected_credential_available,
        ),
    ))
    .map_err(|_| ControlReadError::Corrupt)?;
    let evidence_digest = CanonicalDigest::of(&(
        "hiroute.compute-discovery-evidence/v1",
        &fact_digest,
        &credential,
        &provenance,
    ))
    .map_err(|_| ControlReadError::Corrupt)?;
    let discovery_ref = format!(
        "discovery/{}",
        evidence_digest.as_str().trim_start_matches("sha256:")
    );
    Ok(Some(AuthorizedComputeDiscoveryV1 {
        discovery: ComputeDiscoveryRefV1 {
            discovery_ref,
            discovery_revision: fact.configuration_revision,
        },
        fact,
        credential,
        evidence_digest,
    }))
}

fn discovery_fact(
    discovery: &FilesystemAgentDiscoveryV1,
) -> Option<RegisteredComputeDiscoveryFactV1> {
    let agent_id = match &discovery.outcome {
        AgentDiscoveryOutcomeV1::Supported { installation } => installation.agent_id.clone(),
        AgentDiscoveryOutcomeV1::ReportOnly { agent_id, .. } => agent_id.clone(),
    };
    let configuration = discovery.claude_configuration.as_ref()?;
    let credential = discovery.discovered_credential.as_ref()?;
    Some(RegisteredComputeDiscoveryFactV1 {
        agent_id,
        scanner_id: credential.scanner_id.clone(),
        scanner_version: credential.scanner_version.clone(),
        discovered_source_ref: credential.discovered_source_ref.clone(),
        configuration_revision: configuration
            .configuration_revision
            .max(credential.observed_revision),
        connection_option_id: configuration.connection_option_id.clone(),
        endpoint_profile_id: configuration.endpoint_profile_id.clone(),
        endpoint_profile_revision: configuration.endpoint_profile_revision,
        registered_base_url: configuration.base_url.clone(),
        observed_model_id: configuration.observed_model_id.clone(),
        model_configuration_id: configuration.model_configuration_id.clone()?,
        protected_credential_available: true,
    })
}

fn scan_item(
    catalog: &hiroute_integrations::TrustedReleaseCatalog,
    discovery: &FilesystemAgentDiscoveryV1,
) -> Result<ComputeScanItemV1, ControlReadError> {
    let (agent_id, supported, executable_state) = match &discovery.outcome {
        AgentDiscoveryOutcomeV1::Supported { installation } => {
            (installation.agent_id.clone(), true, "supported".to_owned())
        }
        AgentDiscoveryOutcomeV1::ReportOnly {
            agent_id, reason, ..
        } => (
            agent_id.clone(),
            false,
            serde_json::to_value(reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or(ControlReadError::Corrupt)?,
        ),
    };
    let configuration_state = if discovery.claude_configuration.is_some() {
        if discovery.discovered_credential.is_some() {
            "registered_with_protected_input".to_owned()
        } else {
            "credential_required".to_owned()
        }
    } else if let Some(reason) = &discovery.configuration_issue {
        serde_json::to_value(reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(ControlReadError::Corrupt)?
    } else {
        executable_state
    };
    let permission_action =
        discovery
            .permission_hardening
            .as_ref()
            .map(|finding| ComputePermissionActionV1 {
                discovered_source_ref: finding.discovered_source_ref.clone(),
                observed_identity: finding.observed_identity.clone(),
                observed_revision: finding.observed_revision,
                display_path: finding.display_path.clone(),
                required_mode: finding.required_mode,
            });
    let authorized = authorized_compute_discovery(catalog, discovery)?;
    let protected_prepare = authorized.as_ref().is_some_and(|authorized| {
        authorized.fact.connection_option_id == PROTECTED_PREPARE_CONNECTION_OPTION_ID
            && authorized.fact.model_configuration_id == PROTECTED_PREPARE_MODEL_CONFIGURATION_ID
            && authorized.credential.field_selector == PROTECTED_PREPARE_FIELD_SELECTOR
    });
    let credential_import = if protected_prepare {
        None
    } else {
        discovery
            .discovered_credential
            .as_ref()
            .map(|credential| {
                Ok(ComputeDiscoveredCredentialImportV1 {
                    input_slot: super::mutation::protected_input_slot(credential)
                        .map_err(|_| ControlReadError::Corrupt)?,
                    scanner_id: credential.scanner_id.clone(),
                    scanner_version: credential.scanner_version.clone(),
                    source_ref: credential.discovered_source_ref.clone(),
                    field_selector: credential.field_selector.clone(),
                    observed_revision: credential.observed_revision,
                })
            })
            .transpose()?
    };
    let inventory_eligible = authorized.is_some();
    let configuration = discovery.claude_configuration.as_ref();
    let mut actions_required = Vec::new();
    if permission_action.is_some() {
        actions_required.push("permission_hardening_required".into());
    }
    if configuration.is_some() && discovery.discovered_credential.is_none() {
        actions_required.push("credential_import_required".into());
    }
    if configuration.is_some_and(|configuration| configuration.model_configuration_id.is_none()) {
        actions_required.push("registered_model_required".into());
    }
    if let Some(issue) = &discovery.configuration_issue {
        let issue = serde_json::to_value(issue)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(ControlReadError::Corrupt)?;
        if !actions_required.contains(&issue) {
            actions_required.push(issue);
        }
    }
    Ok(ComputeScanItemV1 {
        agent_id,
        supported,
        configuration_state,
        native_provider_id: None,
        discovered_source_ref: if protected_prepare {
            None
        } else {
            discovery
                .discovered_credential
                .as_ref()
                .map(|credential| credential.discovered_source_ref.clone())
                .or_else(|| {
                    discovery
                        .permission_hardening
                        .as_ref()
                        .map(|finding| finding.discovered_source_ref.clone())
                })
        },
        connection_option_id: configuration
            .map(|configuration| configuration.connection_option_id.clone()),
        endpoint_profile_id: configuration
            .map(|configuration| configuration.endpoint_profile_id.clone()),
        registered_base_url: if protected_prepare {
            None
        } else {
            configuration.map(|configuration| configuration.base_url.clone())
        },
        observed_model_id: configuration
            .map(|configuration| configuration.observed_model_id.clone()),
        model_configuration_id: configuration
            .and_then(|configuration| configuration.model_configuration_id.clone()),
        inventory_eligible,
        discovery: authorized.map(|value| value.discovery),
        actions_required,
        permission_action,
        credential_import,
    })
}

fn public_provenance(
    value: &hiroute_domain::ComputeCatalogProvenanceV1,
) -> ComputeCatalogProvenanceViewV1 {
    ComputeCatalogProvenanceViewV1 {
        product_release: value.product_release.clone(),
        catalog_binding_id: value.catalog_binding_id.clone(),
        release_sequence: value.release_sequence,
        connector_registry_digest: value.connector_registry_digest.clone(),
        model_data_digest: value.model_data_digest.clone(),
        cross_reference_digest: value.cross_reference_digest.clone(),
    }
}

#[cfg(test)]
#[path = "compute_routing/tests.rs"]
mod tests;
