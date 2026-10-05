// This support module is path-included by multiple integration-test crates, each of which uses a
// different subset of the shared helpers.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use command_fds::{CommandFdExt, FdMapping};
use hiroute_application_api::{
    ClientEmptyRequestV1, ClientServiceStatusV1, PrincipalKind, ProtectedClientGrantV2,
};
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_domain::{CanonicalDigest, RevisionSetV1, WorkspaceId};
use serde::Deserialize;
use serde_json::{Value, json};

pub const SECRET_SENTINEL: &str = "zhipu-product-secret-must-stay-protected";
pub const CORRECT_PROJECT_SETTINGS: &str = r#"{"env":{"ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic","ANTHROPIC_MODEL":"glm-5.3"}}"#;
pub const CHANGED_PROJECT_SETTINGS: &str = r#"{"env":{"ANTHROPIC_BASE_URL":"https://changed.invalid/api/anthropic","ANTHROPIC_MODEL":"glm-5.3"}}"#;

fn prepared_daemon_binary(value: Option<std::ffi::OsString>) -> PathBuf {
    let path =
        PathBuf::from(value.expect("validation execution requires the prepared daemon fixture"));
    assert!(
        path.is_absolute()
            && fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()),
        "prepared daemon fixture must be an absolute regular file"
    );
    path
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    schema: String,
    role: String,
    control_endpoint: PathBuf,
    gateway_listen: SocketAddr,
    process_id: u32,
}

pub struct ProductDaemon {
    child: Option<Child>,
    shutdown: Option<File>,
    capability: File,
    acknowledgement: File,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    pub client: Client,
}

impl ProductDaemon {
    pub fn start(root: &Path, proxy: SocketAddr) -> Self {
        Self::start_inner(root, proxy, None)
    }

    pub fn start_with_cpa(root: &Path, proxy: SocketAddr, binary: &Path, sha256_hex: &str) -> Self {
        Self::start_inner(root, proxy, Some((binary, sha256_hex)))
    }

    fn start_inner(root: &Path, proxy: SocketAddr, cpa: Option<(&Path, &str)>) -> Self {
        let storage = root.join("storage");
        let runtime = root.join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let gateway = reservation.local_addr().unwrap();
        let (shutdown_read, shutdown_write) = pipe();
        let (capability_read, capability_write) = pipe();
        let (acknowledgement_read, acknowledgement_write) = pipe();
        let binary = if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
            prepared_daemon_binary(std::env::var_os("HIROUTE_VALIDATION_DAEMON_BIN"))
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_hirouted"))
        };
        let mut command = Command::new(binary);
        command
            .args(["--role", "all", "--storage-root"])
            .arg(&storage)
            .arg("--runtime-root")
            .arg(&runtime)
            .arg("--listen")
            .arg(gateway.to_string())
            .arg("--lkg")
            .arg(root.join("gateway.lkg"))
            .args([
                "--shutdown-fd",
                "3",
                "--capability-fd",
                "4",
                "--capability-ack-fd",
                "5",
            ])
            .current_dir(root.join("workspace"))
            .env("HOME", root.join("home"))
            .env("PI_CODING_AGENT_DIR", root.join("home/.pi/agent"))
            .env("PATH", root.join("bin"))
            .env("HTTP_PROXY", format!("http://{proxy}"))
            .env("HTTPS_PROXY", format!("http://{proxy}"))
            .env("ALL_PROXY", format!("http://{proxy}"))
            .env("http_proxy", format!("http://{proxy}"))
            .env("https_proxy", format!("http://{proxy}"))
            .env("all_proxy", format!("http://{proxy}"))
            .env_remove("CODEX_HOME")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .env_remove("ANTHROPIC_BASE_URL")
            .env_remove("ANTHROPIC_MODEL")
            .env_remove("ANTHROPIC_DEFAULT_OPUS_MODEL")
            .env_remove("ANTHROPIC_DEFAULT_SONNET_MODEL")
            .env_remove("ANTHROPIC_DEFAULT_HAIKU_MODEL")
            .env_remove("ANTHROPIC_SMALL_FAST_MODEL")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some((binary, sha256_hex)) = cpa {
            command
                .arg("--cpa-binary")
                .arg(binary)
                .arg("--cpa-sha256")
                .arg(sha256_hex);
        }
        command
            .fd_mappings(vec![
                FdMapping {
                    parent_fd: shutdown_read.into(),
                    child_fd: 3,
                },
                FdMapping {
                    parent_fd: capability_read.into(),
                    child_fd: 4,
                },
                FdMapping {
                    parent_fd: acknowledgement_write.into(),
                    child_fd: 5,
                },
            ])
            .unwrap();
        drop(reservation);
        let mut child = command.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        nonblocking(&stdout);
        nonblocking(&capability_write);
        nonblocking(&acknowledgement_read);
        let ready_frame =
            read_frame(&mut stdout, 4096, Duration::from_secs(20)).unwrap_or_else(|error| {
                let status = child.wait().ok();
                let mut diagnostic = String::new();
                let _ = stderr.read_to_string(&mut diagnostic);
                panic!(
                    "hirouted failed before ready: {error}; status={status:?}; stderr={diagnostic}"
                );
            });
        let ready: Ready = serde_json::from_slice(&ready_frame).unwrap();
        let endpoint = LocalEndpoint::for_child(&runtime, child.id());
        assert_eq!(ready.schema, "hiroute.daemon-ready/v1");
        assert_eq!(ready.role, "all");
        assert_eq!(ready.process_id, child.id());
        assert_eq!(ready.control_endpoint, endpoint.path());
        assert_eq!(ready.gateway_listen, gateway);
        Self {
            child: Some(child),
            shutdown: Some(shutdown_write),
            capability: capability_write,
            acknowledgement: acknowledgement_read,
            stdout: Some(stdout),
            stderr: Some(stderr),
            client: Client::new("hiroute-desktop-product-test", endpoint),
        }
    }

    pub async fn revisions(&self, request_id: &str) -> RevisionSetV1 {
        let envelope = self
            .client
            .query::<_, ClientServiceStatusV1>(
                "GetClientServiceStatus",
                request_id,
                &ClientEmptyRequestV1 {},
            )
            .await
            .unwrap();
        assert_eq!(
            envelope.status,
            hiroute_application_api::MachineStatus::Succeeded,
            "{envelope:?}"
        );
        envelope.data.unwrap().revisions
    }

    pub fn register(
        &mut self,
        operation: &str,
        digest: CanonicalDigest,
        revisions: RevisionSetV1,
    ) -> ProtectedClientGrantV2 {
        // Grants are durable, so restarting the daemon must not reuse a capability.
        static NEXT_REGISTRATION: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let serial = NEXT_REGISTRATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let registration_id = format!("{serial:064x}");
        let capability = format!("desktop-product-capability-{serial:064x}");
        let expires_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 120;
        let mut frame = serde_json::to_vec(&json!({
            "schema": "hiroute.protected-apply-grant/v2",
            "registration_id": registration_id,
            "capability": capability,
            "principal_kind": "desktop",
            "workspace_id": WorkspaceId::default(),
            "operation_kind": operation,
            "accepted_digest": digest,
            "expected_revisions": revisions,
            "expires_at_unix": expires_at_unix,
        }))
        .unwrap();
        frame.push(b'\n');
        write_frame(&mut self.capability, &frame, Duration::from_secs(2));
        let acknowledgement: Value = serde_json::from_slice(
            &read_frame(&mut self.acknowledgement, 1024, Duration::from_secs(2)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            acknowledgement,
            json!({
                "schema": "hiroute.protected-apply-ack/v2",
                "registration_id": registration_id,
                "registered": true,
            })
        );
        ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::Desktop,
            capability,
        }
    }

    pub fn stop(mut self) {
        drop(self.shutdown.take());
        let mut child = self.child.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                break child.wait().unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(self.stdout.take());
        let mut stderr = String::new();
        self.stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(
            status.success(),
            "hirouted status={status:?} stderr={stderr}"
        );
        assert!(!stderr.contains(SECRET_SENTINEL), "secret reached stderr");
    }
}

impl Drop for ProductDaemon {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking() {
            let mut stderr = String::new();
            if let Some(mut stream) = self.stderr.take() {
                let _ = stream.read_to_string(&mut stderr);
            }
            if !stderr.is_empty() {
                eprintln!("hirouted stderr after test failure:\n{stderr}");
            }
        }
    }
}

pub fn configure_product_root(root: &Path) {
    let user_settings = root.join("home/.claude/settings.json");
    let project_settings = root.join("workspace/.claude/settings.json");
    let binary_root = root.join("bin");
    fs::create_dir_all(user_settings.parent().unwrap()).unwrap();
    fs::create_dir_all(project_settings.parent().unwrap()).unwrap();
    fs::set_permissions(root.join("home"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        user_settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(
        project_settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::create_dir_all(&binary_root).unwrap();
    fs::write(
        &user_settings,
        serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_AUTH_TOKEN": SECRET_SENTINEL}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&user_settings, fs::Permissions::from_mode(0o600)).unwrap();
    write_project_settings(root, CORRECT_PROJECT_SETTINGS);
    let claude = binary_root.join("claude");
    fs::write(
        &claude,
        format!(
            "#!/bin/sh\nif [ \"$#\" -ne 1 ] || [ \"$1\" != \"--version\" ]; then\n  printf '%s\\n' \"$*\" > '{}'\n  exit 64\nfi\nprintf probed > '{}'\nprintf '%s\\n' '2.1.231 (Claude Code)'\n",
            root.join("claude-unexpected-invocation").display(),
            root.join("claude-version-probed").display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&binary_root, fs::Permissions::from_mode(0o775)).unwrap();
}

pub fn write_project_settings(root: &Path, contents: &str) {
    let path = root.join("workspace/.claude/settings.json");
    fs::write(&path, contents.as_bytes()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

pub fn assert_tree_omits(path: &Path, sentinel: &[u8]) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            assert_tree_omits(&entry.path(), sentinel);
        } else if entry.file_type().unwrap().is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            assert!(!bytes.windows(sentinel.len()).any(|value| value == sentinel));
        }
    }
}

fn pipe() -> (File, File) {
    let (read, write) = nix::unistd::pipe().unwrap();
    for descriptor in [&read, &write] {
        nix::fcntl::fcntl(
            descriptor,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .unwrap();
    }
    (read.into(), write.into())
}

fn nonblocking(file: &impl AsFd) {
    let flags = nix::fcntl::fcntl(file, nix::fcntl::FcntlArg::F_GETFL).unwrap();
    nix::fcntl::fcntl(
        file,
        nix::fcntl::FcntlArg::F_SETFL(
            nix::fcntl::OFlag::from_bits_truncate(flags) | nix::fcntl::OFlag::O_NONBLOCK,
        ),
    )
    .unwrap();
}

fn read_frame(reader: &mut impl Read, limit: usize, timeout: Duration) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("protected frame timed out".into());
        }
        let mut byte = [0_u8];
        match reader.read(&mut byte) {
            Ok(0) => return Err("protected frame closed".into()),
            Ok(_) if byte[0] == b'\n' => return Ok(bytes),
            Ok(_) => {
                if bytes.len() >= limit {
                    return Err("protected frame oversized".into());
                }
                bytes.push(byte[0]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("protected frame read failed: {error}")),
        }
    }
}

fn write_frame(writer: &mut impl Write, mut bytes: &[u8], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !bytes.is_empty() {
        assert!(Instant::now() < deadline, "protected frame write timed out");
        match writer.write(bytes) {
            Ok(0) => panic!("protected frame closed"),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("protected frame write failed: {error}"),
        }
    }
}

#[cfg(test)]
mod validation_fixture_tests {
    use super::*;

    #[test]
    fn prepared_daemon_fixture_rejects_missing_relative_and_linked_paths() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("hirouted");
        fs::write(&binary, b"fixture").unwrap();
        assert_eq!(
            prepared_daemon_binary(Some(binary.clone().into_os_string())),
            binary
        );
        let linked = root.path().join("linked-hirouted");
        std::os::unix::fs::symlink(&binary, &linked).unwrap();
        for invalid in [
            None,
            Some("relative/hirouted".into()),
            Some(linked.into_os_string()),
        ] {
            assert!(std::panic::catch_unwind(|| prepared_daemon_binary(invalid)).is_err());
        }
    }
}
