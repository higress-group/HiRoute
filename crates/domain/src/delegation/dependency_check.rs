//! Closed, non-sensitive dependency admission facts. Native output is never an error value.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDependencyCheckV1 {
    ClaudeVersion,
    ClaudeHostManagedProvider,
    OpenCodeDatabase,
    OpenCodeConfiguration,
    PiPackage,
    PiNodeVersion,
    PiSdk,
}

impl NativeDependencyCheckV1 {
    pub const fn key(self) -> &'static str {
        match self {
            Self::ClaudeVersion => "claude_version",
            Self::ClaudeHostManagedProvider => "claude_host_managed_provider",
            Self::OpenCodeDatabase => "opencode_database",
            Self::OpenCodeConfiguration => "opencode_configuration",
            Self::PiPackage => "pi_package",
            Self::PiNodeVersion => "pi_node_version",
            Self::PiSdk => "pi_sdk",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDependencyFailureReasonV1 {
    Unavailable,
    Timeout,
    ProcessFailed,
    CleanupFailed,
    InvalidOutput,
    Unsupported,
    ConfigurationConflict,
}

impl NativeDependencyFailureReasonV1 {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::ProcessFailed => "process_failed",
            Self::CleanupFailed => "cleanup_failed",
            Self::InvalidOutput => "invalid_output",
            Self::Unsupported => "unsupported",
            Self::ConfigurationConflict => "configuration_conflict",
        }
    }

    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::Unavailable | Self::Timeout | Self::CleanupFailed
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeDependencyFailureV1 {
    pub check: NativeDependencyCheckV1,
    pub reason: NativeDependencyFailureReasonV1,
}

impl NativeDependencyFailureV1 {
    pub fn message_key(self) -> String {
        format!(
            "worker.dependencies.{}.{}",
            self.check.key(),
            self.reason.key()
        )
    }
}

impl fmt::Display for NativeDependencyFailureV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.check.key(), self.reason.key())
    }
}
