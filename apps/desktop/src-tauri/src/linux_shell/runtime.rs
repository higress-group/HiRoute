use hiroute_application_api::{
    ClientEmptyRequestV1, ConsoleLaunchRequestV1, ConsoleLaunchV1, ErrorCode,
    NativeConsoleSessionV1, QUALIFY_NATIVE_CONSOLE_V1, QualifyNativeConsoleRequestV1,
    STANDALONE_SERVICE_READINESS_OPERATION_V1,
};
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_host_runtime::{
    StandaloneLayout, read_standalone_install_record, standalone_cli_binary,
    standalone_daemon_binary, standalone_host_target, standalone_maintenance_pending,
};
use hiroute_shell::ConsoleOrigin;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};
use url::Url;
use zeroize::Zeroizing;

const UNIT: &str = "ai.hiroute.cli.service";
const PACKAGE: &str = "/usr/share/hiroute-desktop/standalone";
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REPLY: usize = 16 * 1024;

pub(super) struct Installation {
    layout: StandaloneLayout,
    cli: PathBuf,
    daemon: PathBuf,
    unit: PathBuf,
    version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Process {
    pid: u32,
    start_time: u64,
    image: (u64, u64),
}

#[derive(Clone)]
pub(super) struct LiveConsole {
    pub origin: ConsoleOrigin,
    pub url: Url,
    installation: std::sync::Arc<Installation>,
    process: Process,
}

#[derive(Clone)]
pub(super) struct NativeSession {
    pub origin: ConsoleOrigin,
    // Only the digest stays in native state after navigation.
    launch_sha256: String,
    installation: std::sync::Arc<Installation>,
    process: Process,
}

pub(super) fn layout() -> Result<StandaloneLayout, &'static str> {
    StandaloneLayout::from_environment().map_err(|_| "INSTALLATION_UNAVAILABLE")
}

fn present(path: &Path) -> Result<bool, &'static str> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("INSTALLATION_UNAVAILABLE"),
    }
}

fn executable(path: &Path) -> bool {
    path.canonicalize().ok().as_deref() == Some(path)
        && path
            .symlink_metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

impl Installation {
    pub fn discover() -> Result<Self, &'static str> {
        let layout = layout()?;
        if standalone_maintenance_pending(&layout).map_err(|_| "INSTALLATION_UNAVAILABLE")? {
            return Err("SHELL_INSTALL_RECOVERY_REQUIRED");
        }
        let unit = layout.home.join(".config/systemd/user").join(UNIT);
        if !present(&layout.marker_path)? {
            if present(&unit)?
                || present(LocalEndpoint::from_runtime_root(&layout.runtime_root).path())?
            {
                return Err("SERVICE_OWNER_UNVERIFIED");
            }
            return Err("INSTALLATION_REQUIRED");
        }
        let record = read_standalone_install_record(&layout.marker_path)
            .map_err(|_| "SERVICE_OWNER_UNVERIFIED")?;
        let lib = layout.home.join(".local/lib/hiroute");
        let cli = standalone_cli_binary(&record.install_root);
        let daemon = standalone_daemon_binary(&record.install_root);
        if record.target != standalone_host_target()
            || record.install_root.parent() != Some(lib.as_path())
            || record.install_root.file_name() != Some(OsStr::new(&record.version))
            || record.service_definition != unit
            || unit.canonicalize().ok().as_ref() != Some(&unit)
            || !executable(&cli)
            || !executable(&daemon)
        {
            return Err("SERVICE_OWNER_UNVERIFIED");
        }
        Ok(Self {
            layout,
            cli,
            daemon,
            unit,
            version: record.version,
        })
    }

    async fn manager(&self) -> Result<Option<Process>, &'static str> {
        let mut command = command("/usr/bin/systemctl");
        command.args([
            "--user",
            "show",
            "--no-pager",
            "--property=LoadState,ActiveState,FragmentPath,DropInPaths,MainPID,ExecStart",
            UNIT,
        ]);
        let reply = bounded_output(command, QUERY_TIMEOUT)
            .await
            .map_err(|_| "SERVICE_MANAGER_UNAVAILABLE")?;
        let output = std::str::from_utf8(&reply).map_err(|_| "SERVICE_OWNER_UNVERIFIED")?;
        let field = |name: &str| {
            output
                .lines()
                .filter_map(|line| line.split_once('='))
                .find_map(|(key, value)| (key == name).then_some(value))
        };
        let unit_text = hiroute_platform::private_fs::open_regular_file(&self.unit)
            .and_then(|file| std::io::read_to_string(std::io::Read::take(file, MAX_REPLY as u64)))
            .map_err(|_| "SERVICE_OWNER_UNVERIFIED")?;
        let managed = "# Managed by HiRoute standalone installer\n";
        let exec_path = format!("path={} ;", self.daemon.display());
        if field("LoadState") != Some("loaded")
            || field("FragmentPath").map(Path::new) != Some(self.unit.as_path())
            || field("DropInPaths") != Some("")
            || !field("ExecStart").is_some_and(|line| line.contains(&exec_path))
            || !unit_text.starts_with(managed)
        {
            return Err("SERVICE_OWNER_UNVERIFIED");
        }
        let pid: u32 = field("MainPID")
            .and_then(|value| value.parse().ok())
            .ok_or("SERVICE_MANAGER_UNAVAILABLE")?;
        if pid == 0 {
            return match field("ActiveState") {
                Some("inactive" | "failed") => Ok(None),
                _ => Err("SERVICE_NOT_READY"),
            };
        }
        let process = Process::inspect(pid).ok_or("SERVICE_NOT_READY")?;
        if image(&self.daemon) != Some(process.image) {
            return Err("SERVICE_OWNER_UNVERIFIED");
        }
        Ok(Some(process))
    }

    fn client(&self, process: &Process) -> Result<Client, &'static str> {
        if Process::inspect(process.pid).as_ref() != Some(process) {
            return Err("SERVICE_GENERATION_CHANGED");
        }
        Ok(Client::new(
            "hiroute-linux-desktop",
            LocalEndpoint::for_verified_process(&self.layout.runtime_root, process.pid),
        )
        .with_timeout(QUERY_TIMEOUT))
    }

    async fn service(&self, action: &str) -> Result<(), &'static str> {
        let mut command = command(&self.cli);
        command.args(["service", action, "--timeout", "12s", "--output", "json"]);
        bounded_output(command, Duration::from_secs(65))
            .await
            .map(|_| ())
    }

    async fn verify_pair(&self) -> Result<String, &'static str> {
        let mut cli = command(&self.cli);
        cli.args(["version", "--output", "json"]);
        let mut daemon = command(&self.daemon);
        daemon.arg("--version");
        let (cli, daemon) = tokio::try_join!(
            bounded_output(cli, QUERY_TIMEOUT),
            bounded_output(daemon, QUERY_TIMEOUT),
        )
        .map_err(|_| "SERVICE_RELEASE_UNVERIFIED")?;
        let cli: Value = serde_json::from_slice(&cli).map_err(|_| "SERVICE_RELEASE_UNVERIFIED")?;
        let daemon: Value =
            serde_json::from_slice(&daemon).map_err(|_| "SERVICE_RELEASE_UNVERIFIED")?;
        let cli = &cli["data"];
        let revision = cli["source_revision"]
            .as_str()
            .filter(|sha| {
                matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .ok_or("SERVICE_RELEASE_UNVERIFIED")?;
        if cli["component"] != "cli"
            || daemon["component"] != "daemon"
            || cli["version"] != self.version
            || cli["version"] != daemon["version"]
            || cli["source_revision"] != daemon["source_revision"]
            || cli["arch"] != std::env::consts::ARCH
            || daemon["arch"] != std::env::consts::ARCH
            || cli["os"] != "linux"
            || daemon["os"] != "linux"
        {
            return Err("SERVICE_RELEASE_UNVERIFIED");
        }
        Ok(revision.to_owned())
    }

    async fn verify_peer(&self, process: &Process) -> Result<(), &'static str> {
        let endpoint = LocalEndpoint::from_runtime_root(&self.layout.runtime_root);
        let stream = tokio::time::timeout(
            QUERY_TIMEOUT,
            tokio::net::UnixStream::connect(endpoint.path()),
        )
        .await
        .map_err(|_| "SERVICE_NOT_READY")?
        .map_err(|_| "SERVICE_NOT_READY")?;
        let pid = stream
            .peer_cred()
            .ok()
            .and_then(|credential| credential.pid());
        if pid != i32::try_from(process.pid).ok()
            || Process::inspect(process.pid).as_ref() != Some(process)
        {
            return Err("SERVICE_GENERATION_CHANGED");
        }
        Ok(())
    }

    async fn legacy_reply(&self, process: &Process, args: &[&str]) -> Result<Value, &'static str> {
        self.verify_peer(process).await?;
        let mut command = command(&self.cli);
        command.args(args).args(["--output", "json"]);
        let bytes = bounded_output(command, QUERY_TIMEOUT).await?;
        self.verify_peer(process).await?;
        serde_json::from_slice(&bytes).map_err(|_| "SERVICE_RESPONSE_INVALID")
    }

    async fn control_ready(
        &self,
        process: &Process,
        matching_build: bool,
    ) -> Result<bool, &'static str> {
        if matching_build {
            return Ok(self
                .client(process)?
                .query::<_, bool>(
                    STANDALONE_SERVICE_READINESS_OPERATION_V1,
                    "linux-desktop-ready",
                    &ClientEmptyRequestV1::default(),
                )
                .await
                .is_ok_and(|reply| reply.data == Some(true)));
        }
        // The shared client negotiates its own compiled release. The verified
        // installed CLI is the released client's authority for an older daemon.
        Ok(self
            .legacy_reply(process, &["system", "status"])
            .await
            .is_ok_and(|reply| {
                reply["data"]["daemon"] == "role_all" && reply["data"]["gateway"] == "ready"
            }))
    }

    async fn ready(&self, matching_build: bool) -> Result<Process, &'static str> {
        if self.manager().await?.is_none() {
            // A previous CLI can time out after systemd starts. Check the
            // manager and the bound Local Control peer before reporting failure.
            let _ = self.service("start").await;
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                match self.manager().await {
                    Ok(Some(process)) => {
                        if self.control_ready(&process, matching_build).await? {
                            return Ok(process);
                        }
                    }
                    Ok(None) | Err("SERVICE_NOT_READY") => {}
                    Err(code) => return Err(code),
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .map_err(|_| "SERVICE_NOT_READY")?
    }
}

impl Process {
    fn inspect(pid: u32) -> Option<Self> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let start_time = stat
            .rsplit_once(')')?
            .1
            .split_ascii_whitespace()
            .nth(19)?
            .parse()
            .ok()?;
        Some(Self {
            pid,
            start_time,
            image: image(Path::new(&format!("/proc/{pid}/exe")))?,
        })
    }
}

fn image(path: &Path) -> Option<(u64, u64)> {
    let m = path.metadata().ok()?;
    m.is_file().then_some((m.dev(), m.ino()))
}

impl LiveConsole {
    pub async fn open() -> Result<Self, &'static str> {
        let installation = std::sync::Arc::new(Installation::discover()?);
        let revision = installation.verify_pair().await?;
        let matching_build = installation.version == env!("CARGO_PKG_VERSION")
            && option_env!("HIROUTE_BUILD_SOURCE_SHA") == Some(revision.as_str());
        let process = installation.ready(matching_build).await?;
        if !matching_build {
            return Err("CONSOLE_NATIVE_RELEASE_UNAVAILABLE");
        }
        let (origin, url) = launch(&installation, &process).await?;
        qualify(&installation, &process, &url, None).await?;
        Ok(Self {
            origin,
            url,
            installation,
            process,
        })
    }

    pub fn session(&self) -> Result<NativeSession, &'static str> {
        let (_, code) = self
            .url
            .query_pairs()
            .next()
            .ok_or("CONSOLE_LAUNCH_INVALID")?;
        Ok(NativeSession {
            origin: self.origin.clone(),
            launch_sha256: format!("{:x}", Sha256::digest(code.as_bytes())),
            installation: self.installation.clone(),
            process: self.process.clone(),
        })
    }
}

impl NativeSession {
    pub async fn authorize(
        &self,
        url: &Url,
        bearer: Zeroizing<String>,
    ) -> Result<(), &'static str> {
        if !self.origin.allows(url) || bearer.len() != 64 {
            return Err("SHELL_ORIGIN_DENIED");
        }
        qualify(
            &self.installation,
            &self.process,
            url,
            Some(NativeConsoleSessionV1 {
                bearer_sha256: format!("{:x}", Sha256::digest(bearer.as_bytes())),
                launch_sha256: self.launch_sha256.clone(),
            }),
        )
        .await
    }

    pub async fn browser_url(&self) -> Result<Url, &'static str> {
        launch(&self.installation, &self.process)
            .await
            .map(|(_, url)| url)
    }

    pub async fn restart(&self) -> Result<(), &'static str> {
        if self.installation.manager().await?.as_ref() != Some(&self.process) {
            return Err("SERVICE_GENERATION_CHANGED");
        }
        if standalone_maintenance_pending(&self.installation.layout)
            .map_err(|_| "INSTALLATION_UNAVAILABLE")?
        {
            return Err("SHELL_INSTALL_RECOVERY_REQUIRED");
        }
        self.installation.service("restart").await
    }
}

async fn qualify(
    installation: &Installation,
    process: &Process,
    url: &Url,
    session: Option<NativeConsoleSessionV1>,
) -> Result<(), &'static str> {
    let code = if session.is_some() {
        "SHELL_SESSION_DENIED"
    } else {
        "CONSOLE_NATIVE_ASSETS_UNAVAILABLE"
    };
    let assets_sha256 = option_env!("HIROUTE_LINUX_CONSOLE_ASSETS_SHA256").ok_or(code)?;
    let reply = installation
        .client(process)?
        .query::<_, bool>(
            QUALIFY_NATIVE_CONSOLE_V1,
            "linux-desktop-admission",
            &QualifyNativeConsoleRequestV1 {
                origin: url.origin().ascii_serialization(),
                assets_sha256: assets_sha256.to_owned(),
                session,
            },
        )
        .await
        .map_err(|_| code)?;
    if reply.data == Some(true) {
        Ok(())
    } else {
        Err(code)
    }
}

async fn launch(
    installation: &Installation,
    process: &Process,
) -> Result<(ConsoleOrigin, Url), &'static str> {
    let reply = installation
        .client(process)?
        .query::<_, ConsoleLaunchV1>(
            "CreateConsoleLaunch",
            "linux-desktop-launch",
            &ConsoleLaunchRequestV1 {
                loopback_only: true,
            },
        )
        .await
        .map_err(|_| "CONSOLE_LAUNCH_UNAVAILABLE")?;
    if let Some(error) = reply.error {
        return Err(if error.code == ErrorCode::ActionRequired {
            "CONSOLE_LISTENER_UNAVAILABLE"
        } else {
            "CONSOLE_LAUNCH_UNAVAILABLE"
        });
    }
    ConsoleOrigin::launch(&reply.data.ok_or("CONSOLE_LAUNCH_UNAVAILABLE")?.url)
}

pub(super) async fn legacy_browser_url() -> Result<Url, &'static str> {
    let installation = Installation::discover()?;
    installation.verify_pair().await?;
    let process = installation.manager().await?.ok_or("SERVICE_NOT_READY")?;
    if !installation.control_ready(&process, false).await? {
        return Err("SERVICE_NOT_READY");
    }
    let reply = installation
        .legacy_reply(&process, &["console", "open", "--loopback-only"])
        .await
        .map_err(|_| "CONSOLE_LAUNCH_UNAVAILABLE")?;
    let url = reply["data"]["url"]
        .as_str()
        .ok_or("CONSOLE_LAUNCH_UNAVAILABLE")?;
    ConsoleOrigin::launch(url).map(|(_, url)| url)
}

pub(super) async fn enable_login_start() -> Result<(), &'static str> {
    let installation = Installation::discover()?;
    installation.manager().await?;
    let mut command = command(&installation.cli);
    command.args(["service", "autostart", "enable", "--output", "json"]);
    bounded_output(command, QUERY_TIMEOUT)
        .await
        .map(|_| ())
        .map_err(|_| "SHELL_LOGIN_UNAVAILABLE")
}

pub(super) async fn install(recover: bool) -> Result<(), &'static str> {
    let root = Path::new(PACKAGE);
    let installer = root.join("install-standalone.py");
    if !installer
        .symlink_metadata()
        .is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
        || installer.canonicalize().ok().as_ref() != Some(&installer)
    {
        return Err("PACKAGE_PAYLOAD_UNAVAILABLE");
    }
    let archive = format!(
        "HiRoute-{}-linux-{}.tar.gz",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    );
    let mut command = command("/usr/bin/python3");
    command.arg("-I").arg(installer);
    if recover {
        command.arg("recover");
    } else {
        command
            .args([
                "install",
                "--fresh",
                "--console-mode",
                "loopback",
                "--manifest",
            ])
            .arg(root.join("manifest.json"))
            .arg("--archive")
            .arg(root.join(archive));
    }
    bounded_output(command, Duration::from_secs(120))
        .await
        .map(|_| ())
        .map_err(|_| {
            if recover {
                "SHELL_INSTALL_RECOVERY_REQUIRED"
            } else {
                "RUNTIME_INSTALL_FAILED"
            }
        })
}

pub(super) fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .env("PATH", "/usr/bin:/bin")
        .env_remove("HIROUTE_RUNTIME_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

pub(super) async fn bounded_output(
    mut command: Command,
    timeout: Duration,
) -> Result<Vec<u8>, &'static str> {
    let mut child = command.spawn().map_err(|_| "SERVICE_COMMAND_FAILED")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("SERVICE_COMMAND_FAILED")?
        .take((MAX_REPLY + 1) as u64);
    tokio::time::timeout(timeout, async {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "SERVICE_COMMAND_FAILED")?;
        if bytes.len() > MAX_REPLY {
            return Err("SERVICE_RESPONSE_INVALID");
        }
        if child
            .wait()
            .await
            .map_err(|_| "SERVICE_COMMAND_FAILED")?
            .success()
        {
            Ok(bytes)
        } else {
            Err("SERVICE_COMMAND_FAILED")
        }
    })
    .await
    .map_err(|_| "SERVICE_COMMAND_TIMED_OUT")?
}
