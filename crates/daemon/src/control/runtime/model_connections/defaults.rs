//! One source-aware capability projection shared by reads, checks and publication inputs.
use super::*;
use hiroute_application_api::*;
use hiroute_domain::*;
use std::collections::BTreeMap;

pub(in crate::control::runtime) fn scoped_capability<'a>(
    catalog: &'a TrustedReleaseCatalog,
    source: &ComputeManagementSourceV2,
    model: &ComputeManagedModelV2,
) -> Option<&'a ModelEndpointCapabilityV1> {
    let id = model.catalog_configuration_id.as_ref()?;
    let mut candidates = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .filter(|cap| {
            if cap.model_configuration_id != *id
                || cap.upstream_model_id != model.upstream_model_id
                || cap.upstream_protocol != source.target.upstream_protocol
            {
                return false;
            }
            match &source.provenance {
                ComputeManagementProvenanceV2::Registered {
                    connection_option_id,
                    ..
                } => catalog
                    .resolve_connection_option(connection_option_id)
                    .ok()
                    .is_some_and(|resolved| {
                        registered_source_matches_current_option(source, &resolved)
                            && cap.connector_id == resolved.connector.connector_id
                            && cap.connector_revision == resolved.connector.revision
                            && cap.endpoint_profile_id
                                == resolved.endpoint_profile.endpoint_profile_id
                            && cap.endpoint_profile_revision == resolved.endpoint_profile.revision
                    }),
                ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } => {
                    cap.connector_id == *connector_id
                        && cap.endpoint_profile_id == source.target.protocol_profile_id
                        && cap.endpoint_profile_revision == source.target.protocol_profile_revision
                }
                ComputeManagementProvenanceV2::UserConfigured { .. } => false,
            }
        });
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}
pub(in crate::control::runtime) fn project_source(
    catalog: &TrustedReleaseCatalog,
    source: &ComputeManagementSourceV2,
    preferences: &SourceDefaultPreferencesV1,
) -> ComputeManagementSourceV2 {
    let mut effective = source.clone();
    for model in &mut effective.models {
        let fields = preferences
            .get(&model.binding_id)
            .cloned()
            .unwrap_or_default();
        let current = scoped_capability(catalog, source, model).and_then(|cap| {
            let definition = catalog.model_data().model(&cap.model_configuration_id)?;
            let reasoning = catalog
                .native_reasoning()
                .iter()
                .find(|r| r.model_configuration_id == cap.model_configuration_id)?;
            hiroute_integrations::compute::resolve_managed_defaults(
                catalog,
                &model.capabilities,
                &fields,
                cap,
                definition,
                reasoning,
            )
            .ok()
        });
        model.capabilities = current.unwrap_or_else(|| {
            hiroute_integrations::compute::without_current_defaults(
                &model.capabilities,
                &fields,
                model
                    .catalog_configuration_id
                    .as_ref()
                    .is_some_and(|id| catalog.model_data().model(id).is_none()),
            )
        });
        if let Some(default) = hiroute_integrations::compute::missing_api_reasoning_default(
            catalog,
            &source.provenance,
            &source.target,
            &source.authentication,
            &model.upstream_model_id,
            &model.capabilities.native_reasoning,
            fields.get(&ModelDefaultFieldV1::NativeReasoning),
        ) {
            model.capabilities.native_reasoning = ComputeManagementFactValueV2 {
                value: Some(default.capability),
                basis: ComputeManagementFactBasisV2::RuntimeFallback,
            };
        }
    }
    effective
}
impl LocalControlAdapter {
    pub(super) fn compute_defaults(
        &self,
        query: ComputeDefaultsQueryV1,
    ) -> Result<ComputeDefaultsResultV1, ComputeManagementControlError> {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Conflict)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let control = stores.control();
        let source = control
            .compute_management_source(&query.source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        let preferences = control
            .source_default_preferences(&source.source_id)
            .map_err(map_port)?;
        let revisions = control
            .current_revisions(&WorkspaceId::default())
            .map_err(map_port)?;
        let projected = project_source(&catalog, &source, &preferences);
        let mut models = Vec::new();
        for (saved, effective) in source.models.iter().zip(&projected.models) {
            let cap = scoped_capability(&catalog, &source, saved);
            let locator = cap.map(|cap| OfficialModelLocatorV1 {
                model_configuration_id: cap.model_configuration_id.clone(),
                upstream_model_id: cap.upstream_model_id.clone(),
                connector_id: cap.connector_id.clone(),
                endpoint_profile_id: cap.endpoint_profile_id.clone(),
                protocol_endpoint_id: cap.protocol_endpoint_id.clone(),
            });
            let fields_preferences = preferences.get(&saved.binding_id);
            let api_reasoning_default =
                hiroute_integrations::compute::missing_api_reasoning_default(
                    &catalog,
                    &source.provenance,
                    &source.target,
                    &source.authentication,
                    &saved.upstream_model_id,
                    &saved.capabilities.native_reasoning,
                    fields_preferences.and_then(|p| p.get(&ModelDefaultFieldV1::NativeReasoning)),
                )
                .is_some();
            let mut fields = BTreeMap::new();
            macro_rules! field {
                ($name:ident, $key:ident) => {{
                    let saved_fact = &saved.capabilities.$name;
                    let intent = fields_preferences.and_then(|p| p.get(&ModelDefaultFieldV1::$key));
                    let follows = matches!(
                        intent,
                        Some(ModelDefaultPreferenceV1::FollowOfficial { .. })
                    ) || intent.is_none()
                        && saved_fact.basis == ComputeManagementFactBasisV2::RegisteredCatalog
                        && locator.is_some();
                    fields.insert(
                        ModelDefaultFieldV1::$key,
                        ComputeDefaultFieldViewV1 {
                            saved_value: serde_json::to_value(&saved_fact.value)
                                .map_err(|_| ComputeManagementControlError::Corrupt)?,
                            effective_value: if follows && locator.is_none() {
                                serde_json::Value::Null
                            } else {
                                serde_json::to_value(&effective.capabilities.$name.value)
                                    .map_err(|_| ComputeManagementControlError::Corrupt)?
                            },
                            intent: if follows {
                                "follow_official"
                            } else {
                                "preserve_saved"
                            }
                            .into(),
                            origin: if follows
                                || (ModelDefaultFieldV1::$key
                                    == ModelDefaultFieldV1::NativeReasoning
                                    && api_reasoning_default)
                            {
                                if catalog.official_data_from_feed() {
                                    "website_cache"
                                } else {
                                    "embedded"
                                }
                            } else {
                                "preserved"
                            }
                            .into(),
                            locator: if saved_fact.basis
                                == ComputeManagementFactBasisV2::ConnectorVerified
                            {
                                None
                            } else {
                                locator.clone()
                            },
                        },
                    );
                }};
            }
            field!(tool, Tool);
            field!(vision, Vision);
            field!(streaming, Streaming);
            field!(context_tokens, ContextTokens);
            field!(max_output_tokens, MaxOutputTokens);
            field!(native_reasoning, NativeReasoning);
            models.push(ComputeModelDefaultsV1 {
                binding_id: saved.binding_id.clone(),
                model_configuration_id: saved.catalog_configuration_id.clone(),
                fields,
            });
        }
        if self
            .current_catalog()
            .is_none_or(|c| c.generation != catalog.generation)
        {
            return Err(ComputeManagementControlError::PreviewStale);
        }
        Ok(ComputeDefaultsResultV1 {
            source_id: source.source_id,
            source_revision: source.revision,
            revisions,
            reviewed_on: catalog.model_metadata().as_of.clone(),
            models,
        })
    }
    pub(super) fn establish_save_defaults(
        &self,
        change: &mut ComputeManagementChangeV3,
        catalog: &TrustedReleaseCatalog,
    ) -> Result<(), ComputeManagementControlError> {
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let preview = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview(change.change.clone())
        .map_err(map_planning)?;
        let mutation: ComputeManagementMutationV2 =
            serde_json::from_value(preview.plan().control()["compute_management_mutation"].clone())
                .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let Some(desired) = mutation.desired() else {
            return Ok(());
        };
        for model in &desired.models {
            if mutation.expected().is_some_and(|before| {
                before
                    .models
                    .iter()
                    .any(|old| old.binding_id == model.binding_id)
            }) {
                continue;
            }
            let cap = scoped_capability(catalog, desired, model);
            let fields = change
                .default_preferences
                .entry(model.binding_id.clone())
                .or_default();
            macro_rules! establish {
                ($name:ident,$key:ident) => {
                    fields.entry(ModelDefaultFieldV1::$key).or_insert_with(|| {
                        if model.capabilities.$name.basis
                            == ComputeManagementFactBasisV2::RegisteredCatalog
                        {
                            if let Some(cap) = cap {
                                return ModelDefaultPreferenceV1::FollowOfficial {
                                    locator: OfficialModelLocatorV1 {
                                        model_configuration_id: cap.model_configuration_id.clone(),
                                        upstream_model_id: cap.upstream_model_id.clone(),
                                        connector_id: cap.connector_id.clone(),
                                        endpoint_profile_id: cap.endpoint_profile_id.clone(),
                                        protocol_endpoint_id: cap.protocol_endpoint_id.clone(),
                                    },
                                };
                            }
                        }
                        ModelDefaultPreferenceV1::PreserveSaved
                    });
                };
            }
            establish!(tool, Tool);
            establish!(vision, Vision);
            establish!(streaming, Streaming);
            establish!(context_tokens, ContextTokens);
            establish!(max_output_tokens, MaxOutputTokens);
            establish!(native_reasoning, NativeReasoning);
        }
        Ok(())
    }
}
