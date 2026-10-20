//! Read-only recovery of earlier mappings. Applying the returned draft uses normal settings.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPreviousRoutingQueryV1 {
    pub agent: String,
    #[serde(default)]
    pub presets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentPreviousRoutingPreviewV1 {
    Available {
        spec: Box<crate::AgentSettingsSpecV2>,
        current_operation: hiroute_domain::OperationId,
        restored_from: std::collections::BTreeMap<String, hiroute_domain::OperationId>,
    },
    Unavailable {
        reason: String,
        restore_is_separate: bool,
    },
}
