//! Current-fact checks for saved/checked sources. Catalog digests are provenance, not validity.
use super::*;
use hiroute_domain::{
    ComputeManagedModelV2, ComputeManagementMutation, ComputeManagementProvenanceV2,
};

impl LocalControlAdapter {
    pub(super) fn validate_feed_save_plan(
        &self,
        plan: &hiroute_domain::TransactionPlanV1,
        catalog: &TrustedReleaseCatalog,
    ) -> Result<(), ComputeManagementControlError> {
        let mutation: ComputeManagementMutation = serde_json::from_value(
            plan.control()
                .get("compute_management_mutation")
                .cloned()
                .ok_or(ComputeManagementControlError::Corrupt)?,
        )
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let Some(source) = mutation.desired() else {
            return Ok(());
        };
        let ComputeManagementProvenanceV2::Registered {
            connection_option_id,
            ..
        } = &source.provenance
        else {
            return Ok(());
        };
        let resolved = catalog
            .resolve_connection_option(connection_option_id)
            .map_err(|_| ComputeManagementControlError::RegisteredOptionUnavailable)?;
        if !registered_source_matches_current_option(source, &resolved) {
            return Err(ComputeManagementControlError::RegisteredSourceMismatch);
        }
        for model in &source.models {
            let Some(id) = model.catalog_configuration_id.as_deref() else {
                continue;
            };
            let retained = mutation
                .expected()
                .is_some_and(|old| old.models.iter().any(|saved| saved == model));
            // Unchanged authenticated members remain manageable after withdrawal or absorption.
            if retained {
                continue;
            }
            let Some(definition) = catalog.model_data().model(id) else {
                return Err(ComputeManagementControlError::RegisteredOptionUnavailable);
            };
            let capability = catalog
                .model_data()
                .model_endpoint_capabilities
                .iter()
                .any(|cap| {
                    cap.model_configuration_id == id
                        && cap.upstream_model_id == model.upstream_model_id
                        && cap.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                        && cap.endpoint_profile_revision == resolved.endpoint_profile.revision
                        && cap.connector_id == resolved.connector.connector_id
                        && cap.connector_revision == resolved.connector.revision
                        && cap.upstream_protocol == source.target.upstream_protocol
                });
            let offered = catalog.model_data().offers.iter().any(|offer| {
                offer.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                    && offer.endpoint_profile_revision == resolved.endpoint_profile.revision
                    && offer.billing_class == resolved.option.billing_class
                    && offer
                        .model_configuration_ids
                        .iter()
                        .any(|model| model == id)
            });
            let reasoning = catalog
                .native_reasoning()
                .iter()
                .find(|entry| entry.model_configuration_id == id);
            let capability_record = catalog
                .model_data()
                .model_endpoint_capabilities
                .iter()
                .find(|cap| {
                    cap.model_configuration_id == id
                        && cap.upstream_model_id == model.upstream_model_id
                        && cap.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                        && cap.upstream_protocol == source.target.upstream_protocol
                });
            let preferences = mutation
                .preferences()
                .and_then(|(_, after)| after.get(&model.binding_id))
                .cloned()
                .unwrap_or_default();
            let resolved_defaults = capability_record.zip(reasoning).and_then(|(cap, reason)| {
                hiroute_integrations::compute::resolve_managed_defaults(
                    &model.capabilities,
                    &preferences,
                    cap,
                    definition,
                    reason,
                )
                .ok()
            });
            if !capability || !offered || resolved_defaults.is_none() {
                return Err(ComputeManagementControlError::RegisteredCatalogChanged);
            }
        }
        Ok(())
    }
}
