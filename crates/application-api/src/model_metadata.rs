//! Bounded, snapshot-bound reads of the complete official descriptive catalog.
use hiroute_domain::CanonicalDigest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataSnapshotRefV1 {
    pub generation: u64,
    pub data_revision: u64,
    pub data_digest: CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataEntryV1 {
    pub kind: String,
    pub key: String,
    pub origin: String,
    /// Descriptive records never imply account access or an implemented adapter.
    pub qualification: String,
    pub value: Value,
}

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListModelMetadataV1 {
    pub kind: Option<String>,
    pub search: Option<String>,
    pub page_size: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataPageV1 {
    pub snapshot: ModelMetadataSnapshotRefV1,
    pub records: Vec<ModelMetadataEntryV1>,
    pub total: usize,
    pub next_cursor: Option<String>,
    pub complete: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetModelMetadataV1 {
    pub kind: String,
    pub key: String,
    pub snapshot: Option<ModelMetadataSnapshotRefV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataDetailV1 {
    pub snapshot: ModelMetadataSnapshotRefV1,
    pub record: ModelMetadataEntryV1,
}
