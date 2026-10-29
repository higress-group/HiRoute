//! Debug-only, bounded timing and aggregate sizes for Agent reads.
//! No identifiers, file names, content or credential material can be recorded.
use crate::{DiagnosticEvent, DiagnosticLevel, runtime::DiagnosticsPort};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReadCategory {
    HomeSummary,
    Model,
    Collaboration,
    Management,
    SettingsPlanning,
    ResidentService,
    GrantProvenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReadStage {
    Total,
    LockWait,
    LockHeld,
    SnapshotCopy,
    Locator,
    Verify,
    NativeRead,
    Fence,
    GrantAuthentication,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReadCounts {
    pub operations: usize,
    pub operation_bytes: usize,
    pub payloads: usize,
    pub payload_bytes: usize,
    pub grant_authentications: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReadTiming {
    pub category: AgentReadCategory,
    pub stage: AgentReadStage,
    pub elapsed_us: u64,
    pub counts: AgentReadCounts,
}

/// Emits on every exit, including errors and early returns. Diagnostics are observational.
pub struct Timer {
    port: DiagnosticsPort,
    category: AgentReadCategory,
    stage: AgentReadStage,
    started: Option<Instant>,
    pub counts: AgentReadCounts,
}

impl Timer {
    pub fn start(
        port: &DiagnosticsPort,
        category: AgentReadCategory,
        stage: AgentReadStage,
    ) -> Self {
        let enabled = port.handle().is_enabled()
            && !matches!(
                port.handle().level(),
                Some(DiagnosticLevel::Info | DiagnosticLevel::Warn | DiagnosticLevel::Error)
            );
        Self {
            port: port.clone(),
            category,
            stage,
            started: enabled.then(Instant::now),
            counts: AgentReadCounts::default(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        self.port
            .emit(DiagnosticEvent::AgentReadTiming(AgentReadTiming {
                category: self.category,
                stage: self.stage,
                elapsed_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                counts: self.counts,
            }));
    }
}
