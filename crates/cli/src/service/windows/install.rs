use hiroute_host_runtime::standalone_install::windows::{
    self, InstallFailure, InstallRequest, UninstallRequest,
};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
};

fn native_error(error: &io::Error) -> serde_json::Value {
    serde_json::json!({
        "io_kind": format!("{:?}", error.kind()),
        "win32_error": error.raw_os_error(),
        "hresult": hiroute_platform::windows::background::hresult(error),
    })
}

pub(crate) fn run(verb: &str, arguments: &[OsString]) -> u8 {
    let at = InstallFailure::at;
    let execute = || -> Result<serde_json::Value, InstallFailure> {
        if arguments != [OsString::from("--request-stdin")] {
            return Err(at("validate-request")(io::ErrorKind::InvalidInput.into()));
        }
        let mut bytes = Vec::new();
        io::stdin()
            .lock()
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(at("read-request"))?;
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            return Err(at("validate-request")(io::ErrorKind::InvalidInput.into()));
        }
        match verb {
            "__windows-install" => windows::install(
                serde_json::from_slice::<InstallRequest>(&bytes)
                    .map_err(|_| at("decode-request")(io::ErrorKind::InvalidData.into()))?,
            ),
            "__windows-uninstall" => windows::uninstall(
                serde_json::from_slice::<UninstallRequest>(&bytes)
                    .map_err(|_| at("decode-request")(io::ErrorKind::InvalidData.into()))?,
                super::super::restore_agents::restore,
            )
            .map_err(at("remove-installation")),
            _ => Err(at("validate-request")(io::ErrorKind::InvalidInput.into())),
        }
    };
    let (exit, value) = match execute() {
        Ok(value) => (0, value),
        Err(failure) => {
            let text = failure.error.to_string();
            let agents = text
                .strip_prefix("AGENT_RESTORE_REQUIRED:")
                .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok());
            let code = if agents.is_some() {
                "AGENT_RESTORE_REQUIRED"
            } else {
                [
                    "DRAIN_UNSUPPORTED",
                    "UPGRADE_ACTIVE_WORK",
                    "INSTALL_SCM_MANAGED",
                    "INSTALL_ARCHITECTURE_UNSUPPORTED",
                    "INSTALL_MAINTENANCE_BUSY",
                    "INSTALL_OWNERSHIP_CONFLICT",
                    "AGENT_CONTROL_UNAVAILABLE",
                    "AGENT_RESTORE_DATA_REQUIRED",
                    "INSTALL_PACKAGE_IN_USE",
                ]
                .into_iter()
                .find(|code| text == *code)
                .unwrap_or("INSTALL_FAILED")
            };
            (
                1,
                serde_json::json!({
                    "schema": "hiroute.windows-maintenance-error/v1",
                    "error": {
                        "code": code,
                        "agents": agents,
                        "message": if code == "INSTALL_SCM_MANAGED" {
                            Some("SCM service installation is not offered in this release. The existing installation was left unchanged.")
                        } else {
                            None
                        },
                        "detail": {
                            "step": failure.step,
                            "native": native_error(&failure.error),
                            "rollback": failure.rollback_error.as_ref().map(native_error),
                        }
                    }
                }),
            )
        }
    };
    if writeln!(io::stdout().lock(), "{}", value).is_err() {
        return 1;
    }
    exit
}
