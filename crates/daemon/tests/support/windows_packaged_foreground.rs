//! Real complete-package foreground CPA acceptance, without a Task/WTS prerequisite.
//! The private fixture marker admits the actual packaged binaries; it does not
//! represent an installer, Task registration, standard owner or healthy manager.
//! No account, callback, token exchange or model inference is fabricated.
#[path = "windows_subscription_login.rs"]
mod login;
#[path = "windows_packaged_proxy_observation.rs"]
mod proxy_observation;

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout, SubscriptionProxyStore};
use hiroute_platform::windows::job::{Child, Launch, Stream};
use hiroute_platform::windows::{lifecycle, security};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::windows_headless_business::cli;
use super::{IO_BOUND, InstalledProduct, windows_execution_host};

const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
const OUTPUT_BOUND: u64 = 1024 * 1024;

/// Required hook: actual_windows_packaged_foreground_subscription_lifecycle.
/// Caller verifies/restages the complete production package before selection.
pub fn run() {
    let package = Packaged::create();
    let product = &package.product;
    let mut first = Foreground::start(product, None);
    super::gateway_ready(first.listen);
    assert!(cli(product, &["system", "status"], None)["status"] == "succeeded");
    let original = proxy_view(product);
    assert!(original["config"]["policy"]["mode"] == "inherit");
    assert_snapshot(product, &original["config"], true);
    authorization_rejections(product);
    let pending = login::run_before_restart(&mut |arguments, request| {
        // This existing leaf supplies its own output/source flags.
        public_login(product, arguments, request)
    });
    checkpoint("pkce-cancel-forget-before-restart");

    // The stock binary starts a fixed version updater without an account. The
    // observer rejects every connection; only the pinned updater origin is an
    // approved CONNECT attempt. It neither decrypts nor forwards the request.
    let proxy = proxy_observation::Observation::new();
    let proxy_url = proxy.url().to_owned();
    let configured = cli(
        product,
        &[
            "subscription-proxy",
            "set",
            "--mode",
            "manual",
            "--url",
            &proxy_url,
            "--no-proxy",
            "example.invalid",
        ],
        None,
    );
    assert!(configured["data"]["action"] == "service_restart_required");
    assert!(configured["data"]["applied"] == false);
    let manual = proxy_view(product);
    assert!(manual["config"] == configured["data"]["config"]);
    assert!(manual["config"]["policy"]["mode"] == "manual");
    assert!(manual["config"]["policy"]["url"] == proxy_url);
    assert!(manual["config"]["policy"]["no_proxy"] == "example.invalid");
    assert!(manual["config"]["revision"] != original["config"]["revision"]);
    assert_snapshot(product, &manual["config"], false);
    first.stop(product);
    checkpoint("pending-stock-cpa-tree-stopped");

    // Windows chooses its actual KnownFolder profile; HOME is not an admission
    // override. Keep this non-profile directory empty and inspect exact lifecycle
    // at the original native layout, not an env-derived synthetic endpoint.
    let foreign_home = product.root.join("unselected-home-override");
    security::create_owner_only_directory(&foreign_home).unwrap();
    proxy.phase(proxy_observation::Phase::ManualStart);
    let mut second = Foreground::start(product, Some(&foreign_home));
    super::gateway_ready(second.listen);
    proxy.phase(proxy_observation::Phase::RestartRecovery);
    login::assert_after_restart(
        &mut |arguments, request| public_login(product, arguments, request),
        pending,
    );
    assert_snapshot(product, &manual["config"], true);
    assert!(proxy_view(product)["config"] == manual["config"]);
    assert!(std::fs::read_dir(&foreign_home).unwrap().next().is_none());
    manual_policy_login(product, &proxy);
    proxy.assert_approved_only();
    checkpoint("restart-expired-and-launch-snapshot");

    let restored = cli(
        product,
        &["subscription-proxy", "set", "--mode", "inherit"],
        None,
    );
    assert!(restored["data"]["action"] == "service_restart_required");
    let desired = proxy_view(product);
    assert!(desired["config"]["policy"] == original["config"]["policy"]);
    assert!(desired["config"]["revision"] != manual["config"]["revision"]);
    assert_snapshot(product, &desired["config"], false);
    second.stop(product);
    proxy.phase(proxy_observation::Phase::RestoreInherited);
    let mut third = Foreground::start(product, None);
    super::gateway_ready(third.listen);
    assert_snapshot(product, &desired["config"], true);
    assert!(proxy_view(product)["config"] == desired["config"]);
    proxy.assert_approved_only();
    package.verify_bytes();
    third.stop(product);
    // Final ledger is checked only after all actual packaged descendants exited.
    proxy.finish();
    checkpoint("restore-and-natural-job-empty");
    eprintln!(
        "packaged foreground limits: installer=not-executed, logged-owner-task=not-executed, \
         oauth-account=not-executed, outbound-proxy=not-executed"
    );
}

fn checkpoint(stage: &str) {
    eprintln!("packaged foreground checkpoint: {stage}");
}

fn public_login(product: &InstalledProduct, arguments: &[&str], request: Option<&Value>) -> Value {
    // login helper's public args include --request-stdin/--output. The shared
    // bounded real-CLI producer owns those flags, so forward only the command.
    let Some(output) = arguments
        .iter()
        .position(|argument| *argument == "--output")
    else {
        panic!("public OAuth leaf did not use its explicit JSON output")
    };
    let args = arguments[..output]
        .iter()
        .copied()
        .filter(|argument| *argument != "--request-stdin")
        .collect::<Vec<_>>();
    cli(product, &args, request)
}

fn proxy_view(product: &InstalledProduct) -> Value {
    let value = cli(product, &["subscription-proxy", "show"], None)["data"].clone();
    assert!(value["config"]["schema"] == "hiroute.subscription-proxy/v1");
    // The public status includes manager proof. No Task/SCM was installed in
    // this foreground scenario, even when the daemon snapshot is applied.
    assert!(value["applied"] == false);
    value
}

fn assert_snapshot(product: &InstalledProduct, expected: &Value, applied: bool) {
    // Read the real writer's owner-only record via its production reader. This
    // does not write a setting or substitute for public manager/outbound proof.
    let value = SubscriptionProxyStore::new(product.layout.gateway_config_root())
        .view()
        .unwrap();
    assert!(serde_json::to_value(value.config).unwrap() == *expected);
    assert_eq!(
        value.applied, applied,
        "actual daemon launch snapshot differs"
    );
}

fn manual_policy_login(product: &InstalledProduct, proxy: &proxy_observation::Observation) {
    proxy.phase(proxy_observation::Phase::LoginStart);
    let started = cli(
        product,
        &["compute", "connection", "login"],
        Some(&json!({"action":"start", "provider":"codex"})),
    );
    let session = &started["data"]["sessions"][0];
    assert!(session["status"] == "pending");
    assert!(session["account_ref"].is_null() && session["candidate"].is_null());
    assert!(session["authorization_url"].is_string());
    let id = session["login_ref"].as_str().unwrap();
    proxy.require_updater_origin();
    for (action, state, phase) in [
        ("cancel", "cancelled", proxy_observation::Phase::LoginCancel),
        ("forget", "forgotten", proxy_observation::Phase::LoginForget),
    ] {
        proxy.phase(phase);
        let result = cli(
            product,
            &["compute", "connection", "login"],
            Some(&json!({"action":action, "login_ref":id})),
        );
        assert!(result["data"]["sessions"][0]["status"] == state);
        assert!(result["data"]["sessions"][0]["authorization_url"].is_null());
    }
}

fn authorization_rejections(product: &InstalledProduct) {
    super::windows_subscription_authorization::exercise(&mut |arguments, request| {
        let (exit, envelope) =
            super::windows_headless_business::command(product, arguments, request);
        (i64::from(exit), envelope)
    });
    checkpoint("authorize-missing-wrong-kind-and-stale");
}

struct Packaged {
    product: InstalledProduct,
    held: Vec<File>,
    payloads: Vec<(PathBuf, String, u64)>,
}

impl Packaged {
    fn create() -> Self {
        windows_execution_host::current();
        let mut manifest_file = held_input(&required("HIROUTE_WINDOWS_PACKAGE_MANIFEST"));
        let mut bytes = Vec::new();
        (&mut manifest_file)
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= 2 * 1024 * 1024);
        let manifest: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(manifest["schema"] == "hiroute.standalone-package/v1");
        assert!(manifest["distribution"] == "integration-candidate");
        assert!(manifest["target"] == hiroute_platform::windows::install::native_target().unwrap());
        let Some(compiled) = option_env!("HIROUTE_BUILD_SOURCE_SHA") else {
            panic!("complete-package gate requires its compiled candidate revision")
        };
        assert!(manifest["revision"] == compiled);
        assert!(std::env::var("HIROUTE_BUILD_SOURCE_SHA").unwrap() == compiled);
        let archive_path = required("HIROUTE_WINDOWS_PACKAGE_ARCHIVE");
        let mut archive = held_input(&archive_path);
        assert!(
            archive_path.file_name().unwrap().to_str() == manifest["archive"]["filename"].as_str()
        );
        verify_file(&mut archive, &manifest["archive"], MAX_ARCHIVE);
        let prepared_cli = required("HIROUTE_WINDOWS_ADMIN_CLI");
        assert!(prepared_cli.file_name().unwrap() == "hiroute.exe");
        let prepared = prepared_cli.parent().unwrap();
        let layout = StandaloneLayout::from_environment().unwrap();
        let stable = hiroute_host_runtime::standalone_cli_binary(&layout.home.join(".local/bin"));
        let stable_daemon =
            hiroute_host_runtime::standalone_daemon_binary(&layout.home.join(".local/bin"));
        assert!(!layout.marker_path.exists() && !layout.state_root.exists());
        assert!(!stable.exists() && !stable_daemon.exists());
        security::ensure_private_directory(&layout.data_root).unwrap();
        let root = layout
            .data_root
            .join(format!("Packaged foreground {}", std::process::id()));
        security::create_owner_only_directory(&root).unwrap();
        let product = InstalledProduct {
            preserve_on_failure: false,
            layout,
            cli: root.join("hiroute.exe"),
            daemon: root.join("hirouted.exe"),
            root,
            stable,
            stable_daemon,
        };
        // The guard owns only this fresh profile installation, including cleanup
        // if a member/identity assertion fails before marker publication.
        let mut package = Self {
            product,
            held: vec![manifest_file, archive],
            payloads: Vec::new(),
        };
        let files = manifest["files"]
            .as_object()
            .expect("complete production package inventory");
        assert!(!files.is_empty() && files.len() <= 1024);
        assert!(
            files.contains_key("bin/hiroute.exe")
                && files.contains_key("bin/hirouted.exe")
                && files.contains_key("libexec/cliproxyapi.exe")
                && files.contains_key("skills/hiroute-management/SKILL.md")
        );
        for (name, facts) in files {
            let relative = portable_member(name);
            let mut source =
                held_input(&prepared.join(relative.strip_prefix("bin").unwrap_or(&relative)));
            verify_file(&mut source, facts, MAX_FILE);
            if matches!(
                name.as_str(),
                "bin/hiroute.exe" | "bin/hirouted.exe" | "libexec/cliproxyapi.exe"
            ) {
                verify_pe(&mut source);
            }
            let relative = name.strip_prefix("bin/").unwrap_or(name);
            let target = package.product.root.join(relative);
            security::ensure_private_directory(target.parent().unwrap()).unwrap();
            let mut output = security::create_owner_only_file(&target).unwrap();
            use std::io::Seek;
            source.rewind().unwrap();
            let count = std::io::copy(&mut source, &mut output).unwrap();
            assert_eq!(Some(count), facts["size"].as_u64());
            output.sync_all().unwrap();
            security::verify_owner_only_handle(&output, false).unwrap();
            drop(output);
            let mut held = held_input(&target);
            verify_file(&mut held, facts, MAX_FILE);
            drop(held);
            package.held.push(source);
            package
                .payloads
                .push((target, facts["sha256"].as_str().unwrap().to_owned(), count));
        }
        for image in [
            &package.product.cli,
            &package.product.daemon,
            &package.product.root.join("libexec/cliproxyapi.exe"),
        ] {
            windows_execution_host::image(image);
        }
        let pin: Value =
            serde_json::from_str(include_str!("../../../../vendor/cpa/source.json")).unwrap();
        assert!(manifest["cpa"]["version"] == pin["version"]);
        assert!(
            manifest["cpa"]["sha256"] == manifest["files"]["libexec/cliproxyapi.exe"]["sha256"]
        );
        security::ensure_private_directory(package.product.stable.parent().unwrap()).unwrap();
        std::fs::hard_link(&package.product.cli, &package.product.stable).unwrap();
        std::fs::hard_link(&package.product.daemon, &package.product.stable_daemon).unwrap();
        package
            .held
            .extend(package.payloads.iter().map(|(path, _, _)| held_input(path)));
        let record = StandaloneInstallRecordV1 {
            schema_version: hiroute_host_runtime::STANDALONE_INSTALL_SCHEMA_V1.into(),
            version: manifest["version"].as_str().unwrap().into(),
            target: hiroute_host_runtime::standalone_host_target(),
            install_root: package.product.root.clone(),
            service_definition: package
                .product
                .root
                .join("not-registered-foreground-task.xml"),
            installed_skills: vec![],
            cpa_binary: Some(package.product.root.join("libexec/cliproxyapi.exe")),
            cpa_sha256: Some(manifest["cpa"]["sha256"].as_str().unwrap().into()),
        };
        record.validate().unwrap();
        hiroute_platform::durable::write_private_new(
            &package.product.layout.marker_path,
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        checkpoint("complete-package-exact-bytes-and-private-marker");
        package
    }

    fn verify_bytes(&self) {
        for (path, digest, size) in &self.payloads {
            verify_file(
                &mut held_input(path),
                &json!({"sha256":digest,"size":size}),
                MAX_FILE,
            );
        }
    }
}

impl Drop for Packaged {
    fn drop(&mut self) {
        // Release this fixture's no-write/no-delete pins before InstalledProduct
        // removes its own marker/payload; never adjust external source permissions.
        self.held.clear();
    }
}

fn required(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("required package input missing: {name}")),
    );
    assert!(
        path.is_absolute()
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
    );
    assert!(path.is_file());
    path
}

fn portable_member(name: &str) -> PathBuf {
    assert!(!name.is_empty() && name.len() <= 1024 && !name.contains(['\\', ':', '\0']));
    assert!(name.split('/').all(|part| !matches!(part, "" | "." | "..")));
    let path = PathBuf::from(name);
    assert!(
        !path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
    );
    path
}

fn held_input(path: &Path) -> File {
    let identity = security::open_identity(path, false).unwrap();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .unwrap();
    assert_eq!(
        security::file_identity(&identity).unwrap(),
        security::file_identity(&held).unwrap()
    );
    assert!(held.metadata().unwrap().is_file());
    held
}

fn verify_file(file: &mut File, facts: &Value, bound: u64) {
    use std::io::{Seek, SeekFrom};
    let size = facts["size"].as_u64().expect("manifest member size");
    let digest = facts["sha256"].as_str().expect("manifest member hash");
    assert!(
        size > 0
            && size <= bound
            && digest.len() == 64
            && digest
                .bytes()
                .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    );
    assert_eq!(file.metadata().unwrap().len(), size);
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut hasher = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        total += count as u64;
        assert!(total <= size);
        hasher.update(&buffer[..count]);
    }
    assert_eq!(total, size);
    assert!(
        format!("{:x}", hasher.finalize()) == digest,
        "complete packaged bytes differ"
    );
}

fn verify_pe(file: &mut File) {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).unwrap();
    assert!(&dos[..2] == b"MZ", "packaged executable has no DOS header");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap());
    assert!((64..=16 * 1024 * 1024).contains(&offset));
    assert!(u64::from(offset) + 6 <= file.metadata().unwrap().len());
    file.seek(SeekFrom::Start(u64::from(offset))).unwrap();
    let mut pe = [0; 6];
    file.read_exact(&mut pe).unwrap();
    assert!(
        &pe[..4] == b"PE\0\0"
            && u16::from_le_bytes(pe[4..6].try_into().unwrap())
                == hiroute_host_runtime::standalone_install::windows_machine(
                    hiroute_platform::windows::install::native_target().unwrap()
                )
                .unwrap(),
        "packaged executable is not the actual x64 PE target"
    );
}

struct Foreground {
    child: Child,
    readers: Vec<JoinHandle<Vec<u8>>>,
    listen: std::net::SocketAddr,
}

impl Foreground {
    fn start(product: &InstalledProduct, home_override: Option<&Path>) -> Self {
        let native = product.root.join("unlogged-native-config");
        security::ensure_private_directory(&native).unwrap();
        let codex = native.join("codex");
        let claude = native.join("claude");
        security::ensure_private_directory(&codex).unwrap();
        security::ensure_private_directory(&claude).unwrap();
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| {
                !matches!(
                    name.to_string_lossy().to_ascii_uppercase().as_str(),
                    "HOME"
                        | "CODEX_HOME"
                        | "CLAUDE_CONFIG_DIR"
                        | "HIROUTE_CODEX_AUTH_SOURCE"
                        | "HIROUTE_CPA_MANIFEST"
                        | "QODER_PERSONAL_ACCESS_TOKEN"
                )
            })
            .collect();
        environment.extend([
            (OsString::from("CODEX_HOME"), codex.into_os_string()),
            (OsString::from("CLAUDE_CONFIG_DIR"), claude.into_os_string()),
        ]);
        if let Some(home) = home_override {
            environment.push(("HOME".into(), home.as_os_str().to_owned()));
        }
        let args = [OsString::from("service"), OsString::from("run")];
        let mut child = Child::spawn(Launch {
            program: &product.stable,
            args: &args,
            env: &environment,
            cwd: product.stable.parent(),
            stdin: Stream::Null,
            stdout: Stream::Piped,
            stderr: Stream::Piped,
        })
        .unwrap();
        let (sender, ready) = mpsc::sync_channel(1);
        let stdout = child.stdout.take().unwrap();
        let out = thread::spawn(move || {
            let mut reader = BufReader::new(stdout.take(OUTPUT_BOUND + 1));
            let mut captured = Vec::new();
            loop {
                let mut line = Vec::new();
                let length = reader.read_until(b'\n', &mut line).unwrap();
                if length == 0 {
                    break;
                }
                assert!(captured.len() as u64 + length as u64 <= OUTPUT_BOUND);
                if let Ok(frame) = serde_json::from_slice::<Value>(&line)
                    && frame["schema"] == "hiroute.daemon-ready/v1"
                {
                    let _ = sender.try_send(frame);
                }
                captured.extend(line);
            }
            captured
        });
        let stderr = child.stderr.take().unwrap();
        let error = thread::spawn(move || {
            let mut captured = Vec::new();
            stderr
                .take(OUTPUT_BOUND + 1)
                .read_to_end(&mut captured)
                .unwrap();
            assert!(captured.len() as u64 <= OUTPUT_BOUND);
            captured
        });
        let mut foreground = Self {
            child,
            readers: vec![out, error],
            listen: "127.0.0.1:1".parse().unwrap(),
        };
        let frame = ready
            .recv_timeout(Duration::from_secs(45))
            .expect("packaged service run readiness absent");
        let owner = lifecycle::query(
            &product.layout.lifecycle_endpoint(),
            &product.daemon,
            IO_BOUND,
        )
        .unwrap();
        assert_eq!(frame["process_id"].as_u64(), Some(u64::from(owner.pid)));
        assert_ne!(owner.pid, foreground.child.id());
        assert!(!owner.exited().unwrap());
        windows_execution_host::daemon(&owner);
        foreground.listen = frame["gateway_listen"].as_str().unwrap().parse().unwrap();
        checkpoint(if home_override.is_some() {
            "known-profile-with-home-override"
        } else {
            "known-profile-without-home"
        });
        foreground
    }

    fn stop(&mut self, product: &InstalledProduct) {
        lifecycle::stop(
            &product.layout.lifecycle_endpoint(),
            &product.daemon,
            Duration::from_secs(15),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(35);
        let code = loop {
            if let Some(code) = self.child.exit_code().unwrap() {
                break code;
            }
            assert!(
                Instant::now() < deadline,
                "packaged service run did not exit"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(code, 0, "packaged foreground shutdown failed");
        while !self.child.empty().unwrap() {
            assert!(
                Instant::now() < deadline,
                "stock CPA survived foreground shutdown"
            );
            thread::sleep(Duration::from_millis(20));
        }
        for reader in self.readers.drain(..) {
            let bytes = reader.join().unwrap();
            // No OAuth stdout, provider output or arbitrary diagnostic is emitted.
            super::assert_no_secret(&bytes);
        }
        assert!(
            lifecycle::query(
                &product.layout.lifecycle_endpoint(),
                &product.daemon,
                IO_BOUND
            )
            .is_err()
        );
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        if self.readers.is_empty() {
            return; // Normal stop already observed exit/Job empty and joined EOF.
        }
        let terminate_requested = self.child.terminate().is_ok();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut root_exited = false;
        let mut job_empty = false;
        while Instant::now() < deadline {
            root_exited = self.child.exit_code().is_ok_and(|exit| exit.is_some());
            job_empty = self.child.empty().unwrap_or(false);
            if root_exited && job_empty {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // A failure's exact owned-tree cleanup never changes its red outcome.
        // Retain bounded evidence without printing arbitrary captured output.
        if root_exited && job_empty {
            for reader in self.readers.drain(..) {
                let _ = reader.join();
            }
        }
        eprintln!(
            "packaged foreground failure cleanup: terminate-requested={terminate_requested}, root-exited={root_exited}, owned-job-empty={job_empty}, natural-scenario-success=false"
        );
    }
}
