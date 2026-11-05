use super::*;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateDiagnostic {
    pub step: String,
    pub io_kind: Option<String>,
    pub windows_error: Option<i32>,
    pub hresult: Option<u32>,
    pub process_exit: Option<i32>,
}
impl UpdateDiagnostic {
    pub fn valid(&self) -> bool {
        fn label(s: &str) -> bool {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        }
        label(&self.step) && self.io_kind.as_deref().is_none_or(label)
    }
}
#[derive(Clone, Debug)]
pub(super) struct UpdateFailure {
    pub code: &'static str,
    pub detail: UpdateDiagnostic,
}
impl From<&'static str> for UpdateFailure {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            detail: UpdateDiagnostic {
                step: "update".into(),
                io_kind: None,
                windows_error: None,
                hresult: None,
                process_exit: None,
            },
        }
    }
}
impl UpdateFailure {
    pub fn io(step: &'static str) -> impl FnOnce(io::Error) -> Self {
        move |error| Self {
            code: "UPGRADE_STAGE_UNAVAILABLE",
            detail: UpdateDiagnostic {
                step: step.into(),
                io_kind: Some(format!("{:?}", error.kind())),
                windows_error: error.raw_os_error(),
                hresult: background::hresult(&error),
                process_exit: None,
            },
        }
    }
    pub fn io_code(step: &'static str, code: &'static str) -> impl FnOnce(io::Error) -> Self {
        move |error| {
            let mut failure = Self::io(step)(error);
            failure.code = code;
            failure
        }
    }
    pub fn record(&self, service_available: bool) {
        if let Ok(data) = super::super::install_transaction::data() {
            let _ = self.record_at(&data, service_available);
        }
    }
    pub(super) fn record_at(
        &self,
        data: &std::path::Path,
        service_available: bool,
    ) -> io::Result<()> {
        let value = serde_json::json!({
            "schema": "hiroute.windows-shell-update-outcome/v1",
            "status": "failed", "code": self.code,
            "service_available": service_available, "diagnostic": self.detail
        });
        let bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
        hiroute_platform::durable::write_private_atomic(&data.join("update-outcome.json"), &bytes)
    }
}
