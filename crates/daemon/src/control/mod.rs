use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hiroute_application::ApplicationService;
use hiroute_application_api::{
    APPLY_SUBSCRIPTION_CHECK_OPERATION_V2, ClientHelloV1, CommandLifecycle, ErrorCode, ErrorV1,
    LOCAL_CONTROL_MAX_FRAME_BYTES, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2,
    MachineEnvelopeV2, PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1, PrincipalKind,
    RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2, command_by_operation, negotiate_hello,
};
use hiroute_diagnostics::context::DiagnosticHandle;
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::error::EventErrorCode;
use hiroute_diagnostics::event::{
    ControlCallEnd, ControlOperation, DiagnosticEvent, StageOutcome,
    SubmissionState as DiagnosticSubmissionState,
};

#[cfg(unix)]
mod managed;
mod runtime;

#[cfg(unix)]
pub(crate) use managed::start_control_with_agent_grants;
#[cfg(unix)]
pub use managed::{ManagedControlError, ManagedControlHandle, ManagedControlPhase, start_control};
pub use runtime::ProductionControlRuntime;

#[cfg(unix)]
pub(crate) trait AgentGrantResolverPort: Send + Sync {
    fn resolve_agent_grant(
        &self,
        connection_id: &str,
    ) -> Result<hiroute_domain::AgentAccessGrantMaterial, ()>;
}

const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_CONNECTIONS: usize = 128;
const MAX_CONCURRENT_WAIT_CONNECTIONS: usize = 64;
const MAX_UNCLASSIFIED_CONNECTIONS: usize = 16;

#[cfg(unix)]
#[derive(Clone)]
struct CapacityCounter {
    active: Arc<AtomicUsize>,
    maximum: usize,
}

#[cfg(unix)]
impl CapacityCounter {
    fn new(maximum: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            maximum,
        }
    }

    fn try_acquire(&self) -> Option<CapacityPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.maximum).then_some(count + 1)
            })
            .ok()
            .map(|_| CapacityPermit {
                active: Arc::clone(&self.active),
            })
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

#[cfg(unix)]
struct CapacityPermit {
    active: Arc<AtomicUsize>,
}

#[cfg(unix)]
impl Drop for CapacityPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct ControlConnectionBudgets {
    total: CapacityCounter,
    waits: CapacityCounter,
    unclassified: CapacityCounter,
}

#[cfg(unix)]
impl ControlConnectionBudgets {
    fn new(total: usize, waits: usize, unclassified: usize) -> Self {
        debug_assert!(waits <= total);
        debug_assert!(unclassified <= total);
        Self {
            total: CapacityCounter::new(total),
            waits: CapacityCounter::new(waits),
            unclassified: CapacityCounter::new(unclassified),
        }
    }

    fn try_accept(&self) -> Option<ControlConnectionAdmission> {
        let total = self.total.try_acquire()?;
        let unclassified = self.unclassified.try_acquire()?;
        Some(ControlConnectionAdmission {
            _total: total,
            unclassified: Some(unclassified),
            budgets: self.clone(),
        })
    }

    fn active_total(&self) -> usize {
        self.total.active()
    }
}

#[cfg(unix)]
impl Default for ControlConnectionBudgets {
    fn default() -> Self {
        Self::new(
            MAX_CONCURRENT_CONNECTIONS,
            MAX_CONCURRENT_WAIT_CONNECTIONS,
            MAX_UNCLASSIFIED_CONNECTIONS,
        )
    }
}

#[cfg(unix)]
struct ControlConnectionAdmission {
    _total: CapacityPermit,
    unclassified: Option<CapacityPermit>,
    budgets: ControlConnectionBudgets,
}

#[cfg(unix)]
impl ControlConnectionAdmission {
    fn classify(&mut self, operation_id: &str) -> Result<Option<CapacityPermit>, ErrorCode> {
        let wait = if is_wait_operation(operation_id) {
            self.budgets
                .waits
                .try_acquire()
                .map(Some)
                .ok_or(ErrorCode::ControlWaitCapacityExceeded)
        } else {
            Ok(None)
        };
        drop(self.unclassified.take());
        wait
    }

    fn classified_non_wait(&mut self) {
        drop(self.unclassified.take());
    }
}

#[cfg(unix)]
fn is_wait_operation(operation_id: &str) -> bool {
    matches!(operation_id, "WorkerWait" | "WaitDelegation")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlEndpoint {
    path: PathBuf,
}

impl ControlEndpoint {
    pub fn from_runtime_root(root: impl AsRef<Path>) -> Self {
        Self {
            path: root.as_ref().join("hiroute/control.sock"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Mirror of the shared event vocabulary for a product error category. Both enums are
/// closed, so this mapping cannot widen into free-form text.
fn category_event_code(category: hiroute_application_api::ErrorCategory) -> EventErrorCode {
    use hiroute_application_api::ErrorCategory as C;
    match category {
        C::Usage => EventErrorCode::Usage,
        C::Conflict => EventErrorCode::Conflict,
        C::Authorization => EventErrorCode::Authorization,
        C::NotFound => EventErrorCode::NotFound,
        C::Unavailable => EventErrorCode::Unavailable,
        C::ActionRequired => EventErrorCode::ActionRequired,
        C::Recovery => EventErrorCode::Recovery,
        C::Internal => EventErrorCode::Internal,
    }
}

#[derive(Clone)]
pub struct LocalControlDaemon {
    upgrade: Arc<hiroute_host_runtime::UpgradeDrain>,
    application: ApplicationService,
    released_commands_only: bool,
    /// Receive-side diagnostics; a no-op handle keeps every existing host unchanged.
    diagnostics: DiagnosticHandle,
}

impl LocalControlDaemon {
    pub fn new(application: ApplicationService) -> Self {
        Self {
            upgrade: Arc::new(hiroute_host_runtime::UpgradeDrain::default()),
            application,
            released_commands_only: false,
            diagnostics: DiagnosticHandle::noop(),
        }
    }

    pub(crate) fn with_released_commands_only(mut self) -> Self {
        self.released_commands_only = true;
        self
    }

    pub(crate) fn with_upgrade_drain(
        mut self,
        upgrade: Arc<hiroute_host_runtime::UpgradeDrain>,
    ) -> Self {
        self.upgrade = upgrade;
        self
    }

    /// Record one bounded control-call outcome per dispatch on the receiving side. The
    /// token is derived from the wire request id, never stored as text.
    pub fn with_diagnostics(mut self, diagnostics: DiagnosticHandle) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    fn dispatch_wire(
        &self,
        request: LocalControlWireRequestV2,
    ) -> MachineEnvelopeV2<serde_json::Value> {
        let started = Instant::now();
        let operation = ControlOperation::from_wire(&request.operation_id);
        let request_token = self
            .diagnostics
            .token(CorrelationDomain::ControlRequest, &request.request_id);
        let response = self.dispatch_wire_inner(request);
        self.diagnostics
            .try_emit(DiagnosticEvent::ControlCallEnd(ControlCallEnd {
                operation,
                request_token,
                submission: DiagnosticSubmissionState::Sent,
                error: response
                    .error
                    .as_ref()
                    .map(|error| category_event_code(error.category)),
                elapsed_ms: started.elapsed().as_millis() as u64,
            }));
        response
    }

    fn dispatch_wire_inner(
        &self,
        request: LocalControlWireRequestV2,
    ) -> MachineEnvelopeV2<serde_json::Value> {
        if self.released_commands_only
            && !command_by_operation(&request.operation_id)
                .is_some_and(|command| command.lifecycle == CommandLifecycle::Released)
        {
            return MachineEnvelopeV2::failed(
                ErrorV1::new(ErrorCode::UnknownCommand),
                Some(request.request_id),
            );
        }
        let grant_allowed = matches!(
            request.operation_id.as_str(),
            "ListWorkPlans"
                | "ListDelegations"
                | "GetDelegation"
                | "StartDelegation"
                | "WaitDelegation"
                | "ReadDelegationResult"
                | "CancelDelegation"
                | "ContinueDelegation"
                | "ApplySetup"
                | "CancelOperation"
                | "CheckAgentConnection"
                | "CheckNativeModelConnection"
                | "CheckRegisteredModelConnection"
                | PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1
                | "CheckSavedModelConnection"
                | "TestClassifierDecision"
                | APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
                | RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2
                | "PreviewSessionDeletion"
                | "ApplySessionDeletion"
                | "GetSession"
                | "ListSessions"
                | "GetRoutingReceipt"
                | "GetObservationStatus"
                | "GetValue"
                | "GetPlanQualitySamples"
        );
        if request.operation_id.starts_with("Worker") && request.protected_grant.is_some() {
            return MachineEnvelopeV2::failed(
                ErrorV1::new(ErrorCode::CapabilityDenied),
                Some(request.request_id),
            );
        }
        if request.protected_grant.is_some() && !grant_allowed {
            return MachineEnvelopeV2::failed(
                ErrorV1::new(ErrorCode::CapabilityDenied),
                Some(request.request_id),
            );
        }
        if request.protected_grant.as_ref().is_some_and(|grant| {
            (grant.principal_kind.is_collaboration()
                && !matches!(
                    request.operation_id.as_str(),
                    "ListWorkPlans"
                        | "ListDelegations"
                        | "GetDelegation"
                        | "StartDelegation"
                        | "WaitDelegation"
                        | "ReadDelegationResult"
                        | "CancelDelegation"
                        | "ContinueDelegation"
                ))
                || grant.capability.is_empty()
                || grant.capability.len()
                    > if grant.principal_kind == PrincipalKind::SealedCollaboration {
                        4096
                    } else {
                        512
                    }
                || grant.capability.contains('\0')
        }) {
            return MachineEnvelopeV2::failed(
                ErrorV1::new(ErrorCode::CapabilityDenied),
                Some(request.request_id),
            );
        }
        self.application.dispatch(request.authenticate_ambient())
    }
}

#[cfg(unix)]
pub fn serve_production_control(
    storage_root: impl AsRef<Path>,
    runtime_root: impl AsRef<Path>,
    diagnostics: &hiroute_diagnostics::runtime::DiagnosticRuntime,
) -> Result<(), String> {
    use hiroute_diagnostics::event::{StartupEnd, StartupOutcome};

    let runtime = ProductionControlRuntime::open(storage_root)?;
    let (listener, _guard, _endpoint) = bind_control_endpoint(runtime_root, diagnostics)?;
    // The endpoint exists and is owned: startup is over, not merely configured.
    diagnostics.emit(DiagnosticEvent::StartupEnd(StartupEnd {
        outcome: StartupOutcome::Success,
        elapsed_ms: diagnostics.started_elapsed_ms(),
    }));
    let daemon = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()))
        .with_diagnostics(diagnostics.handle().clone());
    serve_listener(listener, daemon, None)
}

/// Bind the control endpoint inside its own startup stage. The stage completes only when a
/// listener exists; a refused or failed bind reports `failed` and never reports that the
/// process started to serve.
#[cfg(unix)]
fn bind_control_endpoint(
    runtime_root: impl AsRef<Path>,
    diagnostics: &hiroute_diagnostics::runtime::DiagnosticRuntime,
) -> Result<
    (
        std::os::unix::net::UnixListener,
        EndpointGuard,
        ControlEndpoint,
    ),
    String,
> {
    use hiroute_diagnostics::event::{StartupFailureCode, StartupStage};

    diagnostics.stage_begin(StartupStage::ControlBind);
    match bind_listener(runtime_root) {
        Ok(bound) => {
            diagnostics.stage_end(StartupStage::ControlBind, StageOutcome::Completed);
            Ok(bound)
        }
        Err(error) => {
            diagnostics.stage_end(
                StartupStage::ControlBind,
                StageOutcome::Failed {
                    code: StartupFailureCode::ControlUnavailable,
                },
            );
            Err(error)
        }
    }
}

#[cfg(unix)]
fn bind_listener(
    runtime_root: impl AsRef<Path>,
) -> Result<
    (
        std::os::unix::net::UnixListener,
        EndpointGuard,
        ControlEndpoint,
    ),
    String,
> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    let endpoint = ControlEndpoint::from_runtime_root(runtime_root);
    let parent = endpoint
        .path()
        .parent()
        .ok_or_else(|| "Local Control endpoint has no parent".to_owned())?;
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|error| error.to_string())?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.file_type().is_dir() {
        return Err("Local Control runtime path is not a regular directory".to_owned());
    }
    if let Ok(metadata) = fs::symlink_metadata(endpoint.path()) {
        if !metadata.file_type().is_socket() {
            return Err("refusing to replace an unowned Local Control endpoint".to_owned());
        }
        match UnixStream::connect(endpoint.path()) {
            Ok(_) => return Err("a Local Control daemon is already active".to_owned()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
        fs::remove_file(endpoint.path()).map_err(|error| error.to_string())?;
    }
    let listener = UnixListener::bind(endpoint.path()).map_err(|error| error.to_string())?;
    fs::set_permissions(endpoint.path(), fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let guard = EndpointGuard(endpoint.path().to_path_buf());
    Ok((listener, guard, endpoint))
}

#[cfg(not(unix))]
pub fn serve_production_control(
    _storage_root: impl AsRef<Path>,
    _runtime_root: impl AsRef<Path>,
    _diagnostics: &hiroute_diagnostics::runtime::DiagnosticRuntime,
) -> Result<(), String> {
    Err("owner-only Windows Named Pipe transport is not available in this control slice".to_owned())
}

#[cfg(unix)]
fn serve_connection(
    daemon: &LocalControlDaemon,
    mut stream: std::os::unix::net::UnixStream,
    mut admission: ControlConnectionAdmission,
) -> Result<(), String> {
    stream
        .set_write_timeout(Some(FRAME_TIMEOUT))
        .map_err(|error| error.to_string())?;
    let reader_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(reader_stream);
    let hello_line = read_frame(&mut reader, frame_deadline()?)?;
    let hello: ClientHelloV1 = serde_json::from_str(&hello_line)
        .map_err(|_| "invalid Local Control client hello".to_owned())?;
    let server_hello = match negotiate_hello(&hello) {
        Ok(hello) => hello,
        Err(error) => {
            write_negotiated_error(&mut stream, error, None)?;
            return Ok(());
        }
    };
    write_frame(&mut stream, &server_hello)?;
    let request_line = read_frame(&mut reader, frame_deadline()?)?;
    let request = match serde_json::from_str::<LocalControlWireRequestV2>(&request_line) {
        Ok(request) if request.schema_version == LOCAL_CONTROL_SCHEMA_V2 => request,
        _ => {
            write_negotiated_error(&mut stream, ErrorV1::new(ErrorCode::InvalidArguments), None)?;
            return Ok(());
        }
    };
    let Some(_upgrade_call) = daemon.upgrade.enter(false) else {
        write_negotiated_error(
            &mut stream,
            ErrorV1::new(ErrorCode::DaemonUnavailable),
            Some(request.request_id),
        )?;
        return Ok(());
    };
    let _wait_permit = match admission.classify(&request.operation_id) {
        Ok(permit) => permit,
        Err(error) => {
            write_negotiated_error(&mut stream, ErrorV1::new(error), Some(request.request_id))?;
            return Ok(());
        }
    };
    write_frame(&mut stream, &daemon.dispatch_wire(request))
}

#[cfg(unix)]
fn spawn_control_connection(
    daemon: LocalControlDaemon,
    stream: std::os::unix::net::UnixStream,
    admission: ControlConnectionAdmission,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hiroute-control-connection".to_owned())
        .spawn(move || {
            let _ = serve_connection(&daemon, stream, admission);
        })
        .map(|_| ())
        .map_err(|error| format!("Local Control connection thread cannot start: {error}"))
}

#[cfg(unix)]
fn serve_listener(
    listener: std::os::unix::net::UnixListener,
    daemon: LocalControlDaemon,
    max_accepts: Option<usize>,
) -> Result<(), String> {
    serve_listener_with_budgets(
        listener,
        daemon,
        max_accepts,
        ControlConnectionBudgets::default(),
    )
}

#[cfg(unix)]
fn serve_listener_with_budgets(
    listener: std::os::unix::net::UnixListener,
    daemon: LocalControlDaemon,
    max_accepts: Option<usize>,
    budgets: ControlConnectionBudgets,
) -> Result<(), String> {
    for (accepted, stream) in listener.incoming().enumerate() {
        if max_accepts.is_some_and(|limit| accepted >= limit) {
            break;
        }
        let stream = stream.map_err(|error| error.to_string())?;
        let Some(admission) = budgets.try_accept() else {
            continue;
        };
        spawn_control_connection(daemon.clone(), stream, admission)?;
        if max_accepts.is_some_and(|limit| accepted + 1 >= limit) {
            break;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn frame_deadline() -> Result<Instant, String> {
    Instant::now()
        .checked_add(FRAME_TIMEOUT)
        .ok_or_else(|| "Local Control frame deadline overflow".to_owned())
}

#[cfg(unix)]
fn read_frame(
    reader: &mut BufReader<std::os::unix::net::UnixStream>,
    deadline: Instant,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| "Local Control frame deadline exceeded".to_owned())?;
        reader
            .get_ref()
            .set_read_timeout(Some(remaining))
            .map_err(|error| error.to_string())?;
        let available = reader.fill_buf().map_err(|error| error.to_string())?;
        if available.is_empty() {
            return Err("invalid or oversized Local Control frame".to_owned());
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if bytes.len() + take > LOCAL_CONTROL_MAX_FRAME_BYTES {
            return Err("invalid or oversized Local Control frame".to_owned());
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if bytes.last() == Some(&b'\n') {
            break;
        }
    }
    let mut line = String::from_utf8(bytes)
        .map_err(|_| "invalid or oversized Local Control frame".to_owned())?;
    if !line.ends_with('\n') {
        return Err("invalid or oversized Local Control frame".to_owned());
    }
    line.pop();
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

fn write_negotiated_error(
    stream: &mut impl Write,
    error: ErrorV1,
    request_id: Option<String>,
) -> Result<(), String> {
    let envelope = MachineEnvelopeV2::<serde_json::Value>::failed(error, request_id);
    write_frame(stream, &envelope)
}

struct BoundedFrame(Vec<u8>);

impl Write for BoundedFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) >= LOCAL_CONTROL_MAX_FRAME_BYTES {
            return Err(std::io::Error::other("frame bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_frame(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<(), String> {
    let mut frame = BoundedFrame(Vec::new());
    serde_json::to_writer(&mut frame, value).map_err(|error| error.to_string())?;
    frame.0.push(b'\n');
    writer
        .write_all(&frame.0)
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

#[cfg(unix)]
struct EndpointGuard(PathBuf);

#[cfg(unix)]
impl Drop for EndpointGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
#[path = "control_capacity_tests.rs"]
mod control_capacity_tests;

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::{Duration, Instant};

    use hiroute_application_api::{
        ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2,
        MACHINE_ENVELOPE_SCHEMA_V2, PrincipalKind, ProtectedClientGrantV2,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn control_endpoint_is_derived_from_runtime_root() {
        assert_eq!(
            ControlEndpoint::from_runtime_root("/run/user/1000")
                .path()
                .to_string_lossy(),
            "/run/user/1000/hiroute/control.sock"
        );
    }

    #[test]
    fn overlong_control_frame_without_newline_is_rejected_incrementally() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            let _ = writer.write_all(&vec![b'x'; LOCAL_CONTROL_MAX_FRAME_BYTES + 1]);
            let _ = release_rx.recv();
        });
        let error = read_frame(
            &mut BufReader::new(reader),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(error.contains("oversized"), "{error}");
        release_tx.send(()).unwrap();
        sender.join().unwrap();
    }

    #[test]
    fn oversized_control_response_is_rejected_before_writing() {
        let mut output = Vec::new();
        let error =
            write_frame(&mut output, &"x".repeat(LOCAL_CONTROL_MAX_FRAME_BYTES)).unwrap_err();
        assert!(error.contains("frame bound"), "{error}");
        assert!(output.is_empty());
    }

    #[test]
    fn slow_trickle_cannot_reset_the_absolute_frame_deadline() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            for _ in 0..20 {
                if writer.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        assert!(
            read_frame(
                &mut BufReader::new(reader),
                started + Duration::from_millis(100),
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(350));
    }

    #[test]
    fn a_failed_control_bind_reports_failure_and_never_startup_success() {
        use hiroute_diagnostics::event::ProcessRole;
        use hiroute_diagnostics::level::DiagnosticLevel;
        use hiroute_diagnostics::record::Component;
        use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

        let directory = crate::test_support::private_tempdir();
        let root = directory.path().join("d");
        let runtime = DiagnosticRuntime::start(RuntimeConfig {
            root: root.clone(),
            role: ProcessRole::Daemon,
            component: Component::Daemon,
            parent_session_id: None,
            level_override: Some(DiagnosticLevel::Debug),
        });
        let runtime_root = directory.path().join("run");
        let endpoint_dir = runtime_root.join("hiroute");
        std::fs::create_dir_all(&endpoint_dir).unwrap();
        std::fs::set_permissions(&endpoint_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Occupying the endpoint with a plain file is refused by name and ownership.
        std::fs::write(endpoint_dir.join("control.sock"), b"not-a-socket").unwrap();

        let error = match bind_control_endpoint(&runtime_root, &runtime) {
            Ok(_) => panic!("bind must fail"),
            Err(error) => error,
        };
        assert!(error.contains("unowned"), "{error}");
        runtime.shutdown();
        let log = std::fs::read_to_string(root.join("daemon").join("current.jsonl")).unwrap();
        assert!(
            log.contains("\"stage_begin\":{\"stage\":\"control_bind\"}"),
            "{log}"
        );
        assert!(
            log.contains(
                "\"stage_end\":{\"stage\":\"control_bind\",\"elapsed_ms\":0,\"outcome\":{\"failed\":{\"code\":\"control_unavailable\"}}}"
            ) || (log.contains("\"stage\":\"control_bind\"")
                && log.contains("\"outcome\":{\"failed\":{\"code\":\"control_unavailable\"}}")),
            "{log}"
        );
        assert!(
            !log.contains("\"startup_end\""),
            "a refused bind must never report startup success: {log}"
        );
    }

    #[test]
    fn a_bound_control_endpoint_completes_the_stage_before_startup_success() {
        use hiroute_diagnostics::event::ProcessRole;
        use hiroute_diagnostics::level::DiagnosticLevel;
        use hiroute_diagnostics::record::Component;
        use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

        let directory = crate::test_support::private_tempdir();
        let root = directory.path().join("d");
        let runtime = DiagnosticRuntime::start(RuntimeConfig {
            root: root.clone(),
            role: ProcessRole::Daemon,
            component: Component::Daemon,
            parent_session_id: None,
            level_override: Some(DiagnosticLevel::Debug),
        });
        let runtime_root = directory.path().join("run");
        let bound = bind_control_endpoint(&runtime_root, &runtime);
        assert!(bound.is_ok(), "{:?}", bound.err());
        runtime.shutdown();
        let log = std::fs::read_to_string(root.join("daemon").join("current.jsonl")).unwrap();
        let begin = log
            .find("\"stage_begin\":{\"stage\":\"control_bind\"}")
            .unwrap();
        let end = log
            .find("\"stage_end\":{\"stage\":\"control_bind\"")
            .expect("the stage completes only after the bind");
        assert!(begin < end, "{log}");
        assert!(
            log[end..].contains("\"outcome\":\"completed\""),
            "the completed bind is the stage outcome: {log}"
        );
        assert!(
            std::fs::symlink_metadata(runtime_root.join("hiroute/control.sock")).is_ok(),
            "the endpoint exists when the stage completes"
        );
    }

    #[test]
    fn received_calls_record_one_outcome_without_the_raw_request_id() {
        use hiroute_diagnostics::level::DiagnosticLevel;
        use hiroute_diagnostics::record::Component;
        use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

        let directory = crate::test_support::private_tempdir();
        let root = directory.path().join("d");
        let runtime = DiagnosticRuntime::start(RuntimeConfig {
            root: root.clone(),
            role: hiroute_diagnostics::event::ProcessRole::Daemon,
            component: Component::Daemon,
            parent_session_id: None,
            level_override: Some(DiagnosticLevel::Debug),
        });
        let daemon = LocalControlDaemon::new(ApplicationService::default())
            .with_diagnostics(runtime.handle().clone());
        let sentinel = "wire-request-4f81ba";
        let response = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: sentinel.to_owned(),
            operation_id: "DesktopSnapshot".to_owned(),
            payload: json!({}),
            protected_grant: None,
        });
        // The empty application service rejects the operation; the call is still recorded.
        assert!(response.error.is_some());
        let stable = category_event_code(response.error.as_ref().unwrap().category);
        runtime.shutdown();
        let log = std::fs::read_to_string(root.join("daemon").join("current.jsonl")).unwrap();
        assert_eq!(log.matches("\"control_call_end\":").count(), 1, "{log}");
        assert!(log.contains("\"operation\":\"desktop_snapshot\""));
        assert!(log.contains("\"submission\":\"sent\""));
        assert!(log.contains(&format!("\"error\":\"{}\"", stable.as_str())));
        assert!(!log.contains(sentinel), "raw request id leaked");
    }

    #[test]
    fn control_capability_is_rejected_on_a_read_operation() {
        let response = LocalControlDaemon::new(ApplicationService::default()).dispatch_wire(
            LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "read-with-capability".to_owned(),
                operation_id: "GetSystemStatus".to_owned(),
                payload: json!({}),
                protected_grant: Some(ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::InteractiveUser,
                    capability: "must-not-be-routed".to_owned(),
                }),
            },
        );
        assert_eq!(response.error.unwrap().code, ErrorCode::CapabilityDenied);
    }

    #[test]
    fn standalone_gate_rejects_planned_commands_and_keeps_released_commands() {
        let daemon =
            LocalControlDaemon::new(ApplicationService::default()).with_released_commands_only();
        let planned = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "planned-command".to_owned(),
            operation_id: "GetSettings".to_owned(),
            payload: json!({}),
            protected_grant: None,
        });
        assert_eq!(planned.error.unwrap().code, ErrorCode::UnknownCommand);

        let released_status = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "released-status".to_owned(),
            operation_id: "GetSystemStatus".to_owned(),
            payload: json!({}),
            protected_grant: None,
        });
        assert_eq!(
            released_status.error.unwrap().code,
            ErrorCode::DaemonUnavailable
        );

        let internal = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "internal-operation".to_owned(),
            operation_id: APPLY_SUBSCRIPTION_CHECK_OPERATION_V2.to_owned(),
            payload: json!({}),
            protected_grant: None,
        });
        assert_eq!(internal.error.unwrap().code, ErrorCode::UnknownCommand);

        let released = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "released-command".to_owned(),
            operation_id: "ListSchemas".to_owned(),
            payload: json!({}),
            protected_grant: None,
        });
        assert!(released.error.is_none(), "{released:?}");
    }

    #[test]
    fn classifier_diagnostic_and_subscription_protected_operations_reach_application_dispatch() {
        for operation_id in [
            "TestClassifierDecision",
            APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
            RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2,
        ] {
            let response = LocalControlDaemon::new(ApplicationService::default()).dispatch_wire(
                LocalControlWireRequestV2 {
                    schema_version: LOCAL_CONTROL_SCHEMA_V2,
                    request_id: format!("{operation_id}-allowlist"),
                    operation_id: operation_id.to_owned(),
                    payload: json!({}),
                    protected_grant: Some(ProtectedClientGrantV2 {
                        principal_kind: PrincipalKind::Desktop,
                        capability: "bounded-test-capability".to_owned(),
                    }),
                },
            );
            assert_eq!(response.error.unwrap().code, ErrorCode::DaemonUnavailable);
        }
    }

    #[test]
    fn classifier_secret_apply_uses_local_control_without_a_grant() {
        let daemon = LocalControlDaemon::new(ApplicationService::default());
        for (protected_grant, expected) in [
            (None, ErrorCode::DaemonUnavailable),
            (
                Some(ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: "bounded-test-capability".to_owned(),
                }),
                ErrorCode::CapabilityDenied,
            ),
        ] {
            let response = daemon.dispatch_wire(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "classifier-secret-apply".to_owned(),
                operation_id: "ApplyClassifierHeaderSecret".to_owned(),
                payload: json!({"accept_digest": "test"}),
                protected_grant,
            });
            assert_eq!(response.error.unwrap().code, expected);
        }
    }

    #[test]
    fn classifier_secret_apply_dispatches_to_local_mutation() {
        #[cfg(unix)]
        if crate::test_support::isolated_agent_home(
            "control::tests::classifier_secret_apply_dispatches_to_local_mutation",
        ) {
            return;
        }
        use hiroute_application::control::ApplicationMutationPort;
        use hiroute_application::{PreparedTransactionV1, TransactionError};
        use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1, PreviewResultV1};
        use hiroute_domain::{CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, OperationV1};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct MutationProbe(AtomicUsize);
        impl ApplicationMutationPort for MutationProbe {
            fn preview_change(
                &self,
                _: PreviewRequestV1,
            ) -> Result<PreviewResultV1, TransactionError> {
                Err(TransactionError::InvalidArguments)
            }
            fn apply_change(
                &self,
                _: PrincipalKind,
                _: ApplyRequestV1,
            ) -> Result<OperationV1, TransactionError> {
                Err(TransactionError::InvalidArguments)
            }
            fn apply_local_change(
                &self,
                _: ApplyRequestV1,
            ) -> Result<OperationV1, TransactionError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(TransactionError::InvalidArguments)
            }
            fn apply_prepared_change(
                &self,
                _: PrincipalKind,
                _: PreparedTransactionV1,
            ) -> Result<OperationV1, TransactionError> {
                Err(TransactionError::InvalidArguments)
            }
            fn apply_local_prepared_change(
                &self,
                _: PreparedTransactionV1,
            ) -> Result<OperationV1, TransactionError> {
                Err(TransactionError::InvalidArguments)
            }
        }

        let dir = crate::test_support::private_tempdir();
        let runtime = runtime::ProductionControlRuntime::open_with_release_catalog(
            dir.path().join("storage"),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let probe = Arc::new(MutationProbe(AtomicUsize::new(0)));
        let mut ports = runtime.application_ports();
        ports.mutation = Some(probe.clone());
        let daemon = LocalControlDaemon::new(ApplicationService::new(ports));
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "routing.classifier.secret.apply".into(),
            resource_id: Some("personal/default".into()),
            desired_state: json!({}),
        };
        let response = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "classifier-secret-local-dispatch".to_owned(),
            operation_id: "ApplyClassifierHeaderSecret".to_owned(),
            payload: serde_json::to_value(ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                accept_digest: hiroute_application_api::CanonicalDigest::of(&spec).unwrap(),
                spec,
                expected_revisions: hiroute_application_api::RevisionSetV1 {
                    target: 0,
                    dependencies: Default::default(),
                },
                idempotency_key: "classifier-secret-local-dispatch".into(),
                apply_capability: None,
            })
            .unwrap(),
            protected_grant: None,
        });
        assert_eq!(response.error.unwrap().code, ErrorCode::InvalidArguments);
        assert_eq!(probe.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn classifier_secret_save_commits_through_local_control() {
        #[cfg(unix)]
        if crate::test_support::isolated_agent_home(
            "control::tests::classifier_secret_save_commits_through_local_control",
        ) {
            return;
        }
        use hiroute_application_api::{
            ApplyRequestV1, ComputeCandidateRefV2, PreviewRequestV1, PreviewResultV1,
        };
        use hiroute_domain::{CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, ProtectedSecret};

        let dir = crate::test_support::private_tempdir();
        let runtime = runtime::ProductionControlRuntime::open_with_release_catalog(
            dir.path().join("storage"),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let input_slot = "candidate/native/classifier-secret-test";
        runtime
            .register_manual_protected_input(
                ComputeCandidateRefV2 {
                    candidate_ref: input_slot.into(),
                    candidate_revision: 1,
                },
                ProtectedSecret::new(b"Bearer isolated-fixture".to_vec()).unwrap(),
            )
            .unwrap();
        let daemon = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "routing.classifier.secret.apply".into(),
            resource_id: Some("personal/default".into()),
            desired_state: json!({
                "secret_id": "classifier/integration-fixture",
                "input_slot": input_slot,
                "expected_generation": 0,
            }),
        };
        let preview = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "classifier-secret-preview".into(),
            operation_id: "ApplyClassifierHeaderSecret".into(),
            payload: serde_json::to_value(PreviewRequestV1::new(spec)).unwrap(),
            protected_grant: None,
        });
        assert!(preview.error.is_none(), "{preview:?}");
        let preview: PreviewResultV1 = serde_json::from_value(preview.data.unwrap()).unwrap();
        let applied = daemon.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "classifier-secret-apply".into(),
            operation_id: "ApplyClassifierHeaderSecret".into(),
            payload: serde_json::to_value(ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                spec: preview.normalized_spec,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: "classifier-secret-apply".into(),
                apply_capability: None,
            })
            .unwrap(),
            protected_grant: None,
        });
        assert!(applied.error.is_none(), "{applied:?}");
        assert_eq!(applied.data.unwrap()["state"], "succeeded");
    }

    #[test]
    fn legacy_local_control_hello_is_rejected_with_the_current_machine_envelope() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("legacy-rejected.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            serve_listener(
                listener,
                LocalControlDaemon::new(ApplicationService::default()),
                Some(1),
            )
            .unwrap();
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        write_frame(
            &mut stream,
            &ClientHelloV1 {
                api_version: hiroute_domain::SchemaVersion::new(1, 0),
                machine_schema_version: hiroute_domain::SchemaVersion::new(1, 0),
                client_name: "legacy-client".into(),
                client_version: env!("CARGO_PKG_VERSION").into(),
            },
        )
        .unwrap();
        let mut reader = BufReader::new(stream);
        let response = read_frame(&mut reader, Instant::now() + Duration::from_secs(1)).unwrap();
        let envelope: MachineEnvelopeV2<serde_json::Value> =
            serde_json::from_str(&response).unwrap();
        assert_eq!(envelope.schema_version, MACHINE_ENVELOPE_SCHEMA_V2);
        assert_eq!(envelope.error.unwrap().code, ErrorCode::SchemaIncompatible);
        server.join().unwrap();
    }

    #[test]
    fn stalled_client_does_not_block_a_concurrent_healthy_client() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("concurrent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            serve_listener(
                listener,
                LocalControlDaemon::new(ApplicationService::default()),
                Some(2),
            )
            .unwrap();
        });
        let mut stalled = UnixStream::connect(&socket).unwrap();
        stalled.write_all(b"{").unwrap();

        let started = Instant::now();
        let mut healthy = UnixStream::connect(&socket).unwrap();
        write_frame(
            &mut healthy,
            &ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: "health-test".to_owned(),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .unwrap();
        let mut reader = BufReader::new(healthy.try_clone().unwrap());
        let hello = read_frame(&mut reader, Instant::now() + Duration::from_secs(1)).unwrap();
        let hello: hiroute_application_api::ServerHelloV1 = serde_json::from_str(&hello).unwrap();
        assert_eq!(hello.daemon_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(hello.release_version, env!("CARGO_PKG_VERSION"));
        write_frame(
            &mut healthy,
            &LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "healthy".to_owned(),
                operation_id: "ListSchemas".to_owned(),
                payload: json!({}),
                protected_grant: None,
            },
        )
        .unwrap();
        let response = read_frame(&mut reader, Instant::now() + Duration::from_secs(1)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap()["status"],
            "succeeded"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(stalled);
        server.join().unwrap();
    }
}
