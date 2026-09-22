//! Explicit, admitted Claude Code authentication probe.
//!
//! The probe runs the trusted installation against a private loopback challenge with an isolated
//! HOME and settings file. It never reads daily Claude configuration or contacts a provider.
use super::native_ingress_probe::{
    NativeIngressProbeError, binary_identity, fail, now, private_directory,
};
use hiroute_domain::{
    AgentCapability, CanonicalDigest, CapabilityState, SupportedAgentInstallationV1,
};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::collaboration_probe as collaboration;
#[path = "native_claude_probe_http.rs"]
mod http;

const CONTRACT: &str = "hiroute.claude-native-messages/v1";
const MARKER: &str = "hiroute-native-probe-no-authority";
const MODEL: &str = "hiroute/0011223344556677";
const CONNECTION: &str = "agent-connection/claude/native-probe";

pub struct ClaudeNativeIngressProbe;

impl ClaudeNativeIngressProbe {
    pub fn run(executable: &Path) -> Result<ClaudeIngressEvidence, NativeIngressProbeError> {
        Self::run_isolated(executable, None)
    }

    pub fn run_collaboration(
        executable: &Path,
        cli: &Path,
    ) -> Result<ClaudeIngressEvidence, NativeIngressProbeError> {
        Self::run_isolated(executable, Some(cli))
    }

    fn run_isolated(
        executable: &Path,
        cli: Option<&Path>,
    ) -> Result<ClaudeIngressEvidence, NativeIngressProbeError> {
        let binary = super::executable::resolve(executable)
            .map_err(|_| fail("trusted executable"))?
            .ok_or_else(|| fail("executable"))?;
        let before = binary_identity(&binary)?;
        let root = tempfile::Builder::new()
            .prefix("hiroute-native-claude-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .map_err(|_| fail("private check directory"))?;
        let root_path =
            fs::canonicalize(root.path()).map_err(|_| fail("private check directory"))?;
        fs::set_permissions(&root_path, fs::Permissions::from_mode(0o700))
            .map_err(|_| fail("private check directory"))?;
        private_directory(&root_path)?;
        let home = root_path.join("home");
        let config = home.join(".claude");
        let workspace = root_path.join("workspace");
        for path in [&home, &config, &workspace] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| fail("private check directory"))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| fail("private check directory"))?;
            private_directory(path)?;
        }
        let (mut challenge, plugin_dir) = if let Some(cli) = cli {
            let (challenge, plugin_dir) =
                collaboration::CollaborationChallenge::prepare_claude(&config, &home, cli)
                    .map_err(fail)?;
            (Some(challenge), Some(plugin_dir))
        } else {
            (None, None)
        };

        let helper = root_path.join("grant-helper");
        let mut helper_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&helper)
            .map_err(|_| fail("private helper"))?;
        write!(
            helper_file,
            "#!/bin/sh\n[ \"$1\" = '{}' ] || exit 2\n[ \"$2\" = '{}' ] || exit 3\nprintf '%s\\n' '{}'\n",
            hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
            CONNECTION,
            MARKER,
        )
        .map_err(|_| fail("private helper"))?;
        helper_file.sync_all().map_err(|_| fail("private helper"))?;
        drop(helper_file);
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))
            .map_err(|_| fail("private helper"))?;

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| fail("loopback bind"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| fail("loopback mode"))?;
        let endpoint = format!(
            "http://{}",
            listener.local_addr().map_err(|_| fail("address"))?
        );
        let settings = root_path.join("settings.json");
        let helper_command = [
            helper.to_string_lossy().into_owned(),
            hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1.to_owned(),
            CONNECTION.to_owned(),
        ]
        .iter()
        .map(|value| shell_quote(value))
        .collect::<Vec<_>>()
        .join(" ");
        let mut settings_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&settings)
            .map_err(|_| fail("private settings"))?;
        serde_json::to_writer(
            &mut settings_file,
            &json!({
                "apiKeyHelper": helper_command,
                "env": {
                    "ANTHROPIC_BASE_URL": endpoint,
                    "ANTHROPIC_MODEL": MODEL,
                }
            }),
        )
        .map_err(|_| fail("private settings"))?;
        settings_file
            .sync_all()
            .map_err(|_| fail("private settings"))?;
        drop(settings_file);

        let output_path = root_path.join("output.private");
        let output = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(output_path)
            .map_err(|_| fail("private output"))?;
        let observer = output.try_clone().map_err(|_| fail("private output"))?;
        let mut command = Command::new(&binary);
        command
            .args([
                "--bare",
                "--print",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--settings",
            ])
            .arg(&settings)
            .args([
                "--tools",
                if cli.is_some() { "Bash" } else { "" },
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--no-chrome",
                "--permission-mode",
                "dontAsk",
                "--output-format",
                "json",
            ]);
        if let Some(plugin_dir) = &plugin_dir {
            command.arg("--plugin-dir").arg(plugin_dir);
            command.args(["--allowedTools", "Bash", "--permission-mode", "dontAsk"]);
        }
        command.arg(if cli.is_some() {
            "/hiroute-native-probe:hiroute-probe"
        } else {
            "Reply with OK only. Do not use tools."
        });
        let mut child = command
            .env_clear()
            .env("HOME", &home)
            .env("CLAUDE_CONFIG_DIR", &config)
            .env("TMPDIR", &workspace)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                output.try_clone().map_err(|_| fail("private output"))?,
            ))
            .stderr(Stdio::from(output))
            .process_group(0)
            .spawn()
            .map_err(|_| fail("native launch"))?;
        let started = Instant::now();
        let mut requests = 0;
        let mut authenticated_message = false;
        let result = (|| {
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        requests += 1;
                        if requests > 8 {
                            return Err(fail("request bound"));
                        }
                        authenticated_message |=
                            http::serve(&mut stream, MARKER, MODEL, challenge.as_mut())
                                .map_err(fail)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(fail("loopback accept")),
                }
                if let Some(status) = child.try_wait().map_err(|_| fail("native wait"))? {
                    return if status.success()
                        && authenticated_message
                        && challenge.as_ref().is_none_or(|check| check.complete())
                    {
                        Ok(())
                    } else {
                        Err(fail("authenticated native completion"))
                    };
                }
                if started.elapsed() > Duration::from_secs(45) {
                    return Err(fail("deadline"));
                }
                if observer
                    .metadata()
                    .map_err(|_| fail("private output"))?
                    .len()
                    > 128 * 1024
                {
                    return Err(fail("output bound"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if child.try_wait().ok().flatten().is_none() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = child.wait();
        result?;
        if binary_identity(&binary)? != before {
            return Err(fail("stable executable"));
        }
        Ok(ClaudeIngressEvidence {
            binary: before,
            observed: Instant::now(),
            observed_at: now(),
            collaboration: cli.is_some(),
        })
    }
}

#[derive(Clone)]
pub struct ClaudeIngressEvidence {
    binary: CanonicalDigest,
    observed: Instant,
    observed_at: u64,
    collaboration: bool,
}

impl ClaudeIngressEvidence {
    pub(super) fn attach_authentication(
        &self,
        executable: &Path,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if !self.collaboration {
            self.attach(executable, installation);
        }
    }

    pub(super) fn attach_collaboration(
        &self,
        executable: &Path,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if self.collaboration {
            self.attach(executable, installation);
        }
    }

    pub(super) fn attach(
        &self,
        executable: &Path,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if self.observed.elapsed() > Duration::from_secs(300)
            || binary_identity(executable).ok().as_ref() != Some(&self.binary)
        {
            return;
        }
        let Some(index) = installation
            .capability_evidence
            .iter()
            .position(|proof| proof.capability == AgentCapability::IngressAuthentication)
        else {
            return;
        };
        installation.observation_digest = CanonicalDigest::of(&(
            CONTRACT,
            &installation.observation_digest,
            &self.binary,
            self.observed_at,
        ))
        .expect("bounded native evidence");
        for proof in &mut installation.capability_evidence {
            proof.dependency_digest = installation.observation_digest.clone();
        }
        let proof = &mut installation.capability_evidence[index];
        proof.state = CapabilityState::Proven;
        proof.reason = None;
        proof.adapter_contract = CONTRACT.into();
        proof.observed_at_unix_ms = self.observed_at;
        if self.collaboration {
            for proof in &mut installation.capability_evidence {
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
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicitly selected Claude Code and HiRoute CLI binaries"]
    fn real_claude_collaboration_checks_skill_and_read_only_cli_in_private_home() {
        let claude = std::path::PathBuf::from(
            std::env::var_os("HIROUTE_NATIVE_CLAUDE").expect("selected Claude binary"),
        );
        let cli = std::path::PathBuf::from(
            std::env::var_os("HIROUTE_NATIVE_HIROUTE_CLI").expect("selected HiRoute CLI"),
        );
        ClaudeNativeIngressProbe::run_collaboration(&claude, &cli).unwrap();
    }

    #[test]
    fn claude_evidence_is_bound_to_the_exact_binary() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let binary = root.path().join("claude");
        fs::write(&binary, b"trusted fixture identity, never executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let evidence = ClaudeIngressEvidence {
            binary: binary_identity(&binary).unwrap(),
            observed: Instant::now(),
            observed_at: now(),
            collaboration: false,
        };
        let sample = || {
            let super::super::AgentDiscoveryOutcomeV1::Supported { mut installation } =
                super::super::resolve_agent_observation(super::super::AgentScanObservationV1 {
                    schema: super::super::AGENT_SCAN_OBSERVATION_SCHEMA_V1.into(),
                    agent_id: "agent_claude_default".into(),
                    kind: hiroute_domain::AgentKindV1::ClaudeCode,
                    version: "2.1.231".into(),
                    config: vec![],
                })
            else {
                panic!("fixture installation");
            };
            super::super::observed_capabilities::attach_file_capabilities(
                &mut installation,
                &binary,
                &root.path().join("settings.json"),
            );
            installation
        };
        let mut proven = sample();
        evidence.attach(&binary, &mut proven);
        let configure = proven.require_action(hiroute_domain::AgentAction::ConfigureModel);
        assert!(
            configure.is_ok(),
            "fresh exact-binary evidence was blocked: {configure:?}"
        );
        fs::rename(&binary, root.path().join("old-claude")).unwrap();
        fs::write(&binary, b"replacement fixture identity, never executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut replaced = sample();
        evidence.attach(&binary, &mut replaced);
        assert!(
            replaced
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_err()
        );
    }

    #[test]
    fn claude_collaboration_requires_a_fresh_exact_binary_check() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let binary = root.path().join("claude");
        fs::write(&binary, b"private fixture, not executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let sample = || {
            let super::super::AgentDiscoveryOutcomeV1::Supported { mut installation } =
                super::super::resolve_agent_observation(super::super::AgentScanObservationV1 {
                    schema: super::super::AGENT_SCAN_OBSERVATION_SCHEMA_V1.into(),
                    agent_id: "agent_claude_default".into(),
                    kind: hiroute_domain::AgentKindV1::ClaudeCode,
                    version: "2.1.231".into(),
                    config: vec![],
                })
            else {
                panic!("fixture installation");
            };
            super::super::observed_capabilities::attach_file_capabilities(
                &mut installation,
                &binary,
                &root.path().join("settings.json"),
            );
            installation
        };
        let mut native_auth_only = sample();
        let authentication = ClaudeIngressEvidence {
            binary: binary_identity(&binary).unwrap(),
            observed: Instant::now(),
            observed_at: now(),
            collaboration: false,
        };
        authentication.attach_authentication(&binary, &mut native_auth_only);
        assert!(
            native_auth_only
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_ok()
        );
        assert!(
            native_auth_only
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );

        let evidence = ClaudeIngressEvidence {
            binary: binary_identity(&binary).unwrap(),
            observed: Instant::now(),
            observed_at: now(),
            collaboration: true,
        };
        let mut checked = sample();
        evidence.attach_collaboration(&binary, &mut checked);
        assert!(
            checked
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_ok()
        );
        let mut model_only = sample();
        evidence.attach_authentication(&binary, &mut model_only);
        assert!(
            model_only
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_err()
        );
        assert!(
            model_only
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );

        let mut expired = evidence.clone();
        expired.observed = Instant::now() - Duration::from_secs(301);
        expired.attach_collaboration(&binary, &mut model_only);
        assert!(
            model_only
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );
        fs::rename(&binary, root.path().join("old-claude")).unwrap();
        fs::write(&binary, b"replacement binary, not executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut replaced = sample();
        evidence.attach_collaboration(&binary, &mut replaced);
        assert!(
            replaced
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );
    }
}
