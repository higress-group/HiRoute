use std::path::{Path, PathBuf};
use std::time::Duration;

use hiroute_application_api::{
    AgentGrantRawRequestV1, LocalControlWireRequestV2, MAX_AGENT_GRANT_TOKEN_BYTES_V1,
    MachineEnvelopeV2, STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1,
    STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1, StandaloneProtectedInputRequestV1,
    StandaloneProtectedInputResponseV1,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug)]
pub struct LocalControlClient {
    client_name: &'static str,
    runtime_root: PathBuf,
    timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalControlClientError {
    LocatorUnavailable,
    Transport,
    Protocol,
    ProtectedInput,
}

impl LocalControlClient {
    pub fn cli_from_environment() -> Result<Self, LocalControlClientError> {
        let standalone_root = standalone_runtime_root()?;
        let runtime_root = resolve_runtime_root(
            std::env::var_os("HIROUTE_RUNTIME_DIR"),
            std::env::var_os("XDG_RUNTIME_DIR"),
            std::env::var_os("HOME"),
            cfg!(target_os = "macos"),
            standalone_root,
        )?;
        Ok(Self::new("hiroute-cli", runtime_root))
    }

    /// Future Desktop adapters use this same typed client, never a shell invocation.
    pub fn desktop(runtime_root: impl Into<PathBuf>) -> Self {
        Self::new("hiroute-desktop", runtime_root)
    }

    pub fn new(client_name: &'static str, runtime_root: impl Into<PathBuf>) -> Self {
        Self {
            client_name,
            runtime_root: runtime_root.into(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> PathBuf {
        self.runtime_root.join("hiroute/control.sock")
    }

    pub(crate) fn shared_client(&self) -> hiroute_client_core::Client {
        hiroute_client_core::Client::new(
            self.client_name,
            hiroute_client_core::LocalEndpoint::from_runtime_root(&self.runtime_root),
        )
        .with_timeout(self.timeout)
    }

    pub fn agent_grant_endpoint(&self) -> PathBuf {
        self.runtime_root.join("hiroute/agent-grant-v1.sock")
    }

    pub fn protected_input_endpoint(&self) -> PathBuf {
        hiroute_host_runtime::protected_input_socket(&self.runtime_root)
    }

    #[cfg(unix)]
    pub(crate) fn call_protected_input(
        &self,
        request: StandaloneProtectedInputRequestV1,
    ) -> Result<StandaloneProtectedInputResponseV1, LocalControlClientError> {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        let endpoint = self.protected_input_endpoint();
        validate_protected_endpoint(&endpoint)?;
        let mut stream =
            UnixStream::connect(endpoint).map_err(|_| LocalControlClientError::Transport)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|_| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|_| LocalControlClientError::Transport)?;
        let bytes = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| LocalControlClientError::Protocol)?,
        );
        if bytes.is_empty() || bytes.len() as u64 > STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 {
            return Err(LocalControlClientError::Protocol);
        }
        stream
            .write_all(&bytes)
            .and_then(|_| stream.flush())
            .and_then(|_| stream.shutdown(std::net::Shutdown::Write))
            .map_err(|_| LocalControlClientError::Transport)?;
        let mut response_bytes = Zeroizing::new(Vec::new());
        stream
            .take(STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 + 1)
            .read_to_end(&mut response_bytes)
            .map_err(|_| LocalControlClientError::Transport)?;
        if response_bytes.is_empty()
            || response_bytes.len() as u64 > STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1
        {
            return Err(LocalControlClientError::Protocol);
        }
        let response: StandaloneProtectedInputResponseV1 = serde_json::from_slice(&response_bytes)
            .map_err(|_| LocalControlClientError::Protocol)?;
        if response.schema != STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1 || !response.registered
        {
            return Err(LocalControlClientError::Protocol);
        }
        Ok(response)
    }

    #[cfg(windows)]
    fn protected_stream(
        &self,
        endpoint: &Path,
    ) -> Result<hiroute_platform::windows::pipe::Stream, LocalControlClientError> {
        let deadline = std::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or(LocalControlClientError::Transport)?;
        let stream =
            hiroute_platform::windows::pipe::Stream::connect_timeout(endpoint, self.timeout)
                .map_err(|_| LocalControlClientError::Transport)?;
        stream
            .set_deadline(Some(deadline))
            .map_err(|_| LocalControlClientError::Transport)?;
        Ok(stream)
    }

    #[cfg(windows)]
    pub(crate) fn call_protected_input(
        &self,
        request: StandaloneProtectedInputRequestV1,
    ) -> Result<StandaloneProtectedInputResponseV1, LocalControlClientError> {
        use hiroute_platform::local_record::{self, Channel, Direction};
        let mut stream = self.protected_stream(&self.protected_input_endpoint())?;
        let bytes = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| LocalControlClientError::Protocol)?,
        );
        local_record::write(
            &mut stream,
            Channel::ProtectedInput,
            Direction::Request,
            &bytes,
            STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
        )
        .map_err(|_| LocalControlClientError::Transport)?;
        let bytes = local_record::read(
            &mut stream,
            Channel::ProtectedInput,
            Direction::Response,
            STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
        )
        .map_err(|_| LocalControlClientError::Transport)?;
        let response: StandaloneProtectedInputResponseV1 =
            serde_json::from_slice(&bytes).map_err(|_| LocalControlClientError::Protocol)?;
        if response.schema != STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1
            || !response.registered
            || response.action != request.action
        {
            return Err(LocalControlClientError::Protocol);
        }
        local_record::acknowledge(&mut stream).map_err(|_| LocalControlClientError::Transport)?;
        Ok(response)
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn call_protected_input(
        &self,
        _request: StandaloneProtectedInputRequestV1,
    ) -> Result<StandaloneProtectedInputResponseV1, LocalControlClientError> {
        Err(LocalControlClientError::Transport)
    }

    pub fn call(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<serde_json::Value>, LocalControlClientError> {
        self.call_detailed(request)
            .map_err(|error| match error.code {
                hiroute_client_core::FailureCode::LocatorUnavailable => {
                    LocalControlClientError::LocatorUnavailable
                }
                hiroute_client_core::FailureCode::TransportUnavailable
                | hiroute_client_core::FailureCode::Deadline
                | hiroute_client_core::FailureCode::UnsupportedPlatform => {
                    LocalControlClientError::Transport
                }
                _ => LocalControlClientError::Protocol,
            })
    }

    pub(crate) fn call_detailed(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<serde_json::Value>, hiroute_client_core::ClientFailure> {
        let client = self.shared_client();
        let execute = move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| {
                    hiroute_client_core::ClientFailure::before_send(
                        hiroute_client_core::FailureCode::TransportUnavailable,
                    )
                })?
                .block_on(client.call_wire(request))
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(execute)
                .join()
                .map_err(|_| hiroute_client_core::ClientFailure {
                    code: hiroute_client_core::FailureCode::TransportUnavailable,
                    submission: hiroute_client_core::SubmissionState::MayHaveReachedServer,
                })?
        } else {
            execute()
        }
    }

    #[cfg(unix)]
    pub(crate) fn read_agent_grant(
        &self,
        connection_id: &str,
    ) -> Result<AgentGrantMaterial, LocalControlClientError> {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        let endpoint = self.agent_grant_endpoint();
        validate_protected_endpoint(&endpoint)?;
        let mut stream =
            UnixStream::connect(endpoint).map_err(|_| LocalControlClientError::Transport)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|_| LocalControlClientError::Transport)?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|_| LocalControlClientError::Transport)?;
        let frame = AgentGrantRawRequestV1::new(connection_id)
            .map_err(|_| LocalControlClientError::Protocol)?
            .encode();
        stream
            .write_all(&frame)
            .map_err(|_| LocalControlClientError::Transport)?;
        stream
            .flush()
            .map_err(|_| LocalControlClientError::Transport)?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|_| LocalControlClientError::Transport)?;

        let mut material = AgentGrantMaterial(Vec::with_capacity(64));
        stream
            .take((MAX_AGENT_GRANT_TOKEN_BYTES_V1 + 2) as u64)
            .read_to_end(&mut material.0)
            .map_err(|_| LocalControlClientError::Transport)?;
        if material.0.len() < 2
            || material.0.len() > MAX_AGENT_GRANT_TOKEN_BYTES_V1 + 1
            || material.0.last() != Some(&b'\n')
            || material.0[..material.0.len() - 1]
                .iter()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        {
            return Err(LocalControlClientError::Protocol);
        }
        Ok(material)
    }

    #[cfg(windows)]
    pub(crate) fn read_agent_grant(
        &self,
        connection_id: &str,
    ) -> Result<AgentGrantMaterial, LocalControlClientError> {
        use hiroute_platform::local_record::{self, Channel, Direction};
        let mut stream = self.protected_stream(&self.agent_grant_endpoint())?;
        let frame = AgentGrantRawRequestV1::new(connection_id)
            .map_err(|_| LocalControlClientError::Protocol)?
            .encode();
        local_record::write(
            &mut stream,
            Channel::AgentGrant,
            Direction::Request,
            &frame,
            12 + hiroute_application_api::MAX_AGENT_GRANT_CONNECTION_ID_BYTES_V1,
        )
        .map_err(|_| LocalControlClientError::Transport)?;
        let bytes = local_record::read(
            &mut stream,
            Channel::AgentGrant,
            Direction::Response,
            MAX_AGENT_GRANT_TOKEN_BYTES_V1 + 1,
        )
        .map_err(|_| LocalControlClientError::Transport)?;
        if bytes.len() < 2
            || bytes.last() != Some(&b'\n')
            || bytes[..bytes.len() - 1]
                .iter()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        {
            return Err(LocalControlClientError::Protocol);
        }
        local_record::acknowledge(&mut stream).map_err(|_| LocalControlClientError::Transport)?;
        Ok(AgentGrantMaterial(bytes.to_vec()))
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn read_agent_grant(
        &self,
        _connection_id: &str,
    ) -> Result<AgentGrantMaterial, LocalControlClientError> {
        Err(LocalControlClientError::Transport)
    }
}

pub(crate) struct AgentGrantMaterial(Vec<u8>);

impl AgentGrantMaterial {
    pub(crate) fn write_to(mut self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        writer.write_all(&self.0)?;
        writer.flush()?;
        self.0.fill(0);
        Ok(())
    }
}

impl Drop for AgentGrantMaterial {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

#[cfg(unix)]
fn validate_protected_endpoint(endpoint: &Path) -> Result<(), LocalControlClientError> {
    use std::os::unix::fs::FileTypeExt;

    let parent = endpoint
        .parent()
        .ok_or(LocalControlClientError::LocatorUnavailable)?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| LocalControlClientError::LocatorUnavailable)?;
    let socket_metadata = std::fs::symlink_metadata(endpoint)
        .map_err(|_| LocalControlClientError::LocatorUnavailable)?;
    if !parent_metadata.is_dir()
        || parent_metadata.file_type().is_symlink()
        || !socket_metadata.file_type().is_socket()
    {
        return Err(LocalControlClientError::Protocol);
    }
    Ok(())
}

// Resolve exactly one endpoint. A malformed explicit override must never fall through
// to a different (possibly live) Desktop instance.
fn resolve_runtime_root(
    explicit: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    macos: bool,
    standalone: Option<PathBuf>,
) -> Result<PathBuf, LocalControlClientError> {
    let root = if let Some(root) = explicit {
        PathBuf::from(root)
    } else if let Some(root) = standalone {
        root
    } else if let Some(root) = xdg {
        PathBuf::from(root)
    } else if macos {
        let home = PathBuf::from(home.ok_or(LocalControlClientError::LocatorUnavailable)?);
        if !home.is_absolute() {
            return Err(LocalControlClientError::LocatorUnavailable);
        }
        // Tauri app_local_data_dir for ai.hiroute.desktop on macOS.
        home.join("Library/Application Support/ai.hiroute.desktop/run")
    } else {
        return Err(LocalControlClientError::LocatorUnavailable);
    };
    if !root.is_absolute() {
        return Err(LocalControlClientError::LocatorUnavailable);
    }
    Ok(root)
}

fn standalone_runtime_root() -> Result<Option<PathBuf>, LocalControlClientError> {
    let Ok(layout) = hiroute_host_runtime::StandaloneLayout::from_environment() else {
        return Ok(None);
    };
    match std::fs::symlink_metadata(&layout.marker_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(LocalControlClientError::LocatorUnavailable),
        Ok(metadata) if metadata.is_file() => {
            hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
                .map_err(|_| LocalControlClientError::LocatorUnavailable)?;
            Ok(Some(layout.runtime_root))
        }
        Ok(_) => Err(LocalControlClientError::LocatorUnavailable),
    }
}

pub(crate) fn read_protected_fd(fd: u32) -> Result<String, LocalControlClientError> {
    if fd < 3 {
        return Err(LocalControlClientError::ProtectedInput);
    }
    #[cfg(unix)]
    {
        read_protected_fd_unix(fd)
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        Err(LocalControlClientError::ProtectedInput)
    }
}

/// Inherited secret/capability material is a regular file (`3< KEY_FILE`) or a pipe.
/// Bound both bytes and time; never load `/dev/zero` or wait indefinitely for EOF.
#[cfg(unix)]
fn read_protected_fd_unix(fd: u32) -> Result<String, LocalControlClientError> {
    let denied = || LocalControlClientError::ProtectedInput;
    let mut bytes = crate::unix_bounded_fd::read_unix_fd_bounded(
        fd,
        crate::unix_bounded_fd::UnixFdReadBounds {
            min_fd: 3,
            max_bytes: 512,
            timeout: std::time::Duration::from_secs(2),
        },
    )
    .map_err(|_| denied())?;
    if bytes.is_empty() || bytes.contains(&0) {
        bytes.fill(0);
        return Err(denied());
    }
    match String::from_utf8(bytes) {
        Ok(mut value) => {
            if value.ends_with("\r\n") {
                value.truncate(value.len() - 2);
            } else if value.ends_with('\n') {
                value.pop();
            }
            if value.is_empty() {
                Err(denied())
            } else {
                Ok(value)
            }
        }
        Err(error) => {
            let mut invalid = error.into_bytes();
            invalid.fill(0);
            Err(denied())
        }
    }
}

#[cfg(windows)]
pub(crate) fn read_protected_handle(
    handle: u64,
    limit: usize,
) -> Result<String, LocalControlClientError> {
    let bytes = hiroute_platform::windows::job::read_inherited_credential(
        handle,
        limit,
        std::time::Duration::from_secs(2),
    )
    .map_err(|_| LocalControlClientError::ProtectedInput)?;
    let value = std::str::from_utf8(&bytes).map_err(|_| LocalControlClientError::ProtectedInput)?;
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value);
    if value.is_empty() || value.contains('\0') {
        return Err(LocalControlClientError::ProtectedInput);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod locator_tests {
    use super::*;

    #[test]
    fn desktop_default_and_explicit_isolation() {
        let absolute = |value| match (cfg!(windows), value) {
            (true, "/isolated") => "C:/isolated",
            (true, "/xdg") => "C:/xdg",
            (_, value) => value,
        };
        let resolve = |explicit: Option<&str>, xdg: Option<&str>, macos| {
            resolve_runtime_root(
                explicit.map(Into::into),
                xdg.map(Into::into),
                Some(
                    if cfg!(windows) {
                        "C:/Users/test"
                    } else {
                        "/Users/test"
                    }
                    .into(),
                ),
                macos,
                None,
            )
        };
        assert_eq!(
            resolve(None, None, true).unwrap(),
            PathBuf::from(if cfg!(windows) {
                "C:/Users/test/Library/Application Support/ai.hiroute.desktop/run"
            } else {
                "/Users/test/Library/Application Support/ai.hiroute.desktop/run"
            })
        );
        assert_eq!(
            resolve(Some(absolute("/isolated")), Some(absolute("/xdg")), true).unwrap(),
            PathBuf::from(absolute("/isolated"))
        );
        assert_eq!(
            resolve(None, Some(absolute("/xdg")), true).unwrap(),
            PathBuf::from(absolute("/xdg"))
        );
        for invalid in ["", "relative"] {
            assert_eq!(
                resolve(Some(invalid), Some(absolute("/xdg")), true),
                Err(LocalControlClientError::LocatorUnavailable)
            );
            assert_eq!(
                resolve(None, Some(invalid), true),
                Err(LocalControlClientError::LocatorUnavailable)
            );
        }
        assert_eq!(
            resolve(None, None, false),
            Err(LocalControlClientError::LocatorUnavailable)
        );
        assert_eq!(
            resolve_runtime_root(None, None, Some("relative".into()), true, None),
            Err(LocalControlClientError::LocatorUnavailable)
        );
        assert_eq!(
            resolve_runtime_root(
                None,
                Some("/xdg".into()),
                Some(
                    if cfg!(windows) {
                        "C:/Users/test"
                    } else {
                        "/Users/test"
                    }
                    .into()
                ),
                true,
                Some(PathBuf::from(if cfg!(windows) {
                    "C:/Users/test/Library/Application Support/ai.hiroute.cli/run"
                } else {
                    "/Users/test/Library/Application Support/ai.hiroute.cli/run"
                })),
            )
            .unwrap(),
            PathBuf::from(if cfg!(windows) {
                "C:/Users/test/Library/Application Support/ai.hiroute.cli/run"
            } else {
                "/Users/test/Library/Application Support/ai.hiroute.cli/run"
            })
        );
        assert_eq!(
            resolve_runtime_root(None, None, Some("relative".into()), true, None),
            Err(LocalControlClientError::LocatorUnavailable)
        );
    }
}

#[cfg(all(test, unix))]
mod protected_fd_tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;

    #[test]
    fn protected_fd_accepts_regular_file_and_pipe_within_512_bytes() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"sk-test\n").unwrap();
        file.flush().unwrap();
        use std::io::Seek;
        file.rewind().unwrap();
        assert_eq!(
            read_protected_fd(file.as_raw_fd() as u32).unwrap(),
            "sk-test"
        );

        let (read, write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write, b"pipe-secret").unwrap();
        drop(write);
        assert_eq!(
            read_protected_fd(read.as_raw_fd() as u32).unwrap(),
            "pipe-secret"
        );
    }

    #[test]
    fn protected_fd_rejects_stdio_device_oversize_and_nul() {
        assert_eq!(
            read_protected_fd(0),
            Err(LocalControlClientError::ProtectedInput)
        );
        let zero = std::fs::File::open("/dev/zero").unwrap();
        assert_eq!(
            read_protected_fd(zero.as_raw_fd() as u32),
            Err(LocalControlClientError::ProtectedInput)
        );
        let mut oversize = tempfile::tempfile().unwrap();
        oversize.write_all(&[b'x'; 513]).unwrap();
        oversize.flush().unwrap();
        use std::io::Seek;
        oversize.rewind().unwrap();
        assert_eq!(
            read_protected_fd(oversize.as_raw_fd() as u32),
            Err(LocalControlClientError::ProtectedInput)
        );
        let mut nul = tempfile::tempfile().unwrap();
        nul.write_all(b"abc\0def").unwrap();
        nul.flush().unwrap();
        nul.rewind().unwrap();
        assert_eq!(
            read_protected_fd(nul.as_raw_fd() as u32),
            Err(LocalControlClientError::ProtectedInput)
        );
    }

    fn timeout_under_watchdog(kind: &str, prefix: &[u8]) {
        if let Some(fd) = crate::unix_bounded_fd::watchdog::child_fd() {
            std::process::exit(if read_protected_fd(fd).is_err() { 0 } else { 1 });
        }
        let (read, write) = nix::unistd::pipe().unwrap();
        if !prefix.is_empty() {
            nix::unistd::write(&write, prefix).unwrap();
        }
        let child = crate::unix_bounded_fd::watchdog::spawn(&read, kind);
        crate::unix_bounded_fd::watchdog::wait(child, std::time::Duration::from_secs(5));
        drop(write);
    }

    #[test]
    fn protected_fd_empty_pipe_times_out_under_watchdog() {
        timeout_under_watchdog("protected-empty", b"");
    }

    #[test]
    fn protected_fd_partial_pipe_times_out_under_watchdog() {
        timeout_under_watchdog("protected-partial", b"partial-secret");
    }
}
