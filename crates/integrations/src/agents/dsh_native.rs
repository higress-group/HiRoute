//! DSH standard Web model target and native Skill resources; no internal SDK or history reader.
use super::super::executable::{self, ProbeFailure};
use super::*;
use hiroute_domain::{AgentCapability, CapabilityState};
#[cfg(windows)]
type DshRuntimeDependency = CanonicalDigest;
#[cfg(not(windows))]
type DshRuntimeDependency = (u64, Option<std::time::SystemTime>);

impl FilesystemAgentScannerV1 {
    /// Higher-priority home configuration is independent of the managed Web file.
    /// Recheck it on configure and restore, without requiring an installed CLI.
    pub fn dsh_web_default_in_use(
        &self,
        namespace: &str,
        models: &[hiroute_domain::AdditionalAgentModelV1],
    ) -> Result<bool, AgentFilesystemScanError> {
        let bytes = super::super::dsh_config::read_patch_bytes(
            &self.layout.dsh_config_root.join("cordis.patch.yml"),
        )?;
        let patch = super::super::dsh_config::Patch::parse(Some(&bytes))
            .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        let invalid = |_| AgentFilesystemScanError::InvalidConfig;
        if patch.config("llm-pi-ai").map_err(invalid)?.is_some() {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        let Some(default) = patch.config("agent-default-model").map_err(invalid)? else {
            return Ok(false);
        };
        let provider = default["provider"]
            .as_str()
            .ok_or(AgentFilesystemScanError::InvalidConfig)?;
        let model = default["model"]
            .as_str()
            .ok_or(AgentFilesystemScanError::InvalidConfig)?;
        let owned = provider == namespace
            || provider
                .strip_prefix(&format!("{namespace}-"))
                .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        Ok(owned
            && !models.iter().any(|m| {
                m.alias == model
                    && hiroute_domain::additional_model_provider_for(namespace, models, m)
                        == provider
            }))
    }
    pub fn dsh_user_models_target(&self) -> PathBuf {
        self.layout
            .dsh_config_root
            .join("profiles/web/cordis.patch.yml")
    }
    pub fn dsh_user_skill_target(&self) -> PathBuf {
        self.layout
            .dsh_config_root
            .join("skills/hiroute-collaboration/SKILL.md")
    }
    pub fn dsh_executable_target(&self) -> Option<PathBuf> {
        dsh_executable(&self.layout.dsh_executable, false)
            .ok()
            .flatten()
            .map(|(path, _)| path)
    }
    fn dsh_native_contract(
        &self,
        models: bool,
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let executable = self
            .dsh_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        self.dsh_native_contract_at(models, &executable, dsh_execution_dependency(&executable)?)
    }
    fn dsh_native_contract_at(
        &self,
        models: bool,
        executable: &Path,
        execution: DshRuntimeDependency,
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let mut dependencies = Vec::new();
        let profiles: &[&str] = if models {
            &["cordis.patch.yml", "profiles/web/cordis.patch.yml"]
        } else {
            &[
                "cordis.patch.yml",
                "profiles/web/cordis.patch.yml",
                "profiles/acp/cordis.patch.yml",
            ]
        };
        for relative in profiles {
            let path = self.layout.dsh_config_root.join(relative);
            let document = super::super::dsh_config::read_patch_bytes(&path)?;
            let bytes = document.as_slice();
            let patch = super::super::dsh_config::Patch::parse(Some(bytes))
                .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
            if models
                && *relative == "cordis.patch.yml"
                && patch
                    .config("llm-pi-ai")
                    .map_err(|_| AgentFilesystemScanError::InvalidConfig)?
                    .is_some()
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
            if models {
                dependencies.push(CanonicalDigest::of_bytes(bytes));
            } else {
                let resources = [
                    "skill-filesystem",
                    "tool-skill",
                    "tool-fs",
                    "tool-bash",
                    "skill",
                ]
                .map(|id| patch.config(id))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
                if let Some(config) = resources[0]
                    && (config
                        .get("dshHome")
                        .is_some_and(|v| v.as_str() != self.layout.dsh_config_root.to_str())
                        || config["includeDefaultRoots"] == false)
                {
                    return Err(AgentFilesystemScanError::InvalidConfig);
                }
                // Model-only changes cannot revoke a still-valid collaboration resource proof.
                dependencies.push(
                    CanonicalDigest::of(&resources)
                        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?,
                );
            }
        }
        #[cfg(windows)]
        let observation = CanonicalDigest::of(&(executable, execution, dependencies));
        #[cfg(not(windows))]
        let observation =
            CanonicalDigest::of(&(executable, execution.0, execution.1, dependencies));
        observation.map_err(|_| AgentFilesystemScanError::InvalidConfig)
    }
    pub fn dsh_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        self.dsh_discovery(false)
    }
    pub(super) fn scan_dsh(&self) -> FilesystemAgentDiscoveryV1 {
        self.dsh_discovery(true)
    }
    fn dsh_discovery(&self, version_diagnostic: bool) -> FilesystemAgentDiscoveryV1 {
        let (executable, version) =
            match dsh_executable(&self.layout.dsh_executable, version_diagnostic) {
                Ok(Some(observation)) => observation,
                result => {
                    return probe_report(
                        "agent_dsh_default",
                        AgentKindV1::DeepseekHarness,
                        match result {
                            Ok(None) => ExecutableProbe::NotFound,
                            Err(reason) => ExecutableProbe::Unknown(reason),
                            Ok(Some(_)) => unreachable!(),
                        },
                    );
                }
            };
        let mut outcome = resolve_agent_observation(base_observation(
            "agent_dsh_default",
            AgentKindV1::DeepseekHarness,
            version,
        ));
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            super::super::observed_capabilities::attach_file_capabilities(
                installation,
                &executable,
                &self.dsh_user_skill_target(),
            );
            if let Ok(digest) = dsh_execution_dependency(&executable)
                .and_then(|dependency| self.dsh_native_contract_at(false, &executable, dependency))
            {
                installation.observation_digest =
                    CanonicalDigest::of(&(&installation.observation_digest, &digest))
                        .expect("native contract");
                let checked = self
                    .dsh_collaboration_cli
                    .lock()
                    .ok()
                    .and_then(|v| v.clone());
                for proof in &mut installation.capability_evidence {
                    proof.dependency_digest = installation.observation_digest.clone();
                    if matches!(
                        proof.capability,
                        AgentCapability::SkillLoading | AgentCapability::TrustedCliExecution
                    ) && checked.as_ref().is_some_and(|(cli, recorded)| {
                        *recorded == digest
                            && super::super::executable::resolve(cli)
                                .ok()
                                .flatten()
                                .as_ref()
                                == Some(cli)
                    }) {
                        proof.state = CapabilityState::Proven;
                        proof.reason = None;
                        proof.adapter_contract = "hiroute.dsh-public-native-resources/v1".into();
                    }
                }
            }
        }
        FilesystemAgentDiscoveryV1 {
            outcome,
            configuration_issue: None,
            claude_configuration: None,
            discovered_credential: None,
            permission_hardening: None,
            managed_launch: None,
        }
    }
    #[cfg(any(unix, windows))]
    pub fn check_dsh_model_configuration(
        &self,
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        self.check_dsh_public_cli(true, &["llm-pi-ai"])
    }
    /// Revalidate the exact installation/configuration observed by this capture.
    /// This proof is scoped to the caller's capture and is never a persisted grant.
    pub fn dsh_model_configuration_is_current(
        &self,
        checked: &CanonicalDigest,
    ) -> Result<bool, AgentFilesystemScanError> {
        self.dsh_native_contract(true)
            .map(|current| current == *checked)
    }
    #[cfg(any(unix, windows))]
    pub fn check_dsh_collaboration(&self, cli: &Path) -> Result<(), AgentFilesystemScanError> {
        *self
            .dsh_collaboration_cli
            .lock()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)? = None;
        let before = self.check_dsh_public_cli(
            false,
            &[
                "skill-filesystem",
                "tool-skill",
                "tool-fs",
                if cfg!(windows) {
                    "tool-pwsh"
                } else {
                    "tool-bash"
                },
                "skill",
            ],
        )?;
        let cli = executable::resolve(cli)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        if before != self.dsh_native_contract(false)? {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        *self
            .dsh_collaboration_cli
            .lock()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)? = Some((cli, before));
        Ok(())
    }
    #[cfg(any(unix, windows))]
    fn check_dsh_public_cli(
        &self,
        models: bool,
        required: &[&str],
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let executable = self
            .dsh_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        #[cfg(windows)]
        let (mut command, dependency) = {
            let invocation = super::super::dsh_windows::invocation(&executable)
                .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
            let mut command = std::process::Command::new(invocation.program);
            command.args(invocation.entry);
            (command, invocation.dependency)
        };
        #[cfg(unix)]
        let (mut command, dependency) = (
            std::process::Command::new(&executable),
            dsh_execution_dependency(&executable)?,
        );
        let before = self.dsh_native_contract_at(models, &executable, dependency)?;
        command.args(["--profile", "acp", "--dump-default-config"]);
        check_composition(&dsh_public_probe(command, true)?, required)
            .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        if before != self.dsh_native_contract(models)? {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        Ok(before)
    }
}

#[cfg(windows)]
fn dsh_execution_dependency(
    executable: &Path,
) -> Result<DshRuntimeDependency, AgentFilesystemScanError> {
    super::super::dsh_windows::invocation(executable)
        .map(|invocation| invocation.dependency)
        .map_err(|_| AgentFilesystemScanError::SourceUnavailable)
}

#[cfg(not(windows))]
fn dsh_execution_dependency(
    executable: &Path,
) -> Result<DshRuntimeDependency, AgentFilesystemScanError> {
    let metadata =
        std::fs::metadata(executable).map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
    Ok((metadata.len(), metadata.modified().ok()))
}

fn dsh_executable(
    path: &Path,
    version_diagnostic: bool,
) -> Result<Option<(PathBuf, String)>, ProbeFailure> {
    #[cfg(windows)]
    if executable::searches_path(path) {
        return super::super::dsh_windows::find(path, version_diagnostic);
    }
    executable::resolve(path)?
        .map(|path| dsh_observation(path, version_diagnostic))
        .transpose()
}

pub(in crate::agents) fn dsh_observation(
    path: PathBuf,
    version_diagnostic: bool,
) -> Result<(PathBuf, String), ProbeFailure> {
    // Settings snapshots inspect native files without launching the engine.
    let version = if version_diagnostic {
        match executable::probe_resolved(&path, std::time::Duration::from_secs(2)) {
            ExecutableProbe::Installed(value) => value.version,
            ExecutableProbe::Unknown(
                ProbeFailure::LaunchFailed(_) | ProbeFailure::NotExecutable,
            ) => {
                return Err(ProbeFailure::NotExecutable);
            }
            ExecutableProbe::Unknown(reason) => return Err(reason),
            ExecutableProbe::NotFound => unreachable!("probe of a resolved path"),
        }
    } else {
        "not-probed".into()
    };
    Ok((path, version))
}

#[cfg(any(unix, windows))]
pub(super) enum DshCompositionError {
    InvalidOutput,
    MissingRequiredPlugin,
}

#[cfg(any(unix, windows))]
pub(super) fn check_composition(
    bytes: &[u8],
    required: &[&str],
) -> Result<(), DshCompositionError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DshCompositionError::InvalidOutput)?;
    if text.trim().is_empty() {
        return Err(DshCompositionError::InvalidOutput);
    }
    if required.iter().any(|id| {
        !text
            .lines()
            .any(|line| line.trim() == format!("- id: {id}"))
    }) {
        return Err(DshCompositionError::MissingRequiredPlugin);
    }
    Ok(())
}

/// Inspect shipped composition without loading native user plugins or credentials.
#[cfg(any(unix, windows))]
pub(super) fn dsh_public_probe(
    mut command: std::process::Command,
    _inherit_windows_environment: bool,
) -> Result<zeroize::Zeroizing<Vec<u8>>, AgentFilesystemScanError> {
    let root = tempfile::tempdir().map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
    command
        .current_dir(root.path())
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .env("DSH_HOME", root.path());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        let output = root.path().join("config");
        let file = std::fs::File::create(&output)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        let mut child = super::super::NativeProbeProcess::new(child);
        let started = std::time::Instant::now();
        loop {
            match child
                .observe()
                .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            {
                Some(true) => break,
                Some(false) => return Err(AgentFilesystemScanError::InvalidConfig),
                None if started.elapsed() > std::time::Duration::from_secs(15) => {
                    return Err(AgentFilesystemScanError::SourceUnavailable);
                }
                None => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
            if std::fs::metadata(&output).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
        }
        child
            .stop()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        super::super::native_probe_process::read_bounded(&output, 2 * 1024 * 1024)
            .map_err(|_| AgentFilesystemScanError::InvalidConfig)
    }
    #[cfg(windows)]
    {
        let result = super::super::windows_runtime::bounded(
            &command,
            _inherit_windows_environment,
            std::time::Duration::from_secs(15),
            2 * 1024 * 1024,
        );
        if result
            .as_ref()
            .is_err_and(super::super::windows_runtime::cleanup_unknown)
        {
            let _ = root.keep();
        }
        result.map_err(|_| AgentFilesystemScanError::SourceUnavailable)
    }
}
