//! Independent OpenCode metadata and settings/Skill targets; no native process is started.
use super::super::{QoderNativeError, qoder_error};
use super::*;

impl FilesystemAgentScannerV1 {
    pub fn opencode_native_context(
        &self,
    ) -> Result<&super::super::OpenCodeNativeContext, AgentFilesystemScanError> {
        self.layout
            .opencode_context
            .as_ref()
            .ok_or(AgentFilesystemScanError::InvalidConfig)
    }
    pub fn opencode_user_models_target(&self) -> PathBuf {
        self.opencode_model_target.clone()
    }
    /// Recovery may retain the registered JSON/JSONC sibling, never another root or explicit file.
    pub fn opencode_model_target_candidates(&self) -> Vec<PathBuf> {
        let Ok(native) = self.opencode_native_context() else {
            return Vec::new();
        };
        let selected = self.opencode_user_models_target();
        let mut candidates = vec![selected.clone()];
        if native.config_file.is_none() {
            candidates.push(native.resource_root().join(
                if selected.extension().is_some_and(|x| x == "json") {
                    "opencode.jsonc"
                } else {
                    "opencode.json"
                },
            ));
        }
        candidates
    }
    pub fn bind_opencode_model_target(
        &mut self,
        path: PathBuf,
    ) -> Result<(), AgentFilesystemScanError> {
        if !self.opencode_model_target_candidates().contains(&path) {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        self.opencode_model_target = path;
        Ok(())
    }
    pub fn opencode_user_skill_target(&self) -> PathBuf {
        self.layout
            .opencode_context
            .as_ref()
            .map(|c| c.skill_target())
            .unwrap_or_default()
    }
    pub fn opencode_executable_target(&self) -> Option<PathBuf> {
        let selected = &self.layout.opencode_executable;
        match super::super::opencode_native_executable(selected) {
            Ok(None) if selected == std::path::Path::new("opencode") => {
                // Finder-launched clients may not inherit the installer's shell PATH.
                // An explicit saved installation must never fall back to another binary.
                let home = &self.opencode_native_context().ok()?.home;
                super::super::opencode_native_executable(&home.join(".opencode/bin/opencode"))
                    .ok()
                    .flatten()
            }
            result => result.ok().flatten(),
        }
    }
    pub fn opencode_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        let executable = self.opencode_executable_target();
        let Ok(context) = self.opencode_native_context() else {
            return report_only_agent(
                "agent_opencode_default",
                AgentKindV1::OpenCode,
                "not-probed".into(),
                AgentReportOnlyReasonV1::ConfigUnavailable,
            );
        };
        if executable.is_none()
            && !context.resource_root().is_dir()
            && !context.model_target().is_file()
        {
            return probe_report(
                "agent_opencode_default",
                AgentKindV1::OpenCode,
                ExecutableProbe::NotFound,
            );
        }
        let mut outcome = resolve_agent_observation(base_observation(
            "agent_opencode_default",
            AgentKindV1::OpenCode,
            "not-probed".into(),
        ));
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            super::super::observed_capabilities::attach_optional_file_capabilities(
                installation,
                executable.as_deref(),
                &context.skill_target(),
            );
            #[cfg(any(unix, windows))]
            if let Some(executable) = &executable
                && let Ok(cache) = self.opencode_collaboration.lock()
                && let Some(evidence) = &cache.evidence
            {
                evidence.attach(executable, context, installation);
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
}

impl FilesystemAgentScannerV1 {
    /// Non-target user layers are independent dependencies of the existing owned-file effect.
    pub fn opencode_model_facts(
        &self,
        namespace: &str,
        models: &[hiroute_domain::AdditionalAgentModelV1],
    ) -> Result<(CanonicalDigest, bool), AgentFilesystemScanError> {
        let native = self.opencode_native_context()?;
        if native.has_remote_configuration()? {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        let config = native.read_config(None)?;
        let resources = super::super::opencode_resources::model_references(native);
        if super::super::opencode::dynamic_keys(&config.value) {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        let target = self.opencode_user_models_target();
        let owned = |id: &str| {
            id == namespace
                || id
                    .strip_prefix(&format!("{namespace}-"))
                    .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        };
        let retain = models
            .iter()
            .map(|m| {
                format!(
                    "{}/{}",
                    hiroute_domain::additional_model_plan_provider_id(namespace, &m.alias),
                    m.alias
                )
            })
            .collect::<BTreeSet<_>>();
        for (path, _) in &config.documents {
            if *path == target {
                continue;
            }
            if let Some((bytes, _)) =
                super::super::filesystem_config::read_system_config_bytes(path)?
            {
                let document = super::super::pi_sources::Document(
                    super::super::opencode::parse_jsonc(&bytes)
                        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?,
                );
                if document.0["provider"]
                    .as_object()
                    .is_some_and(|providers| providers.keys().any(|id| owned(id)))
                {
                    return Err(AgentFilesystemScanError::InvalidConfig);
                }
            }
        }
        let mut default_in_use = false;
        if let Some(providers) = config.value["provider"].as_object() {
            for id in providers.keys().filter(|id| owned(id)) {
                if let Err(error) = super::super::opencode::validate_references(
                    &config.value,
                    id,
                    &config.value["provider"][id],
                    &retain,
                    resources.as_ref().ok().map(|r| r.models.as_slice()),
                ) {
                    if error.stage != "default in use" {
                        return Err(AgentFilesystemScanError::InvalidConfig);
                    }
                    default_in_use = true;
                }
            }
        }
        for model in models {
            let id = hiroute_domain::additional_model_plan_provider_id(namespace, &model.alias);
            if config.value["enabled_providers"]
                .as_array()
                .is_some_and(|v| !v.iter().any(|x| x.as_str() == Some(&id)))
                || config.value["disabled_providers"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|x| x.as_str() == Some(&id)))
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
        }
        let mut documents = config
            .documents
            .iter()
            .filter(|(path, _)| *path != target)
            .collect::<Vec<_>>();
        let digest = if let Ok(resources) = &resources {
            documents.extend(resources.documents.iter());
            CanonicalDigest::of(&documents)
        } else {
            CanonicalDigest::of(&(documents, "unavailable native model references"))
        }
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        Ok((digest, default_in_use))
    }
    pub fn opencode_model_dependencies(
        &self,
        namespace: &str,
        models: &[hiroute_domain::AdditionalAgentModelV1],
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let (digest, in_use) = self.opencode_model_facts(namespace, models)?;
        if in_use {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        Ok(digest)
    }
}

#[cfg(any(unix, windows))]
impl FilesystemAgentScannerV1 {
    pub fn check_opencode_collaboration(
        &self,
        cli: &Path,
        target: super::super::QoderCollaborationProbeTarget,
    ) -> Result<(), QoderNativeError> {
        let generation = self
            .opencode_collaboration
            .lock()
            .map_err(|_| qoder_error("evidence cache"))?
            .begin()?;
        let executable = self
            .opencode_executable_target()
            .ok_or_else(|| qoder_error("selected executable"))?;
        let evidence = super::super::OpenCodeCollaborationProbe::run(
            &executable,
            self.opencode_native_context()
                .map_err(|_| qoder_error("native context"))?,
            cli,
            target,
        )?;
        self.opencode_collaboration
            .lock()
            .map_err(|_| qoder_error("evidence cache"))?
            .finish(generation, evidence)
    }
}
