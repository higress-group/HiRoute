use serde::Deserialize;
use std::{fmt::Write, io};

#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) step: &'static str,
    pub(crate) code: String,
    pub(crate) command_code: Option<String>,
    pub(crate) daemon_code: Option<&'static str>,
    pub(crate) task_active: Option<bool>,
    pub(crate) control_ready: Option<bool>,
    pub(crate) io_kind: Option<io::ErrorKind>,
    pub(crate) os_error: Option<i32>,
    hresult: Option<u32>,
    detail: Option<Box<CommandDetail>>,
}

#[derive(Debug, Deserialize)]
struct CommandDetail {
    step: String,
    native: NativeError,
    rollback: Option<NativeError>,
}

#[derive(Debug, Deserialize)]
struct NativeError {
    io_kind: String,
    win32_error: Option<i32>,
    hresult: Option<u32>,
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

impl NativeError {
    fn report(&self, report: &mut String, prefix: &str) {
        if safe_label(&self.io_kind) {
            let _ = write!(report, "\r\n{prefix}I/O kind: {}", self.io_kind);
        }
        if let Some(code) = self.win32_error {
            let _ = write!(report, "\r\n{prefix}Windows error: {code}");
        }
        if let Some(code) = self.hresult {
            let _ = write!(report, "\r\n{prefix}HRESULT: 0x{code:08X}");
        }
    }
}

// Only the fixed code at the start of an error is shareable. The rest can
// contain paths, provider text, or a native message in the user's language.
pub(crate) fn safe_code(value: &str) -> Option<&str> {
    let token = value.split(':').next()?;
    (token.len() <= 64
        && token.contains('_')
        && token.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && token
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'))
    .then_some(token)
}

impl Failure {
    pub(crate) fn new(step: &'static str, code: &str) -> Self {
        Self {
            step,
            code: safe_code(code)
                .unwrap_or("INSTALL_DETAIL_UNAVAILABLE")
                .into(),
            command_code: None,
            daemon_code: None,
            task_active: None,
            control_ready: None,
            io_kind: None,
            os_error: None,
            hresult: None,
            detail: None,
        }
    }

    pub(crate) fn io(step: &'static str, error: io::Error) -> Self {
        let code = error.to_string();
        let mut failure = Self::new(step, safe_code(&code).unwrap_or("INSTALL_IO_ERROR"));
        failure.io_kind = Some(error.kind());
        failure.os_error = error.raw_os_error();
        #[cfg(windows)]
        {
            failure.hresult = hiroute_platform::windows::background::hresult(&error);
        }
        failure
    }

    pub(crate) fn from_command(step: &'static str, value: &serde_json::Value) -> Self {
        let error = &value["error"];
        let mut failure = Self::new(step, error["code"].as_str().unwrap_or("INSTALL_FAILED"));
        if value["schema"] == "hiroute.windows-maintenance-error/v1" {
            failure.detail = serde_json::from_value(error["detail"].clone()).ok();
        }
        failure
    }

    pub(crate) fn report(&self) -> String {
        let mut report = format!(
            "HiRoute {} installer diagnostic\r\nStep: {}\r\nCode: {}",
            env!("CARGO_PKG_VERSION"),
            self.step,
            self.code
        );
        if let Some(code) = &self.command_code {
            let _ = write!(report, "\r\nCommand code: {code}");
        }
        if let Some(code) = self.daemon_code {
            let _ = write!(report, "\r\nDaemon startup code: {code}");
        }
        if let Some(active) = self.task_active {
            let _ = write!(report, "\r\nTask active: {active}");
        }
        if let Some(ready) = self.control_ready {
            let _ = write!(report, "\r\nLocal control ready: {ready}");
        }
        if let Some(kind) = self.io_kind {
            let _ = write!(report, "\r\nI/O kind: {kind:?}");
        }
        if let Some(code) = self.os_error {
            let _ = write!(report, "\r\nWindows error: {code}");
        }
        if let Some(code) = self.hresult {
            let _ = write!(report, "\r\nHRESULT: 0x{code:08X}");
        }
        if let Some(detail) = &self.detail {
            if safe_label(&detail.step) {
                let _ = write!(report, "\r\nOperation: {}", detail.step);
            }
            detail.native.report(&mut report, "");
            if let Some(rollback) = &detail.rollback {
                let _ = write!(report, "\r\nStandalone rollback: failed");
                rollback.report(&mut report, "Rollback ");
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_keep_actionable_codes_and_exclude_private_error_text() {
        let private = r"C:\Users\PRIVATE\install.json";
        let error = io::Error::other(format!("INSTALL_OWNERSHIP_CONFLICT: {private}"));
        let report = Failure::io("recover-standalone", error).report();
        assert!(report.is_ascii());
        assert!(report.contains("Step: recover-standalone"));
        assert!(report.contains("Code: INSTALL_OWNERSHIP_CONFLICT"));
        assert!(!report.contains("PRIVATE"));
        assert!(safe_code(private).is_none());
        assert!(safe_code("INSTALL_FAILED\r\nPRIVATE").is_none());
    }

    #[test]
    fn command_diagnostic_keeps_hresult_and_rollback_without_raw_text() {
        let reply = serde_json::json!({
            "schema": "hiroute.windows-maintenance-error/v1",
            "error": {
                "code": "INSTALL_FAILED",
                "message": "private native detail",
                "detail": {
                    "step": "register-task",
                    "native": {"io_kind": "Other", "hresult": 0x80070005u32},
                    "rollback": {"io_kind": "PermissionDenied", "win32_error": 5}
                }
            }
        });
        let report = Failure::from_command("install-standalone", &reply).report();
        assert!(report.contains("Operation: register-task"));
        assert!(report.contains("HRESULT: 0x80070005"));
        assert!(report.contains("Standalone rollback: failed"));
        assert!(report.contains("Rollback Windows error: 5"));
        assert!(!report.contains("private native detail"));
    }
}
