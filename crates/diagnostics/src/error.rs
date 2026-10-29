//! Error and reason codes that may appear in diagnostics.
//!
//! These are fixed vocabularies, not `Display` output: a call site maps its own error to
//! one of these codes and may attach the raw OS integer errno. No error string, panic
//! payload or third-party message ever enters a record.

use serde::{Deserialize, Serialize};

/// The fixed native command error set. Unmapped causes fold into `diagnostics_unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StableErrorCode {
    WindowDenied,
    PathUnsafe,
    SettingsInvalid,
    SettingsConflict,
    SettingsBusy,
    SettingsUnwritable,
    OverrideActive,
    UnsupportedPlatform,
    DiagnosticsUnavailable,
}

impl StableErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            StableErrorCode::WindowDenied => "window_denied",
            StableErrorCode::PathUnsafe => "path_unsafe",
            StableErrorCode::SettingsInvalid => "settings_invalid",
            StableErrorCode::SettingsConflict => "settings_conflict",
            StableErrorCode::SettingsBusy => "settings_busy",
            StableErrorCode::SettingsUnwritable => "settings_unwritable",
            StableErrorCode::OverrideActive => "override_active",
            StableErrorCode::UnsupportedPlatform => "unsupported_platform",
            StableErrorCode::DiagnosticsUnavailable => "diagnostics_unavailable",
        }
    }

    /// Map a degraded subsystem reason onto the native command vocabulary.
    pub fn from_subsystem_reason(reason: SubsystemReason) -> Self {
        match reason {
            SubsystemReason::PathUnsafe => StableErrorCode::PathUnsafe,
            SubsystemReason::UnsupportedPlatform => StableErrorCode::UnsupportedPlatform,
            _ => StableErrorCode::DiagnosticsUnavailable,
        }
    }
}

/// Stable error classification carried by execution-path events. A value is always mapped
/// from a real failure at the call site that holds it; no `Display`/`Debug` text is
/// forwarded, and a mapped failure without a dedicated code becomes `ExternalError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventErrorCode {
    // Local Control transport and local client failures.
    LocatorUnavailable,
    TransportUnavailable,
    Deadline,
    PeerRejected,
    SchemaIncompatible,
    FrameInvalid,
    FrameTooLarge,
    ResponseMismatch,
    ProtectedInputUnavailable,
    ObservationStopped,
    // Native command vocabulary, so one event field can carry either source.
    WindowDenied,
    PathUnsafe,
    SettingsInvalid,
    SettingsConflict,
    SettingsBusy,
    SettingsUnwritable,
    OverrideActive,
    UnsupportedPlatform,
    DiagnosticsUnavailable,
    ResidentUnavailable,
    ClientContextInvalid,
    ReadSnapshotChanged,
    ConfirmationStale,
    ResponseDataMissing,
    // Stable business/action rejections: the first eight mirror the product error
    // categories, the rest describe confirmation-gated and cancelled work.
    Usage,
    Conflict,
    Authorization,
    NotFound,
    Unavailable,
    ActionRequired,
    Recovery,
    Internal,
    Denied,
    Stale,
    Cancelled,
    Timeout,
    // A real failure was mapped but has no dedicated code.
    ExternalError,
}

impl EventErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            EventErrorCode::LocatorUnavailable => "locator_unavailable",
            EventErrorCode::TransportUnavailable => "transport_unavailable",
            EventErrorCode::Deadline => "deadline",
            EventErrorCode::PeerRejected => "peer_rejected",
            EventErrorCode::SchemaIncompatible => "schema_incompatible",
            EventErrorCode::FrameInvalid => "frame_invalid",
            EventErrorCode::FrameTooLarge => "frame_too_large",
            EventErrorCode::ResponseMismatch => "response_mismatch",
            EventErrorCode::ProtectedInputUnavailable => "protected_input_unavailable",
            EventErrorCode::ObservationStopped => "observation_stopped",
            EventErrorCode::WindowDenied => "window_denied",
            EventErrorCode::PathUnsafe => "path_unsafe",
            EventErrorCode::SettingsInvalid => "settings_invalid",
            EventErrorCode::SettingsConflict => "settings_conflict",
            EventErrorCode::SettingsBusy => "settings_busy",
            EventErrorCode::SettingsUnwritable => "settings_unwritable",
            EventErrorCode::OverrideActive => "override_active",
            EventErrorCode::UnsupportedPlatform => "unsupported_platform",
            EventErrorCode::DiagnosticsUnavailable => "diagnostics_unavailable",
            EventErrorCode::ResidentUnavailable => "resident_unavailable",
            EventErrorCode::ClientContextInvalid => "client_context_invalid",
            EventErrorCode::ReadSnapshotChanged => "read_snapshot_changed",
            EventErrorCode::ConfirmationStale => "confirmation_stale",
            EventErrorCode::ResponseDataMissing => "response_data_missing",
            EventErrorCode::Usage => "usage",
            EventErrorCode::Conflict => "conflict",
            EventErrorCode::Authorization => "authorization",
            EventErrorCode::NotFound => "not_found",
            EventErrorCode::Unavailable => "unavailable",
            EventErrorCode::ActionRequired => "action_required",
            EventErrorCode::Recovery => "recovery",
            EventErrorCode::Internal => "internal",
            EventErrorCode::Denied => "denied",
            EventErrorCode::Stale => "stale",
            EventErrorCode::Cancelled => "cancelled",
            EventErrorCode::Timeout => "timeout",
            EventErrorCode::ExternalError => "external_error",
        }
    }
}

impl From<StableErrorCode> for EventErrorCode {
    fn from(code: StableErrorCode) -> Self {
        match code {
            StableErrorCode::WindowDenied => EventErrorCode::WindowDenied,
            StableErrorCode::PathUnsafe => EventErrorCode::PathUnsafe,
            StableErrorCode::SettingsInvalid => EventErrorCode::SettingsInvalid,
            StableErrorCode::SettingsConflict => EventErrorCode::SettingsConflict,
            StableErrorCode::SettingsBusy => EventErrorCode::SettingsBusy,
            StableErrorCode::SettingsUnwritable => EventErrorCode::SettingsUnwritable,
            StableErrorCode::OverrideActive => EventErrorCode::OverrideActive,
            StableErrorCode::UnsupportedPlatform => EventErrorCode::UnsupportedPlatform,
            StableErrorCode::DiagnosticsUnavailable => EventErrorCode::DiagnosticsUnavailable,
        }
    }
}

/// Controlled reasons for the diagnostic subsystem's own state. Only these codes reach the
/// runtime status and `diagnostics_degraded`; arbitrary text is never forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemReason {
    WriterOwned,
    WriterOpenFailed,
    WriterWriteFailed,
    RotationFailed,
    RetentionFailed,
    QueueSaturated,
    SettingsUnreadable,
    SettingsInvalid,
    CorrelationUnavailable,
    PathUnsafe,
    UnsupportedPlatform,
}

impl SubsystemReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SubsystemReason::WriterOwned => "writer_owned",
            SubsystemReason::WriterOpenFailed => "writer_open_failed",
            SubsystemReason::WriterWriteFailed => "writer_write_failed",
            SubsystemReason::RotationFailed => "rotation_failed",
            SubsystemReason::RetentionFailed => "retention_failed",
            SubsystemReason::QueueSaturated => "queue_saturated",
            SubsystemReason::SettingsUnreadable => "settings_unreadable",
            SubsystemReason::SettingsInvalid => "settings_invalid",
            SubsystemReason::CorrelationUnavailable => "correlation_unavailable",
            SubsystemReason::PathUnsafe => "path_unsafe",
            SubsystemReason::UnsupportedPlatform => "unsupported_platform",
        }
    }

    /// Map a settings/file failure into the controlled subsystem vocabulary.
    pub fn from_settings_error(error: crate::settings::SettingsError) -> Self {
        use crate::settings::SettingsError as E;
        match error {
            E::UnsafePath | E::UnsafeFile => SubsystemReason::PathUnsafe,
            E::Invalid | E::RevisionExhausted | E::Conflict => SubsystemReason::SettingsInvalid,
            E::Busy => SubsystemReason::SettingsUnreadable,
            E::Unwritable | E::Io => SubsystemReason::SettingsUnreadable,
            E::UnsupportedPlatform => SubsystemReason::UnsupportedPlatform,
        }
    }
}
