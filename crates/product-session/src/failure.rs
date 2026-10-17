//! Preserve machine failures without turning a local fault into a business outcome.
use hiroute_application_api::MachineEnvelopeV2;
use hiroute_client_core::ClientFailure;
use hiroute_diagnostics::error::EventErrorCode;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DesktopFailure {
    Native {
        code: String,
    },
    Transport {
        failure: ClientFailure,
    },
    Backend {
        envelope: Box<MachineEnvelopeV2<Value>>,
    },
}
impl From<String> for DesktopFailure {
    fn from(code: String) -> Self {
        Self::Native { code }
    }
}
impl From<&str> for DesktopFailure {
    fn from(code: &str) -> Self {
        code.to_owned().into()
    }
}
impl From<ClientFailure> for DesktopFailure {
    fn from(failure: ClientFailure) -> Self {
        Self::Transport { failure }
    }
}
impl DesktopFailure {
    pub fn backend(envelope: MachineEnvelopeV2<Value>) -> Self {
        Self::Backend {
            envelope: Box::new(envelope),
        }
    }

    /// Stable event classification for this failure. A native code without a dedicated
    /// event code folds into `external_error` instead of forwarding arbitrary text.
    pub fn event_code(&self) -> EventErrorCode {
        match self {
            Self::Transport { failure } => failure.code.into(),
            Self::Backend { envelope } => envelope
                .error
                .as_ref()
                .map(|error| hiroute_client_core::category_error_code(error.category))
                .unwrap_or(EventErrorCode::ExternalError),
            Self::Native { code } => match code.as_str() {
                "window_denied" => EventErrorCode::WindowDenied,
                "path_unsafe" => EventErrorCode::PathUnsafe,
                "settings_invalid" => EventErrorCode::SettingsInvalid,
                "settings_conflict" => EventErrorCode::SettingsConflict,
                "settings_busy" => EventErrorCode::SettingsBusy,
                "settings_unwritable" => EventErrorCode::SettingsUnwritable,
                "override_active" => EventErrorCode::OverrideActive,
                "unsupported_platform" => EventErrorCode::UnsupportedPlatform,
                "diagnostics_unavailable" => EventErrorCode::DiagnosticsUnavailable,
                _ => EventErrorCode::ExternalError,
            },
        }
    }
}
