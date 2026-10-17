//! Installer identity, private roots and actual standalone role-all startup.

use std::time::Duration;

use hiroute_daemon::{RoleAllConfig, RoleAllCpaConfig, start_role_all};
use hiroute_diagnostics::event::{
    DiagnosticEvent, StageOutcome, StartupEnd, StartupFailureCode, StartupOutcome, StartupStage,
};
use hiroute_diagnostics::runtime::DiagnosticRuntime;

use super::super::{
    Arguments, StartupError, classify_role_all_error, diagnostics_elapsed_ms, write_ready,
};
use super::ProtectedInputServer;

pub(crate) fn apply_standalone_defaults(arguments: &mut Arguments) -> Result<(), String> {
    if arguments.role.as_deref() != Some("all")
        || arguments.storage_root.is_some()
        || arguments.runtime_root.is_some()
        || arguments.listen.is_some()
        || arguments.lkg.is_some()
        || arguments.shutdown_fd.is_some()
        || arguments.capability_fd.is_some()
        || arguments.capability_ack_fd.is_some()
        || arguments.codex_desktop_engine.is_some()
    {
        return Err("standalone role=all arguments are invalid".into());
    }
    let layout = hiroute_host_runtime::StandaloneLayout::from_environment()
        .map_err(|error| error.to_string())?;
    #[cfg(windows)]
    if hiroute_host_runtime::standalone_maintenance_pending(&layout)
        .map_err(|_| "standalone maintenance state is unavailable")?
    {
        return Err("standalone installation maintenance is incomplete".into());
    }
    let installation = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(|_| "standalone installation marker is unavailable")?;
    if installation.target != hiroute_host_runtime::standalone_host_target()
        || !standalone_daemon_identity_matches(&installation.install_root)
            .map_err(|_| "standalone executable identity is unavailable")?
    {
        return Err("standalone installation identity does not match this daemon".into());
    }
    match (&arguments.cpa_binary, &arguments.cpa_sha256) {
        (None, None) => {
            arguments.cpa_binary = installation.cpa_binary.clone();
            arguments.cpa_sha256 = installation.cpa_sha256.clone();
        }
        (Some(binary), Some(digest))
            if Some(binary) == installation.cpa_binary.as_ref()
                && Some(digest) == installation.cpa_sha256.as_ref() => {}
        _ => return Err("standalone installation CPA identity does not match this daemon".into()),
    }
    #[cfg(target_os = "macos")]
    {
        let desktop_endpoint = layout
            .home
            .join("Library/Application Support/ai.hiroute.desktop/run/hiroute/control.sock");
        if [
            std::path::PathBuf::from("/Applications/HiRoute.app"),
            layout.home.join("Applications/HiRoute.app"),
        ]
        .iter()
        .any(|path| std::fs::symlink_metadata(path).is_ok())
            || std::fs::symlink_metadata(desktop_endpoint).is_ok()
        {
            return Err("HiRoute Desktop conflicts with standalone mode".into());
        }
    }
    standalone_private_directory(&layout.state_root)?;
    standalone_private_directory(&layout.storage_root())?;
    standalone_private_directory(&layout.runtime_root)?;
    standalone_private_directory(&layout.diagnostics_root())?;
    arguments.storage_root = Some(layout.storage_root());
    arguments.runtime_root = Some(layout.runtime_root.clone());
    arguments.lkg = Some(layout.gateway_lkg());
    let diagnostics_root = layout.diagnostics_root();
    if arguments
        .diagnostics_root
        .as_ref()
        .is_some_and(|root| root != &diagnostics_root)
    {
        return Err("standalone diagnostics root does not match the current user layout".into());
    }
    arguments.diagnostics_root = Some(diagnostics_root);
    Ok(())
}

fn standalone_daemon_identity_matches(install_root: &std::path::Path) -> std::io::Result<bool> {
    let executable = std::env::current_exe()?;
    let bundled = hiroute_host_runtime::standalone_daemon_binary(install_root);
    #[cfg(windows)]
    {
        use hiroute_platform::windows::security;
        // Reparse-free handles bind executable admission to actual native objects,
        // rather than resolving a replacement/junction path with canonicalize.
        let actual = security::open_identity(&executable, false)?;
        let expected = security::open_identity(&bundled, false)?;
        // The versioned image also backs the stable hirouted.exe hardlink. Native
        // object identity admits that installer contract without following reparses.
        Ok(security::file_identity(&actual)? == security::file_identity(&expected)?)
    }
    #[cfg(not(windows))]
    {
        Ok(std::fs::canonicalize(executable)? == std::fs::canonicalize(bundled)?)
    }
}

#[cfg(unix)]
fn standalone_private_directory(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| "standalone directory is unavailable")?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "standalone directory is unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("standalone directory is unsafe".into());
    }
    Ok(())
}

#[cfg(windows)]
fn standalone_private_directory(path: &std::path::Path) -> Result<(), String> {
    hiroute_platform::windows::security::ensure_private_directory(path)
        .map_err(|_| "standalone directory is unavailable")?;
    hiroute_platform::windows::security::open_owner_only(path, true)
        .map_err(|_| "standalone directory is unsafe")?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn standalone_private_directory(_path: &std::path::Path) -> Result<(), String> {
    Err("standalone role=all is not available on this platform".into())
}

#[cfg(any(unix, windows))]
pub(crate) fn run_all_standalone(
    arguments: Arguments,
    diagnostics: &std::sync::Arc<DiagnosticRuntime>,
    #[cfg(windows)] service: Option<hiroute_platform::windows::service::ServiceShutdown>,
) -> Result<(), StartupError> {
    diagnostics.stage_begin(StartupStage::RootValidate);
    let layout = hiroute_host_runtime::StandaloneLayout::from_environment()
        .map_err(|error| error.to_string())?;
    let (Some(storage_root), Some(runtime_root), Some(lkg)) = (
        arguments.storage_root,
        arguments.runtime_root,
        arguments.lkg,
    ) else {
        return Err("standalone roots are unavailable".into());
    };
    if storage_root != layout.storage_root()
        || runtime_root != layout.runtime_root
        || lkg != layout.gateway_lkg()
    {
        return Err("standalone roots do not match the current user layout".into());
    }
    let store = hiroute_host_runtime::GatewayListenerStore::new(layout.gateway_config_root());
    let reservation = store.reserve().map_err(|error| {
        StartupError::new(StartupFailureCode::InvalidConfiguration, error.to_string())
    })?;
    let listen = reservation.listen;
    diagnostics.stage_end(StartupStage::RootValidate, StageOutcome::Completed);
    let mut config = RoleAllConfig::new(storage_root, runtime_root, listen, lkg)
        .with_diagnostics(diagnostics.port())
        .with_released_commands_only();
    match (arguments.cpa_binary, arguments.cpa_sha256) {
        (Some(binary), Some(expected_sha256_hex)) => {
            config = config.with_cpa(RoleAllCpaConfig {
                binary,
                expected_sha256_hex,
            });
        }
        (None, None) => {}
        _ => return Err("standalone CPA arguments are incomplete".into()),
    }
    reservation.release();
    let mut role = match start_role_all(config) {
        Ok(role) => role,
        Err(error) => {
            let _ = store.mark_failed("GATEWAY_START_FAILED");
            return Err(StartupError::new(
                classify_role_all_error(&error),
                error.to_string(),
            ));
        }
    };
    let server = match ProtectedInputServer::bind(&layout) {
        Ok(server) => server,
        Err(error) => {
            role.shutdown();
            let _ = role.join(Duration::from_secs(30));
            let _ = store.mark_failed("PROTECTED_INPUT_UNAVAILABLE");
            return Err(StartupError::new(
                StartupFailureCode::ControlUnavailable,
                error,
            ));
        }
    };
    let admission = (|| {
        store.mark_applied(listen).map_err(|error| {
            StartupError::new(StartupFailureCode::StorageUnavailable, error.to_string())
        })?;
        write_ready(&role, diagnostics)
            .map_err(|error| StartupError::new(StartupFailureCode::ReadyChannelFailed, error))?;
        #[cfg(windows)]
        if let Some(service) = service.as_ref() {
            service.mark_running().map_err(|error| {
                StartupError::new(StartupFailureCode::Internal, error.to_string())
            })?;
        }
        Ok(())
    })();
    if let Err(error) = finish_running_admission(&mut role, admission) {
        let _ = store.mark_failed("STARTUP_ADMISSION_FAILED");
        return Err(error);
    }
    diagnostics.emit(DiagnosticEvent::StartupEnd(StartupEnd {
        outcome: StartupOutcome::Success,
        elapsed_ms: diagnostics_elapsed_ms(diagnostics),
    }));
    let assets = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map(|installation| installation.install_root.join("console"))
        .unwrap_or_default();
    #[cfg(debug_assertions)]
    let assets = arguments.console_assets.unwrap_or(assets);
    let console = hiroute_daemon::console::ConsoleService::start(
        &role,
        layout.clone(),
        assets,
        Some(diagnostics.clone()),
    )
    .ok();
    server
        .run(
            role,
            console,
            #[cfg(windows)]
            service,
        )
        .map_err(|error| StartupError::new(StartupFailureCode::Internal, error))
}

/// Keep the primary admission error while draining the started production role.
/// A cleanup failure cannot turn a failed SCM status report into success or
/// obscure its original diagnosis. Resident/child Jobs remain held until their
/// owners finish cleanup or the failing daemon exits.
fn finish_running_admission(
    role: &mut hiroute_daemon::RoleAllHandle,
    admission: Result<(), StartupError>,
) -> Result<(), StartupError> {
    if admission.is_err() {
        role.shutdown();
        let _ = role.join(Duration::from_secs(30));
    }
    admission
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_reporting_failure_drains_actual_role_and_preserves_the_primary_error() {
        let builder = tempfile::Builder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::PermissionsExt;
            let mut builder = builder;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
            builder
        };
        let cleanup = builder.tempdir().unwrap();
        #[cfg(not(windows))]
        let root = cleanup.path().to_path_buf();
        #[cfg(windows)]
        let root = {
            let root = cleanup.path().join("private");
            drop(hiroute_platform::windows::security::create_owner_only_directory(&root).unwrap());
            root
        };
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let listen = reservation.local_addr().unwrap();
        drop(reservation);
        let mut role = start_role_all(RoleAllConfig::new(
            root.join("storage"),
            root.join("runtime"),
            listen,
            root.join("gateway-lkg.json"),
        ))
        .unwrap();
        assert_eq!(
            role.phases(),
            (
                hiroute_daemon::control::ManagedControlPhase::Ready,
                hiroute_gateway::server::ManagedGatewayPhase::Ready
            )
        );
        let native = std::io::Error::from_raw_os_error(6);
        let message = native.to_string();
        let primary = StartupError::new(StartupFailureCode::Internal, &message);
        let error = finish_running_admission(&mut role, Err(primary))
            .err()
            .unwrap();
        assert_eq!(error.code, StartupFailureCode::Internal);
        assert_eq!(error.message, message);
        assert_eq!(
            role.phases(),
            (
                hiroute_daemon::control::ManagedControlPhase::Terminated,
                hiroute_gateway::server::ManagedGatewayPhase::Terminated
            )
        );
        assert!(
            std::net::TcpStream::connect_timeout(&listen, Duration::from_millis(100)).is_err(),
            "failed startup report left the real Gateway listener serving"
        );
        assert!(
            std::net::TcpListener::bind(listen).is_ok(),
            "failed startup report retained the Gateway port"
        );
    }
}
