#![forbid(unsafe_code)]
//! Bounded, secret-free local runtime diagnostics for HiRoute.
//!
//! This crate owns the typed diagnostic event vocabulary, the bounded non-blocking writer,
//! the safe private-directory file adapter and the persisted level setting. It never
//! depends on a business crate, never installs global logging, never reads the environment
//! or `HOME` implicitly, and never lets a diagnostics failure change business execution:
//! queue overflow, disk errors and unsafe paths degrade the report instead.

pub mod agent_read;
pub mod context;
pub mod correlation;
pub mod error;
pub mod event;
pub mod files;
pub mod identity;
pub mod level;
mod maintenance;
pub mod panic;
pub mod publication;
pub mod queue;
pub mod record;
pub mod runtime;
pub mod settings;
mod shared;
pub mod writer;

pub use context::{DiagnosticContext, DiagnosticHandle, EmitOutcome};
pub use error::{EventErrorCode, StableErrorCode, SubsystemReason};
pub use event::{DiagnosticEvent, ProcessRole};
pub use level::{DiagnosticLevel, LevelSource};
pub use record::{Component, DiagnosticRecordV1, RECORD_SCHEMA_V1};
pub use runtime::{
    DiagnosticRuntime, DiagnosticsPort, RuntimeConfig, RuntimeStatus, StageReporter,
    WorkerStageReporter,
};

#[cfg(test)]
pub(crate) fn private_tempdir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temp dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private temp dir");
    }
    directory
}
