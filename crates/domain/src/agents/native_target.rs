//! Explicit native scope. Opaque context identities never recover a filesystem path.
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentNativeTargetV1 {
    Hermes { config_root: String },
}

impl AgentNativeTargetV1 {
    pub fn config_root(&self) -> &str {
        match self {
            Self::Hermes { config_root } => config_root,
        }
    }

    pub fn validate(&self) -> Result<(), crate::AgentConnectionError> {
        let root = self.config_root();
        if root.is_empty()
            || root.len() > 4096
            || root.chars().any(char::is_control)
            || !std::path::Path::new(root).is_absolute()
        {
            return Err(crate::AgentConnectionError::InvalidGrant);
        }
        Ok(())
    }
}

/// Only the bundled canonical resource is selectable; callers cannot supply Skill content.
#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentManagementSkillSelectionV1 {}
