//! Resolve current defaults only after the caller has established exact source/endpoint authority.
use hiroute_domain::*;
use std::collections::BTreeMap;

pub fn resolve_managed_defaults(
    saved: &ComputeManagedCapabilitiesV2,
    preferences: &BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
    capability: &ModelEndpointCapabilityV1,
    model: &ModelDefinitionV1,
    reasoning: &ModelNativeReasoningV1,
) -> Result<ComputeManagedCapabilitiesV2, ComputeContractError> {
    if capability.model_configuration_id != model.model_configuration_id
        || reasoning.model_configuration_id != model.model_configuration_id
    {
        return Err(ComputeContractError::InvalidModelData);
    }
    let mut result = saved.clone();
    macro_rules! resolve {
        ($field:ident, $key:ident, $value:expr) => {
            let follow = match preferences.get(&ModelDefaultFieldV1::$key) {
                Some(ModelDefaultPreferenceV1::PreserveSaved) => false,
                Some(ModelDefaultPreferenceV1::FollowOfficial { locator }) => {
                    if saved.$field.basis == ComputeManagementFactBasisV2::ConnectorVerified {
                        return Err(ComputeContractError::InvalidModelData);
                    }
                    if locator.model_configuration_id != capability.model_configuration_id
                        || locator.upstream_model_id != capability.upstream_model_id
                        || locator.connector_id != capability.connector_id
                        || locator.endpoint_profile_id != capability.endpoint_profile_id
                        || locator.protocol_endpoint_id != capability.protocol_endpoint_id
                    {
                        return Err(ComputeContractError::InvalidModelData);
                    }
                    true
                }
                None => saved.$field.basis == ComputeManagementFactBasisV2::RegisteredCatalog,
            };
            if follow {
                result.$field = ComputeManagementFactValueV2 {
                    value: Some($value),
                    basis: ComputeManagementFactBasisV2::RegisteredCatalog,
                };
            }
        };
    }
    resolve!(tool, Tool, model.capabilities.tool);
    resolve!(vision, Vision, model.capabilities.vision);
    resolve!(streaming, Streaming, model.capabilities.streaming);
    resolve!(
        context_tokens,
        ContextTokens,
        model.capabilities.context_tokens
    );
    resolve!(
        max_output_tokens,
        MaxOutputTokens,
        model.capabilities.max_output_tokens
    );
    resolve!(
        native_reasoning,
        NativeReasoning,
        reasoning.capability.clone()
    );
    result
        .validate()
        .map_err(|_| ComputeContractError::InvalidModelData)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_defaults_follow_registered_fields_and_preserve_manual_false_and_unknown() {
        let catalog = crate::TrustedReleaseCatalog::load_release_facts(
            include_bytes!("../../../../assets/release-facts/current/bundle/manifest.json"),
            include_bytes!(
                "../../../../assets/release-facts/current/bundle/connector-registry.json"
            ),
            include_bytes!("../../../../assets/release-facts/current/bundle/model-data.json"),
        )
        .unwrap();
        let cap = &catalog.model_data().model_endpoint_capabilities[0];
        let model = catalog
            .model_data()
            .model(&cap.model_configuration_id)
            .unwrap();
        let reason = catalog
            .native_reasoning()
            .iter()
            .find(|v| v.model_configuration_id == model.model_configuration_id)
            .unwrap();
        fn registered<T>(v: T) -> ComputeManagementFactValueV2<T> {
            ComputeManagementFactValueV2 {
                value: Some(v),
                basis: ComputeManagementFactBasisV2::RegisteredCatalog,
            }
        }
        let mut saved = ComputeManagedCapabilitiesV2 {
            tool: registered(false),
            vision: registered(false),
            streaming: registered(false),
            context_tokens: registered(1234),
            max_output_tokens: registered(12),
            native_reasoning: registered(reason.capability.clone()),
        };
        saved.vision.basis = ComputeManagementFactBasisV2::UserDeclared;
        saved.streaming = ComputeManagementFactValueV2 {
            value: None,
            basis: ComputeManagementFactBasisV2::Unknown,
        };
        let original = saved.clone();
        let effective =
            resolve_managed_defaults(&saved, &BTreeMap::new(), cap, model, reason).unwrap();
        assert_eq!(
            effective.context_tokens.value,
            Some(model.capabilities.context_tokens)
        );
        assert_eq!(effective.vision.value, Some(false));
        assert_eq!(effective.streaming.value, None);
        assert_eq!(saved, original);
        let locator = OfficialModelLocatorV1 {
            model_configuration_id: cap.model_configuration_id.clone(),
            upstream_model_id: cap.upstream_model_id.clone(),
            connector_id: cap.connector_id.clone(),
            endpoint_profile_id: cap.endpoint_profile_id.clone(),
            protocol_endpoint_id: cap.protocol_endpoint_id.clone(),
        };
        let mut preferences = BTreeMap::from([
            (
                ModelDefaultFieldV1::Vision,
                ModelDefaultPreferenceV1::FollowOfficial {
                    locator: locator.clone(),
                },
            ),
            (
                ModelDefaultFieldV1::ContextTokens,
                ModelDefaultPreferenceV1::PreserveSaved,
            ),
        ]);
        let reset = resolve_managed_defaults(&saved, &preferences, cap, model, reason).unwrap();
        assert_eq!(reset.vision.value, Some(model.capabilities.vision));
        assert_eq!(reset.context_tokens.value, Some(1234));
        let mut wrong = locator;
        wrong.upstream_model_id.push_str("-other");
        preferences.insert(
            ModelDefaultFieldV1::Vision,
            ModelDefaultPreferenceV1::FollowOfficial { locator: wrong },
        );
        assert!(resolve_managed_defaults(&saved, &preferences, cap, model, reason).is_err());
    }
}
