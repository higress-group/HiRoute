//! Resolve current defaults only after the caller has established exact source/endpoint authority.
use super::TrustedReleaseCatalog;
use hiroute_domain::*;
mod api_capabilities;
mod api_scope;
pub use api_capabilities::{ApiCapabilityCompletion, complete_missing_api_capabilities};
mod native_reasoning;
pub use native_reasoning::{ApiReasoningDefault, missing_api_reasoning_default};
mod scoped;
pub use scoped::*;
use std::collections::BTreeMap;

pub fn resolve_managed_defaults(
    catalog: &TrustedReleaseCatalog,
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
    let official = official_capability_defaults(catalog, capability, model, reasoning);
    let mut result = saved.clone();
    macro_rules! resolve {
        ($field:ident, $key:ident, $value:expr) => {
            let follow = match preferences.get(&ModelDefaultFieldV1::$key) {
                Some(
                    ModelDefaultPreferenceV1::PreserveSaved
                    | ModelDefaultPreferenceV1::Custom { .. },
                ) => false,
                Some(ModelDefaultPreferenceV1::FollowMetadata) => true,
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
                    value: $value,
                    basis: if $value.is_some() {
                        ComputeManagementFactBasisV2::RegisteredCatalog
                    } else {
                        ComputeManagementFactBasisV2::Unknown
                    },
                };
            }
        };
    }
    resolve!(tool, Tool, official.tool.value);
    resolve!(vision, Vision, official.vision.value);
    resolve!(streaming, Streaming, official.streaming.value);
    resolve!(context_tokens, ContextTokens, official.context_tokens.value);
    resolve!(
        max_output_tokens,
        MaxOutputTokens,
        official.max_output_tokens.value
    );
    resolve!(
        native_reasoning,
        NativeReasoning,
        official.native_reasoning.value.clone()
    );
    result = apply_custom_capability_defaults(&result, preferences);
    result
        .validate()
        .map_err(|_| ComputeContractError::InvalidModelData)?;
    Ok(result)
}

/// A withdrawn or mismatched exact default never falls back to the saved default value.
/// Ambiguous legacy provenance may retain its saved meaning until explicitly changed.
pub fn without_current_defaults(
    saved: &ComputeManagedCapabilitiesV2,
    preferences: &BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
    clear_implicit: bool,
) -> ComputeManagedCapabilitiesV2 {
    let mut result = apply_custom_capability_defaults(saved, preferences);
    macro_rules! clear {
        ($name:ident,$field:ident) => {
            if matches!(
                preferences.get(&ModelDefaultFieldV1::$field),
                Some(ModelDefaultPreferenceV1::FollowOfficial { .. })
                    | Some(ModelDefaultPreferenceV1::FollowMetadata)
            ) || clear_implicit
                && !preferences.contains_key(&ModelDefaultFieldV1::$field)
                && saved.$name.basis == ComputeManagementFactBasisV2::RegisteredCatalog
            {
                result.$name = ComputeManagementFactValueV2 {
                    value: None,
                    basis: ComputeManagementFactBasisV2::Unknown,
                };
            }
        };
    }
    clear!(tool, Tool);
    clear!(vision, Vision);
    clear!(streaming, Streaming);
    clear!(context_tokens, ContextTokens);
    clear!(max_output_tokens, MaxOutputTokens);
    clear!(native_reasoning, NativeReasoning);
    result
}

/// Custom choices live in the authenticated source-default operation, not raw source facts.
pub fn apply_custom_capability_defaults(
    saved: &ComputeManagedCapabilitiesV2,
    preferences: &BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
) -> ComputeManagedCapabilitiesV2 {
    let mut result = saved.clone();
    macro_rules! custom {
        ($name:ident, $field:ident, $variant:ident) => {
            if let Some(ModelDefaultPreferenceV1::Custom { value }) =
                preferences.get(&ModelDefaultFieldV1::$field)
            {
                let value = match value.clone() {
                    ModelDefaultValueV1::$variant(value) => Some(value),
                    _ => None,
                };
                let basis = if value.is_some() {
                    ComputeManagementFactBasisV2::UserDeclared
                } else {
                    ComputeManagementFactBasisV2::Unknown
                };
                result.$name = ComputeManagementFactValueV2 { value, basis };
            }
        };
    }
    custom!(tool, Tool, Boolean);
    custom!(vision, Vision, Boolean);
    custom!(streaming, Streaming, Boolean);
    custom!(context_tokens, ContextTokens, Tokens);
    custom!(max_output_tokens, MaxOutputTokens, Tokens);
    custom!(native_reasoning, NativeReasoning, Reasoning);
    result
}

/// Verify a native admission against the saved facts, never against today's defaults.
pub fn saved_native_capability_digest(
    upstream: &str,
    saved: &ComputeManagedCapabilitiesV2,
) -> Option<CanonicalDigest> {
    use crate::{
        NativeCandidateFactBasisV1 as Basis, NativeCandidateFactValueV1 as Fact,
        NativeModelCapabilityDeclarationV1,
    };
    fn fact<T: Clone>(saved: &ComputeManagementFactValueV2<T>) -> Option<Fact<T>> {
        let basis = match saved.basis {
            ComputeManagementFactBasisV2::RegisteredCatalog => Basis::RegisteredCatalog,
            ComputeManagementFactBasisV2::RuntimeFallback => Basis::RuntimeFallback,
            ComputeManagementFactBasisV2::UserDeclared => Basis::UserDeclared,
            ComputeManagementFactBasisV2::Observed => Basis::Observed,
            ComputeManagementFactBasisV2::Unknown => Basis::Unknown,
            ComputeManagementFactBasisV2::ConnectorVerified => return None,
        };
        Some(Fact {
            value: saved.value.clone(),
            basis,
        })
    }
    let declaration = NativeModelCapabilityDeclarationV1 {
        tool: fact(&saved.tool)?,
        vision: fact(&saved.vision)?,
        streaming: fact(&saved.streaming)?,
        context_tokens: fact(&saved.context_tokens)?,
        max_output_tokens: fact(&saved.max_output_tokens)?,
        native_reasoning: fact(&saved.native_reasoning)?,
    };
    CanonicalDigest::of(&("declared-model-capabilities/v1", upstream, &declaration)).ok()
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
            resolve_managed_defaults(&catalog, &saved, &BTreeMap::new(), cap, model, reason)
                .unwrap();
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
        let reset =
            resolve_managed_defaults(&catalog, &saved, &preferences, cap, model, reason).unwrap();
        assert_eq!(reset.vision.value, Some(model.capabilities.vision));
        assert_eq!(reset.context_tokens.value, Some(1234));
        let mut wrong = locator;
        wrong.upstream_model_id.push_str("-other");
        preferences.insert(
            ModelDefaultFieldV1::Vision,
            ModelDefaultPreferenceV1::FollowOfficial { locator: wrong },
        );
        assert!(
            resolve_managed_defaults(&catalog, &saved, &preferences, cap, model, reason).is_err()
        );
    }
}
