//! Native diagnostics handoff between the Tauri bridge and the resident bootstrap.
//!
//! The bridge owns the diagnostics runtime; the bootstrap only receives this small
//! description so it can pass the private root and parent session to the managed daemon and
//! report startup stages.

use std::path::PathBuf;

use hiroute_diagnostics::identity::SessionId;
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::runtime::DiagnosticsPort;

/// Read the smoke override only in an explicit desktop-pilot build; production ignores the
/// variable entirely. The parsed level is the only value ever passed to the managed daemon.
pub(crate) fn level_override() -> Option<DiagnosticLevel> {
    #[cfg(feature = "desktop-pilot")]
    {
        parse_level_override(
            std::env::var("HIROUTE_DIAGNOSTIC_LEVEL_OVERRIDE")
                .ok()
                .as_deref(),
        )
    }
    #[cfg(not(feature = "desktop-pilot"))]
    {
        None
    }
}

/// Exact four-level parsing; any other value (including a fifth selection) is ignored.
#[cfg(any(feature = "desktop-pilot", test))]
pub(crate) fn parse_level_override(value: Option<&str>) -> Option<DiagnosticLevel> {
    value.and_then(|value| value.parse().ok())
}

#[derive(Clone)]
pub struct NativeDiagnostics {
    pub root: PathBuf,
    pub parent_session: Option<SessionId>,
    port: DiagnosticsPort,
    upgrade_progress:
        Option<std::sync::Arc<std::sync::Mutex<Option<hiroute_host_runtime::StorageUpgradePhase>>>>,
}

impl NativeDiagnostics {
    pub fn new(root: PathBuf, parent_session: Option<SessionId>, port: DiagnosticsPort) -> Self {
        Self {
            root,
            parent_session,
            port,
            upgrade_progress: None,
        }
    }

    /// A no-op handoff for callers that do not run diagnostics (library use and tests).
    pub fn disabled() -> Self {
        Self::new(PathBuf::new(), None, DiagnosticsPort::default())
    }

    pub fn port(&self) -> DiagnosticsPort {
        self.port.clone()
    }

    pub fn with_upgrade_progress(
        mut self,
        progress: std::sync::Arc<
            std::sync::Mutex<Option<hiroute_host_runtime::StorageUpgradePhase>>,
        >,
    ) -> Self {
        self.upgrade_progress = Some(progress);
        self
    }
    pub(super) fn report_upgrade(&self, phase: hiroute_host_runtime::StorageUpgradePhase) {
        if let Some(progress) = &self.upgrade_progress
            && let Ok(mut current) = progress.lock()
        {
            *current = Some(phase);
        }
    }

    /// A process-lifetime context for modules that need a diagnostic anchor, for example
    /// the shared Local Control client. Long-lived callers derive per-unit-of-work children.
    pub fn context(&self) -> hiroute_diagnostics::context::DiagnosticContext {
        self.port.handle().root_context()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_parsing_is_exact_and_drops_everything_else() {
        assert_eq!(
            parse_level_override(Some("debug")),
            Some(DiagnosticLevel::Debug)
        );
        assert_eq!(
            parse_level_override(Some("error")),
            Some(DiagnosticLevel::Error)
        );
        assert_eq!(parse_level_override(Some("Debug")), None);
        assert_eq!(parse_level_override(Some("trace")), None);
        // `persisted` is a launcher selection, never a level.
        assert_eq!(parse_level_override(Some("persisted")), None);
        assert_eq!(parse_level_override(Some("")), None);
        assert_eq!(parse_level_override(None), None);
    }

    #[test]
    fn production_builds_never_expose_an_override() {
        #[cfg(not(feature = "desktop-pilot"))]
        assert_eq!(level_override(), None);
        #[cfg(feature = "desktop-pilot")]
        assert_eq!(
            level_override(),
            parse_level_override(
                std::env::var("HIROUTE_DIAGNOSTIC_LEVEL_OVERRIDE")
                    .ok()
                    .as_deref()
            )
        );
    }
}
