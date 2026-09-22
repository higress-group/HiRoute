//! The closed diagnostic event vocabulary.
//!
//! Events are grouped by the execution path they describe. Every event payload is a Rust
//! enum/struct with an exhaustive field list: there is no generic map, `serde_json::Value`
//! or free-form message channel, so a call site cannot smuggle request bodies, prompts,
//! tool arguments, credentials, raw third-party output or panic payloads into a record.

mod common;
mod content;
mod control;
mod cpa;
mod model;
mod observation;
mod startup;
mod system;
mod worker;

use serde::{Deserialize, Serialize};

use crate::level::DiagnosticLevel;

pub use common::*;
pub use content::*;
pub use control::*;
pub use cpa::*;
pub use model::*;
pub use observation::*;
pub use startup::*;
pub use system::*;
pub use worker::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticEvent {
    PublicationTiming(crate::publication::PublicationTiming),
    ProcessStart(ProcessStart),
    StartupEnd(StartupEnd),
    StageBegin(StageBegin),
    StageEnd(StageEnd),
    StageProgress(StageProgress),
    ReadyRead(ReadyIo),
    ReadyWrite(ReadyIo),
    ChildExit(ChildExit),
    PanicObserved(PanicObserved),
    LevelApplied(LevelApplied),
    DiagnosticsDegraded(DiagnosticsDegraded),
    DroppedSummary(DroppedSummary),
    WriterStats(WriterStats),
    CorrelationLink(CorrelationLink),
    ActionEnd(ActionEnd),
    Confirmation(ConfirmationEvent),
    Preview(PreviewEvent),
    Apply(ApplyEvent),
    ControlCallEnd(ControlCallEnd),
    ControlStage(ControlStage),
    RequestBegin(RequestBegin),
    RequestEnd(RequestEnd),
    RouteSelected(RouteSelected),
    AttemptBegin(AttemptBegin),
    AttemptEnd(AttemptEnd),
    UpstreamWire(UpstreamWire),
    Fallback(Fallback),
    SemanticCommit(SemanticCommit),
    RequestCancel(RequestCancel),
    RequestTimeout(RequestTimeout),
    ModelStage(ModelStage),
    ContentCaptureEnd(ContentCaptureEnd),
    SpillBegin(SpillBegin),
    IntegrityFailure(IntegrityFailure),
    ObservationGap(ObservationGap),
    SinkNack(SinkNack),
    ProjectionRejected(ProjectionRejected),
    WriterHealth(WriterHealth),
    InstallCheckEnd(InstallCheckEnd),
    RunAdmission(RunAdmission),
    WorkerLifecycle(WorkerLifecycle),
    TaskLifecycle(TaskLifecycle),
    WorkerStage(WorkerStage),
    CpaLifecycle(CpaLifecycle),
    CredentialLease(CredentialLease),
    CpaStage(CpaStage),
}

impl DiagnosticEvent {
    /// Severity of this event. Failures are `error`/`warn` so that they survive an `info`
    /// threshold; routine stage detail is `debug`.
    pub fn level(&self) -> DiagnosticLevel {
        use DiagnosticLevel::{Debug, Error, Info, Warn};
        match self {
            DiagnosticEvent::ProcessStart(_) | DiagnosticEvent::ChildExit(_) => Info,
            DiagnosticEvent::StartupEnd(end) => match end.outcome {
                StartupOutcome::Success => Info,
                StartupOutcome::Failure { .. } => Error,
                StartupOutcome::Cancelled => Warn,
            },
            DiagnosticEvent::PublicationTiming(_)
            | DiagnosticEvent::StageBegin(_)
            | DiagnosticEvent::StageEnd(_)
            | DiagnosticEvent::StageProgress(_) => Debug,
            DiagnosticEvent::ReadyRead(io) | DiagnosticEvent::ReadyWrite(io) => {
                if io.ok {
                    Info
                } else {
                    Error
                }
            }
            DiagnosticEvent::PanicObserved(_) => Error,
            DiagnosticEvent::LevelApplied(_) => Info,
            DiagnosticEvent::DiagnosticsDegraded(_) => Warn,
            DiagnosticEvent::DroppedSummary(summary) => {
                if summary.dropped_events > 0 {
                    Warn
                } else {
                    Debug
                }
            }
            DiagnosticEvent::WriterStats(stats) => {
                if stats.write_failures > 0 || stats.lost_at_shutdown > 0 {
                    Warn
                } else {
                    Debug
                }
            }
            DiagnosticEvent::CorrelationLink(_) => Debug,
            DiagnosticEvent::ActionEnd(_) => Info,
            DiagnosticEvent::Confirmation(_)
            | DiagnosticEvent::Preview(_)
            | DiagnosticEvent::Apply(_) => Debug,
            DiagnosticEvent::ControlCallEnd(call) => {
                if call.error.is_some() {
                    Warn
                } else {
                    Info
                }
            }
            DiagnosticEvent::ControlStage(_) => Debug,
            DiagnosticEvent::RequestBegin(_)
            | DiagnosticEvent::RouteSelected(_)
            | DiagnosticEvent::AttemptBegin(_) => Info,
            DiagnosticEvent::RequestEnd(end) => match end.outcome {
                RequestOutcome::Completed => Info,
                _ => Warn,
            },
            DiagnosticEvent::AttemptEnd(end) => match end.outcome {
                AttemptOutcome::Completed => Info,
                _ => Warn,
            },
            DiagnosticEvent::Fallback(_)
            | DiagnosticEvent::RequestCancel(_)
            | DiagnosticEvent::RequestTimeout(_) => Warn,
            DiagnosticEvent::SemanticCommit(_) => Info,
            DiagnosticEvent::ModelStage(_) | DiagnosticEvent::UpstreamWire(_) => Debug,
            DiagnosticEvent::ContentCaptureEnd(end) => match end.outcome {
                CaptureOutcome::Success => Info,
                CaptureOutcome::Abort => Warn,
            },
            DiagnosticEvent::SpillBegin(_) => Info,
            DiagnosticEvent::IntegrityFailure(_) => Error,
            DiagnosticEvent::ObservationGap(_)
            | DiagnosticEvent::SinkNack(_)
            | DiagnosticEvent::ProjectionRejected(_) => Warn,
            DiagnosticEvent::WriterHealth(_) => Debug,
            DiagnosticEvent::InstallCheckEnd(end) => match end.outcome {
                OutcomeKind::Completed => Info,
                _ => Warn,
            },
            DiagnosticEvent::RunAdmission(admission) => match admission.outcome {
                AdmissionOutcome::Admitted => Info,
                _ => Warn,
            },
            DiagnosticEvent::WorkerLifecycle(_)
            | DiagnosticEvent::TaskLifecycle(_)
            | DiagnosticEvent::CpaLifecycle(_)
            | DiagnosticEvent::CredentialLease(_) => Info,
            DiagnosticEvent::WorkerStage(_) | DiagnosticEvent::CpaStage(_) => Debug,
        }
    }

    /// Configuration evidence rather than a business record: the level this process applied
    /// is what makes every other record judgeable, so it is kept even when the level it
    /// reports filters records of its own severity. It is still one typed event on the same
    /// bounded queue, and its record keeps its `info` severity.
    pub fn bypasses_level_filter(&self) -> bool {
        matches!(self, DiagnosticEvent::LevelApplied(_))
    }

    /// Stable event name used in JSON and by tests.
    pub fn kind(&self) -> &'static str {
        match self {
            DiagnosticEvent::PublicationTiming(_) => "publication_timing",
            DiagnosticEvent::ProcessStart(_) => "process_start",
            DiagnosticEvent::StartupEnd(_) => "startup_end",
            DiagnosticEvent::StageBegin(_) => "stage_begin",
            DiagnosticEvent::StageEnd(_) => "stage_end",
            DiagnosticEvent::StageProgress(_) => "stage_progress",
            DiagnosticEvent::ReadyRead(_) => "ready_read",
            DiagnosticEvent::ReadyWrite(_) => "ready_write",
            DiagnosticEvent::ChildExit(_) => "child_exit",
            DiagnosticEvent::PanicObserved(_) => "panic_observed",
            DiagnosticEvent::LevelApplied(_) => "level_applied",
            DiagnosticEvent::DiagnosticsDegraded(_) => "diagnostics_degraded",
            DiagnosticEvent::DroppedSummary(_) => "dropped_summary",
            DiagnosticEvent::WriterStats(_) => "writer_stats",
            DiagnosticEvent::CorrelationLink(_) => "correlation_link",
            DiagnosticEvent::ActionEnd(_) => "action_end",
            DiagnosticEvent::Confirmation(_) => "confirmation",
            DiagnosticEvent::Preview(_) => "preview",
            DiagnosticEvent::Apply(_) => "apply",
            DiagnosticEvent::ControlCallEnd(_) => "control_call_end",
            DiagnosticEvent::ControlStage(_) => "control_stage",
            DiagnosticEvent::RequestBegin(_) => "request_begin",
            DiagnosticEvent::RequestEnd(_) => "request_end",
            DiagnosticEvent::RouteSelected(_) => "route_selected",
            DiagnosticEvent::AttemptBegin(_) => "attempt_begin",
            DiagnosticEvent::AttemptEnd(_) => "attempt_end",
            DiagnosticEvent::UpstreamWire(_) => "upstream_wire",
            DiagnosticEvent::Fallback(_) => "fallback",
            DiagnosticEvent::SemanticCommit(_) => "semantic_commit",
            DiagnosticEvent::RequestCancel(_) => "request_cancel",
            DiagnosticEvent::RequestTimeout(_) => "request_timeout",
            DiagnosticEvent::ModelStage(_) => "model_stage",
            DiagnosticEvent::ContentCaptureEnd(_) => "content_capture_end",
            DiagnosticEvent::SpillBegin(_) => "spill_begin",
            DiagnosticEvent::IntegrityFailure(_) => "integrity_failure",
            DiagnosticEvent::ObservationGap(_) => "observation_gap",
            DiagnosticEvent::SinkNack(_) => "sink_nack",
            DiagnosticEvent::ProjectionRejected(_) => "projection_rejected",
            DiagnosticEvent::WriterHealth(_) => "writer_health",
            DiagnosticEvent::InstallCheckEnd(_) => "install_check_end",
            DiagnosticEvent::RunAdmission(_) => "run_admission",
            DiagnosticEvent::WorkerLifecycle(_) => "worker_lifecycle",
            DiagnosticEvent::TaskLifecycle(_) => "task_lifecycle",
            DiagnosticEvent::WorkerStage(_) => "worker_stage",
            DiagnosticEvent::CpaLifecycle(_) => "cpa_lifecycle",
            DiagnosticEvent::CredentialLease(_) => "credential_lease",
            DiagnosticEvent::CpaStage(_) => "cpa_stage",
        }
    }
}
