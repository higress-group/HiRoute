//! Versioned source presentation. V2 operations keep their strict published response shape.
use crate::{ComputeManagedSourceViewV2, ComputeManagementRuntimeReadStateV2};
use hiroute_domain::RevisionSetV1;
use serde::{Deserialize, Serialize};

pub const LIST_COMPUTE_OPERATION_V3: &str = "ListComputeV3";
pub const GET_COMPUTE_OPERATION_V3: &str = "GetComputeV3";
pub const COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V3: &str = "hiroute.compute-management-snapshot/v3";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeSubscriptionModeV1 {
    NativeBorrowed,
    CpaManaged,
}

/// Joined only to the same source revision; this presentation grants no access authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionModeViewV1 {
    pub source_id: String,
    pub source_revision: u64,
    pub mode: ComputeSubscriptionModeV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementSnapshotV3 {
    pub schema: String,
    pub revisions: RevisionSetV1,
    pub runtime_state: ComputeManagementRuntimeReadStateV2,
    pub sources: Vec<ComputeManagedSourceViewV2>,
    pub subscription_modes: Vec<ComputeSubscriptionModeViewV1>,
}

impl ComputeManagementSnapshotV3 {
    /// Compatibility projection for the still-published ListCompute/GetCompute operations.
    pub fn into_v2(self) -> crate::ComputeManagementSnapshotV2 {
        crate::ComputeManagementSnapshotV2 {
            schema: crate::COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V2.into(),
            revisions: self.revisions,
            runtime_state: self.runtime_state,
            sources: self.sources,
        }
    }
}
