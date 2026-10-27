use crate::LogicalRequestId;
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationCatalogQueryV2 {
    pub request_id: LogicalRequestId,
    pub limit: u16,
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_input_selection: bool,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationCatalogPageV2 {
    pub contents: Vec<ObservationCatalogEntryV2>,
    pub next_cursor: Option<String>,
    pub transcript_roots: Vec<String>,
    pub roots_partial: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_selection: Option<super::RequestInputSelectionV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationCatalogEntryV2 {
    pub request_id: LogicalRequestId,
    pub content_id: String,
    pub message_occurrence_id: String,
    pub message_ordinal: u64,
    pub part_ordinal: u64,
    pub role: String,
    /// Canonical Gateway part type, including text, tool fields and provider state.
    pub kind: String,
    pub direction: String,
    pub fork_id: String,
    pub media_type: String,
    pub byte_count: u64,
    pub state: String,
    pub downstream_delivery: Option<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationAncestryQueryV2 {
    pub request_id: LogicalRequestId,
    pub transcript_root: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationAncestryV2 {
    /// Nearest root first. Only exact same-conversation, authorized links enter
    /// the result; callers preserve each occurrence rather than deduplicating blobs.
    pub roots: Vec<ObservationTranscriptRootV2>,
    pub gap: Option<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationTranscriptRootV2 {
    pub transcript_root: String,
    pub request_id: LogicalRequestId,
    pub fork_id: String,
    pub direction: String,
    pub state: String,
}
