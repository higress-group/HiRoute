//! Reference-only catalog queries; execution configuration remains owned by routing.
use hiroute_domain::{ExactNativeReasoningV1, RatingSnapshotRefV1, ResolvedModelRatingV1};
use serde::{Deserialize, Serialize};

pub const MAX_RATING_QUERY_ITEMS: usize = 256;
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RatingSnapshotSelectionV1 {
    Latest,
    Version { version: String },
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRatingQueryItemV1 {
    pub query_id: String,
    pub model_configuration_id: String,
    pub exact_native_reasoning: ExactNativeReasoningV1,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveModelRatingsV1 {
    pub snapshot: RatingSnapshotSelectionV1,
    pub items: Vec<ModelRatingQueryItemV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRatingResultItemV1 {
    pub query_id: String,
    pub rating: ResolvedModelRatingV1,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveModelRatingsResultV1 {
    pub snapshot_ref: RatingSnapshotRefV1,
    pub items: Vec<ModelRatingResultItemV1>,
}

/// The existing ShowModel query carries the reference feature until the source inventory owner
/// composes its complete model page. Both variants remain read-only and versioned.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelCatalogQueryV1 {
    Reference { model_configuration_id: String },
    Ratings { query: ResolveModelRatingsV1 },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelCatalogResultV1 {
    Reference {
        model: Box<hiroute_domain::ModelReferenceViewV1>,
    },
    Ratings {
        result: ResolveModelRatingsResultV1,
    },
}
