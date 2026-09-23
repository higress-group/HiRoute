//! Explicit, admitted native consumption probe. Discovery/Preview never call this module.
//! The existing Operation owns all configuration writes and restoration; no temp-root cleanup
//! occurs here. A pending result becomes usable evidence only after a recorded restoration.
use super::{CodexFileConfiguration, CodexSelectionTarget, configure_codex_native};
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentCapability, CanonicalDigest, CapabilityState,
    EffectReconciliation, ExternalEffectIntentV1, ModelAlias, NativeAgentArtifactPort, OperationId,
    SupportedAgentInstallationV1,
};
use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "ephemeral_ingress_probe.rs"]
mod ephemeral;
#[path = "native_probe_http.rs"]
mod http;
const CONTRACT: &str = "hiroute.codex-native-responses/v1";

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("native ingress probe could not establish {0}")]
pub struct NativeIngressProbeError(&'static str);

/// A fresh local challenge, not an upstream credential or a HiRoute authorization grant.
/// No Debug/serde: the local challenge must not enter public diagnostics.
pub struct CodexNativeIngressProbe {
    listener: TcpListener,
    endpoint: String,
    grant: AgentAccessGrantMaterial,
    model: ModelAlias,
    empty: CanonicalDigest,
}
impl CodexNativeIngressProbe {
    pub fn bind() -> Result<Self, NativeIngressProbeError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| fail("loopback bind"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| fail("loopback mode"))?;
        let endpoint = format!(
            "http://{}/v1",
            listener.local_addr().map_err(|_| fail("address"))?
        );
        let mut entropy = [0; 32];
        getrandom::fill(&mut entropy).map_err(|_| fail("randomness"))?;
        Ok(Self {
            listener,
            endpoint,
            grant: AgentAccessGrantMaterial::from_csprng_entropy(entropy),
            model: ModelAlias::parse("hiroute/0011223344556677")
                .map_err(|_| fail("probe model"))?,
            empty: CanonicalDigest::of_bytes(b""),
        })
    }
    /// Stage this configuration using the existing admitted Operation artifact effect.
    /// The caller must allocate an isolated, originally absent home/.codex/config.toml target.
    pub fn configuration(&self) -> CodexFileConfiguration<'_> {
        CodexFileConfiguration {
            expected_content: &self.empty,
            selection: CodexSelectionTarget::Root,
            provider_id: "hiroute",
            endpoint: &self.endpoint,
            model: Some(self.model.as_str()),
            local_grant: &self.grant,
            model_catalog: None,
            managed_aliases: &[],
        }
    }
    /// Only the trusted backend calls this after explicit verification admission. The supplied
    /// home must contain exactly .codex/config.toml; cwd must be empty and private. No inherited
    /// auth, shell environment, project settings, rules or copied session material is accepted.
    /// The caller must restore the applied effect on both Ok and Err before cleaning its root.
    pub fn run(
        self,
        executable: &Path,
        home: &Path,
        workspace: &Path,
        port: &dyn NativeAgentArtifactPort,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> Result<PendingCodexIngressEvidence, NativeIngressProbeError> {
        let binary = super::executable::resolve(executable)
            .map_err(|_| fail("trusted executable"))?
            .ok_or_else(|| fail("executable"))?;
        private_directory(home)?;
        private_directory(workspace)?;
        let config_root = home.join(".codex");
        private_directory(&config_root)?;
        if entries(home)? != [".codex"]
            || entries(&config_root)? != ["config.toml"]
            || !entries(workspace)?.is_empty()
            || home == workspace
            || Path::new("/etc/codex/config.toml").exists()
            || Path::new("/etc/codex/managed_config.toml").exists()
        {
            return Err(fail("isolated native configuration"));
        }
        if intent.before_fingerprint().is_some()
            || !matches!(
                port.observe_artifact(operation, intent),
                Ok(EffectReconciliation::Applied(_))
            )
        {
            return Err(fail("applied isolated effect"));
        }
        let expected = configure_codex_native(
            "",
            &self.empty,
            CodexSelectionTarget::Root,
            "hiroute",
            &self.endpoint,
            Some(self.model.as_str()),
            &self.grant,
        )
        .map_err(|_| fail("native rendering"))?;
        let config_path = config_root.join("config.toml");
        let bytes = super::filesystem_config::read_validated_config_bytes(&config_path)
            .map_err(|_| fail("native configuration read"))?
            .ok_or_else(|| fail("native configuration"))?
            .0;
        let stored = port
            .read_native_target(intent.target())
            .map_err(|_| fail("artifact read"))?
            .ok_or_else(|| fail("artifact target"))?;
        if bytes.as_slice() != expected.rendered.as_bytes() || *stored != *bytes {
            return Err(fail("native target binding"));
        }
        let mut child = Command::new(&binary)
            .args([
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--ignore-rules",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--json",
                "Reply with OK only. Do not use tools.",
            ])
            .env_clear()
            .env("HOME", home)
            .env("CODEX_HOME", &config_root)
            .env("TMPDIR", workspace)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| fail("native launch"))?;
        let started = Instant::now();
        let mut requests = 0;
        let result = (|| {
            loop {
                match self.listener.accept() {
                    Ok((mut stream, _)) => {
                        requests += 1;
                        if requests > 4 {
                            return Err(fail("request bound"));
                        }
                        http::serve(&mut stream, self.grant.expose(), self.model.as_str())
                            .map_err(fail)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(fail("loopback accept")),
                }
                if let Some(status) = child.try_wait().map_err(|_| fail("native wait"))? {
                    return if status.success() && requests > 0 {
                        Ok(())
                    } else {
                        Err(fail("authenticated native completion"))
                    };
                }
                if started.elapsed() > Duration::from_secs(25) {
                    return Err(fail("deadline"));
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
        Ok(PendingCodexIngressEvidence {
            observed: Instant::now(),
            observed_at: now(),
            original_operation: operation.clone(),
            target: intent.target().to_owned(),
        })
    }
}

/// Cannot be serialized or attached to discovery until the formal restore effect is observed.
pub struct PendingCodexIngressEvidence {
    observed: Instant,
    observed_at: u64,
    original_operation: OperationId,
    target: String,
}
impl PendingCodexIngressEvidence {
    pub fn after_restoration(
        self,
        port: &dyn NativeAgentArtifactPort,
        restore_operation: &OperationId,
        restore_intent: &ExternalEffectIntentV1,
    ) -> Result<CodexIngressEvidence, NativeIngressProbeError> {
        if restore_operation == &self.original_operation
            || restore_intent.target() != self.target
            || !matches!(
                port.observe_artifact(restore_operation, restore_intent),
                Ok(EffectReconciliation::Applied(_))
            )
            || port
                .read_native_target(&self.target)
                .map_err(|_| fail("restored target read"))?
                .is_some()
        {
            return Err(fail("recorded native restoration"));
        }
        Ok(CodexIngressEvidence {
            observed: self.observed,
            observed_at: self.observed_at,
            collaboration: false,
        })
    }
}

/// Backend-only cache entry. Five-minute expiry requires a new explicit probe. The successful
/// challenge is intentionally not rebound to a Codex binary identity, version, or release.
#[derive(Clone)]
pub struct CodexIngressEvidence {
    observed: Instant,
    observed_at: u64,
    collaboration: bool,
}
impl CodexIngressEvidence {
    pub(super) fn attach_collaboration(&self, installation: &mut SupportedAgentInstallationV1) {
        if self.collaboration {
            self.attach(installation);
        }
    }

    pub(super) fn attach(&self, installation: &mut SupportedAgentInstallationV1) {
        if self.observed.elapsed() > Duration::from_secs(300) {
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
            self.collaboration,
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
                    proof.adapter_contract = "hiroute.codex-collaboration/v1".into();
                    proof.observed_at_unix_ms = self.observed_at;
                }
            }
        }
    }
}
pub(super) fn fail(reason: &'static str) -> NativeIngressProbeError {
    NativeIngressProbeError(reason)
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(1)
        .max(1)
}
pub(super) fn private_directory(path: &Path) -> Result<(), NativeIngressProbeError> {
    let meta = fs::symlink_metadata(path).map_err(|_| fail("private directory"))?;
    if !path.is_absolute()
        || !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != nix::unistd::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(fail("private directory"));
    }
    Ok(())
}
fn entries(path: &Path) -> Result<Vec<String>, NativeIngressProbeError> {
    let mut names = Vec::new();
    for item in fs::read_dir(path)
        .map_err(|_| fail("isolated directory"))?
        .take(3)
    {
        names.push(
            item.map_err(|_| fail("isolated entry"))?
                .file_name()
                .into_string()
                .map_err(|_| fail("isolated entry name"))?,
        );
    }
    names.sort();
    Ok(names)
}
pub(super) fn binary_identity(
    executable: &Path,
) -> Result<CanonicalDigest, NativeIngressProbeError> {
    let path = super::executable::resolve(executable)
        .map_err(|_| fail("trusted executable"))?
        .ok_or_else(|| fail("executable"))?;
    let meta = fs::metadata(&path).map_err(|_| fail("executable metadata"))?;
    CanonicalDigest::of(&(
        path,
        meta.dev(),
        meta.ino(),
        meta.uid(),
        meta.gid(),
        meta.mode(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    ))
    .map_err(|_| fail("executable identity"))
}

pub(super) fn cache_error() -> NativeIngressProbeError {
    fail("native evidence cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{ConnectorRegistryBundleV1, ReleaseModelDataBundleV2};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn native_evidence_expires_but_does_not_gate_on_codex_binary_identity() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let binary = root.path().join("codex");
        fs::write(&binary, b"trusted fixture identity, never executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let proof = CodexIngressEvidence {
            observed: Instant::now(),
            observed_at: now(),
            collaboration: false,
        };
        let sample = || {
            let super::super::AgentDiscoveryOutcomeV1::Supported { mut installation } =
                super::super::resolve_agent_observation(super::super::AgentScanObservationV1 {
                    schema: super::super::AGENT_SCAN_OBSERVATION_SCHEMA_V1.into(),
                    agent_id: "agent.codex".into(),
                    kind: hiroute_domain::AgentKindV1::Codex,
                    version: "future-build".into(),
                    config: vec![],
                })
            else {
                panic!("fixture installation");
            };
            super::super::observed_capabilities::attach_target_file_capabilities(
                &mut installation,
                &root.path().join("config.toml"),
            );
            installation
        };
        let mut fresh = sample();
        proof.attach(&mut fresh);
        assert!(
            fresh
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_ok()
        );
        let mut expired = proof.clone();
        expired.observed = Instant::now() - Duration::from_secs(301);
        let mut without_fresh_proof = sample();
        expired.attach(&mut without_fresh_proof);
        assert!(
            without_fresh_proof
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_err()
        );
        fs::rename(&binary, root.path().join("old-codex")).unwrap();
        fs::write(&binary, b"replacement fixture identity, never executed").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut replaced = sample();
        proof.attach(&mut replaced);
        assert!(
            replaced
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_ok()
        );
    }

    #[test]
    fn collaboration_settings_consume_only_an_explicit_collaboration_check() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let home = root.path().join("home");
        let config_dir = home.join(".codex");
        fs::create_dir_all(&config_dir).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let config = config_dir.join("config.toml");
        fs::write(&config, b"# private fixture\n").unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let binary = root.path().join("codex");
        fs::write(&binary, b"#!/bin/sh\nprintf 'codex-cli 99.99.99\\n'\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut layout = super::super::AgentFilesystemLayoutV1::from_process(&home, root.path());
        layout.codex_user_config = config;
        layout.codex_executable = binary;
        layout.codex_desktop_executable = None;
        let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
            "../../../../assets/connector-registry/current/registry-seed.json"
        ))
        .unwrap();
        let models: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
            "../../../../assets/release-facts/current/bundle/model-data.json"
        ))
        .unwrap();
        let registry = super::super::ClaudeRegistrationIndexV1::from_verified_model_data(
            &registry,
            &models.data,
        )
        .unwrap();
        let scanner = super::super::FilesystemAgentScannerV1::new(layout.clone(), registry.clone())
            .with_codex_ingress_evidence(CodexIngressEvidence {
                observed: Instant::now(),
                observed_at: now(),
                collaboration: true,
            });
        let supported = |discovery: super::super::FilesystemAgentDiscoveryV1| {
            let super::super::AgentDiscoveryOutcomeV1::Supported { installation } =
                discovery.outcome
            else {
                panic!("supported Codex fixture");
            };
            installation
        };
        let model_only = supported(scanner.codex_settings_discovery(false));
        let collaboration = supported(scanner.codex_settings_discovery(true));
        assert!(
            model_only
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );
        assert!(
            collaboration
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_ok()
        );
        assert_ne!(
            model_only.observation_digest,
            collaboration.observation_digest
        );

        let auth_only = super::super::FilesystemAgentScannerV1::new(layout, registry)
            .with_codex_ingress_evidence(CodexIngressEvidence {
                observed: Instant::now(),
                observed_at: now(),
                collaboration: false,
            });
        assert!(
            supported(auth_only.codex_settings_discovery(true))
                .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
                .is_err()
        );
    }
}
