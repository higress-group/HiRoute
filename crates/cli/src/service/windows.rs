use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hiroute_host_runtime::{StandaloneLayout, standalone_daemon_binary};
use hiroute_platform::windows::{
    background::{self, Definition, Manager},
    lifecycle, service, task,
};
use serde_json::json;
use zeroize::Zeroizing;

use super::ServiceFailure;

mod install;
mod logs;
mod scm_owner;
pub(crate) fn install_hidden(verb: &str, args: &[std::ffi::OsString]) -> u8 {
    install::run(verb, args)
}
pub(crate) fn scm_owner_helper_verb() -> &'static str {
    scm_owner::VERB
}
pub(crate) fn scm_owner_helper(args: &[std::ffi::OsString]) -> u8 {
    scm_owner::run_hidden(args)
}

pub(crate) const FINALIZE_VERB: &str = "__windows-background-finalize";

/// Internal installer protocol. No published command inventory or owner-SID /
/// pathname input: the native profile, owner marker and real SCM supply identity.
pub(crate) fn finalize_hidden(arguments: &[std::ffi::OsString]) -> u8 {
    use std::io::Write;
    if arguments != [std::ffi::OsString::from("--request-stdin")] {
        return 2;
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        schema: String,
        operation: String,
    }
    let run = || -> Result<(), ServiceFailure> {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take(1025)
            .read_to_end(&mut bytes)
            .map_err(failure)?;
        if bytes.is_empty() || bytes.len() > 1024 {
            return Err(ServiceFailure::Unavailable);
        }
        let request: Request = serde_json::from_slice(&bytes).map_err(failure)?;
        if request.schema != "hiroute.windows-owner-finalize-request/v1" {
            return Err(ServiceFailure::Unavailable);
        }
        let (layout, record) = super::installed()?;
        if record.service_definition != definition_path(&layout)
            || !hiroute_host_runtime::standalone_maintenance_pending(&layout).map_err(failure)?
        {
            return Err(ServiceFailure::Unavailable);
        }
        let value = match request.operation.as_str() {
            "install" => {
                let value =
                    Definition::new(Manager::Scm, standalone_daemon_binary(&record.install_root))
                        .map_err(failure)?;
                service::verify_registration(&value).map_err(failure)?;
                background::write_definition(&record.service_definition, &value)
                    .map_err(failure)?;
                value
            }
            "uninstall" => {
                let value = definition(&layout)?;
                if value.manager != Manager::Scm
                    || !service::registration_absent(&value).map_err(failure)?
                {
                    return Err(ServiceFailure::Unavailable);
                }
                value
            }
            _ => return Err(ServiceFailure::Unavailable),
        };
        let response = serde_json::to_vec(&json!({"schema":"hiroute.windows-owner-finalize-result/v1","operation":request.operation,"definition":value,"completed":true})).map_err(failure)?;
        let mut output = std::io::stdout().lock();
        output.write_all(&response).map_err(failure)?;
        output.write_all(b"\n").map_err(failure)?;
        output.flush().map_err(failure)
    };
    if run().is_ok() {
        0
    } else {
        let _ = std::io::stderr().write_all(b"hiroute Windows owner finalization unavailable\n");
        6
    }
}

fn failure(_: impl std::fmt::Display) -> ServiceFailure {
    ServiceFailure::Unavailable
}
pub(super) fn definition_path(layout: &StandaloneLayout) -> PathBuf {
    layout.data_root.join("service/windows-background.json")
}

fn definition(layout: &StandaloneLayout) -> Result<Definition, ServiceFailure> {
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(failure)?;
    if record.service_definition != definition_path(layout)
        || record.target != hiroute_host_runtime::standalone_host_target()
    {
        return Err(ServiceFailure::Unavailable);
    }
    let value = background::read_definition(&record.service_definition).map_err(failure)?;
    if value.daemon != standalone_daemon_binary(&record.install_root) {
        return Err(ServiceFailure::Unavailable);
    }
    Ok(value)
}

pub(super) fn manager_name(layout: &StandaloneLayout) -> Result<&'static str, ServiceFailure> {
    Ok(match definition(layout)?.manager {
        Manager::Task => "windows-task-user",
        Manager::Scm => "windows-scm-user",
    })
}
pub(super) fn active(layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    let value = definition(layout)?;
    match value.manager {
        Manager::Task => task::active(&value),
        Manager::Scm => service::active(&value),
    }
    .map_err(failure)
}
pub(super) fn autostart(layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    let value = definition(layout)?;
    match value.manager {
        Manager::Task => task::autostart(&value),
        Manager::Scm => service::autostart(&value),
    }
    .map_err(failure)
}
pub(super) fn set_autostart(
    layout: &StandaloneLayout,
    enabled: bool,
) -> Result<(), ServiceFailure> {
    let value = definition(layout)?;
    match value.manager {
        Manager::Task => task::set_autostart(&value, enabled),
        Manager::Scm => service::set_autostart(&value, enabled),
    }
    .map_err(failure)
}
pub(super) fn start(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    let value = definition(layout)?;
    match value.manager {
        Manager::Task => task::start(&value),
        Manager::Scm => service::start(&value),
    }
    .map_err(failure)
}
pub(super) fn stop(layout: &StandaloneLayout, timeout: Duration) -> Result<(), ServiceFailure> {
    let deadline = Instant::now() + timeout;
    let value = definition(layout)?;
    let endpoint = layout.lifecycle_endpoint();
    if value.manager == Manager::Scm {
        if !service::active(&value).map_err(failure)? {
            return Ok(());
        }
        let process = match lifecycle::retain_for_stop(
            &endpoint,
            &value.daemon,
            timeout.min(Duration::from_secs(2)),
        ) {
            Ok(process) => Some(process),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(error) => return Err(failure(error)),
        };
        // Mark the trusted host's stop intent before touching the business child.
        // A forced child exit then completes a requested stop with exit code 0,
        // rather than scheduling SCM's unexpected-failure recovery restart.
        service::request_stop(&value).map_err(failure)?;
        let force_at = deadline
            .checked_sub(Duration::from_secs(2))
            .unwrap_or(deadline);
        let mut forced = false;
        while Instant::now() < deadline {
            if !service::active(&value).map_err(failure)? {
                return Ok(());
            }
            if !forced && Instant::now() >= force_at {
                if let Some(process) = &process {
                    if !process.exited().map_err(failure)? {
                        process.terminate().map_err(failure)?;
                    }
                }
                forced = true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        return Err(ServiceFailure::Unavailable);
    }
    match lifecycle::stop_with_fallback(&endpoint, &value.daemon, timeout, || {
        task::force_stop(&value)
    }) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::TimedOut
            ) =>
        {
            if !active(layout)? {
                return Ok(());
            }
            // The task may still be starting, or may fail to acknowledge stop.
            // Only its exact verified registration authorizes this fallback.
            match value.manager {
                Manager::Task => task::force_stop(&value),
                Manager::Scm => service::request_stop(&value),
            }
            .map_err(failure)?;
        }
        Err(error) => return Err(failure(error)),
    }
    while Instant::now() < deadline {
        if !active(layout)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(ServiceFailure::Unavailable)
}
pub(super) fn restart(layout: &StandaloneLayout, timeout: Duration) -> Result<(), ServiceFailure> {
    let deadline = Instant::now() + timeout;
    stop(layout, timeout)?;
    if Instant::now() >= deadline {
        return Err(ServiceFailure::Unavailable);
    }
    start(layout)
}

pub(super) fn logs(layout: &StandaloneLayout) -> Result<serde_json::Value, ServiceFailure> {
    logs::read(layout, manager_name(layout)?)
}

pub(super) fn install(
    manager: &str,
    password_stdin: bool,
) -> Result<serde_json::Value, ServiceFailure> {
    let layout = StandaloneLayout::from_environment().map_err(failure)?;
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(failure)?;
    if record.service_definition != definition_path(&layout)
        || record.target != hiroute_host_runtime::standalone_host_target()
    {
        return Err(ServiceFailure::Unavailable);
    }
    let manager = match manager {
        "task" if !password_stdin => Manager::Task,
        "scm" if password_stdin => Manager::Scm,
        _ => return Err(ServiceFailure::Unavailable),
    };
    let value = Definition::new(manager, standalone_daemon_binary(&record.install_root))
        .map_err(failure)?;
    let previous = if record.service_definition.try_exists().map_err(failure)? {
        Some(background::read_definition(&record.service_definition).map_err(failure)?)
    } else {
        None
    };
    if let Some(previous) = &previous {
        if previous.manager != manager {
            return Err(ServiceFailure::Unavailable);
        }
        let active = match manager {
            Manager::Task => task::active(previous).or_else(|_| task::active(&value)),
            Manager::Scm => service::active(previous).or_else(|_| service::active(&value)),
        }
        .map_err(failure)?;
        if active {
            return Err(ServiceFailure::Unavailable);
        }
    }
    match manager {
        Manager::Task => task::register(&value, previous.as_ref()).map_err(failure)?,
        Manager::Scm => {
            let password = read_password()?;
            service::register(&value, previous.as_ref(), &password).map_err(failure)?;
        }
    }
    background::write_definition(&record.service_definition, &value).map_err(failure)?;
    Ok(
        json!({"schema":"hiroute.windows-background-install/v1", "manager":manager_name(&layout)?, "name":value.name, "owner_sid":value.owner_sid, "autostart_enabled":autostart(&layout)?, "started":false}),
    )
}

fn read_password() -> Result<Zeroizing<String>, ServiceFailure> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::stdin()
        .lock()
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(failure)?;
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(ServiceFailure::Unavailable);
    }
    let text = std::str::from_utf8(&bytes).map_err(failure)?;
    let text = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .strip_suffix('\r')
        .unwrap_or(text.strip_suffix('\n').unwrap_or(text));
    if text.is_empty() || text.contains(['\0', '\n', '\r']) {
        return Err(ServiceFailure::Unavailable);
    }
    Ok(Zeroizing::new(text.to_owned()))
}

pub(super) fn uninstall(timeout: Duration) -> Result<serde_json::Value, ServiceFailure> {
    let (layout, _) = super::installed()?;
    let value = definition(&layout)?;
    stop(&layout, timeout)?;
    match value.manager {
        Manager::Task => task::unregister(&value),
        Manager::Scm => service::unregister(&value, timeout),
    }
    .map_err(failure)?;
    // Preserve install marker/data until manager deletion is confirmed. This
    // definition is removed by the installer's journalled maintenance commit.
    Ok(json!({"schema":"hiroute.windows-background-install/v1", "removed":true,"name":value.name}))
}

pub(super) fn publish_record(bytes: &[u8]) -> Result<serde_json::Value, ServiceFailure> {
    let record: hiroute_host_runtime::StandaloneInstallRecordV1 =
        serde_json::from_slice(bytes).map_err(failure)?;
    record.validate().map_err(failure)?;
    let layout = StandaloneLayout::from_environment().map_err(failure)?;
    if record.target != hiroute_host_runtime::standalone_host_target()
        || record.service_definition != definition_path(&layout)
        || !record
            .install_root
            .starts_with(layout.home.join(".local/lib/hiroute"))
    {
        return Err(ServiceFailure::Unavailable);
    }
    for path in [
        hiroute_host_runtime::standalone_cli_binary(&record.install_root),
        standalone_daemon_binary(&record.install_root),
    ] {
        hiroute_platform::windows::security::open_identity(&path, false).map_err(failure)?;
    }
    background::write_private_record(&layout.marker_path, bytes).map_err(failure)?;
    Ok(json!({"schema":"hiroute.windows-background-install/v1","marker_written":true}))
}

pub(super) fn private_record(path: &Path, bytes: &[u8]) -> Result<(), ServiceFailure> {
    let layout = StandaloneLayout::from_environment().map_err(failure)?;
    if !path.starts_with(&layout.data_root) {
        return Err(ServiceFailure::Unavailable);
    }
    background::write_private_record(path, bytes).map_err(failure)
}
