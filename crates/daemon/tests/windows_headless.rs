#![cfg(windows)]
//! Real installed binaries on a disposable Windows user profile. No mocked HiRoute runtime.

#[path = "support/windows_business_http.rs"]
mod windows_business_http;
#[path = "support/windows_claude_connection.rs"]
mod windows_claude_connection;
#[path = "support/windows_decision_business.rs"]
mod windows_decision_business;
#[path = "support/windows_execution_host.rs"]
mod windows_execution_host;
#[path = "support/windows_foreground_console.rs"]
mod windows_foreground_console;
#[path = "support/windows_free_admission.rs"]
mod windows_free_admission;
#[path = "support/windows_gateway_foreground_recovery.rs"]
mod windows_gateway_foreground_recovery;
#[path = "support/windows_headless_business.rs"]
mod windows_headless_business;
#[path = "support/windows_management_business.rs"]
mod windows_management_business;
#[path = "support/windows_model_ratings.rs"]
mod windows_model_ratings;
#[path = "support/windows_native_protocols.rs"]
mod windows_native_protocols;
#[path = "support/windows_opencode_business.rs"]
mod windows_opencode_business;
#[path = "support/windows_packaged_foreground.rs"]
mod windows_packaged_foreground;
#[path = "support/windows_qoder_daemon_diagnostic.rs"]
mod windows_qoder_daemon_diagnostic;
#[path = "support/windows_qoder_user_models.rs"]
mod windows_qoder_user_models;
#[path = "support/windows_routing_business.rs"]
mod windows_routing_business;
#[path = "support/windows_routing_modes.rs"]
mod windows_routing_modes;
#[path = "support/windows_subscription_authorization.rs"]
mod windows_subscription_authorization;
#[path = "support/windows_user_model_receipts.rs"]
mod windows_user_model_receipts;
#[path = "support/windows_user_models.rs"]
mod windows_user_models;
#[path = "support/windows_worker_business.rs"]
mod windows_worker_business;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use hiroute_application_api::{
    STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1, StandaloneProtectedInputRequestV1,
    StandaloneProtectedInputResponseV1,
};
use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use hiroute_platform::local_record::{self, Channel, Direction};
use hiroute_platform::windows::{lifecycle, pipe, security};
use serde_json::{Value, json};

const SECRET: &str = "windows_headless_stdin_only_secret_0123456789";
const CANDIDATE: &str = "candidate/native/agent-token-windows-headless";
const IO_BOUND: Duration = Duration::from_secs(5);

pub(crate) struct InstalledProduct {
    layout: StandaloneLayout,
    root: PathBuf,
    cli: PathBuf,
    daemon: PathBuf,
    stable: PathBuf,
    stable_daemon: PathBuf,
    preserve_on_failure: bool,
}

impl InstalledProduct {
    fn create() -> Self {
        windows_execution_host::current();
        let layout = StandaloneLayout::from_environment().unwrap();
        // Native KnownFolders ignore HOME; require a fresh disposable profile.
        assert!(
            !layout.marker_path.exists() && !layout.state_root.exists(),
            "Windows product gate requires a fresh disposable Windows profile"
        );
        let stable = hiroute_host_runtime::standalone_cli_binary(&layout.home.join(".local/bin"));
        let stable_daemon =
            hiroute_host_runtime::standalone_daemon_binary(&layout.home.join(".local/bin"));
        assert!(
            !stable.exists() && !stable_daemon.exists(),
            "product gate must not replace an existing stable entry"
        );
        security::ensure_private_directory(&layout.data_root).unwrap();
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let nonce: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = layout
            .data_root
            .join(format!("Windows headless 测试 {nonce}"));
        security::create_owner_only_directory(&root).unwrap();
        if windows_execution_host::selected() {
            assert!(
                std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR").is_some(),
                "artifact replay requires an explicit verified product binary directory"
            );
        }
        let prepared = std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_BIN_EXE_hirouted"))
                    .parent()
                    .unwrap()
                    .to_owned()
            });
        let source_cli = hiroute_host_runtime::standalone_cli_binary(&prepared);
        let source_daemon = hiroute_host_runtime::standalone_daemon_binary(&prepared);
        windows_execution_host::image(&source_cli);
        windows_execution_host::image(&source_daemon);
        assert!(
            source_cli.is_file() && source_daemon.is_file(),
            "both real CLI and daemon binaries must be built before this product gate"
        );
        let cli = hiroute_host_runtime::standalone_cli_binary(&root);
        let daemon = hiroute_host_runtime::standalone_daemon_binary(&root);
        for (source, target) in [(&source_cli, &cli), (&source_daemon, &daemon)] {
            let mut input = security::open_identity(source, false).unwrap();
            let mut output = security::create_owner_only_file(target).unwrap();
            std::io::copy(&mut input, &mut output).unwrap();
            output.sync_all().unwrap();
            security::verify_owner_only_handle(&output, false).unwrap();
        }
        security::ensure_private_directory(stable.parent().unwrap()).unwrap();
        std::fs::hard_link(&cli, &stable).unwrap();
        std::fs::hard_link(&daemon, &stable_daemon).unwrap();
        let record = StandaloneInstallRecordV1 {
            schema_version: hiroute_host_runtime::STANDALONE_INSTALL_SCHEMA_V1.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            target: hiroute_host_runtime::standalone_host_target(),
            install_root: root.clone(),
            service_definition: root.join("fixture-task.xml"),
            installed_skills: vec![],
            cpa_binary: None,
            cpa_sha256: None,
        };
        record.validate().unwrap();
        let mut marker = security::create_owner_only_file(&layout.marker_path).unwrap();
        serde_json::to_writer(&mut marker, &record).unwrap();
        marker.flush().unwrap();
        Self {
            preserve_on_failure: false,
            layout,
            root,
            cli,
            daemon,
            stable,
            stable_daemon,
        }
    }

    fn start(&self) -> RunningDaemon {
        self.start_with_entry(false)
    }

    fn start_with_entry(&self, cli: bool) -> RunningDaemon {
        self.start_with_environment(cli, &[])
    }

    fn start_with_environment(
        &self,
        cli: bool,
        environment: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> RunningDaemon {
        let mut command = Command::new(if cli { &self.stable } else { &self.daemon });
        let discovery = windows_worker_business::discovery_environment();
        command.envs(discovery.values.iter().cloned());
        command.envs(environment.iter().cloned());
        if cli {
            command.args(["service", "run"]);
        } else {
            command.args(["--role", "all", "--standalone"]);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (send_ready, readiness) = mpsc::sync_channel(1);
        let stdout_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut output = Vec::new();
            loop {
                let mut line = Vec::new();
                if reader.read_until(b'\n', &mut line).unwrap() == 0 {
                    break;
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&line)
                    && value["schema"] == "hiroute.daemon-ready/v1"
                {
                    let _ = send_ready.try_send(value);
                }
                assert!(
                    output.len() + line.len() <= 256 * 1024,
                    "daemon output exceeded gate bound"
                );
                output.extend(line);
            }
            output
        });
        let stderr_reader = thread::spawn(move || {
            let mut output = Vec::new();
            stderr
                .take(256 * 1024 + 1)
                .read_to_end(&mut output)
                .unwrap();
            assert!(
                output.len() <= 256 * 1024,
                "daemon stderr exceeded gate bound"
            );
            output
        });
        let mut running = RunningDaemon {
            child,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            owned_daemon: None,
            listen: "127.0.0.1:1".parse().unwrap(),
        };
        let ready = readiness
            .recv_timeout(Duration::from_secs(45))
            .expect("real standalone daemon must emit its production readiness frame");
        let endpoint = self.layout.lifecycle_endpoint();
        running.owned_daemon =
            Some(lifecycle::retain_for_stop(&endpoint, &self.daemon, IO_BOUND).unwrap());
        let actual = running.owned_daemon.as_ref().unwrap();
        assert_eq!(ready["process_id"].as_u64(), Some(u64::from(actual.pid)));
        assert!(!actual.exited().unwrap());
        if cli {
            assert_ne!(
                actual.pid,
                running.child.id(),
                "service run wrapper must identify the actual daemon separately"
            );
        } else {
            assert_eq!(actual.pid, running.child.id());
        }
        running.listen = ready["gateway_listen"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            ready["control_endpoint"],
            json!(self.layout.runtime_root.join("hiroute/control.sock"))
        );
        running
    }

    fn verify_prepared_installation_refuses_business_startup(&self) {
        use hiroute_diagnostics::files::PrivateDir;
        use hiroute_host_runtime::ServiceProxyEnvironment;
        use hiroute_platform::file_security;
        use std::ffi::{OsStr, OsString};
        use std::io::{Seek, SeekFrom};

        const JOURNAL: &str = "install-journal.json";
        const PROXY: &str = "proxy-environment.json";
        const PROXY_BOUND: u64 = 16 * 1024;

        let service_root = self.layout.data_root.join("service");
        let data = PrivateDir::open_existing(&self.layout.data_root).unwrap();
        let existing_service = data.open_existing_child("service").unwrap();
        let existing_owner = existing_service
            .as_ref()
            .map(|_| security::open_owner_only(&service_root, true).unwrap());
        if let Some(service) = &existing_service {
            assert!(
                service.open_read(JOURNAL).unwrap().is_none(),
                "product fixture must not replace an existing installation journal"
            );
        }

        // A real service run leaves this snapshot. On a fresh profile, use its
        // actual writer; borrow existing snapshots unchanged.
        let owns_proxy = existing_service
            .as_ref()
            .is_none_or(|service| service.open_read(PROXY).unwrap().is_none());
        if owns_proxy {
            ServiceProxyEnvironment::capture(std::iter::empty::<(OsString, OsString)>())
                .unwrap()
                .store(&self.layout.home)
                .unwrap();
            assert_eq!(
                ServiceProxyEnvironment::load(&self.layout.home)
                    .unwrap()
                    .unwrap()
                    .variables()
                    .count(),
                0
            );
        } else {
            assert!(
                ServiceProxyEnvironment::load(&self.layout.home)
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            service_root.is_dir(),
            "the old freshness condition is false"
        );
        let service = existing_service
            .unwrap_or_else(|| data.open_existing_child("service").unwrap().unwrap());
        let owner = existing_owner
            .unwrap_or_else(|| security::open_owner_only(&service_root, true).unwrap());
        let parent_identity = security::file_identity(&owner).unwrap();
        assert!(
            service.open_read(JOURNAL).unwrap().is_none(),
            "only a fresh case-owned installation journal may be created"
        );
        let proxy_identity = service.open_read(PROXY).unwrap().unwrap().identity();
        let mut proxy = security::open_identity_at(&owner, OsStr::new(PROXY), false).unwrap();
        assert_eq!(security::file_link_count(&proxy).unwrap(), 1);
        let proxy_native_identity = security::file_identity(&proxy).unwrap();
        let proxy_permissions = file_security::capture(&proxy).unwrap();
        let mut proxy_bytes = Vec::new();
        (&mut proxy)
            .take(PROXY_BOUND + 1)
            .read_to_end(&mut proxy_bytes)
            .unwrap();
        assert!(proxy_bytes.len() as u64 <= PROXY_BOUND);
        let preserved = |proxy: &mut std::fs::File| {
            service.recheck_identity().unwrap();
            assert_eq!(
                PrivateDir::open_existing(&service_root).unwrap().identity(),
                service.identity()
            );
            let reopened_parent = security::open_owner_only(&service_root, true).unwrap();
            assert_eq!(
                security::file_identity(&reopened_parent).unwrap(),
                parent_identity
            );
            let reopened = security::open_identity_at(&owner, OsStr::new(PROXY), false).unwrap();
            assert_eq!(
                security::file_identity(&reopened).unwrap(),
                proxy_native_identity
            );
            assert!(
                file_security::capture(&reopened).unwrap() == proxy_permissions,
                "maintenance admission must preserve the snapshot's native permissions"
            );
            proxy.seek(SeekFrom::Start(0)).unwrap();
            let mut actual = Vec::new();
            proxy
                .take(PROXY_BOUND + 1)
                .read_to_end(&mut actual)
                .unwrap();
            assert!(
                actual == proxy_bytes,
                "maintenance admission must preserve the snapshot's exact bytes"
            );
        };
        preserved(&mut proxy);
        let mut journal = service.create_new(JOURNAL).unwrap();
        let journal_identity = journal.identity();
        assert!(
            matches!(
                service.create_new(JOURNAL),
                Err(hiroute_diagnostics::files::FileSafetyError::AlreadyExists)
            ),
            "journal creation must retain its exclusive freshness gate"
        );
        journal
            .append(
                &serde_json::to_vec(&json!({
                    "schema": "hiroute.windows-install-journal/v1",
                    "operation": "install",
                    "phase": "prepared",
                    "old_marker": null,
                    "new_marker": null,
                    "old_definition": null,
                    "new_definition": null,
                    "files": []
                }))
                .unwrap(),
            )
            .unwrap();
        journal.sync().unwrap();
        service.sync().unwrap();
        drop(journal);
        // Remove only this fixture's held-identity journal on assertion failure,
        // so a failed admission assertion cannot poison later foreground cases.
        struct JournalCleanup<'a>(&'a PrivateDir, hiroute_diagnostics::files::FileIdentity);
        impl Drop for JournalCleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.remove_verified(JOURNAL, self.1);
            }
        }
        let _journal_cleanup = JournalCleanup(&service, journal_identity);
        // The bare entry first checks the Task dispatcher; the explicit business
        // child checks standalone admission. Both must reject before state creation.
        for (task_runtime, expected) in [
            (false, "Task installation maintenance pending"),
            (true, "standalone installation maintenance is incomplete"),
        ] {
            let mut command = Command::new(&self.daemon);
            command.args(["--role", "all", "--standalone"]);
            if task_runtime {
                command.arg("--windows-task-runtime");
            }
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut rejected = RunningDaemon {
                child,
                stdout_reader: None,
                stderr_reader: None,
                owned_daemon: None,
                listen: "127.0.0.1:1".parse().unwrap(),
            };
            let deadline = Instant::now() + IO_BOUND;
            loop {
                if let Some(status) = rejected.child.try_wait().unwrap() {
                    assert!(!status.success());
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "prepared maintenance must reject before starting the business runtime"
                );
                thread::sleep(Duration::from_millis(10));
            }
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            rejected
                .child
                .stdout
                .take()
                .unwrap()
                .take(4096)
                .read_to_end(&mut stdout)
                .unwrap();
            rejected
                .child
                .stderr
                .take()
                .unwrap()
                .take(4096)
                .read_to_end(&mut stderr)
                .unwrap();
            assert!(
                stdout.is_empty(),
                "rejected admission cannot publish daemon readiness"
            );
            assert_no_secret(&stderr);
            assert!(
                String::from_utf8_lossy(&stderr).contains(expected),
                "actual maintenance denial must identify its rejecting entry: {expected}"
            );
        }
        assert!(!self.layout.state_root.exists());
        assert!(
            lifecycle::query(&self.layout.lifecycle_endpoint(), &self.daemon, IO_BOUND).is_err()
        );
        preserved(&mut proxy);
        service.remove_verified(JOURNAL, journal_identity).unwrap();
        assert!(service.open_read(JOURNAL).unwrap().is_none());
        preserved(&mut proxy);
        drop(proxy);
        if owns_proxy {
            service.remove_verified(PROXY, proxy_identity).unwrap();
        }
        // The parent may predate this case or contain other legitimate files.
        // Only the exact journal (and our own empty snapshot) is removed.
        service.sync().unwrap();
    }

    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(&self.stable)
            .args(args)
            .args(["--output", "json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "actual public CLI failed: {:?}",
            output.status
        );
        assert_no_secret(&output.stdout);
        assert_no_secret(&output.stderr);
        serde_json::from_slice(&output.stdout).expect("public CLI must return JSON")
    }

    fn register_inherited_secret(&self) -> Value {
        self.register_inherited_secret_for(CANDIDATE, SECRET)
    }

    fn register_inherited_secret_for(&self, candidate: &str, secret: &str) -> Value {
        track_secret(secret.as_bytes());
        use hiroute_platform::windows::job::{self, Launch, Stream};
        use std::ffi::OsString;
        use std::os::windows::io::AsRawHandle;
        let (read, mut write) = job::credential_pipe_pair().unwrap();
        let carrier = job::duplicate_inheritable_pipe(&read).unwrap();
        let args = [
            OsString::from("protected-input"),
            "register".into(),
            "--candidate".into(),
            candidate.into(),
            "--secret-handle".into(),
            (carrier.as_raw_handle() as usize as u64).to_string().into(),
            "--output".into(),
            "json".into(),
        ];
        let environment: Vec<_> = std::env::vars_os().collect();
        assert!(
            args.iter()
                .all(|value| !value.to_string_lossy().contains(secret))
        );
        assert!(
            environment
                .iter()
                .all(|(_, value)| !value.to_string_lossy().contains(secret))
        );
        let mut child = job::Child::spawn_with_inherited_pipes(
            Launch {
                program: &self.stable,
                args: &args,
                env: &environment,
                cwd: self.stable.parent(),
                stdin: Stream::Null,
                stdout: Stream::Piped,
                stderr: Stream::Piped,
            },
            &[&carrier],
        )
        .unwrap();
        write.write_all(secret.as_bytes()).unwrap();
        drop(write);
        drop(carrier);
        drop(read);
        let deadline = Instant::now() + Duration::from_secs(35);
        let exit = loop {
            if let Some(exit) = child.exit_code().unwrap() {
                break exit;
            }
            assert!(
                Instant::now() < deadline,
                "actual protected-input CLI did not consume inherited EOF"
            );
            thread::sleep(Duration::from_millis(20));
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(65537)
            .read_to_end(&mut stdout)
            .unwrap();
        child
            .stderr
            .take()
            .unwrap()
            .take(65537)
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(stdout.len() <= 65536 && stderr.len() <= 65536);
        assert_no_secret(&stdout);
        assert_no_secret(&stderr);
        assert_eq!(
            exit, 0,
            "actual protected-input CLI inherited handle failed"
        );
        // Wait for natural Job drainage after root exit; forced cleanup would
        // hide a descendant leak.
        let drained_by = Instant::now() + Duration::from_secs(5);
        while !child.empty().unwrap() {
            assert!(
                Instant::now() < drained_by,
                "actual protected-input CLI leaked descendants"
            );
            thread::sleep(Duration::from_millis(20));
        }
        serde_json::from_slice(&stdout).unwrap()
    }

    fn input(
        &self,
        request: StandaloneProtectedInputRequestV1,
        ack: bool,
    ) -> std::io::Result<StandaloneProtectedInputResponseV1> {
        let mut stream =
            pipe::Stream::connect_timeout(&self.layout.protected_input_socket(), IO_BOUND)?;
        stream.set_deadline(Some(Instant::now() + IO_BOUND))?;
        let body = serde_json::to_vec(&request).unwrap();
        local_record::write(
            &mut stream,
            Channel::ProtectedInput,
            Direction::Request,
            &body,
            STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
        )?;
        let response = local_record::read(
            &mut stream,
            Channel::ProtectedInput,
            Direction::Response,
            STANDALONE_PROTECTED_INPUT_MAX_FRAME_BYTES_V1 as usize,
        )?;
        if ack {
            local_record::acknowledge(&mut stream)?;
        }
        serde_json::from_slice(&response).map_err(std::io::Error::other)
    }
}

impl Drop for InstalledProduct {
    fn drop(&mut self) {
        let _ = lifecycle::stop(
            &self.layout.lifecycle_endpoint(),
            &self.daemon,
            Duration::from_secs(5),
        );
        if std::thread::panicking() {
            windows_qoder_daemon_diagnostic::retain(&self.layout.diagnostics_root());
        }
        if self.preserve_on_failure && std::thread::panicking() {
            // Borrowed settings may reference this installation. Keep the public
            // restore point and exact binaries until recovery has completed.
            eprintln!("Windows borrowed-account failure: installation retained for public restore");
            return;
        }
        let _ = std::fs::remove_file(&self.stable);
        let _ = std::fs::remove_file(&self.stable_daemon);
        let _ = std::fs::remove_file(&self.layout.marker_path);
        let _ = std::fs::remove_dir_all(&self.layout.state_root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct RunningDaemon {
    child: Child,
    owned_daemon: Option<lifecycle::DaemonProcess>,
    stdout_reader: Option<thread::JoinHandle<Vec<u8>>>,
    stderr_reader: Option<thread::JoinHandle<Vec<u8>>>,
    listen: SocketAddr,
}

impl RunningDaemon {
    fn wait(&mut self, success: bool) {
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.success(), success, "unexpected daemon termination");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not terminate within its join bound"
            );
            thread::sleep(Duration::from_millis(20));
        }
        windows_qoder_daemon_diagnostic::collect(&mut self.stdout_reader, false);
        windows_qoder_daemon_diagnostic::collect(&mut self.stderr_reader, true);
    }
}

impl Drop for RunningDaemon {
    fn drop(&mut self) {
        // Stop the retained kernel process before joining pipes left open by its wrapper.
        if let Some(daemon) = &self.owned_daemon
            && daemon.exited().is_ok_and(|exited| !exited)
        {
            let _ = daemon.terminate();
            let _ = daemon.wait(Duration::from_secs(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        windows_qoder_daemon_diagnostic::collect(&mut self.stdout_reader, false);
        windows_qoder_daemon_diagnostic::collect(&mut self.stderr_reader, true);
    }
}

fn assert_no_secret(bytes: &[u8]) {
    assert!(
        !bytes
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes()),
        "protected input leaked into process output"
    );
    if let Some(secrets) = PRIVATE_OUTPUT_SECRETS.get() {
        for secret in secrets.lock().unwrap().iter() {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_slice()),
                "protected runtime material leaked into process output"
            );
        }
    }
}

static PRIVATE_OUTPUT_SECRETS: OnceLock<Mutex<Vec<zeroize::Zeroizing<Vec<u8>>>>> = OnceLock::new();

fn track_secret(secret: &[u8]) {
    assert!((16..=4096).contains(&secret.len()));
    let mut tracked = PRIVATE_OUTPUT_SECRETS
        .get_or_init(Default::default)
        .lock()
        .unwrap();
    if !tracked.iter().any(|existing| existing.as_slice() == secret) {
        assert!(
            tracked.len() < 32,
            "secret fixture tracking exceeded its bound"
        );
        tracked.push(zeroize::Zeroizing::new(secret.to_vec()));
    }
}

fn gateway_ready(listen: SocketAddr) {
    let mut client = TcpStream::connect_timeout(&listen, IO_BOUND).unwrap();
    client.set_read_timeout(Some(IO_BOUND)).unwrap();
    client
        .write_all(b"GET /_hiroute/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    let body_start = response
        .windows(4)
        .position(|value| value == b"\r\n\r\n")
        .unwrap();
    let body: Value = serde_json::from_slice(&response[body_start + 4..]).unwrap();
    assert_eq!(body["schema_version"], "hiroute.gateway.ready/v2");
    assert_eq!(body["status"], "ready");
}

#[test]
fn installed_native_headless_roundtrip_and_graceful_then_forced_restart() {
    let product = InstalledProduct::create();
    product.verify_prepared_installation_refuses_business_startup();
    let mut first = product.start();
    gateway_ready(first.listen);
    assert_eq!(product.cli(&["system", "status"])["status"], "succeeded");
    let owner = lifecycle::query(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        IO_BOUND,
    )
    .unwrap();
    assert_eq!(owner.pid, first.child.id());
    assert!(!owner.exited().unwrap());
    windows_execution_host::daemon(&owner);

    let response = product
        .input(
            StandaloneProtectedInputRequestV1::register(CANDIDATE, SECRET.into()),
            false,
        )
        .unwrap();
    assert!(response.registered);
    // The successful response was consumed, but intentionally has no ACK. A duplicate
    // registration must still reject, proving timeout did not silently revoke its input.
    thread::sleep(Duration::from_millis(2200));
    assert!(
        product
            .input(
                StandaloneProtectedInputRequestV1::register(CANDIDATE, SECRET.into()),
                true
            )
            .is_err()
    );
    assert_eq!(
        product.cli(&["protected-input", "release", "--candidate", CANDIDATE])["data"]["registered"],
        true
    );
    assert!(
        product
            .input(
                StandaloneProtectedInputRequestV1::register(CANDIDATE, SECRET.into()),
                true
            )
            .unwrap()
            .registered
    );
    assert!(
        product
            .input(StandaloneProtectedInputRequestV1::release(CANDIDATE), true)
            .unwrap()
            .registered
    );
    let inherited = product.register_inherited_secret();
    assert_eq!(inherited["status"], "succeeded");
    assert_eq!(inherited["data"]["registered"], true);
    assert_eq!(
        product.cli(&["protected-input", "release", "--candidate", CANDIDATE])["data"]["registered"],
        true
    );

    let grant_endpoint = product
        .layout
        .runtime_root
        .join("hiroute/agent-grant-v1.sock");
    let mut bad_grant = pipe::Stream::connect_timeout(&grant_endpoint, IO_BOUND).unwrap();
    bad_grant
        .set_deadline(Some(Instant::now() + IO_BOUND))
        .unwrap();
    local_record::write(
        &mut bad_grant,
        Channel::AgentGrant,
        Direction::Request,
        b"invalid-grant",
        65536,
    )
    .unwrap();
    assert!(
        local_record::read(
            &mut bad_grant,
            Channel::AgentGrant,
            Direction::Response,
            65536
        )
        .is_err()
    );
    assert_eq!(product.cli(&["system", "status"])["status"], "succeeded");

    let business = windows_headless_business::Business::save(&product);
    let port = first.listen;
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    first.wait(true);
    assert!(owner.exited().unwrap());
    drop(TcpListener::bind(port).expect("graceful RoleAll join must release the listener"));

    let mut second = product.start();
    gateway_ready(second.listen);
    assert_eq!(product.cli(&["system", "status"])["status"], "succeeded");
    business.recovered(&product, 1);
    second.child.kill().unwrap();
    second.wait(false);
    let mut recovered = product.start();
    gateway_ready(recovered.listen);
    assert_eq!(product.cli(&["system", "status"])["status"], "succeeded");
    business.recovered(&product, 2);
    business.delete(&product);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    recovered.wait(true);
    assert!(product.cli.is_file());
}

/// Required separately after exact pinned native clients and their discovery PATH
/// are prepared. Missing artifacts/login/versions are failures, never optional skips.
#[test]
#[ignore = "requires explicit pinned real Codex, Claude, Pi and Node artifacts"]
fn actual_windows_complete_public_headless_business() {
    public_headless_business(true);
}

/// Explicit development subset; never substitutes for the complete release gate.
#[test]
#[ignore = "development business journey excluding native Claude acceptance"]
fn actual_windows_public_headless_business_without_claude() {
    public_headless_business(false);
}

/// Focused reproduction of the existing full journey's rejected large POST.
#[test]
#[ignore = "focused installed Gateway rejection diagnosis"]
fn actual_windows_gateway_rejects_unauthorized_large_request() {
    let product = InstalledProduct::create();
    let mut running = product.start();
    gateway_ready(running.listen);
    let body = json!({"model":"hiroute/0011223344556677","input":[{"role":"user","content":[{"type":"input_text","text":"windows-routing-responses"}]}],"stream":false,"instructions":"bounded Windows route prompt".repeat(4000)});
    for token in [None, Some("invalid-windows-gateway-token")] {
        eprintln!(
            "Windows Gateway rejection stage: token-present={}",
            token.is_some()
        );
        let response = windows_business_http::post_expect_continue(
            running.listen,
            "/v1/responses",
            token,
            &body,
        );
        assert!(matches!(response.status, 401 | 403));
    }
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    running.wait(true);
}

/// Focused execution of the full journey's multi-turn routing assertions.
#[test]
#[ignore = "focused installed smart/custom route diagnosis"]
fn actual_windows_public_routing_modes() {
    windows_routing_modes::focused();
}

/// Bounded payload/credential acceptance, independent of native Agent accounts.
#[test]
#[ignore = "focused installed decision service and C capability acceptance"]
fn actual_windows_decision_services_c_capability() {
    let product = InstalledProduct::create();
    let mut first = product.start();
    gateway_ready(first.listen);
    windows_decision_business::roundtrip(&product);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        IO_BOUND,
    )
    .unwrap();
    first.wait(true);
    let mut reopened = product.start();
    gateway_ready(reopened.listen);
    assert!(
        product.cli(&["decision", "services", "list"])["data"]["services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        IO_BOUND,
    )
    .unwrap();
    reopened.wait(true);
}

fn public_headless_business(include_claude: bool) {
    let product = InstalledProduct::create();
    let contexts = windows_user_models::Contexts::prepare(&product);
    let mut first = product.start_with_environment(false, contexts.variables());
    gateway_ready(first.listen);
    assert_eq!(product.cli(&["system", "status"])["status"], "succeeded");
    let schema = product.cli(&["schema", "list"]);
    let manifest = &schema["data"];
    assert!(
        manifest["descriptor_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let commands = manifest["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 80);
    for command_id in [
        "decision.sources.list",
        "decision.connections.list",
        "decision.connections.apply",
        "decision.connections.test",
        "prices.effective",
        "prices.override.preview",
        "prices.override.apply",
        "compute.defaults.get",
        "compute.defaults.preview",
        "models.metadata.list",
        "models.metadata.get",
    ] {
        assert!(
            commands
                .iter()
                .any(|command| command["command_id"] == command_id)
        );
    }
    let descriptor = product.cli(&["schema", "show", "--command-id", "compute.connection.login"]);
    assert_eq!(descriptor["data"]["command_id"], "compute.connection.login");
    assert_eq!(descriptor["data"]["lifecycle"], "released");

    // A single installed owner/runtime now carries all public business leaves.
    // Referenced sources stay alive because retained plan revisions own them.
    windows_decision_business::roundtrip(&product);
    let routing = windows_routing_business::Routing::setup(&product);
    let modes = windows_routing_modes::Modes::setup(&product);
    let mut native = windows_native_protocols::NativeProtocols::setup_with_messages_plan(
        &product,
        (!include_claude).then_some(routing.plan.plan_id.as_str()),
    );
    let ratings = windows_model_ratings::Ratings::run(&product);
    let mut workers = windows_worker_business::WorkerBusiness::run(
        &product,
        &[
            &routing.plan.plan_id,
            &modes.smart.plan_id,
            &modes.custom.plan_id,
            &native.chat.plan.plan_id,
        ],
    );
    let claude = include_claude.then(|| {
        windows_claude_connection::ClaudeConnection::configure(&product, &routing.plan.plan_id)
    });
    let messages_connection = claude
        .as_ref()
        .map(|client| client.connection_id())
        .unwrap_or_else(|| native.connection_id());
    routing.traffic_and_observe(
        &product,
        first.listen,
        workers.connection_id(),
        &messages_connection,
    );
    if let Some(claude) = &claude {
        claude.launch(&product, &routing.plan.alias);
        routing.assert_native_launch_reached_upstream();
    }
    modes.traffic(&product, first.listen, workers.connection_id());
    native.traffic(&product, first.listen);
    let mut gateway_recovery =
        windows_gateway_foreground_recovery::Recovery::begin(&product, &first);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    first.wait(true);
    gateway_recovery.recover_without_manager(&product);
    let mut business_recovered = product.start_with_environment(true, contexts.variables());
    gateway_ready(business_recovered.listen);
    gateway_recovery.recovered(&product, &business_recovered);
    ratings.recovered(&product);
    modes.recovered(&product, business_recovered.listen, workers.connection_id());
    native.recovered(&product);
    workers.recovered(&product);
    if let Some(claude) = &claude {
        claude.recovered(&product);
    }
    workers.restore(&product);
    if let Some(claude) = claude {
        claude.restore(&product);
    }
    native.restore(&product);

    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    business_recovered.wait(true);
    assert!(product.cli.is_file());
    println!(
        "Windows native Claude acceptance: {}",
        if include_claude {
            "executed"
        } else {
            "excluded"
        }
    );
    println!(
        "Windows public headless: exact pinned selected clients, Compute, Decision, plan preview/apply/list/show, independent Responses/Messages grants, streaming, safe facts/usage/value, selected native launch scope, actual Main Skill to public Worker, durable restart and native settings restore"
    );
}

/// Ordinary native user configuration is distinct from per-run Worker profiles.
/// The selected client roots exist before the real daemon freezes its discovery
/// environment; production public transactions alone write their configuration.
#[test]
#[ignore = "requires explicit pinned real Pi, DSH and Node artifacts"]
fn actual_windows_pi_and_dsh_persisted_user_models() {
    let product = InstalledProduct::create();
    let contexts = windows_user_models::Contexts::prepare(&product);
    let mut first = product.start_with_environment(false, contexts.variables());
    gateway_ready(first.listen);
    let mut models = windows_user_models::UserModels::run(&product, &contexts, first.listen);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    first.wait(true);
    let mut recovered = product.start_with_environment(true, contexts.variables());
    gateway_ready(recovered.listen);
    models.recovered(&product, &contexts);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    recovered.wait(true);
    println!(
        "Windows ordinary Pi/DSH user models: public Preview/Apply, persisted providers, ordinary native Gateway requests, private grant rotation, installed user Skill and real public CLI invocation, actual restart, independent Restore and retained same-request receipts"
    );
}

#[test]
#[ignore = "requires the fixed real Qoder image and official private account login"]
fn actual_windows_qoder_persisted_user_models_with_official_account() {
    let mut product = InstalledProduct::create();
    let contexts = windows_qoder_user_models::Contexts::borrow(&product);
    let mut first = product.start_with_environment(false, contexts.variables());
    gateway_ready(first.listen);
    let mut models =
        windows_qoder_user_models::UserModels::run(&mut product, &contexts, first.listen);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    first.wait(true);
    let mut recovered = product.start_with_environment(true, contexts.variables());
    gateway_ready(recovered.listen);
    models.recovered(&product, &contexts, recovered.listen);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    recovered.wait(true);
    println!(
        "Windows ordinary Qoder user models: official private account, public Source/Plan transactions, saved Responses/Messages providers, actual native requests, grant rotation, independent installed Skill and trusted CLI, durable restart and exact native settings restore"
    );
}

#[test]
#[ignore = "requires explicit pinned real Pi, DSH and Node artifacts"]
fn actual_windows_installed_management_commands() {
    let product = InstalledProduct::create();
    let contexts = windows_user_models::Contexts::prepare(&product);
    let mut daemon = product.start_with_environment(false, contexts.variables());
    gateway_ready(daemon.listen);
    windows_management_business::run(&product, &contexts);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    daemon.wait(true);
}

#[test]
#[ignore = "requires the verified complete production Windows package"]
fn actual_windows_packaged_foreground_subscription_lifecycle() {
    windows_packaged_foreground::run();
}

/// Focused native OpenCode coverage; the standard Windows runner owns execution.
#[test]
#[ignore = "requires the pinned official OpenCode Windows native package"]
fn actual_windows_opencode_worker_native_history_and_cancellation() {
    let product = InstalledProduct::create();
    let context = windows_opencode_business::Context::prepare(&product);
    let mut first = product.start_with_environment(false, context.variables());
    gateway_ready(first.listen);
    let journey = windows_opencode_business::Journey::run(&product);
    context.preserved();
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    first.wait(true);
    let mut recovered = product.start_with_environment(true, context.variables());
    gateway_ready(recovered.listen);
    journey.recovered(&product);
    context.preserved();
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    recovered.wait(true);
    println!(
        "Windows native OpenCode: public Plan and installation selection, actual Skill/tool receipts, Job cancellation, daemon restart and exact Continue; borrowed native configuration preserved"
    );
}
