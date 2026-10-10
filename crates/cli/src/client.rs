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

    #[cfg(not(unix))]
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
        let client = self.shared_client();
        let execute = move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| LocalControlClientError::Transport)?
                .block_on(client.call_wire(request))
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
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(execute)
                .join()
                .map_err(|_| LocalControlClientError::Transport)?
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

    #[cfg(not(unix))]
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
    let path = Path::new("/dev/fd").join(fd.to_string());
    let bytes = std::fs::read(path).map_err(|_| LocalControlClientError::ProtectedInput)?;
    if bytes.is_empty() || bytes.len() > 512 || bytes.contains(&0) {
        return Err(LocalControlClientError::ProtectedInput);
    }
    let mut value =
        String::from_utf8(bytes).map_err(|_| LocalControlClientError::ProtectedInput)?;
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\n') {
        value.pop();
    }
    if value.is_empty() {
        Err(LocalControlClientError::ProtectedInput)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod locator_tests {
    use super::*;

    #[test]
    fn desktop_default_and_explicit_isolation() {
        let resolve = |explicit: Option<&str>, xdg: Option<&str>, macos| {
            resolve_runtime_root(
                explicit.map(Into::into),
                xdg.map(Into::into),
                Some("/Users/test".into()),
                macos,
                None,
            )
        };
        assert_eq!(
            resolve(None, None, true).unwrap(),
            PathBuf::from("/Users/test/Library/Application Support/ai.hiroute.desktop/run")
        );
        assert_eq!(
            resolve(Some("/isolated"), Some("/xdg"), true).unwrap(),
            PathBuf::from("/isolated")
        );
        assert_eq!(
            resolve(None, Some("/xdg"), true).unwrap(),
            PathBuf::from("/xdg")
        );
        for invalid in ["", "relative"] {
            assert_eq!(
                resolve(Some(invalid), Some("/xdg"), true),
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
                Some("/Users/test".into()),
                true,
                Some(PathBuf::from(
                    "/Users/test/Library/Application Support/ai.hiroute.cli/run"
                )),
            )
            .unwrap(),
            PathBuf::from("/Users/test/Library/Application Support/ai.hiroute.cli/run")
        );
        assert_eq!(
            resolve_runtime_root(None, None, Some("relative".into()), true, None),
            Err(LocalControlClientError::LocatorUnavailable)
        );
    }
}
