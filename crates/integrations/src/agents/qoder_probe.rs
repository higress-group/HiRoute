//! Explicit collaboration checks in the selected, normally logged-in Qoder context.
//! Auth is read only by Qoder. HiRoute owns the private workspace and local challenge, not HOME.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::native_probe_process::{NativeProbeProcess, read_bounded};
use super::qoder::{
    QoderNativeContext, QoderNativeError, QoderTransientRouteInput, qoder_error,
    render_qoder_transient_route,
};
use hiroute_domain::{
    AgentCapability, CanonicalDigest, CapabilityState, SupportedAgentInstallationV1,
};
use serde_json::Value;

#[path = "qoder_probe_http.rs"]
mod http;

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
        let root = tempfile::Builder::new()
            .prefix("hiroute-qoder-check-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .map_err(|_| qoder_error("private workspace"))?;
        let root_path =
            fs::canonicalize(root.path()).map_err(|_| qoder_error("private workspace"))?;
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
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(path.parent().unwrap())
                    .map_err(|_| qoder_error("private Skill"))?;
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
        let skill_name = skill_directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| qoder_error("Skill target encoding"))?
            .to_owned();
        let body = skill_body(&content)?;
        let expected_cli = cli_baseline(&cli, &root_path)?;
        let command = format!("{} schema list --output json", shell_quote(&cli)?);
        let mut challenge = http::Challenge {
            skill_name,
            skill_directory: skill_directory
                .to_str()
                .ok_or_else(|| qoder_error("Skill target encoding"))?
                .into(),
            skill_body: body,
            command,
            expected_cli,
            step: 0,
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
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin")
            .env("HIROUTE_QODER_PROBE_TOKEN", MARKER)
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .current_dir(&workspace)
            .process_group(0);
        let mut child =
            NativeProbeProcess::new(command.spawn().map_err(|_| qoder_error("native launch"))?);
        let started = Instant::now();
        let status = loop {
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
                    http::serve(&mut stream, MARKER, MODEL, &mut challenge).map_err(qoder_error)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(qoder_error("loopback accept")),
            }
            if let Some(status) = child.observe().map_err(|_| qoder_error("native wait"))? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        child
            .stop()
            .map_err(|_| qoder_error("native process scope"))?;
        let output = read_bounded(&root_path.join("output.private"), 512 * 1024)
            .map_err(|_| qoder_error("native output bound"))?;
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

fn confirmed_skill(path: &Path, expected: &CanonicalDigest) -> Result<String, QoderNativeError> {
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

fn skill_body(content: &str) -> Result<String, QoderNativeError> {
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
        Ok((path, m.dev(), m.ino(), m.uid(), m.gid(), m.mode()))
    };
    let settings = match fs::symlink_metadata(context.config_root.join("settings.json")) {
        Ok(m) => Some((
            m.dev(),
            m.ino(),
            m.mode(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )),
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
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| qoder_error("private directory"))
}
fn private_file(path: &Path) -> Result<File, QoderNativeError> {
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| qoder_error("private file"))
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

fn cli_baseline(cli: &Path, root: &Path) -> Result<String, QoderNativeError> {
    let output = private_file(&root.join("cli.private"))?;
    let observer = output
        .try_clone()
        .map_err(|_| qoder_error("trusted CLI baseline"))?;
    let mut child = NativeProbeProcess::new(
        Command::new(cli)
            .args(["schema", "list", "--output", "json"])
            .env_clear()
            .env("HOME", root)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| qoder_error("trusted CLI baseline"))?,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

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
