#[cfg(windows)]
use hiroute_platform::windows::pipe::{Listener as LocalListener, Stream as LocalStream};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(all(test, unix))]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as LocalListener, UnixStream as LocalStream};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use hiroute_application_api::{
    STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1, STANDALONE_PROTECTED_INPUT_REQUEST_SCHEMA_V1,
    STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1, StandaloneProtectedInputRequestV1,
    StandaloneProtectedInputResponseV1,
};
use hiroute_domain::ProtectedSecret;
use hiroute_host_runtime::StandaloneLayout;
use zeroize::{Zeroize, Zeroizing};

const IO_TIMEOUT: Duration = Duration::from_secs(2);

mod bootstrap;
pub(super) use bootstrap::{apply_standalone_defaults, run_all_standalone};

#[cfg(windows)]
mod windows;

/// Local credential input shared by standalone and Desktop-owned role-all processes.
pub(super) struct ProtectedInputServer {
    listener: LocalListener,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    lifecycle: Option<windows::LifecycleServer>,
    #[cfg(windows)]
    shutdown: Option<std::sync::mpsc::Receiver<()>>,
}

impl ProtectedInputServer {
    pub(super) fn bind(layout: &StandaloneLayout) -> Result<Self, String> {
        let server = Self::bind_path(layout.protected_input_socket())?;
        #[cfg(windows)]
        let server = {
            let mut server = server;
            server.lifecycle = Some(windows::LifecycleServer::bind(layout)?);
            // Register console shutdown before publishing the daemon readiness frame.
            server.shutdown = Some(windows::shutdown_signal()?);
            server
        };
        Ok(server)
    }

    pub(super) fn bind_path(path: PathBuf) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or("protected input socket parent is unavailable")?;
        validate_private_directory(parent)?;
        remove_stale_socket(&path)?;
        let listener =
            LocalListener::bind(&path).map_err(|_| "protected input socket bind failed")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "protected input socket permissions failed")?;
        }
        listener
            .set_nonblocking(true)
            .map_err(|_| "protected input socket setup failed")?;
        Ok(Self {
            listener,
            #[cfg(unix)]
            path,
            #[cfg(windows)]
            lifecycle: None,
            #[cfg(windows)]
            shutdown: None,
        })
    }

    pub(super) fn poll(&self, role: &hiroute_daemon::RoleAllHandle) -> Result<(), String> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                // Reject one malformed/unauthorized request without stopping the service.
                let _ = handle_connection(role, stream);
                Ok(())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(_) => Err("protected input socket accept failed".into()),
        }
    }

    pub(super) fn run(
        #[allow(unused_mut)] mut self,
        mut role: hiroute_daemon::RoleAllHandle,
        mut console: Option<hiroute_daemon::console::ConsoleService>,
        #[cfg(windows)] service: Option<hiroute_platform::windows::service::ServiceShutdown>,
    ) -> Result<(), String> {
        let result = (|| {
            #[cfg(unix)]
            let shutdown = Some(shutdown_signal()?);
            #[cfg(windows)]
            let shutdown = if service.is_some() {
                None
            } else {
                Some(
                    self.shutdown
                        .take()
                        .ok_or("standalone signal handler unavailable")?,
                )
            };
            loop {
                #[cfg(windows)]
                if service
                    .as_ref()
                    .is_some_and(|service| service.is_requested())
                {
                    break Ok(());
                }
                #[cfg(windows)]
                if let Some(server) = self.lifecycle.as_ref()
                    && server.poll_stop(&role)?
                {
                    break Ok(());
                }
                if shutdown
                    .as_ref()
                    .is_some_and(|shutdown| shutdown.try_recv().is_ok())
                {
                    break Ok(());
                }
                self.poll(&role)?;
                std::thread::sleep(Duration::from_millis(20));
            }
        })();
        if let Some(console) = console.as_mut() {
            console.shutdown();
        }
        role.shutdown();
        let joined = role
            .join(Duration::from_secs(30))
            .map_err(|error| error.to_string());
        result.and(joined)
    }
}

impl Drop for ProtectedInputServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_connection(
    role: &hiroute_daemon::RoleAllHandle,
    mut stream: LocalStream,
) -> Result<(), String> {
    let mut request = read_request(&mut stream)?;
    match request.action.as_str() {
        "register" => {
            if request.candidate_revision != Some(1) {
                return Err("protected input request is invalid".into());
            }
            let candidate_ref = request
                .candidate_ref
                .take()
                .filter(|value| valid_candidate_ref(value))
                .ok_or("protected input candidate is invalid")?;
            let mut secret = request.secret.take().ok_or("protected input is missing")?;
            if secret.is_empty() || secret.len() > 32 * 1024 {
                secret.zeroize();
                return Err("protected input is invalid".into());
            }
            let protected = ProtectedSecret::new(secret.as_bytes().to_vec())
                .map_err(|_| "protected input is invalid")?;
            secret.zeroize();
            role.register_manual_protected_input(
                hiroute_application_api::ComputeCandidateRefV2 {
                    candidate_ref: candidate_ref.clone(),
                    candidate_revision: 1,
                },
                protected,
            )?;
            let response = write_response(
                &mut stream,
                &StandaloneProtectedInputResponseV1 {
                    schema: STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1.into(),
                    action: "register".into(),
                    registered: true,
                },
            );
            if response.is_err() {
                let _ = role.release_manual_protected_input(&candidate_ref);
            }
            response?;
            response_acknowledgement(&mut stream)
        }
        "release" => {
            if request.candidate_revision.is_some() || request.secret.is_some() {
                return Err("protected input release is invalid".into());
            }
            let candidate_ref = request
                .candidate_ref
                .take()
                .filter(|value| valid_candidate_ref(value))
                .ok_or("protected input candidate is invalid")?;
            role.release_manual_protected_input(&candidate_ref)?;
            write_response(
                &mut stream,
                &StandaloneProtectedInputResponseV1 {
                    schema: STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1.into(),
                    action: "release".into(),
                    registered: true,
                },
            )?;
            response_acknowledgement(&mut stream)
        }
        _ => Err("protected input action is invalid".into()),
    }
}

fn read_request(stream: &mut LocalStream) -> Result<StandaloneProtectedInputRequestV1, String> {
    // BSD sockets can inherit O_NONBLOCK from the polling listener. The bounded
    // frame reader needs a blocking accepted stream for its I/O timeout to apply.
    #[cfg(unix)]
    stream
        .set_nonblocking(false)
        .map_err(|_| "protected input blocking setup failed")?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "protected input read timeout setup failed")?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "protected input write timeout setup failed")?;
    #[cfg(windows)]
    stream
        .set_deadline(Some(std::time::Instant::now() + IO_TIMEOUT))
        .map_err(|_| "protected input deadline setup failed")?;
    #[cfg(unix)]
    let mut bytes = Zeroizing::new(Vec::new());
    #[cfg(unix)]
    Read::by_ref(stream)
        .take(STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "protected input request read failed")?;
    #[cfg(windows)]
    let bytes = hiroute_platform::local_record::read(
        stream,
        hiroute_platform::local_record::Channel::ProtectedInput,
        hiroute_platform::local_record::Direction::Request,
        STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
    )
    .map_err(|_| "protected input request read failed")?;
    if bytes.is_empty() || bytes.len() as u64 > STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 {
        return Err("protected input request size is invalid".into());
    }
    let request: StandaloneProtectedInputRequestV1 =
        serde_json::from_slice(&bytes).map_err(|_| "protected input request is invalid")?;
    if request.schema != STANDALONE_PROTECTED_INPUT_REQUEST_SCHEMA_V1 {
        return Err("protected input request schema is invalid".into());
    }
    Ok(request)
}

fn write_response(
    stream: &mut LocalStream,
    response: &StandaloneProtectedInputResponseV1,
) -> Result<(), String> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(response).map_err(|_| "protected input response is invalid")?,
    );
    #[cfg(unix)]
    {
        let mut bytes = bytes;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .and_then(|_| stream.flush())
            .map_err(|_| "protected input response write failed".into())
    }
    #[cfg(windows)]
    hiroute_platform::local_record::write(
        stream,
        hiroute_platform::local_record::Channel::ProtectedInput,
        hiroute_platform::local_record::Direction::Response,
        &bytes,
        STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
    )
    .map_err(|_| "protected input response write failed".into())
}

fn response_acknowledgement(stream: &mut LocalStream) -> Result<(), String> {
    #[cfg(unix)]
    let _ = stream;
    #[cfg(windows)]
    hiroute_platform::local_record::await_acknowledgement(stream)
        .map_err(|_| "protected input response acknowledgement failed")?;
    // An absent ACK cannot revoke a registration whose successful response
    // may already have been read. The caller can release it explicitly.
    Ok(())
}

fn valid_candidate_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn validate_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return hiroute_platform::windows::security::open_owner_only(path, true)
        .map(|_| ())
        .map_err(|_| "protected input socket directory is unsafe".into());
    #[cfg(unix)]
    {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "protected input socket directory is unavailable")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("protected input socket directory is unsafe".into());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn remove_stale_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt;
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("protected input socket state is unavailable".into()),
        Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)
            .map_err(|_| "protected input stale socket cannot be removed".into()),
        Ok(_) => Err("protected input socket path conflicts with another entry".into()),
    }
}

#[cfg(windows)]
fn remove_stale_socket(path: &Path) -> Result<(), String> {
    // The logical locator never becomes a filesystem socket on Windows.
    if std::fs::symlink_metadata(path).is_ok() {
        return Err("protected input socket path conflicts with another entry".into());
    }
    Ok(())
}

#[cfg(unix)]
fn shutdown_signal() -> Result<Receiver<()>, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("hiroute-standalone-signals".into())
        .spawn(move || {
            use tokio::signal::unix::{SignalKind, signal};
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let (Ok(mut terminate), Ok(mut interrupt)) = (
                    signal(SignalKind::terminate()),
                    signal(SignalKind::interrupt()),
                ) else {
                    return;
                };
                tokio::select! {
                    _ = terminate.recv() => {}
                    _ = interrupt.recv() => {}
                }
                let _ = sender.try_send(());
            });
        })
        .map_err(|_| "standalone signal handler unavailable")?;
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn protected_input_waits_for_fragmented_request_from_connected_client() {
        let directory = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap()).unwrap();
        let server = ProtectedInputServer::bind_path(directory.path().join("input.sock")).unwrap();
        let mut client = UnixStream::connect(&server.path).unwrap();
        let (mut stream, _) = server.listener.accept().unwrap();
        // Exercise the BSD inherited flag on every Unix test host.
        stream.set_nonblocking(true).unwrap();
        let (started, ready) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            started.send(()).unwrap();
            read_request(&mut stream)
        });
        ready.recv_timeout(IO_TIMEOUT).unwrap();
        let frame = br#"{"schema":"hiroute.standalone-protected-input-request/v1","action":"register","candidate_ref":"candidate/manual","candidate_revision":1,"secret":"isolated-fixture"}"#;
        std::thread::sleep(Duration::from_millis(50));
        let first = client.write_all(&frame[..frame.len() / 2]);
        std::thread::sleep(Duration::from_millis(50));
        let second = client.write_all(&frame[frame.len() / 2..]);
        let eof = client.shutdown(std::net::Shutdown::Write);
        let request = reader.join().unwrap().unwrap();
        first.unwrap();
        second.unwrap();
        eof.unwrap();
        assert_eq!(request.candidate_ref.as_deref(), Some("candidate/manual"));
        assert_eq!(request.candidate_revision, Some(1));
        assert_eq!(request.secret.as_deref(), Some("isolated-fixture"));
    }

    #[test]
    #[cfg(unix)]
    fn protected_input_stalled_connected_client_retains_read_timeout() {
        let directory = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap()).unwrap();
        let server = ProtectedInputServer::bind_path(directory.path().join("input.sock")).unwrap();
        let _client = UnixStream::connect(&server.path).unwrap();
        let (mut stream, _) = server.listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            read_request(&mut stream).unwrap_err(),
            "protected input request read failed"
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < IO_TIMEOUT + Duration::from_secs(3));
    }

    #[test]
    fn protocol_rejects_authority_fields_and_invalid_candidate_shapes() {
        assert!(valid_candidate_ref("candidate/manual:one"));
        assert!(!valid_candidate_ref("candidate with spaces"));
        let request: Result<StandaloneProtectedInputRequestV1, _> = serde_json::from_slice(
            br#"{"schema":"hiroute.standalone-protected-input-request/v1","action":"register","candidate_ref":"candidate/manual","candidate_revision":1,"secret":"x","operation_kind":"ApplyAgentPlanChange"}"#,
        );
        assert!(request.is_err());
    }

    #[test]
    fn stale_socket_cleanup_refuses_regular_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("protected-input.sock");
        std::fs::write(&path, b"unrelated").unwrap();
        assert!(remove_stale_socket(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"unrelated");
    }

    #[test]
    #[cfg(unix)]
    fn protected_input_fits_a_runtime_root_that_supports_local_control() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap()).unwrap();
        let prefix = directory.path().as_os_str().as_encoded_bytes().len();
        // control.sock fits macOS's 104-byte sockaddr_un including the trailing NUL.
        // The old protected-input-v1.sock name exceeded it in ordinary Desktop roots.
        let runtime = directory.path().join("r".repeat(82 - prefix - 1));
        let private = runtime.join("hiroute");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _control = UnixListener::bind(private.join("control.sock")).unwrap();
        let mut layout = StandaloneLayout::from_values(
            true,
            Some(directory.path().to_owned()),
            None,
            None,
            None,
        )
        .unwrap();
        layout.runtime_root = runtime;
        let server = ProtectedInputServer::bind(&layout).unwrap();
        let _client = UnixStream::connect(layout.protected_input_socket()).unwrap();
        let (_peer, _) = server.listener.accept().unwrap();
        drop(server);
        assert!(!layout.protected_input_socket().exists());
    }
}
