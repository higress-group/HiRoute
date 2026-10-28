//! Narrow, read-only facts for one trial setup capture.
use crate::{
    ComputeCandidateProvenanceKindV2, ComputeConnectionAccessKindV1, ComputeManagementActionV2,
    ComputeModelAvailabilityReasonV1, ComputeModelAvailabilityV1, ComputeModelMembershipV2,
    PlanEditorOptionsV1,
};
use hiroute_domain::{MaterializationState, RevisionSetV1};
use serde::{Deserialize, Serialize};

pub const GET_TRIAL_SETUP_SNAPSHOT_OPERATION_V1: &str = "GetTrialSetupSnapshotV1";
pub const TRIAL_SETUP_SNAPSHOT_SCHEMA_V1: &str = "hiroute.trial-setup-snapshot/v1";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSetupSnapshotV1 {
    pub schema: String,
    pub options: PlanEditorOptionsV1,
    pub management: TrialManagementSnapshotV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialManagementSnapshotV1 {
    pub revisions: RevisionSetV1,
    pub sources: Vec<TrialSourceViewV1>,
    /// Already-admitted API projections without any V2 management owner.
    pub retained_api_connections: Vec<TrialRetainedApiConnectionV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialRetainedApiConnectionV1 {
    pub source_id: String,
    pub source_revision: u64,
    pub binding_id: String,
    pub binding_revision: u64,
    pub model_configuration_id: String,
    pub display_name: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSourceViewV1 {
    pub source_id: String,
    pub revision: u64,
    pub display_name: String,
    pub provenance: ComputeCandidateProvenanceKindV2,
    pub access_kind: ComputeConnectionAccessKindV1,
    pub state: MaterializationState,
    pub models: Vec<TrialModelViewV1>,
    pub actions: Vec<ComputeManagementActionV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialModelViewV1 {
    pub model_ref: String,
    pub binding_id: String,
    pub revision: u64,
    pub upstream_model_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_configuration_id: Option<String>,
    pub membership: ComputeModelMembershipV2,
    /// Current candidate and the exact trusted catalog agree for this binding.
    pub catalog_admitted: bool,
    pub availability: ComputeModelAvailabilityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ComputeModelAvailabilityReasonV1>,
}
