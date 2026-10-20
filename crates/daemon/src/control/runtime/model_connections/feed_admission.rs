//! Current-fact checks for saved/checked sources. Catalog digests are provenance, not validity.
use super::*;
use hiroute_domain::{
    ComputeManagedModelV2, ComputeManagementMutationV2, ComputeManagementProvenanceV2,
};

impl LocalControlAdapter {
    pub(super) fn validate_feed_save_plan(
        &self,
        plan: &hiroute_domain::TransactionPlanV1,
        catalog: &TrustedReleaseCatalog,
    ) -> Result<(), ComputeManagementControlError> {
        let mutation: ComputeManagementMutationV2 = serde_json::from_value(
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
            let embedded = self.model_feed.as_ref().map_or_else(
                || catalog.model_data().model(id).is_some(),
                |feed| feed.is_embedded_model(id),
            );
            // Lifecycle edits retain saved embedded facts across catalog upgrades.
            // New members and feed-only members must still pass current admission.
            if retained && embedded {
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
            if !capability || !offered || !current_model(model, definition, reasoning) {
                return Err(ComputeManagementControlError::RegisteredCatalogChanged);
            }
        }
        Ok(())
    }
}
fn current_model(
    model: &ComputeManagedModelV2,
    definition: &hiroute_domain::ModelDefinitionV1,
    reasoning: Option<&hiroute_domain::ModelNativeReasoningV1>,
) -> bool {
    let facts = &model.capabilities;
    facts.tool.value == Some(definition.capabilities.tool)
        && facts.vision.value == Some(definition.capabilities.vision)
        && facts.streaming.value == Some(definition.capabilities.streaming)
        && facts.context_tokens.value == Some(definition.capabilities.context_tokens)
        && facts.max_output_tokens.value == Some(definition.capabilities.max_output_tokens)
        && facts.native_reasoning.value.as_ref() == reasoning.map(|entry| &entry.capability)
}
