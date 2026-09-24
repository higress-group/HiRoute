//! Safe Local Control views for saved compute management state.

use hiroute_domain::{
    BillingClass, ComputeManagedCapabilitiesV2, ComputeNativeEndpointV3,
    GatewayAuthenticationSemanticsV1, MaterializationState, NativeReasoningCapabilityV1,
    PriceValuationKindV1, RevisionSetV1,
};
use serde::{Deserialize, Serialize};

use crate::{ComputeCandidateTargetV2, ComputeModelMembershipV2};

pub const COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V2: &str = "hiroute.compute-management-snapshot/v2";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementQueryV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagedKeyAvailabilityV2 {
    Available,
    CoolingDown,
    Disabled,
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedKeyModelStatusV2 {
    pub binding_id: String,
    pub availability: ComputeManagedKeyAvailabilityV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedKeyViewV2 {
    pub key_id: String,
    pub generation: u64,
    pub fingerprint_hint: String,
    pub ordinal: u32,
    pub enabled: bool,
    pub model_statuses: Vec<ComputeManagedKeyModelStatusV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedModelViewV2 {
    pub model_ref: String,
    pub binding_id: String,
    pub revision: u64,
    pub upstream_model_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_configuration_id: Option<String>,
    pub membership: ComputeModelMembershipV2,
    pub capabilities: ComputeManagedCapabilitiesV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_reasoning: Option<NativeReasoningCapabilityV1>,
    pub presentation: ComputeModelPresentationV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeConnectionAccessKindV1 {
    Api,
    Subscription,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionIdentityV1 {
    pub access_kind: ComputeConnectionAccessKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_label: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeModelAvailabilityV1 {
    Available,
    CoolingDown,
    Disabled,
    NeedsCredentials,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeModelAvailabilityReasonV1 {
    SourceDisabled,
    BindingDisabled,
    CredentialsMissing,
    AllCredentialsCooling,
    BindingNotReady,
    SourceNotReady,
    SubscriptionUpdating,
    AuthenticationRequired,
    ModelNotAllowed,
    RuntimeUnavailable,
    FactsUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputePriceContextV1 {
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeModelPresentationV1 {
    pub billing_class: BillingClass,
    pub availability: ComputeModelAvailabilityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ComputeModelAvailabilityReasonV1>,
    pub evaluated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub price_contexts: Vec<ComputePriceContextV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagementActionV2 {
    Edit,
    AddKey,
    Recheck,
    Reauthorize,
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagementRuntimeReadStateV2 {
    Complete,
    Partial,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedSourceViewV2 {
    /// Display only; does not confer provider, capability, billing or credential authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_template_id: Option<String>,
    /// Non-secret directory path used to restore the source edit form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_path: Option<String>,
    pub source_id: String,
    pub revision: u64,
    pub display_name: String,
    pub provenance: crate::ComputeCandidateProvenanceKindV2,
    pub connection_identity: ComputeConnectionIdentityV1,
    pub target: ComputeCandidateTargetV2,
    pub authentication: GatewayAuthenticationSemanticsV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_native_endpoints: Vec<ComputeNativeEndpointV3>,
    pub state: MaterializationState,
    pub models: Vec<ComputeManagedModelViewV2>,
    pub keys: Vec<ComputeManagedKeyViewV2>,
    pub ready_model_count: u32,
    pub actions: Vec<ComputeManagementActionV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementSnapshotV2 {
    pub schema: String,
    pub revisions: RevisionSetV1,
    pub runtime_state: ComputeManagementRuntimeReadStateV2,
    pub sources: Vec<ComputeManagedSourceViewV2>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_availability_reasons_are_a_closed_wire_enum() {
        assert_eq!(
            serde_json::to_value(ComputeModelAvailabilityReasonV1::AllCredentialsCooling).unwrap(),
            "all_credentials_cooling"
        );
        assert!(
            serde_json::from_value::<ComputeModelAvailabilityReasonV1>(serde_json::json!(
                "provider_said_something"
            ))
            .is_err()
        );
    }
}
