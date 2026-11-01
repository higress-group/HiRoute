//! Internal managed-text boundary. Callers must derive the scope from authenticated
//! admission; this module is not a transport endpoint or an authorization issuer.
//! References carry no authority and expose no filesystem locations.

mod progress;
#[cfg(test)]
mod progress_tests;
mod retention;
mod schema;
mod storage;
#[cfg(test)]
mod tests;

pub use storage::ManagedTextReader;

use hiroute_domain::WorkspaceId;
use serde::{Deserialize, Serialize};

pub const CHUNK_BYTES: usize = 64 * 1024;
pub const PAGE_BYTES: usize = 1024 * 1024;
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const PROGRESS_BATCH_BYTES: usize = 64 * 1024;
pub const PROGRESS_READ_MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTextScope {
    pub workspace_id: WorkspaceId,
    pub task_id: String,
    pub run_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedTextPurpose {
    Summary,
    Goal,
    Result,
    Debrief,
    Feedback,
    Evidence,
    NativeRecovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedTextState {
    Pending,
    Complete,
    Deleted,
    Expired,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTextRef {
    pub opaque_id: String,
    pub scope: ManagedTextScope,
    pub visibility_generation: u64,
    pub original_retention_deadline_ms: i64,
    pub state: ManagedTextState,
}

/// A historical import must cite a still-visible original reference. The service
/// verifies its creation time and generation rather than accepting a new TTL.
#[derive(Clone, Debug)]
pub struct ManagedTextInput {
    pub scope: ManagedTextScope,
    pub purpose: ManagedTextPurpose,
    pub source_event_id: String,
    pub source_revision: u64,
    pub original_created_at_ms: i64,
    pub import_origin: Option<ManagedTextRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextPage {
    pub reference: ManagedTextRef,
    pub bytes: Vec<u8>,
    pub next_chunk: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextDeletePreview {
    pub scope: ManagedTextScope,
    pub through_ms: i64,
    pub visibility_generation: u64,
    pub reference_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextDeleteResult {
    pub visibility_generation: u64,
    pub logically_hidden: u64,
    pub object_gc_pending: bool,
    /// MVP-20 acknowledges its own managed cache cleanup separately.
    pub native_gc_pending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextNativeCleanup {
    pub scope: ManagedTextScope,
    pub visibility_generation: u64,
    pub through_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextNativeCleanupCursor {
    pub scope: ManagedTextScope,
    pub visibility_generation: u64,
}

/// Read-only retention facts used by the daemon's trusted native-root consumer.  This does not
/// grant deletion and never returns managed text bytes or paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextScopeCleanupState {
    pub visibility_generation: u64,
    pub deleted_through_ms: i64,
    pub reference_count: u64,
    pub visible_reference_count: u64,
    pub latest_created_ms: Option<i64>,
}

/// Immutable identity for one run's best-effort public progress window. The timestamp comes from
/// the committed run acceptance record; callers may not refresh it on retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextProgressTarget {
    pub scope: ManagedTextScope,
    pub created_at_ms: i64,
}

/// Decoded, authenticated cursor facts. Encoding and signature ownership remains with the daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedTextProgressCursor {
    pub visibility_generation: u64,
    pub segment: u64,
    pub position: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedTextProgressRecovery {
    pub visibility_generation: u64,
    pub segment: u64,
    pub position: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTextProgressPage {
    pub visibility_generation: u64,
    pub segment: u64,
    pub window_start: u64,
    pub window_end: u64,
    pub text: String,
    pub next_position: u64,
    pub has_more: bool,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedTextProgressRead {
    Missing { visibility_generation: u64 },
    Available(ManagedTextProgressPage),
    Deleted,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedTextProgressWriteOutcome {
    Noop,
    Committed {
        visibility_generation: u64,
        segment: u64,
        head: u64,
        end: u64,
    },
    Hidden,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ManagedTextProgressReadError {
    #[error("worker progress cursor is invalid")]
    InvalidCursor,
    #[error("worker progress page is too small for one character")]
    PageTooSmall,
    #[error("worker progress cursor visibility is stale")]
    Stale,
    #[error("worker progress cursor was evicted")]
    Evicted(ManagedTextProgressRecovery),
    #[error("worker progress cursor crosses a capture gap")]
    Gap(ManagedTextProgressRecovery),
    #[error("worker progress storage is unavailable")]
    Storage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ManagedTextError {
    #[error("managed text input exceeds its bounds or is invalid")]
    Invalid,
    #[error("managed text scope is not authorized")]
    ScopeMismatch,
    #[error("managed text is missing, deleted or expired")]
    Unavailable,
    #[error("managed text visibility changed")]
    Stale,
    #[error("managed text event conflicts with persisted input")]
    Conflict,
    #[error("managed text storage is unavailable")]
    Storage,
}

impl From<rusqlite::Error> for ManagedTextError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl ManagedTextScope {
    fn key(&self) -> Result<String, ManagedTextError> {
        if WorkspaceId::parse(self.workspace_id.as_str()).is_err()
            || !valid_id(&self.task_id)
            || !valid_id(&self.run_id)
        {
            return Err(ManagedTextError::Invalid);
        }
        serde_json::to_string(self).map_err(|_| ManagedTextError::Invalid)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) use schema::migrate;

pub(crate) use retention::apply_managed_delete;
