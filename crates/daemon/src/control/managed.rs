//! Bounded Local Control listener lifetime for the role=all composition root.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hiroute_application::ApplicationService;
use hiroute_application_api::{
    AgentGrantRawRequestV1, MAX_AGENT_GRANT_CONNECTION_ID_BYTES_V1, MAX_AGENT_GRANT_TOKEN_BYTES_V1,
};

use super::{
    AgentGrantResolverPort, ControlConnectionAdmission, ControlConnectionBudgets, EndpointGuard,
    LocalControlDaemon, bind_listener, same_uid, spawn_control_connection,
};

const ACCEPT_POLL: Duration = Duration::from_millis(20);
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedControlPhase {
    Ready,
    ShutdownRequested,
    Terminated,
    Failed,
}

pub struct ManagedControlHandle {
    endpoint: super::ControlEndpoint,
    phase: Arc<std::sync::Mutex<ManagedControlPhase>>,
    shutdown: Arc<AtomicBool>,
    completion: std::sync::mpsc::Receiver<Result<(), String>>,
    worker: Option<std::thread::JoinHandle<()>>,
    outcome: Option<Result<(), String>>,
}

impl ManagedControlHandle {
    pub fn endpoint(&self) -> &super::ControlEndpoint {
        &self.endpoint
    }

    pub fn phase(&self) -> ManagedControlPhase {
        *self.phase.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let mut phase = self.phase.lock().unwrap_or_else(|error| error.into_inner());
        if *phase == ManagedControlPhase::Ready {
            *phase = ManagedControlPhase::ShutdownRequested;
        }
    }

    pub fn join(&mut self, timeout: Duration) -> Result<(), ManagedControlError> {
        if let Some(outcome) = self.outcome.clone() {
            return outcome.map_err(ManagedControlError::Worker);
        }
        let outcome = self
            .completion
            .recv_timeout(timeout)
            .map_err(|error| match error {
                std::sync::mpsc::RecvTimeoutError::Timeout => ManagedControlError::JoinTimeout,
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    ManagedControlError::Worker("Local Control worker disconnected".to_owned())
                }
            })?;
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            return Err(ManagedControlError::Worker(
                "Local Control worker panicked".to_owned(),
            ));
        }
        self.outcome = Some(outcome.clone());
        outcome.map_err(ManagedControlError::Worker)
    }
}

impl Drop for ManagedControlHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start_control(
    application: ApplicationService,
    runtime_root: impl AsRef<Path>,
) -> Result<ManagedControlHandle, ManagedControlError> {
    start_control_inner(application, runtime_root.as_ref(), None, false, None)
}

pub(crate) fn start_control_with_agent_grants(
    application: ApplicationService,
    runtime_root: impl AsRef<Path>,
    resolver: Arc<dyn AgentGrantResolverPort>,
    released_commands_only: bool,
    upgrade: Option<Arc<hiroute_host_runtime::UpgradeDrain>>,
) -> Result<ManagedControlHandle, ManagedControlError> {
    start_control_inner(
        application,
        runtime_root.as_ref(),
        Some(resolver),
        released_commands_only,
        upgrade,
    )
}

fn start_control_inner(
    application: ApplicationService,
    runtime_root: &Path,
    resolver: Option<Arc<dyn AgentGrantResolverPort>>,
    released_commands_only: bool,
    upgrade: Option<Arc<hiroute_host_runtime::UpgradeDrain>>,
) -> Result<ManagedControlHandle, ManagedControlError> {
    let (listener, guard, endpoint) =
        bind_listener(runtime_root).map_err(ManagedControlError::Start)?;
    listener
        .set_nonblocking(true)
        .map_err(|error| ManagedControlError::Start(error.to_string()))?;
    let raw = resolver
        .map(|resolver| {
            let (listener, guard) = bind_agent_grant_listener(runtime_root)?;
            listener
                .set_nonblocking(true)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((listener, guard, resolver))
        })
        .transpose()
        .map_err(ManagedControlError::Start)?;
    let daemon = if released_commands_only {
        LocalControlDaemon::new(application).with_released_commands_only()
    } else {
        LocalControlDaemon::new(application)
    };
    let daemon = if let Some(upgrade) = upgrade {
        daemon.with_upgrade_drain(upgrade)
    } else {
        daemon
    };
    let phase = Arc::new(std::sync::Mutex::new(ManagedControlPhase::Ready));
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_phase = Arc::clone(&phase);
    let worker_shutdown = Arc::clone(&shutdown);
    let (completion_tx, completion) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("hiroute-local-control".to_owned())
        .spawn(move || {
            let budgets = ControlConnectionBudgets::default();
            let (raw_listener, raw_guard, resolver) = match raw {
                Some((listener, guard, resolver)) => (Some(listener), Some(guard), Some(resolver)),
                None => (None, None, None),
            };
            let result = run_listener(
                listener,
                raw_listener,
                resolver,
                daemon,
                &worker_shutdown,
                &budgets,
            );
            drop(raw_guard);
            drop(guard);
            let mut phase = worker_phase
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *phase = if result.is_ok() {
                ManagedControlPhase::Terminated
            } else {
                ManagedControlPhase::Failed
            };
            drop(phase);
            let _ = completion_tx.send(result);
        })
        .map_err(|error| ManagedControlError::Start(error.to_string()))?;
    Ok(ManagedControlHandle {
        endpoint,
        phase,
        shutdown,
        completion,
        worker: Some(worker),
        outcome: None,
    })
}

fn run_listener(
    listener: std::os::unix::net::UnixListener,
    raw_listener: Option<std::os::unix::net::UnixListener>,
    resolver: Option<Arc<dyn AgentGrantResolverPort>>,
    daemon: LocalControlDaemon,
    shutdown: &AtomicBool,
    budgets: &ControlConnectionBudgets,
) -> Result<(), String> {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if !same_uid(&stream)? {
                    continue;
                }
                // Some Unix implementations inherit O_NONBLOCK from the listener. Frame reads
                // use an absolute socket deadline and therefore require a blocking accepted fd.
                stream
                    .set_nonblocking(false)
                    .map_err(|error| error.to_string())?;
                let Some(admission) = budgets.try_accept() else {
                    continue;
                };
                spawn_control_connection(daemon.clone(), stream, admission)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.to_string()),
        }
        if let (Some(raw_listener), Some(resolver)) = (&raw_listener, &resolver) {
            match raw_listener.accept() {
                Ok((stream, _)) => {
                    if !same_uid(&stream)? {
                        continue;
                    }
                    stream
                        .set_nonblocking(false)
                        .map_err(|error| error.to_string())?;
                    let Some(admission) = budgets.try_accept() else {
                        continue;
                    };
                    let resolver = Arc::clone(resolver);
                    std::thread::Builder::new()
                        .name("hiroute-agent-grant-connection".to_owned())
                        .spawn(move || {
                            let _ =
                                serve_agent_grant_connection(resolver.as_ref(), stream, admission);
                        })
                        .map_err(|error| {
                            format!("Agent grant connection thread cannot start: {error}")
                        })?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        std::thread::sleep(ACCEPT_POLL);
    }
    let drain_deadline = Instant::now() + CONNECTION_DRAIN_TIMEOUT;
    while budgets.active_total() != 0 {
        if Instant::now() >= drain_deadline {
            return Err("Local Control connections did not drain before deadline".to_owned());
        }
        std::thread::sleep(ACCEPT_POLL);
    }
    Ok(())
}

fn bind_agent_grant_listener(
    runtime_root: &Path,
) -> Result<(std::os::unix::net::UnixListener, EndpointGuard), String> {
    use std::fs;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    let path = runtime_root.join("hiroute/agent-grant-v1.sock");
    let parent = path
        .parent()
        .ok_or_else(|| "Agent grant endpoint has no parent".to_owned())?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if parent_metadata.file_type().is_symlink()
        || !parent_metadata.file_type().is_dir()
        || parent_metadata.permissions().mode() & 0o777 != 0o700
        || parent_metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        return Err("Agent grant runtime directory is not owner-only".to_owned());
    }
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() || metadata.uid() != nix::unistd::geteuid().as_raw() {
            return Err("refusing to replace an unowned Agent grant endpoint".to_owned());
        }
        match UnixStream::connect(&path) {
            Ok(_) => return Err("an Agent grant server is already active".to_owned()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
        fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        let _ = fs::remove_file(&path);
        return Err("Agent grant endpoint is not owner-only".to_owned());
    }
    Ok((listener, EndpointGuard(path)))
}

fn serve_agent_grant_connection(
    resolver: &dyn AgentGrantResolverPort,
    mut stream: std::os::unix::net::UnixStream,
    mut admission: ControlConnectionAdmission,
) -> Result<(), ()> {
    use std::io::{Read, Write};

    stream
        .set_read_timeout(Some(super::FRAME_TIMEOUT))
        .map_err(|_| ())?;
    stream
        .set_write_timeout(Some(super::FRAME_TIMEOUT))
        .map_err(|_| ())?;
    let mut header = [0_u8; 12];
    stream.read_exact(&mut header).map_err(|_| ())?;
    let length = usize::from(u16::from_be_bytes([header[10], header[11]]));
    if length == 0 || length > MAX_AGENT_GRANT_CONNECTION_ID_BYTES_V1 {
        return Err(());
    }
    let mut frame = Vec::with_capacity(header.len() + length);
    frame.extend_from_slice(&header);
    frame.resize(header.len() + length, 0);
    stream
        .read_exact(&mut frame[header.len()..])
        .map_err(|_| ())?;
    let mut trailing = [0_u8; 1];
    if stream.read(&mut trailing).map_err(|_| ())? != 0 {
        return Err(());
    }
    let request = AgentGrantRawRequestV1::decode(&frame).map_err(|_| ())?;
    admission.classified_non_wait();
    let material = resolver.resolve_agent_grant(request.connection_id())?;
    if material.expose().is_empty() || material.expose().len() > MAX_AGENT_GRANT_TOKEN_BYTES_V1 {
        return Err(());
    }
    stream.write_all(material.expose()).map_err(|_| ())?;
    stream.write_all(b"\n").map_err(|_| ())?;
    stream.flush().map_err(|_| ())
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ManagedControlError {
    #[error("Local Control startup failed: {0}")]
    Start(String),
    #[error("Local Control worker failed: {0}")]
    Worker(String),
    #[error("Local Control join deadline exceeded")]
    JoinTimeout,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    use hiroute_application_api::{
        ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, MACHINE_ENVELOPE_SCHEMA_V2,
    };

    use super::*;

    struct FixtureGrantResolver;

    impl AgentGrantResolverPort for FixtureGrantResolver {
        fn resolve_agent_grant(
            &self,
            connection_id: &str,
        ) -> Result<hiroute_domain::AgentAccessGrantMaterial, ()> {
            if connection_id != "agent-connection/claude" {
                return Err(());
            }
            Ok(hiroute_domain::AgentAccessGrantMaterial::from_csprng_entropy([0x5a; 32]))
        }
    }

    #[test]
    fn managed_control_is_ready_and_removes_endpoint_after_bounded_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let mut handle = start_control(ApplicationService::default(), root.path()).unwrap();
        assert_eq!(handle.phase(), ManagedControlPhase::Ready);
        let endpoint = handle.endpoint().path().to_path_buf();
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        super::super::write_frame(
            &mut stream,
            &ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: "managed-control-test".to_owned(),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .unwrap();
        drop(stream);
        handle.shutdown();
        handle.shutdown();
        handle.join(Duration::from_secs(8)).unwrap();
        assert_eq!(handle.phase(), ManagedControlPhase::Terminated);
        assert!(!endpoint.exists());
    }

    #[test]
    fn raw_grant_socket_is_owner_only_and_failures_are_eof_only() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let mut handle = start_control_with_agent_grants(
            ApplicationService::default(),
            root.path(),
            Arc::new(FixtureGrantResolver),
            false,
            None,
        )
        .unwrap();
        let protected = root.path().join("hiroute");
        let endpoint = protected.join("agent-grant-v1.sock");
        let owner = nix::unistd::geteuid().as_raw();
        let parent_metadata = std::fs::symlink_metadata(&protected).unwrap();
        let endpoint_metadata = std::fs::symlink_metadata(&endpoint).unwrap();
        assert_eq!(parent_metadata.permissions().mode() & 0o777, 0o700);
        assert_eq!(endpoint_metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(parent_metadata.uid(), owner);
        assert_eq!(endpoint_metadata.uid(), owner);

        let mut malformed = AgentGrantRawRequestV1::new("agent-connection/claude")
            .unwrap()
            .encode();
        malformed[9] = 2;
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        stream.write_all(&malformed).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(response.is_empty());

        let request = AgentGrantRawRequestV1::new("agent-connection/claude")
            .unwrap()
            .encode();
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        stream.write_all(&request).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        response.clear();
        stream.read_to_end(&mut response).unwrap();
        let expected = hiroute_domain::AgentAccessGrantMaterial::from_csprng_entropy([0x5a; 32]);
        assert_eq!(response.len(), expected.expose().len() + 1);
        assert_eq!(response.last(), Some(&b'\n'));
        assert_eq!(&response[..response.len() - 1], expected.expose());

        handle.shutdown();
        handle.join(Duration::from_secs(8)).unwrap();
        assert!(!endpoint.exists());
    }
}
