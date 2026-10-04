//! Qoder discovery locates an executable and one user Skill target, never model/auth settings.
use super::super::{QoderNativeContext, QoderNativeError, qoder::qoder_error};
use super::*;

impl FilesystemAgentScannerV1 {
    pub fn qoder_native_context(&self) -> Result<QoderNativeContext, QoderNativeError> {
        QoderNativeContext::from_selected(
            &self.layout.qoder_home,
            Some(&self.layout.qoder_config_root),
        )
    }

    /// Identity only. Discovery does not read settings; explicit configuration effects own
    /// one additional provider in this file, independently of collaboration discovery.
    pub fn qoder_user_config_target(&self) -> PathBuf {
        self.layout.qoder_config_root.join("settings.json")
    }

    pub fn qoder_user_skill_target(&self) -> PathBuf {
        self.layout
            .qoder_config_root
            .join("skills/hiroute-collaboration/SKILL.md")
    }

    pub fn qoder_executable_target(&self) -> Option<PathBuf> {
        self.located_qoder().ok().flatten()
    }

    fn located_qoder(&self) -> Result<Option<PathBuf>, super::super::executable::ProbeFailure> {
        let selected = &self.layout.qoder_executable;
        if selected.is_absolute() || selected.components().count() > 1 {
            // A selected installation must fail visibly rather than switch to a PATH candidate.
            return super::super::executable::resolve(selected);
        }
        let mut directories = std::env::var_os("PATH")
            .map(|value| {
                std::env::split_paths(&value)
                    .filter(|p| p.is_absolute())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        directories.extend([
            self.layout.qoder_home.join(".local/bin"),
            self.layout.qoder_home.join(".qoder/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
        #[cfg(target_os = "macos")]
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        let mut first_failure = None;
        for directory in directories.into_iter().take(128) {
            for name in [selected.as_os_str(), std::ffi::OsStr::new("qodercli")] {
                match super::super::executable::resolve(&directory.join(name)) {
                    Ok(Some(path)) => return Ok(Some(path)),
                    Ok(None) => {}
                    Err(error) => {
                        first_failure.get_or_insert(error);
                    }
                }
            }
        }
        // Standalone installations may expose only a versioned native binary. One bounded
        // directory listing is sufficient; never recurse through HOME or inspect binary bytes.
        let directory = self.layout.qoder_home.join(".qoder/bin/qodercli");
        if let Ok(entries) = std::fs::read_dir(directory) {
            let mut candidates = entries
                .take(64)
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("qodercli-"))
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            candidates.sort();
            for path in candidates.into_iter().rev() {
                match super::super::executable::resolve(&path) {
                    Ok(Some(path)) => return Ok(Some(path)),
                    Ok(None) => {}
                    Err(error) => {
                        first_failure.get_or_insert(error);
                    }
                }
            }
        }
        first_failure.map_or(Ok(None), Err)
    }

    pub fn qoder_settings_discovery(
        &self,
        include_collaboration_evidence: bool,
    ) -> FilesystemAgentDiscoveryV1 {
        let executable = match self.located_qoder() {
            Ok(Some(executable)) => executable,
            Ok(None) => {
                return probe_report(
                    "agent_qoder_default",
                    AgentKindV1::Qoder,
                    ExecutableProbe::NotFound,
                );
            }
            Err(reason) => {
                return probe_report(
                    "agent_qoder_default",
                    AgentKindV1::Qoder,
                    ExecutableProbe::Unknown(reason),
                );
            }
        };
        let Ok(context) = self.qoder_native_context() else {
            return report_only_agent(
                "agent_qoder_default",
                AgentKindV1::Qoder,
                "not-probed".into(),
                AgentReportOnlyReasonV1::ConfigUnavailable,
            );
        };
        let mut outcome = resolve_agent_observation(base_observation(
            "agent_qoder_default",
            AgentKindV1::Qoder,
            "not-probed".into(),
        ));
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            super::super::observed_capabilities::attach_file_capabilities(
                installation,
                &executable,
                &context.skill_target(),
            );
            installation.capability_evidence.retain(|proof| {
                !matches!(
                    proof.capability,
                    hiroute_domain::AgentCapability::EffectiveConfiguration
                        | hiroute_domain::AgentCapability::IngressAuthentication
                        | hiroute_domain::AgentCapability::ModelCatalog
                )
            });
            // This check borrows normal native login and Skills. Its controlled local request
            // is not evidence that the entire HOME or native client environment was isolated.
            for proof in &mut installation.capability_evidence {
                if proof.capability == hiroute_domain::AgentCapability::IsolatedVerification {
                    proof.reason = Some(hiroute_domain::CapabilityReason::IsolationUnproven);
                }
            }
            #[cfg(unix)]
            if include_collaboration_evidence
                && let Ok(cache) = self.qoder_collaboration.lock()
                && let Some(evidence) = cache.evidence.as_ref()
            {
                evidence.attach(&executable, &context, installation);
            }
            #[cfg(not(unix))]
            let _ = include_collaboration_evidence;
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
    pub fn qoder_installed_collaboration_verified(&self) -> bool {
        let (Some(executable), Ok(context)) =
            (self.qoder_executable_target(), self.qoder_native_context())
        else {
            return false;
        };
        self.qoder_collaboration
            .lock()
            .ok()
            .and_then(|cache| {
                cache
                    .evidence
                    .as_ref()
                    .map(|evidence| evidence.installed_user_verified(&executable, &context))
            })
            .unwrap_or(false)
    }

    #[cfg(unix)]
    pub fn check_qoder_collaboration(
        &self,
        cli: &Path,
        target: super::super::QoderCollaborationProbeTarget,
    ) -> Result<(), QoderNativeError> {
        let generation = self
            .qoder_collaboration
            .lock()
            .map_err(|_| qoder_error("evidence cache"))?
            .begin()?;
        let executable = self
            .qoder_executable_target()
            .ok_or_else(|| qoder_error("selected executable"))?;
        let evidence = super::super::QoderCollaborationProbe::run(
            &executable,
            &self.qoder_native_context()?,
            cli,
            target,
        )?;
        self.qoder_collaboration
            .lock()
            .map_err(|_| qoder_error("evidence cache"))?
            .finish(generation, evidence)
    }
}
