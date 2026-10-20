//! Explicit collaboration checks in the selected, normally logged-in Qoder context.
//! Auth is read only by Qoder. HiRoute owns the private workspace and local challenge, not HOME.
use std::fs::{self, File};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::native_probe_process::{NativeProbeProcess, read_bounded, read_bounded_file};
use super::qoder::{
    QoderNativeContext, QoderNativeError, QoderTransientRouteInput, qoder_error,
    render_qoder_transient_route,
};
use hiroute_domain::{
    AgentCapability, CanonicalDigest, CapabilityState, SupportedAgentInstallationV1,
};
use serde_json::Value;

#[cfg(all(test, unix))]
#[path = "qoder_probe_bash_parse_tests.rs"]
mod bash_parse_tests;
#[path = "qoder_probe_http.rs"]
mod http;
#[cfg(test)]
#[path = "qoder_probe_output_diagnostic_tests.rs"]
mod output_diagnostic_tests;
#[cfg(any(test, windows))]
#[path = "qoder_probe_skill_diagnostic.rs"]
mod skill_diagnostic;
#[cfg(all(test, windows))]
#[path = "qoder_probe_windows_cli.rs"]
mod windows_cli;
#[cfg(windows)]
#[path = "qoder_probe_workspace.rs"]
mod workspace;

const CONTRACT: &str = "hiroute.qoder-borrowed-context-collaboration/v1";
const MARKER: &str = "hiroute-local-challenge-no-authority";
const MODEL: &str = "hiroute/collaboration-probe";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QoderCollaborationProbeTarget {
    PreinstallCapability,
    InstalledUserSkill { expected_content: CanonicalDigest },
}

#[derive(Clone)]
pub struct QoderCollaborationEvidence {
    executable: PathBuf,
    cli: PathBuf,
    dependencies: CanonicalDigest,
    target: QoderCollaborationProbeTarget,
    proof: CanonicalDigest,
    observed: Instant,
    observed_at: u64,
}

/// A new explicit attempt invalidates old success before discovery or launch can fail.
#[derive(Default)]
pub(super) struct QoderProofCache {
    generation: u64,
    pub(super) evidence: Option<QoderCollaborationEvidence>,
}

impl QoderProofCache {
    pub(super) fn begin(&mut self) -> Result<u64, QoderNativeError> {
        self.evidence = None;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| qoder_error("evidence generation"))?;
        Ok(self.generation)
    }
    pub(super) fn finish(
        &mut self,
        generation: u64,
        evidence: QoderCollaborationEvidence,
    ) -> Result<(), QoderNativeError> {
        if generation != self.generation {
            return Err(qoder_error("native check superseded"));
        }
        self.evidence = Some(evidence);
        Ok(())
    }
}

impl QoderCollaborationEvidence {
    #[cfg(test)]
    pub(crate) fn fixture(
        executable: &Path,
        context: &QoderNativeContext,
        cli: &Path,
        target: QoderCollaborationProbeTarget,
    ) -> Self {
        Self {
            executable: executable.to_owned(),
            cli: cli.to_owned(),
            dependencies: dependencies(executable, context, cli, &target).unwrap(),
            target,
            proof: CanonicalDigest::of_bytes(b"fixture"),
            observed: Instant::now(),
            observed_at: 1,
        }
    }
    fn current(&self, executable: &Path, context: &QoderNativeContext) -> bool {
        self.observed.elapsed() <= Duration::from_secs(300)
            && executable == self.executable
            && dependencies(executable, context, &self.cli, &self.target)
                .ok()
                .as_ref()
                == Some(&self.dependencies)
    }

    pub(super) fn installed_user_verified(
        &self,
        executable: &Path,
        context: &QoderNativeContext,
    ) -> bool {
        matches!(
            self.target,
            QoderCollaborationProbeTarget::InstalledUserSkill { .. }
        ) && self.current(executable, context)
    }

    pub(super) fn attach(
        &self,
        executable: &Path,
        context: &QoderNativeContext,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if !self.current(executable, context) {
            return;
        }
        installation.observation_digest = CanonicalDigest::of(&(
            CONTRACT,
            &installation.observation_digest,
            &self.proof,
            self.observed_at,
        ))
        .expect("bounded native proof");
        for proof in &mut installation.capability_evidence {
            proof.dependency_digest = installation.observation_digest.clone();
            if matches!(
                proof.capability,
                AgentCapability::SkillLoading | AgentCapability::TrustedCliExecution
            ) {
                proof.state = CapabilityState::Proven;
                proof.reason = None;
                proof.adapter_contract = CONTRACT.into();
                proof.observed_at_unix_ms = self.observed_at;
            }
        }
    }
}

pub struct QoderCollaborationProbe;

impl QoderCollaborationProbe {
    pub fn run(
        executable: &Path,
        context: &QoderNativeContext,
        cli: &Path,
        target: QoderCollaborationProbeTarget,
    ) -> Result<QoderCollaborationEvidence, QoderNativeError> {
        let executable = super::executable::resolve(executable)
            .map_err(|_| qoder_error("selected executable"))?
            .ok_or_else(|| qoder_error("selected executable"))?;
        let cli = super::executable::resolve(cli)
            .map_err(|_| qoder_error("trusted CLI"))?
            .ok_or_else(|| qoder_error("trusted CLI"))?;
        let before = dependencies(&executable, context, &cli, &target)?;
        let settings_before = settings_receipt(context)?;
        let root = super::probe_files::temporary("hiroute-qoder-check-")
            .map_err(|_| qoder_error("private workspace"))?;
        #[cfg(not(windows))]
        let root_path =
            fs::canonicalize(root.path()).map_err(|_| qoder_error("private workspace"))?;
        #[cfg(windows)]
        let (root_path, _root_identity) =
            workspace::retain(root.path()).map_err(|_| qoder_error("private workspace"))?;
        let workspace = root_path.join("workspace");
        let temporary = root_path.join("tmp");
        for path in [&workspace, &temporary] {
            private_dir(path)?;
        }
        let (skill_path, content, sources) = match &target {
            QoderCollaborationProbeTarget::PreinstallCapability => {
                let mut entropy = [0u8; 16];
                getrandom::fill(&mut entropy).map_err(|_| qoder_error("challenge entropy"))?;
                let suffix = entropy
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let path = workspace.join(format!(".qoder/skills/hiroute-probe-{suffix}/SKILL.md"));
                let text = format!(
                    "---\nname: hiroute-probe-{suffix}\ndescription: HiRoute native discovery {suffix}\n---\n\nPrivate content receipt: {suffix}\nFollow only this local compatibility check.\n"
                );
                for path in [
                    workspace.join(".qoder"),
                    workspace.join(".qoder/skills"),
                    path.parent().unwrap().to_owned(),
                ] {
                    private_dir(&path)?;
                }
                write_private(&path, text.as_bytes())?;
                (path, text, "user,project,local")
            }
            QoderCollaborationProbeTarget::InstalledUserSkill { expected_content } => {
                let path = context.skill_target();
                let content = confirmed_skill(&path, expected_content)?;
                (path, content, "user")
            }
        };
        let skill_directory = fs::canonicalize(
            skill_path
                .parent()
                .ok_or_else(|| qoder_error("Skill target"))?,
        )
        .map_err(|_| qoder_error("Skill target"))?;
        #[cfg(windows)]
        let expected_skill = skill_diagnostic::NativeSkill::retain(
            skill_path
                .parent()
                .ok_or_else(|| qoder_error("Skill target"))?,
            content.as_bytes(),
            matches!(target, QoderCollaborationProbeTarget::PreinstallCapability),
        )
        .map_err(|_| qoder_error("Skill target"))?;
        let skill_name = skill_directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| qoder_error("Skill target encoding"))?
            .to_owned();
        let body = skill_body(&content)?;
        let expected_cli = cli_baseline(&cli, &root_path)?;
        let command = cli_command(&cli)?;
        let mut challenge = http::Challenge {
            skill_name,
            #[cfg(any(test, not(windows)))]
            skill_directory: skill_directory
                .to_str()
                .ok_or_else(|| qoder_error("Skill target encoding"))?
                .into(),
            skill_body: body,
            command,
            expected_cli,
            step: 0,
            #[cfg(windows)]
            expected_skill: Some(expected_skill),
            #[cfg(windows)]
            expected_workspace: Some(
                workspace::Receipt::retain(&workspace)
                    .map_err(|_| qoder_error("private workspace"))?,
            ),
        };
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| qoder_error("loopback bind"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| qoder_error("loopback mode"))?;
        let endpoint = format!(
            "http://{}/v1",
            listener
                .local_addr()
                .map_err(|_| qoder_error("loopback address"))?
        );
        let route = render_qoder_transient_route(QoderTransientRouteInput {
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            provider_id: "hiroute-native-probe",
            endpoint: &endpoint,
            alias: MODEL,
            credential_env: "HIROUTE_QODER_PROBE_TOKEN",
            context_window_tokens: None,
            max_output_tokens: 2048,
        })?;
        let settings = root_path.join("settings.json");
        write_private(
            &settings,
            &serde_json::to_vec(&route.settings).map_err(|_| qoder_error("transient settings"))?,
        )?;
        let output = private_file(&root_path.join("output.private"))?;
        let observer = output
            .try_clone()
            .map_err(|_| qoder_error("private output"))?;
        let mut command = Command::new(&executable);
        command
            .args(["--cwd"])
            .arg(&workspace)
            .arg("--config-dir")
            .arg(&context.config_root)
            .args(["--setting-sources", sources, "--settings"])
            .arg(&settings)
            .args([
                "--model",
                &route.native_model_id,
                "--print",
                "--no-session-persistence",
                "--output-format",
                "stream-json",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--tools",
                "Skill,Read,Bash",
                "--allowed-tools",
                "Skill,Bash",
                "--permission-mode",
                "dont_ask",
                "--max-model-request-retries",
                "0",
                "--max-output-tokens",
                "128",
                "-p",
                "Complete the local compatibility check; follow the local tool requests.",
            ])
            .env_clear()
            .env("HOME", &context.home)
            .env("QODER_CONFIG_DIR", &context.config_root)
            .env("TMPDIR", &temporary)
            .env("PATH", super::native_ingress_probe::native_probe_path())
            .env("HIROUTE_QODER_PROBE_TOKEN", MARKER)
            .current_dir(&workspace);
        #[cfg(unix)]
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        #[cfg(windows)]
        let child = super::windows_runtime::spawn(
            &command,
            false,
            hiroute_platform::windows::job::Stream::Null,
            hiroute_platform::windows::job::Stream::File(&output),
            hiroute_platform::windows::job::Stream::Null,
        );
        let mut child = NativeProbeProcess::new(child.map_err(|_| qoder_error("native launch"))?);
        let started = Instant::now();
        #[cfg(test)]
        let mut requests = 0;
        let result = (|| {
            loop {
                if started.elapsed() > Duration::from_secs(45) {
                    return Err(qoder_error("native check deadline"));
                }
                if observer
                    .metadata()
                    .map_err(|_| qoder_error("private output"))?
                    .len()
                    > 512 * 1024
                {
                    return Err(qoder_error("native output bound"));
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        #[cfg(test)]
                        {
                            requests += 1;
                        }
                        http::serve(&mut stream, MARKER, MODEL, &mut challenge)
                            .map_err(qoder_error)?;
                        #[cfg(test)]
                        eprintln!(
                            "native Qoder acceptance stage=model-served requests={requests} challenge-step={}",
                            challenge.step
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(qoder_error("loopback accept")),
                }
                if let Some(status) = child.observe().map_err(|_| qoder_error("native wait"))? {
                    break Ok(status);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if child.stop().is_err() {
            let _ = root.keep();
            return Err(qoder_error("native process scope"));
        }
        #[cfg(test)]
        if result
            .as_ref()
            .is_err_and(|error| error.stage == "native output bound")
        {
            // A duplicated File shares its seek pointer. Inspect only after the actual
            // owned scope has stopped, never disturb a live native stdout writer.
            safe_output_bound(&observer, challenge.step, requests, started.elapsed());
        }
        let status = result?;
        #[cfg(test)]
        let final_read = observed_final_output(
            &observer,
            challenge.step,
            requests,
            &started,
            |diagnostic| diagnostic.emit(),
        );
        #[cfg(not(test))]
        let final_read = read_bounded_file(&observer, 512 * 1024);
        let output = final_read.map_err(|_| qoder_error("native output bound"))?;
        let result = output
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .find(|value| value["type"] == "result");
        if !status
            || !challenge.complete()
            || result
                .as_ref()
                .is_none_or(|result| result["is_error"] != false)
        {
            let login = result.as_ref().is_some_and(|value| {
                let text = value.to_string().to_ascii_lowercase();
                text.contains("authentication") || text.contains("login") || text.contains("log in")
            });
            return Err(qoder_error(if login {
                "native login required"
            } else {
                "native collaboration execution"
            }));
        }
        if before != dependencies(&executable, context, &cli, &target)?
            || settings_before != settings_receipt(context)?
        {
            return Err(qoder_error("native context changed"));
        }
        let phase = match target {
            QoderCollaborationProbeTarget::PreinstallCapability => "preinstall",
            QoderCollaborationProbeTarget::InstalledUserSkill { .. } => "installed-user",
        };
        let proof = CanonicalDigest::of(&(
            CONTRACT,
            "borrowed-logged-in-context",
            phase,
            &before,
            &skill_path,
            CanonicalDigest::of_bytes(content.as_bytes()),
        ))
        .map_err(|_| qoder_error("evidence digest"))?;
        Ok(QoderCollaborationEvidence {
            executable,
            cli,
            dependencies: before,
            target,
            proof,
            observed: Instant::now(),
            observed_at: super::native_ingress_probe::now(),
        })
    }
}

pub(super) fn confirmed_skill(
    path: &Path,
    expected: &CanonicalDigest,
) -> Result<String, QoderNativeError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| qoder_error("installed user Skill missing"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 * 1024 {
        return Err(qoder_error("installed user Skill target"));
    }
    let bytes =
        read_bounded(path, 128 * 1024).map_err(|_| qoder_error("installed user Skill read"))?;
    if &CanonicalDigest::of_bytes(&bytes) != expected {
        return Err(qoder_error("installed user Skill changed"));
    }
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| qoder_error("installed user Skill encoding"))
}

pub(super) fn skill_body(content: &str) -> Result<String, QoderNativeError> {
    content
        .strip_prefix("---\n")
        .and_then(|text| text.split_once("\n---\n"))
        .map(|(_, body)| body.trim().to_owned())
        .filter(|body| !body.is_empty())
        .ok_or_else(|| qoder_error("Skill front matter"))
}

fn dependencies(
    executable: &Path,
    context: &QoderNativeContext,
    cli: &Path,
    target: &QoderCollaborationProbeTarget,
) -> Result<CanonicalDigest, QoderNativeError> {
    let binary = super::native_ingress_probe::binary_identity(executable)
        .map_err(|_| qoder_error("native executable identity"))?;
    let cli = super::native_ingress_probe::binary_identity(cli)
        .map_err(|_| qoder_error("trusted CLI identity"))?;
    let root_identity = |path: &Path| -> Result<_, QoderNativeError> {
        let path = fs::canonicalize(path).map_err(|_| qoder_error("selected native context"))?;
        let m = fs::metadata(&path).map_err(|_| qoder_error("selected native context"))?;
        if !m.is_dir() {
            return Err(qoder_error("selected native context"));
        }
        #[cfg(unix)]
        return CanonicalDigest::of(&(path, m.dev(), m.ino(), m.uid(), m.gid(), m.mode()))
            .map_err(|_| qoder_error("selected native context"));
        #[cfg(windows)]
        {
            let file = hiroute_platform::windows::security::open_identity(&path, true)
                .map_err(|_| qoder_error("selected native context"))?;
            let identity = hiroute_platform::windows::security::file_identity(&file)
                .map_err(|_| qoder_error("selected native context"))?;
            CanonicalDigest::of(&(path, identity))
                .map_err(|_| qoder_error("selected native context"))
        }
    };
    let settings = match fs::symlink_metadata(context.config_root.join("settings.json")) {
        Ok(metadata) => {
            #[cfg(unix)]
            let receipt = CanonicalDigest::of(&(
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ))
            .map_err(|_| qoder_error("native settings metadata"))?;
            #[cfg(windows)]
            let receipt = {
                use std::os::windows::fs::MetadataExt;
                let file = hiroute_platform::windows::security::open_identity(
                    &context.config_root.join("settings.json"),
                    false,
                )
                .map_err(|_| qoder_error("native settings metadata"))?;
                let identity = hiroute_platform::windows::security::file_identity(&file)
                    .map_err(|_| qoder_error("native settings metadata"))?;
                CanonicalDigest::of(&(
                    identity,
                    metadata.file_size(),
                    metadata.creation_time(),
                    metadata.last_write_time(),
                ))
                .map_err(|_| qoder_error("native settings metadata"))?
            };
            Some(receipt)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(qoder_error("native settings metadata")),
    };
    let content = match target {
        QoderCollaborationProbeTarget::PreinstallCapability => None,
        QoderCollaborationProbeTarget::InstalledUserSkill { expected_content } => {
            confirmed_skill(&context.skill_target(), expected_content)?;
            Some(expected_content)
        }
    };
    CanonicalDigest::of(&(
        CONTRACT,
        root_identity(&context.home)?,
        root_identity(&context.config_root)?,
        settings,
        binary,
        cli,
        context.skill_target(),
        content,
    ))
    .map_err(|_| qoder_error("native context identity"))
}

fn settings_receipt(
    context: &QoderNativeContext,
) -> Result<Option<CanonicalDigest>, QoderNativeError> {
    let path = context.config_root.join("settings.json");
    let metadata = match fs::metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(qoder_error("native settings receipt")),
    };
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(qoder_error("native settings bound"));
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    File::open(path)
        .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| qoder_error("native settings receipt"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(qoder_error("native settings bound"));
    }
    Ok(Some(CanonicalDigest::of_bytes(&bytes)))
}

fn private_dir(path: &Path) -> Result<(), QoderNativeError> {
    super::probe_files::directory(path).map_err(|_| qoder_error("private directory"))
}
fn private_file(path: &Path) -> Result<File, QoderNativeError> {
    super::probe_files::file(path).map_err(|_| qoder_error("private file"))
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), QoderNativeError> {
    private_file(path)?
        .write_all(bytes)
        .map_err(|_| qoder_error("private file"))
}
fn shell_quote(path: &Path) -> Result<String, QoderNativeError> {
    Ok(format!(
        "'{}'",
        path.to_str()
            .ok_or_else(|| qoder_error("trusted CLI path"))?
            .replace('\'', "'\\''")
    ))
}

pub(super) fn cli_command(cli: &Path) -> Result<String, QoderNativeError> {
    // Qoder 1.1.65's Bash tool uses Git Bash on Windows, independently of its
    // separate PowerShell tool. The resolved native image already passed the
    // shared ordinary-drive projection; retain that same identity when making
    // its separators suitable for Bash. Never send PowerShell's '&' or quoting.
    #[cfg(windows)]
    let quoted = {
        use hiroute_platform::windows::security;
        let ordinary = super::managed_launch::windows_executable_path(cli)
            .map_err(|_| qoder_error("trusted CLI path"))?;
        let original = security::open_identity(&ordinary, false)
            .map_err(|_| qoder_error("trusted CLI path"))?;
        let spelling = PathBuf::from(
            ordinary
                .to_str()
                .ok_or_else(|| qoder_error("trusted CLI path"))?
                .replace('\\', "/"),
        );
        let projected = security::open_identity(&spelling, false)
            .map_err(|_| qoder_error("trusted CLI path"))?;
        if security::file_identity(&original).map_err(|_| qoder_error("trusted CLI path"))?
            != security::file_identity(&projected).map_err(|_| qoder_error("trusted CLI path"))?
        {
            return Err(qoder_error("trusted CLI path"));
        }
        shell_quote(&spelling)?
    };
    #[cfg(not(windows))]
    let quoted = shell_quote(cli)?;
    Ok(format!("{quoted} schema list --output json"))
}

#[cfg(unix)]
pub(super) fn cli_baseline(cli: &Path, root: &Path) -> Result<String, QoderNativeError> {
    let output = private_file(&root.join("cli.private"))?;
    let observer = output
        .try_clone()
        .map_err(|_| qoder_error("trusted CLI baseline"))?;
    let mut command = Command::new(cli);
    command
        .args(["schema", "list", "--output", "json"])
        .env_clear()
        .env("HOME", root)
        .env("PATH", super::native_ingress_probe::native_probe_path());
    #[cfg(unix)]
    let child = command
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    #[cfg(windows)]
    let child = super::windows_runtime::spawn(
        &command,
        false,
        hiroute_platform::windows::job::Stream::Null,
        hiroute_platform::windows::job::Stream::File(&output),
        hiroute_platform::windows::job::Stream::Null,
    );
    let mut child =
        NativeProbeProcess::new(child.map_err(|_| qoder_error("trusted CLI baseline"))?);
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10)
            || observer
                .metadata()
                .map_err(|_| qoder_error("trusted CLI output"))?
                .len()
                > 128 * 1024
        {
            return Err(qoder_error("trusted CLI bound"));
        }
        if let Some(status) = child
            .observe()
            .map_err(|_| qoder_error("trusted CLI wait"))?
        {
            if !status {
                return Err(qoder_error("trusted CLI baseline"));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child
        .stop()
        .map_err(|_| qoder_error("trusted CLI process scope"))?;
    let bytes = read_bounded(&root.join("cli.private"), 128 * 1024)
        .map_err(|_| qoder_error("trusted CLI output"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| qoder_error("trusted CLI output"))?;
    let body: Value =
        serde_json::from_str(text).map_err(|_| qoder_error("trusted CLI contract"))?;
    if body["status"] != "succeeded"
        || !body["data"]["commands"].as_array().is_some_and(|commands| {
            commands
                .iter()
                .any(|command| command["command_id"] == "schema.list")
        })
    {
        return Err(qoder_error("trusted CLI contract"));
    }
    Ok(text.trim().to_owned())
}

#[cfg(windows)]
pub(super) fn cli_baseline(cli: &Path, root: &Path) -> Result<String, QoderNativeError> {
    let mut command = Command::new(cli);
    command
        .args(["schema", "list", "--output", "json"])
        .env_clear()
        .env("HOME", root)
        .env("PATH", super::native_ingress_probe::native_probe_path());
    let bytes =
        super::windows_runtime::bounded(&command, false, Duration::from_secs(10), 128 * 1024)
            .map_err(|_| qoder_error("trusted CLI baseline"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| qoder_error("trusted CLI output"))?;
    let body: Value =
        serde_json::from_str(text).map_err(|_| qoder_error("trusted CLI contract"))?;
    if body["status"] != "succeeded"
        || !body["data"]["commands"].as_array().is_some_and(|commands| {
            commands
                .iter()
                .any(|command| command["command_id"] == "schema.list")
        })
    {
        return Err(qoder_error("trusted CLI contract"));
    }
    Ok(text.trim().to_owned())
}

#[cfg(test)]
fn observed_final_output(
    observer: &File,
    step: usize,
    requests: usize,
    started: &Instant,
    observe: impl FnOnce(OutputReadDiagnostic),
) -> std::io::Result<zeroize::Zeroizing<Vec<u8>>> {
    read_bounded_file(observer, 512 * 1024).inspect_err(|error| {
        observe(output_read_diagnostic(
            observer,
            "final_read",
            Some(error),
            step,
            requests,
            started.elapsed(),
        ));
    })
}

#[cfg(test)]
fn safe_output_bound(file: &File, step: usize, requests: usize, elapsed: Duration) {
    let diagnostic = output_read_diagnostic(file, "live_guard", None, step, requests, elapsed);
    diagnostic.emit();
    // Preserve the original closed frame for older observers. Only this
    // original live-guard branch emits it; final reads have their own phase.
    if let (Some(bytes), Some(types)) = (diagnostic.sampled_bytes, diagnostic.record_counts) {
        eprintln!(
            "native Qoder acceptance stage=output-bound challenge-step={step} requests={requests} elapsed-ms={} sampled-bytes={bytes} record-counts-system-assistant-user-result-stream-other-invalid={types:?}",
            elapsed.as_millis(),
        );
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
struct ClosedOutputIo {
    kind: &'static str,
    raw_os_error: Option<i32>,
}

#[cfg(test)]
impl ClosedOutputIo {
    fn from_error(error: &std::io::Error) -> Self {
        use std::io::ErrorKind;
        let kind = match error.kind() {
            ErrorKind::NotFound => "not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::InvalidData => "invalid_data",
            ErrorKind::UnexpectedEof => "unexpected_eof",
            ErrorKind::Interrupted => "interrupted",
            ErrorKind::TimedOut => "timed_out",
            ErrorKind::WouldBlock => "would_block",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::OutOfMemory => "out_of_memory",
            ErrorKind::IsADirectory => "is_a_directory",
            ErrorKind::NotADirectory => "not_a_directory",
            ErrorKind::Other => "other",
            _ => "unknown",
        };
        Self {
            kind,
            raw_os_error: error.raw_os_error(),
        }
    }
}

#[cfg(test)]
#[derive(Debug, serde::Serialize)]
struct OutputReadDiagnostic {
    phase: &'static str,
    challenge_step: usize,
    requests: usize,
    elapsed_ms: u128,
    observer_length: Option<u64>,
    observer_metadata_error: Option<ClosedOutputIo>,
    read_error: Option<ClosedOutputIo>,
    sample_stage: &'static str,
    sample_error: Option<ClosedOutputIo>,
    sampled_bytes: Option<usize>,
    record_counts: Option<[usize; 7]>,
}

#[cfg(test)]
impl OutputReadDiagnostic {
    fn emit(&self) {
        eprintln!(
            "native Qoder acceptance stage=output-read diagnostic={}",
            serde_json::to_string(self).expect("closed output-read facts are serializable")
        );
    }
}

#[cfg(test)]
fn output_read_diagnostic(
    file: &File,
    phase: &'static str,
    error: Option<&std::io::Error>,
    step: usize,
    requests: usize,
    elapsed: Duration,
) -> OutputReadDiagnostic {
    let metadata = file.metadata();
    let mut diagnostic = OutputReadDiagnostic {
        phase,
        challenge_step: step,
        requests,
        elapsed_ms: elapsed.as_millis(),
        observer_length: metadata.as_ref().ok().map(std::fs::Metadata::len),
        observer_metadata_error: metadata.as_ref().err().map(ClosedOutputIo::from_error),
        read_error: error.map(ClosedOutputIo::from_error),
        sample_stage: "clone",
        sample_error: None,
        sampled_bytes: None,
        record_counts: None,
    };
    if let Err(error) = sample_output(file, &mut diagnostic) {
        diagnostic.sample_error = Some(ClosedOutputIo::from_error(&error));
    }
    diagnostic
}

#[cfg(test)]
fn sample_output(file: &File, diagnostic: &mut OutputReadDiagnostic) -> std::io::Result<()> {
    // All callers run only after stop has confirmed the native process scope.
    // Cloning shares the writer's seek pointer, so this must never run live.
    use std::io::{Seek, SeekFrom};
    let mut file = file.try_clone()?;
    diagnostic.sample_stage = "seek";
    file.seek(SeekFrom::Start(0))?;
    diagnostic.sample_stage = "read";
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(512 * 1024).read_to_end(&mut bytes)?;
    let mut types = [0usize; 7];
    for line in bytes.split(|byte| *byte == b'\n') {
        let parsed = serde_json::from_slice::<Value>(line).ok();
        let index = match parsed.as_ref().and_then(|value| value["type"].as_str()) {
            Some("system") => 0,
            Some("assistant") => 1,
            Some("user") => 2,
            Some("result") => 3,
            Some("stream_event") => 4,
            Some(_) => 5,
            None => 6,
        };
        types[index] += 1;
    }
    diagnostic.sample_stage = "complete";
    diagnostic.sampled_bytes = Some(bytes.len());
    diagnostic.record_counts = Some(types);
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn qoder_bash_command_rejects_the_old_windows_powershell_prefix() {
        let path = Path::new("C:/literal space ' ; $probe & 中文/hiroute.exe");
        let old = format!(
            "& '{}' schema list --output json",
            path.to_str().unwrap().replace('\'', "''")
        );
        let new = cli_command(path).unwrap();
        let parse = |command: &str| {
            Command::new("/bin/bash")
                .args(["-n", "-c", command])
                .env_clear()
                .output()
                .unwrap()
        };
        let rejected = parse(&old);
        let control = parse(bash_parse_tests::MINIMAL_INVALID);
        assert!(bash_parse_tests::correlated_rejection(
            &old, &rejected, &control
        ));
        let parsed = parse(&new);
        assert!(parsed.status.success() && parsed.stdout.is_empty() && parsed.stderr.is_empty());
    }

    #[test]
    fn qoder_bash_quote_preserves_literal_apostrophe_and_shell_characters() {
        let path = Path::new("C:/literal space ' ; $probe & 中文/hiroute.exe");
        let arguments = format!("printf '%s' {}", shell_quote(path).unwrap());
        let output = Command::new("/bin/bash")
            .args(["-c", &arguments])
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success() && output.stderr.is_empty());
        assert!(output.stdout == path.as_os_str().as_encoded_bytes());
        let old = format!(
            "printf '%s' '{}'",
            path.to_str().unwrap().replace('\'', "''")
        );
        let output = Command::new("/bin/bash")
            .args(["-c", &old])
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success() && output.stderr.is_empty());
        assert!(output.stdout != path.as_os_str().as_encoded_bytes());
    }

    #[test]
    fn older_completion_cannot_replace_a_newer_explicit_check() {
        let mut cache = QoderProofCache::default();
        let older = cache.begin().unwrap();
        let newer = cache.begin().unwrap();
        let evidence = QoderCollaborationEvidence {
            executable: "/fixture".into(),
            cli: "/fixture-cli".into(),
            dependencies: CanonicalDigest::of_bytes(b"fixture"),
            target: QoderCollaborationProbeTarget::PreinstallCapability,
            proof: CanonicalDigest::of_bytes(b"fixture"),
            observed: Instant::now(),
            observed_at: 1,
        };
        cache.finish(newer, evidence.clone()).unwrap();
        assert!(cache.finish(older, evidence).is_err());
        assert!(cache.evidence.is_some());
        cache.begin().unwrap();
        assert!(
            cache.evidence.is_none(),
            "a failed new attempt must not reuse prior success"
        );
    }

    #[test]
    fn installed_skill_drift_is_not_an_installation_or_proof() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("SKILL.md");
        fs::write(&path, b"confirmed").unwrap();
        let digest = CanonicalDigest::of_bytes(b"confirmed");
        assert!(confirmed_skill(&path, &digest).is_ok());
        fs::write(&path, b"changed").unwrap();
        assert!(confirmed_skill(&path, &digest).is_err());
    }

    #[test]
    #[ignore = "requires explicitly selected normally logged-in Qoder context and trusted HiRoute CLI"]
    fn real_qoder_discovers_skill_and_runs_read_only_cli_in_selected_context() {
        let path = |key| PathBuf::from(std::env::var_os(key).expect("explicit native check input"));
        let context = QoderNativeContext::from_selected(
            &path("HIROUTE_NATIVE_QODER_HOME"),
            Some(&path("HIROUTE_NATIVE_QODER_CONFIG")),
        )
        .unwrap();
        QoderCollaborationProbe::run(
            &path("HIROUTE_NATIVE_QODER"),
            &context,
            &path("HIROUTE_NATIVE_HIROUTE_CLI"),
            QoderCollaborationProbeTarget::PreinstallCapability,
        )
        .unwrap();
    }
}
