//! DSH standard Web model target and native Skill resources; no internal SDK or history reader.
use super::*;
use hiroute_domain::{AgentCapability, CapabilityState};

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
        super::super::executable::resolve(&self.layout.dsh_executable)
            .ok()
            .flatten()
    }
    fn dsh_native_contract(
        &self,
        models: bool,
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
        let executable = self
            .dsh_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let metadata = std::fs::metadata(&executable)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        CanonicalDigest::of(&(
            executable,
            metadata.len(),
            metadata.modified().ok(),
            dependencies,
        ))
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)
    }
    pub fn dsh_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        let Some(executable) = self.dsh_executable_target() else {
            return probe_report(
                "agent_dsh_default",
                AgentKindV1::DeepseekHarness,
                ExecutableProbe::NotFound,
            );
        };
        let version = match executable_probe(&executable) {
            ExecutableProbe::Installed(value) => value.version,
            _ => String::new(),
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
            if let Ok(digest) = self.dsh_native_contract(false) {
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
    #[cfg(unix)]
    pub fn check_dsh_model_configuration(&self) -> Result<(), AgentFilesystemScanError> {
        self.dsh_native_contract(true)?;
        self.check_dsh_public_cli(&["llm-pi-ai"])
    }
    #[cfg(unix)]
    pub fn check_dsh_collaboration(&self, cli: &Path) -> Result<(), AgentFilesystemScanError> {
        *self
            .dsh_collaboration_cli
            .lock()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)? = None;
        let before = self.dsh_native_contract(false)?;
        self.check_dsh_public_cli(&[
            "skill-filesystem",
            "tool-skill",
            "tool-fs",
            "tool-bash",
            "skill",
        ])?;
        let cli = super::super::executable::resolve(cli)
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
    #[cfg(unix)]
    fn check_dsh_public_cli(&self, required: &[&str]) -> Result<(), AgentFilesystemScanError> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let root = tempfile::tempdir().map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        let output = root.path().join("config");
        let file = std::fs::File::create(&output)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        // Dumping the shipped composition is public and does not activate plugins or model calls.
        // Its profile initialization is contained here; the real HOME and settings stay intact.
        let child = Command::new(
            self.dsh_executable_target()
                .ok_or(AgentFilesystemScanError::SourceUnavailable)?,
        )
        .args(["--profile", "acp", "--dump-default-config"])
        .env("DSH_HOME", root.path())
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
        let bytes = super::super::native_probe_process::read_bounded(&output, 2 * 1024 * 1024)
            .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        if required.iter().any(|id| {
            !text
                .lines()
                .any(|line| line.trim() == format!("- id: {id}"))
        }) {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        Ok(())
    }
}
