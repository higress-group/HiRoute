#![forbid(unsafe_code)]
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use hiroute_application_api::{CanonicalDigest, PrincipalKind, RevisionSetV1};
use hiroute_daemon::{
    RoleAllConfig, RoleAllCpaConfig, RoleAllError, serve_production_control, start_role_all,
};
use hiroute_diagnostics::event::{
    DiagnosticEvent, ReadyDirection, ReadyIo, ReadyIoError, StageOutcome, StartupEnd,
    StartupFailureCode, StartupOutcome, StartupStage,
};
use hiroute_diagnostics::identity::SessionId;
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::record::Component;
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
use hiroute_domain::WorkspaceId;
use hiroute_local_storage::ApplyCapabilityRegistrationV1;
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

mod protected_exchange;
mod ready;
#[cfg(any(unix, windows))]
mod standalone;
mod startup_recovery;
mod upgrade_preflight;
#[cfg(windows)]
mod windows_task;
use ready::{write_ready_frame, write_startup_failure_frame};
#[cfg(any(unix, windows))]
use standalone::{apply_standalone_defaults, run_all_standalone};

const MAX_PROTECTED_FRAME_BYTES: usize = 64 * 1024;
const PROTECTED_FRAME_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Arguments {
    role: Option<String>,
    standalone: bool,
    #[cfg(debug_assertions)]
    console_assets: Option<PathBuf>,
    #[cfg(windows)]
    windows_service: Option<String>,
    #[cfg(windows)]
    foreground_parent_console: bool,
    #[cfg(windows)]
    windows_task_runtime: bool,
    storage_root: Option<PathBuf>,
    runtime_root: Option<PathBuf>,
    listen: Option<SocketAddr>,
    lkg: Option<PathBuf>,
    shutdown_fd: Option<i32>,
    capability_fd: Option<i32>,
    capability_ack_fd: Option<i32>,
    cpa_binary: Option<PathBuf>,
    cpa_sha256: Option<String>,
    codex_desktop_engine: Option<PathBuf>,
    diagnostics_root: Option<PathBuf>,
    diagnostics_parent_session: Option<String>,
    diagnostic_level_override: Option<String>,
}

fn main() {
    if let Some(code) = upgrade_preflight::run_if_requested() {
        std::process::exit(code);
    }
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("app-server"))
        && std::env::var_os("HIROUTE_NATIVE_LAUNCH_DESCRIPTOR").is_some()
    {
        if hiroute_platform::windows::security::initialize_current_process_private_defaults()
            .is_err()
        {
            eprintln!("hirouted private process defaults unavailable");
            std::process::exit(6);
        }
        match hiroute_platform::windows::launcher::run_from_environment() {
            Ok(code) => std::process::exit(code),
            Err(_) => {
                eprintln!("hirouted native app-server launcher failed");
                std::process::exit(1);
            }
        }
    }
    let mut arguments = parse_arguments();
    #[cfg(windows)]
    if arguments.windows_task_runtime
        && (!arguments.standalone
            || arguments.role.as_deref() != Some("all")
            || arguments.windows_service.is_some()
            || arguments.foreground_parent_console)
    {
        exit_usage();
    }
    #[cfg(windows)]
    if arguments.foreground_parent_console
        && (!arguments.standalone
            || arguments.role.as_deref() != Some("all")
            || arguments.windows_service.is_some())
    {
        exit_usage();
    }
    #[cfg(windows)]
    if let Some(name) = arguments.windows_service.take() {
        if !arguments.standalone
            || arguments.role.as_deref() != Some("all")
            || arguments.storage_root.is_some()
            || arguments.runtime_root.is_some()
            || arguments.listen.is_some()
            || arguments.lkg.is_some()
            || arguments.shutdown_fd.is_some()
            || arguments.capability_fd.is_some()
            || arguments.capability_ack_fd.is_some()
            || arguments.cpa_binary.is_some()
            || arguments.cpa_sha256.is_some()
            || arguments.codex_desktop_engine.is_some()
            || arguments.diagnostics_root.is_some()
            || arguments.diagnostics_parent_session.is_some()
            || arguments.diagnostic_level_override.is_some()
        {
            exit_usage();
        }
        if let Err(error) = hiroute_platform::windows::service::run_service(&name, |_| {
            // The sealed, privileged SCM host may only dispatch its protected
            // descriptor to an owner-SID restricted child. Never run business here.
            Err("Windows service host cannot execute the business runtime".into())
        }) {
            eprintln!("hirouted failed: {error}");
            std::process::exit(6);
        }
        return;
    }
    #[cfg(windows)]
    if hiroute_platform::windows::security::initialize_current_process_private_defaults().is_err() {
        eprintln!("hirouted private process defaults unavailable");
        std::process::exit(6);
    }
    #[cfg(windows)]
    if arguments.standalone
        && arguments.role.as_deref() == Some("all")
        && !arguments.windows_task_runtime
        && !arguments.foreground_parent_console
        && std::env::args_os()
            .skip(1)
            .eq(["--role", "all", "--standalone"]
                .into_iter()
                .map(std::ffi::OsString::from))
    {
        match windows_task::run_if_installed_task() {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                eprintln!("hirouted failed: {error}");
                std::process::exit(6);
            }
        }
    }
    if arguments.standalone
        && let Err(error) = apply_standalone_defaults(&mut arguments)
    {
        eprintln!("hirouted failed: {error}");
        std::process::exit(6);
    }
    #[cfg(windows)]
    if arguments.foreground_parent_console
        && hiroute_platform::windows::console::attach_parent_preserving_stdio().is_err()
    {
        eprintln!("hirouted foreground parent console unavailable");
        std::process::exit(6);
    }
    if let Err(error) = run_daemon(
        arguments,
        #[cfg(windows)]
        None,
    ) {
        eprintln!("hirouted failed: {error}");
        std::process::exit(6);
    }
}

fn run_daemon(
    arguments: Arguments,
    #[cfg(windows)] service: Option<hiroute_platform::windows::service::ServiceShutdown>,
) -> Result<(), String> {
    #[cfg(windows)]
    hiroute_platform::windows::job::initialize_resident()
        .map_err(|_| "daemon process-tree lifetime unavailable")?;
    let managed_role = arguments.role.as_deref() == Some("all");
    let level_override = match arguments.diagnostic_level_override.as_deref() {
        Some(value) => match value.parse::<DiagnosticLevel>() {
            Ok(level) => Some(level),
            Err(_) => exit_usage(),
        },
        None => None,
    };
    let parent_session = match arguments.diagnostics_parent_session.as_deref() {
        Some(value) => match SessionId::parse(value) {
            Ok(session) => Some(session),
            Err(_) => exit_usage(),
        },
        None => None,
    };
    // Diagnostics start as soon as the root is known and never wait for business readiness.
    let diagnostics = std::sync::Arc::new(DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root(&arguments),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: parent_session,
        level_override,
    }));
    hiroute_diagnostics::panic::install_panic_hook(diagnostics.handle().clone());
    let recovery_root = arguments.storage_root.clone();
    let result = match arguments.role.as_deref() {
        Some("control") => run_control(arguments, &diagnostics),
        Some("all") => run_all(
            arguments,
            &diagnostics,
            #[cfg(windows)]
            service,
        ),
        _ => exit_usage(),
    };
    if let Err(error) = result {
        diagnostics.emit(DiagnosticEvent::StartupEnd(StartupEnd {
            outcome: StartupOutcome::Failure { code: error.code },
            elapsed_ms: diagnostics_elapsed_ms(&diagnostics),
        }));
        if managed_role {
            let _ = write_startup_failure_frame(error.code);
        }
        if let Some(recovery) = startup_recovery::failure_report(
            error.code,
            recovery_root.as_deref(),
            error.recovery.as_deref(),
        ) {
            eprintln!("{}", recovery.operator_guidance());
        }
        diagnostics.shutdown();
        return Err(error.message);
    }
    diagnostics.shutdown();
    Ok(())
}

/// One stable startup failure: a fixed code plus a local diagnostic message that is never
/// written to the diagnostic files.
struct StartupError {
    code: StartupFailureCode,
    message: String,
    recovery: Option<Box<hiroute_host_runtime::StorageRecovery>>,
}

impl From<String> for StartupError {
    fn from(message: String) -> Self {
        StartupError::new(StartupFailureCode::InvalidConfiguration, message)
    }
}

impl From<&str> for StartupError {
    fn from(message: &str) -> Self {
        StartupError::new(StartupFailureCode::InvalidConfiguration, message.to_owned())
    }
}

impl StartupError {
    fn new(code: StartupFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            recovery: None,
        }
    }
}

fn diagnostics_root(arguments: &Arguments) -> PathBuf {
    match &arguments.diagnostics_root {
        Some(root) if root.is_absolute() => root.clone(),
        Some(_) => exit_usage(),
        None => match &arguments.storage_root {
            Some(storage) => storage.join("diagnostics"),
            None => exit_usage(),
        },
    }
}

fn diagnostics_elapsed_ms(diagnostics: &DiagnosticRuntime) -> u64 {
    diagnostics.started_elapsed_ms()
}

fn classify_role_all_error(error: &RoleAllError) -> StartupFailureCode {
    match error {
        RoleAllError::StorageRecovery(recovery) => match recovery.cause {
            hiroute_host_runtime::StorageRecoveryCause::AccessDenied => {
                StartupFailureCode::StorageAccessDenied
            }
            hiroute_host_runtime::StorageRecoveryCause::Locked => StartupFailureCode::StorageLocked,
            hiroute_host_runtime::StorageRecoveryCause::MissingKey => {
                StartupFailureCode::StorageKeyUnavailable
            }
            hiroute_host_runtime::StorageRecoveryCause::UnsupportedFormat => {
                StartupFailureCode::UpgradeSourceUnsupported
            }
            hiroute_host_runtime::StorageRecoveryCause::UnknownState => {
                StartupFailureCode::StorageUnavailable
            }
            _ => StartupFailureCode::UpgradeStorageFailed,
        },
        RoleAllError::InvalidConfiguration => StartupFailureCode::InvalidConfiguration,
        RoleAllError::StandaloneProxyUnavailable => StartupFailureCode::DependencyUnavailable,
        RoleAllError::JoinTimeout(_) => StartupFailureCode::Internal,
        RoleAllError::Component(component, detail) => match *component {
            "Storage" if detail.starts_with("UPGRADE_SOURCE_UNSUPPORTED") => {
                StartupFailureCode::UpgradeSourceUnsupported
            }

            "Storage" if detail.starts_with("stable storage upgrade failed") => {
                StartupFailureCode::UpgradeStorageFailed
            }
            "Storage" | "Gateway publication" | "Runtime state" => {
                StartupFailureCode::StorageUnavailable
            }
            "Release facts" => StartupFailureCode::ReleaseFactsInvalid,
            "Gateway" => StartupFailureCode::GatewayUnavailable,
            "Local Control" => StartupFailureCode::ControlUnavailable,
            "CPA" => StartupFailureCode::DependencyUnavailable,
            _ => StartupFailureCode::Internal,
        },
    }
}

fn run_control(arguments: Arguments, diagnostics: &DiagnosticRuntime) -> Result<(), StartupError> {
    diagnostics.stage_begin(StartupStage::RootValidate);
    let (Some(storage_root), Some(runtime_root)) = (arguments.storage_root, arguments.runtime_root)
    else {
        exit_usage();
    };
    if arguments.listen.is_some()
        || arguments.lkg.is_some()
        || arguments.shutdown_fd.is_some()
        || arguments.capability_fd.is_some()
        || arguments.capability_ack_fd.is_some()
        || arguments.cpa_binary.is_some()
        || arguments.cpa_sha256.is_some()
        || arguments.codex_desktop_engine.is_some()
        || arguments.standalone
    {
        exit_usage();
    }
    diagnostics.stage_end(StartupStage::RootValidate, StageOutcome::Completed);
    serve_production_control(storage_root, runtime_root, diagnostics)
        .map_err(|error| StartupError::new(StartupFailureCode::ControlUnavailable, error))
}

fn run_all(
    arguments: Arguments,
    diagnostics: &std::sync::Arc<DiagnosticRuntime>,
    #[cfg(windows)] service: Option<hiroute_platform::windows::service::ServiceShutdown>,
) -> Result<(), StartupError> {
    if arguments.standalone {
        #[cfg(any(unix, windows))]
        return run_all_standalone(
            arguments,
            diagnostics,
            #[cfg(windows)]
            service,
        );
        #[cfg(not(any(unix, windows)))]
        return Err("standalone role=all is not available on this platform".into());
    }
    diagnostics.stage_begin(StartupStage::RootValidate);
    let (
        Some(storage_root),
        Some(runtime_root),
        Some(listen),
        Some(lkg),
        Some(shutdown_fd),
        Some(capability_fd),
    ) = (
        arguments.storage_root,
        arguments.runtime_root,
        arguments.listen,
        arguments.lkg,
        arguments.shutdown_fd,
        arguments.capability_fd,
    )
    else {
        exit_usage();
    };
    if shutdown_fd == capability_fd {
        return Err(StartupError::new(
            StartupFailureCode::InvalidConfiguration,
            "shutdown and capability descriptors must be distinct",
        ));
    }
    if arguments
        .capability_ack_fd
        .is_some_and(|fd| fd == shutdown_fd || fd == capability_fd)
    {
        return Err(StartupError::new(
            StartupFailureCode::InvalidConfiguration,
            "protected channel descriptors must be distinct",
        ));
    }
    let mut acknowledgements = arguments
        .capability_ack_fd
        .map(protected_inherited_writer)
        .transpose()?;
    let mut shutdown = ProtectedReader::new(protected_inherited_reader(shutdown_fd)?)?;
    let mut capabilities = ProtectedFrameReader::new(protected_inherited_reader(capability_fd)?)?;
    diagnostics.stage_end(StartupStage::RootValidate, StageOutcome::Completed);
    #[cfg(unix)]
    let protected_input_path = hiroute_host_runtime::protected_input_socket(&runtime_root);
    let mut config = RoleAllConfig::new(storage_root, runtime_root, listen, lkg)
        .with_diagnostics(diagnostics.port());
    match (arguments.cpa_binary, arguments.cpa_sha256) {
        (Some(binary), Some(expected_sha256_hex)) => {
            config = config.with_cpa(RoleAllCpaConfig {
                binary,
                expected_sha256_hex,
            });
        }
        (None, None) => {}
        _ => exit_usage(),
    }
    if let Some(engine) = arguments.codex_desktop_engine {
        config = config.with_codex_desktop_engine(engine);
    }
    let (progress, phases) = std::sync::mpsc::channel();
    config.upgrade_progress = Some(progress.into());
    let progress_writer = std::thread::spawn(move || -> Result<(), String> {
        let mut stdout = std::io::stdout().lock();
        for phase in phases {
            serde_json::to_writer(
                &mut stdout,
                &serde_json::json!({
                    "schema":"hiroute.daemon-upgrade-progress/v1", "phase":phase
                }),
            )
            .map_err(|_| "upgrade progress write failed")?;
            stdout
                .write_all(b"\n")
                .and_then(|_| stdout.flush())
                .map_err(|_| "upgrade progress write failed")?;
        }
        Ok(())
    });
    let result = start_role_all(config);
    // Drain progress before writing ready/failure. No second listener or business writer is
    // started to report migration, and frames cannot be interleaved with readiness.
    progress_writer
        .join()
        .map_err(|_| {
            StartupError::new(
                StartupFailureCode::ReadyChannelFailed,
                "upgrade progress writer stopped",
            )
        })?
        .map_err(|error| StartupError::new(StartupFailureCode::ReadyChannelFailed, error))?;
    let mut role = result.map_err(startup_recovery::role_failure)?;
    #[cfg(unix)]
    let protected_inputs = match standalone::ProtectedInputServer::bind_path(protected_input_path) {
        Ok(server) => server,
        Err(error) => {
            role.shutdown();
            let _ = role.join(Duration::from_secs(30));
            return Err(StartupError::new(
                StartupFailureCode::ControlUnavailable,
                error,
            ));
        }
    };
    write_ready(&role, diagnostics)
        .map_err(|error| StartupError::new(StartupFailureCode::ReadyChannelFailed, error))?;
    diagnostics.emit(DiagnosticEvent::StartupEnd(StartupEnd {
        outcome: StartupOutcome::Success,
        elapsed_ms: diagnostics_elapsed_ms(diagnostics),
    }));
    loop {
        if shutdown.poll_eof()? {
            break;
        }
        #[cfg(unix)]
        protected_inputs.poll(&role)?;
        for frame in capabilities.read_available()? {
            if acknowledgements.is_none() {
                return Err(
                    "v2 capability registration requires a protected acknowledgement channel"
                        .into(),
                );
            }
            protected_exchange::process(
                &role,
                frame,
                acknowledgements.as_mut().expect("checked channel"),
                diagnostics,
            )?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    role.shutdown();
    role.join(Duration::from_secs(30))
        .map_err(|error| StartupError::new(StartupFailureCode::Internal, error.to_string()))
}

fn parse_arguments() -> Arguments {
    let mut parsed = Arguments::default();
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let Some(name) = argument.to_str() else {
            exit_usage();
        };
        match name {
            "--role" => parsed.role = value(&mut arguments),
            "--standalone" => parsed.standalone = true,
            #[cfg(debug_assertions)]
            "--console-assets" => parsed.console_assets = value(&mut arguments).map(PathBuf::from),
            #[cfg(windows)]
            "--windows-service" => {
                parsed.windows_service = Some(value(&mut arguments).unwrap_or_else(|| exit_usage()))
            }
            #[cfg(windows)]
            "--foreground-parent-console" => parsed.foreground_parent_console = true,
            #[cfg(windows)]
            "--windows-task-runtime" => parsed.windows_task_runtime = true,
            "--storage-root" => parsed.storage_root = value(&mut arguments).map(PathBuf::from),
            "--runtime-root" => parsed.runtime_root = value(&mut arguments).map(PathBuf::from),
            "--listen" => {
                parsed.listen = value(&mut arguments).and_then(|value| value.parse().ok())
            }
            "--lkg" => parsed.lkg = value(&mut arguments).map(PathBuf::from),
            "--shutdown-fd" => {
                parsed.shutdown_fd = value(&mut arguments).and_then(|value| value.parse().ok())
            }
            "--capability-fd" => {
                parsed.capability_fd = value(&mut arguments).and_then(|value| value.parse().ok())
            }
            "--capability-ack-fd" => {
                parsed.capability_ack_fd =
                    value(&mut arguments).and_then(|value| value.parse().ok())
            }
            "--cpa-binary" => parsed.cpa_binary = value(&mut arguments).map(PathBuf::from),
            "--cpa-sha256" => parsed.cpa_sha256 = value(&mut arguments),
            "--codex-desktop-engine" => {
                parsed.codex_desktop_engine = value(&mut arguments).map(PathBuf::from)
            }
            "--diagnostics-root" => {
                parsed.diagnostics_root = value(&mut arguments).map(PathBuf::from)
            }
            "--diagnostics-parent-session" => {
                parsed.diagnostics_parent_session = value(&mut arguments)
            }
            "--diagnostic-level-override" => {
                parsed.diagnostic_level_override = value(&mut arguments)
            }
            _ => exit_usage(),
        }
    }
    parsed
}

fn value(arguments: &mut impl Iterator<Item = std::ffi::OsString>) -> Option<String> {
    arguments.next()?.into_string().ok()
}

#[cfg(unix)]
fn protected_inherited_reader(fd: i32) -> Result<std::fs::File, String> {
    use std::os::unix::fs::FileTypeExt;

    if fd < 3 {
        return Err("shutdown fd must be an inherited non-stdio descriptor".to_owned());
    }
    let file = std::fs::File::open(format!("/dev/fd/{fd}")).map_err(|error| error.to_string())?;
    let file_type = file
        .metadata()
        .map_err(|error| error.to_string())?
        .file_type();
    if !file_type.is_fifo() && !file_type.is_socket() {
        return Err("shutdown fd must be a protected inherited pipe or socket".to_owned());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn protected_inherited_reader(_fd: i32) -> Result<std::fs::File, String> {
    Err("role=all protected launcher is not available on this platform".to_owned())
}

#[cfg(unix)]
fn make_nonblocking(file: &std::fs::File) -> Result<(), String> {
    use std::os::fd::AsFd;

    let current = nix::fcntl::fcntl(file.as_fd(), nix::fcntl::FcntlArg::F_GETFL)
        .map_err(|error| error.to_string())?;
    let flags = nix::fcntl::OFlag::from_bits_truncate(current) | nix::fcntl::OFlag::O_NONBLOCK;
    nix::fcntl::fcntl(file.as_fd(), nix::fcntl::FcntlArg::F_SETFL(flags))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn make_nonblocking(_file: &std::fs::File) -> Result<(), String> {
    Err("protected launcher channels are not available on this platform".to_owned())
}

struct ProtectedReader {
    file: std::fs::File,
}

impl ProtectedReader {
    fn new(file: std::fs::File) -> Result<Self, String> {
        make_nonblocking(&file)?;
        Ok(Self { file })
    }

    fn poll_eof(&mut self) -> Result<bool, String> {
        let mut byte = [0_u8; 1];
        match self.file.read(&mut byte) {
            Ok(0) => Ok(true),
            Ok(_) => Err("shutdown descriptor must not carry data".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }
}

struct ProtectedFrameReader {
    file: std::fs::File,
    buffer: Zeroizing<Vec<u8>>,
    frame_started: Option<std::time::Instant>,
}

impl ProtectedFrameReader {
    fn new(file: std::fs::File) -> Result<Self, String> {
        make_nonblocking(&file)?;
        Ok(Self {
            file,
            buffer: Zeroizing::new(Vec::new()),
            frame_started: None,
        })
    }

    fn read_available(&mut self) -> Result<Vec<ProtectedChannelFrame>, String> {
        let mut chunk = Zeroizing::new([0_u8; 4096]);
        loop {
            match self.file.read(chunk.as_mut()) {
                Ok(0) => return Err("capability descriptor closed while daemon is running".into()),
                Ok(count) => {
                    if self.buffer.is_empty() {
                        self.frame_started = Some(std::time::Instant::now());
                    }
                    if self.buffer.len().saturating_add(count) > MAX_PROTECTED_FRAME_BYTES {
                        return Err("capability frame exceeds the protected-channel bound".into());
                    }
                    self.buffer.extend_from_slice(&chunk[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        if self
            .frame_started
            .is_some_and(|started| started.elapsed() > PROTECTED_FRAME_DEADLINE)
        {
            return Err("capability frame exceeded its absolute deadline".into());
        }
        let mut frames = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut frame = Zeroizing::new(self.buffer.drain(..=newline).collect::<Vec<_>>());
            frame.pop();
            if frame.is_empty() {
                return Err("capability frame is empty".into());
            }
            frames.push(parse_capability_frame(&frame)?);
            if self.buffer.is_empty() {
                self.frame_started = None;
            }
        }
        Ok(frames)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedCapabilityFrameV1 {
    schema: String,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    registration_id: Option<String>,
    #[serde(default)]
    principal_kind: Option<PrincipalKind>,
    #[serde(default)]
    workspace_id: Option<WorkspaceId>,
    #[serde(default)]
    operation_kind: Option<String>,
    #[serde(default)]
    accepted_digest: Option<CanonicalDigest>,
    #[serde(default)]
    expected_revisions: Option<RevisionSetV1>,
    #[serde(default)]
    expires_at_unix: Option<i64>,
    #[serde(default)]
    candidate_ref: Option<String>,
    #[serde(default)]
    candidate_revision: Option<u64>,
    #[serde(default)]
    secret: Option<String>,
}

impl Drop for ProtectedCapabilityFrameV1 {
    fn drop(&mut self) {
        if let Some(capability) = &mut self.capability {
            capability.zeroize();
        }
        if let Some(secret) = &mut self.secret {
            secret.zeroize();
        }
    }
}

enum ProtectedChannelFrame {
    Upgrade(hiroute_host_runtime::LauncherUpgradeRequest),
    Apply {
        registration_id: String,
        registration: ApplyCapabilityRegistrationV1,
    },
    InputRegister {
        registration_id: String,
        candidate: hiroute_application_api::ComputeCandidateRefV2,
        secret: hiroute_domain::ProtectedSecret,
    },
    InputRelease {
        registration_id: String,
        candidate_ref: String,
    },
}

impl ProtectedChannelFrame {
    fn registration_id(&self) -> String {
        match self {
            Self::Upgrade(request) => request.registration_id.clone(),
            Self::Apply {
                registration_id, ..
            } => registration_id.clone(),
            Self::InputRegister {
                registration_id, ..
            }
            | Self::InputRelease {
                registration_id, ..
            } => registration_id.clone(),
        }
    }
}

fn parse_capability_frame(bytes: &[u8]) -> Result<ProtectedChannelFrame, String> {
    let schema: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "protected frame is invalid")?;
    if schema["schema"] == "hiroute.launcher-upgrade/v1" {
        let request: hiroute_host_runtime::LauncherUpgradeRequest =
            serde_json::from_slice(bytes).map_err(|_| "upgrade frame is invalid")?;
        if request.registration_id.len() != 64
            || !request
                .registration_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err("upgrade registration id is invalid".into());
        }
        return Ok(ProtectedChannelFrame::Upgrade(request));
    }
    let mut frame: ProtectedCapabilityFrameV1 =
        serde_json::from_slice(bytes).map_err(|_| "capability frame is invalid".to_owned())?;
    let valid_id = |id: &str| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit());
    if frame.schema == "hiroute.protected-input/v1" {
        let id = frame
            .registration_id
            .take()
            .filter(|id| valid_id(id))
            .ok_or_else(|| "protected input registration id is invalid".to_owned())?;
        if frame.capability.is_some()
            || frame.principal_kind.is_some()
            || frame.workspace_id.is_some()
            || frame.operation_kind.is_some()
            || frame.accepted_digest.is_some()
            || frame.expected_revisions.is_some()
            || frame.expires_at_unix.is_some()
        {
            return Err("protected input frame contains invalid authority fields".into());
        }
        let candidate_ref = frame
            .candidate_ref
            .take()
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 256
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
                    })
            })
            .ok_or_else(|| "protected input candidate is invalid".to_owned())?;
        return match (frame.candidate_revision.take(), frame.secret.take()) {
            (Some(1), Some(secret)) if !secret.is_empty() && secret.len() <= 32 * 1024 => {
                let secret = hiroute_domain::ProtectedSecret::new(secret.into_bytes())
                    .map_err(|_| "protected input secret is invalid".to_owned())?;
                Ok(ProtectedChannelFrame::InputRegister {
                    registration_id: id,
                    candidate: hiroute_application_api::ComputeCandidateRefV2 {
                        candidate_ref,
                        candidate_revision: 1,
                    },
                    secret,
                })
            }
            (None, None) => Ok(ProtectedChannelFrame::InputRelease {
                registration_id: id,
                candidate_ref,
            }),
            _ => Err("protected input action is invalid".into()),
        };
    }
    let registration_id = frame
        .registration_id
        .take()
        .filter(|id| valid_id(id))
        .ok_or_else(|| "capability frame registration id is invalid".to_owned())?;
    if frame.schema != "hiroute.protected-apply-grant/v2" {
        return Err("capability frame schema is invalid".into());
    }
    if frame.candidate_ref.is_some() || frame.candidate_revision.is_some() || frame.secret.is_some()
    {
        return Err("capability frame contains protected input fields".into());
    }
    let principal = match frame.principal_kind.take() {
        Some(PrincipalKind::InteractiveUser) => "interactive-user",
        Some(PrincipalKind::Desktop) => "desktop",
        Some(PrincipalKind::Skill) | Some(PrincipalKind::SealedCollaboration) | None => {
            return Err("Skill cannot receive Apply authority".to_owned());
        }
    };
    ApplyCapabilityRegistrationV1::from_protected_launcher(
        frame
            .capability
            .take()
            .ok_or_else(|| "capability is missing".to_owned())?,
        principal,
        frame
            .workspace_id
            .take()
            .ok_or_else(|| "workspace is missing".to_owned())?,
        frame
            .operation_kind
            .take()
            .ok_or_else(|| "operation is missing".to_owned())?,
        frame
            .accepted_digest
            .take()
            .ok_or_else(|| "digest is missing".to_owned())?,
        frame
            .expected_revisions
            .take()
            .ok_or_else(|| "revisions are missing".to_owned())?,
        frame
            .expires_at_unix
            .take()
            .ok_or_else(|| "expiry is missing".to_owned())?,
    )
    .map(|registration| ProtectedChannelFrame::Apply {
        registration_id,
        registration,
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod upgrade_frame_tests {
    use super::*;
    #[test]
    fn upgrade_frame_is_exact_and_cannot_carry_business_authority() {
        let mut value = serde_json::json!({"schema":"hiroute.launcher-upgrade/v1","registration_id":"a".repeat(64),"action":"prepare"});
        assert!(matches!(
            parse_capability_frame(&serde_json::to_vec(&value).unwrap()).unwrap(),
            ProtectedChannelFrame::Upgrade(_)
        ));
        value["capability"] = serde_json::json!("not an upgrade field");
        assert!(parse_capability_frame(&serde_json::to_vec(&value).unwrap()).is_err());
        value.as_object_mut().unwrap().remove("capability");
        value["action"] = serde_json::json!("force_stop");
        assert!(parse_capability_frame(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[cfg(unix)]
fn protected_inherited_writer(fd: i32) -> Result<std::fs::File, String> {
    use std::os::unix::fs::FileTypeExt;
    if fd < 3 {
        return Err("ack fd must be an inherited non-stdio descriptor".into());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(format!("/dev/fd/{fd}"))
        .map_err(|_| "ack descriptor unavailable".to_owned())?;
    let kind = file
        .metadata()
        .map_err(|_| "ack descriptor unavailable".to_owned())?
        .file_type();
    if !kind.is_fifo() && !kind.is_socket() {
        return Err("ack descriptor must be a protected pipe or socket".into());
    }
    make_nonblocking(&file)?;
    Ok(file)
}
#[cfg(not(unix))]
fn protected_inherited_writer(_: i32) -> Result<std::fs::File, String> {
    Err("protected launcher channels are unavailable on this platform".into())
}
fn write_ack(file: &mut std::fs::File, registration_id: &str) -> Result<(), String> {
    write_ack_result(file, registration_id, true)
}

fn write_ack_result(
    file: &mut std::fs::File,
    registration_id: &str,
    registered: bool,
) -> Result<(), String> {
    let frame = format!(
        "{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{registration_id}\",\"registered\":{registered}}}\n"
    );
    let deadline = std::time::Instant::now() + PROTECTED_FRAME_DEADLINE;
    let mut remaining = frame.as_bytes();
    while !remaining.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err("capability acknowledgement timed out".into());
        }
        match file.write(remaining) {
            Ok(0) => return Err("capability acknowledgement channel closed".into()),
            Ok(n) => remaining = &remaining[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("capability acknowledgement unavailable".into()),
        }
    }
    Ok(())
}

fn write_ready(
    role: &hiroute_daemon::RoleAllHandle,
    diagnostics: &DiagnosticRuntime,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let outcome = write_ready_frame(role);
    let (error, os_errno) = match &outcome {
        Ok(()) => (None, None),
        Err(error) => (Some(classify_ready_error(error)), error.raw_os_error()),
    };
    diagnostics.emit(DiagnosticEvent::ReadyWrite(ReadyIo {
        direction: ReadyDirection::Write,
        ok: outcome.is_ok(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        error,
        os_errno,
    }));
    outcome.map_err(|error| error.to_string())
}

fn classify_ready_error(error: &std::io::Error) -> ReadyIoError {
    match error.kind() {
        std::io::ErrorKind::BrokenPipe => ReadyIoError::BrokenPipe,
        std::io::ErrorKind::WriteZero => ReadyIoError::Closed,
        _ => ReadyIoError::Io,
    }
}

fn exit_usage() -> ! {
    eprintln!(
        "usage: hirouted --role control --storage-root <DIR> --runtime-root <DIR>\n       hirouted --role all --standalone [--cpa-binary <PATH> --cpa-sha256 <HEX>] [--diagnostics-root <ABSOLUTE_DIR>] [--diagnostic-level-override error|warn|info|debug]\n       hirouted --role all --storage-root <DIR> --runtime-root <DIR> --listen <IPV4:PORT> --lkg <PATH> --shutdown-fd <INHERITED_FD> --capability-fd <INHERITED_FD> [--capability-ack-fd <INHERITED_FD>] [--cpa-binary <PATH> --cpa-sha256 <HEX>] [--codex-desktop-engine <PATH>] [--diagnostics-root <ABSOLUTE_DIR>] [--diagnostics-parent-session <HEX32>] [--diagnostic-level-override error|warn|info|debug]"
    );
    std::process::exit(2)
}

#[cfg(all(test, unix))]
mod ready_frame_tests {
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    use super::ready::write_ready_to;
    use super::*;

    #[test]
    fn closed_ready_reader_returns_an_error_instead_of_panicking() {
        let (reader, mut writer) = UnixStream::pair().expect("socket pair");
        drop(reader);
        let error = write_ready_to(
            &mut writer,
            Path::new("/tmp/hiroute-control.sock"),
            "127.0.0.1:43210".parse().expect("listen"),
        )
        .expect_err("a closed reader must produce an error, not a panic");
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn successful_ready_frame_is_the_only_stdout_payload() {
        let mut buffer = Vec::new();
        write_ready_to(
            &mut buffer,
            Path::new("/tmp/hiroute-control.sock"),
            "127.0.0.1:43210".parse().expect("listen"),
        )
        .expect("ready frame");
        let text = String::from_utf8(buffer.clone()).expect("utf8");
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with('\n'));
        let value: serde_json::Value = serde_json::from_slice(&buffer).expect("json");
        assert_eq!(value["schema"], "hiroute.daemon-ready/v1");
        assert_eq!(value["role"], "all");
        assert_eq!(value["control_endpoint"], "/tmp/hiroute-control.sock");
        assert_eq!(value["process_id"], std::process::id());
        assert!(!text.contains("diagnostic"));
        assert!(!text.contains("panic"));
    }

    #[test]
    fn ready_failures_map_to_stable_io_codes() {
        let broken = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert_eq!(classify_ready_error(&broken), ReadyIoError::BrokenPipe);
        let zero = std::io::Error::from(std::io::ErrorKind::WriteZero);
        assert_eq!(classify_ready_error(&zero), ReadyIoError::Closed);
        let other = std::io::Error::from(std::io::ErrorKind::Other);
        assert_eq!(classify_ready_error(&other), ReadyIoError::Io);
    }

    #[test]
    fn ready_write_never_mixes_diagnostics_into_stdout() {
        let mut buffer = Vec::new();
        write_ready_to(
            &mut buffer,
            Path::new("/tmp/hiroute-control.sock"),
            "127.0.0.1:43210".parse().expect("listen"),
        )
        .expect("ready frame");
        // Exactly one complete JSON document terminated by one newline.
        let mut lines = buffer.split(|byte| *byte == b'\n');
        let first = lines.next().expect("first line");
        assert!(!first.is_empty());
        assert_eq!(lines.next(), Some([].as_slice()));
        assert_eq!(lines.next(), None);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    use serde_json::json;

    use super::*;

    fn frame(principal: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": "hiroute.protected-apply-grant/v2",
            "registration_id": "a".repeat(64),
            "capability": "0123456789abcdef0123456789abcdef0123456789abcdef",
            "principal_kind": principal,
            "workspace_id": "personal/default",
            "operation_kind": "ApplySetup",
            "accepted_digest": CanonicalDigest::of_bytes(b"change"),
            "expected_revisions": {"target": 0, "dependencies": {}},
            "expires_at_unix": 4_102_444_800_i64,
        }))
        .unwrap()
    }

    #[test]
    fn protected_channel_incrementally_accepts_one_strict_non_skill_grant() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let file = std::fs::File::from(std::os::fd::OwnedFd::from(reader));
        let mut reader = ProtectedFrameReader::new(file).unwrap();
        let mut encoded = frame("interactive_user");
        let remainder = encoded.split_off(encoded.len() / 2);
        writer.write_all(&encoded).unwrap();
        assert!(reader.read_available().unwrap().is_empty());
        writer.write_all(&remainder).unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(reader.read_available().unwrap().len(), 1);
    }

    #[test]
    fn protected_channel_rejects_skill_unknown_fields_and_unterminated_deadline() {
        assert!(parse_capability_frame(&frame("skill")).is_err());
        let mut unknown: serde_json::Value = serde_json::from_slice(&frame("desktop")).unwrap();
        unknown["path"] = json!("/tmp/not-authority");
        assert!(parse_capability_frame(&serde_json::to_vec(&unknown).unwrap()).is_err());

        let (reader, mut writer) = UnixStream::pair().unwrap();
        let file = std::fs::File::from(std::os::fd::OwnedFd::from(reader));
        let mut reader = ProtectedFrameReader::new(file).unwrap();
        writer.write_all(b"{").unwrap();
        assert!(reader.read_available().unwrap().is_empty());
        reader.frame_started = Some(std::time::Instant::now() - PROTECTED_FRAME_DEADLINE);
        assert!(reader.read_available().is_err());
    }
}

#[cfg(all(test, unix))]
mod acknowledgement_tests {
    use super::*;
    #[test]
    fn v2_registration_requires_a_correlated_id_and_rejects_v1() {
        let mut value = serde_json::json!({
            "schema":"hiroute.protected-apply-grant/v2", "registration_id":"a".repeat(64),
            "capability":"b".repeat(64), "principal_kind":"desktop", "workspace_id":"personal/default",
            "operation_kind":"ApplyAgentPlanChange", "accepted_digest":CanonicalDigest::of_bytes(b"change"),
            "expected_revisions":{"target":0,"dependencies":{}}, "expires_at_unix":4102444800_i64
        });
        let parse =
            |value: &serde_json::Value| parse_capability_frame(&serde_json::to_vec(value).unwrap());
        assert_eq!(parse(&value).unwrap().registration_id(), "a".repeat(64));
        value["registration_id"] = serde_json::json!("bad");
        assert!(parse(&value).is_err());
        value.as_object_mut().unwrap().remove("registration_id");
        assert!(parse(&value).is_err());
        value["schema"] = serde_json::json!("hiroute.protected-apply-grant/v1");
        value["registration_id"] = serde_json::json!("a".repeat(64));
        assert!(parse(&value).is_err());
    }
}
