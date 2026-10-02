//! Storage readiness phases shared by daemon startup and the Desktop status projection.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageUpgradePhase {
    SourceCheck,
    Backup,
    Conversion,
    Validation,
    ServiceRecovery,
}
