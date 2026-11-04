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
        }
    }

    pub(crate) fn io(step: &'static str, error: io::Error) -> Self {
        let code = error.to_string();
        let mut failure = Self::new(step, safe_code(&code).unwrap_or("INSTALL_IO_ERROR"));
        failure.io_kind = Some(error.kind());
        failure.os_error = error.raw_os_error();
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
}
