use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub const DEFAULT: &'static str = "personal/default";

    pub fn parse(value: impl Into<String>) -> Result<Self, WorkspaceIdError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
            })
            && !value.starts_with('/')
            && !value.ends_with('/')
            && !value.contains("//");
        if valid {
            Ok(Self(value))
        } else {
            Err(WorkspaceIdError)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("workspace id is not a bounded portable identifier")]
pub struct WorkspaceIdError;
