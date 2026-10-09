//! Pi's effective user targets and bounded settings facts. No native process or helper is run.
use super::*;
use hiroute_domain::{AdditionalAgentModelV1, AgentCapability, CapabilityState};
use serde::Serialize;

fn settings_value(bytes: &[u8]) -> Result<serde_json::Value, AgentFilesystemScanError> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?
    };
    if !value.is_object() {
        return Err(AgentFilesystemScanError::InvalidConfig);
    }
    Ok(value)
}

#[derive(Serialize)]
pub struct PiDefaultModel {
    pub content_digest: CanonicalDigest,
    provider: Option<String>,
    model: Option<String>,
}
impl PiDefaultModel {
    pub fn removes_default(&self, provider: &str, models: &[AdditionalAgentModelV1]) -> bool {
        self.provider.as_deref().is_some_and(|selected_provider| {
            let owned_namespace = selected_provider == provider
                || selected_provider
                    .strip_prefix(&format!("{provider}-"))
                    .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()));
            owned_namespace
                && self.model.as_ref().is_none_or(|selected| {
                    !models.iter().any(|m| {
                        &m.alias == selected
                            && hiroute_domain::additional_model_provider_for(provider, models, m)
                                == selected_provider
                    })
                })
        })
    }
}

impl FilesystemAgentScannerV1 {
    pub fn pi_user_models_target(&self) -> PathBuf {
        self.layout.pi_config_root.join("models.json")
    }
    pub fn pi_user_skill_target(&self) -> PathBuf {
        self.layout
            .pi_config_root
            .join("skills/hiroute-collaboration/SKILL.md")
    }
    pub fn pi_default_model(&self) -> Result<PiDefaultModel, AgentFilesystemScanError> {
        let observed = super::super::filesystem_config::read_validated_config_bytes(
            &self.layout.pi_config_root.join("settings.json"),
        )?;
        let bytes = observed
            .as_ref()
            .map_or(&[][..], |(bytes, _)| bytes.as_slice());
        let value = settings_value(bytes)?;
        let field = |key: &str| -> Result<Option<String>, AgentFilesystemScanError> {
            value
                .get(key)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or(AgentFilesystemScanError::InvalidConfig)
                })
                .transpose()
        };
        Ok(PiDefaultModel {
            content_digest: CanonicalDigest::of_bytes(bytes),
            provider: field("defaultProvider")?,
            model: field("defaultModel")?,
        })
    }
    pub fn pi_executable_target(&self) -> Option<PathBuf> {
        let selected = &self.layout.pi_executable;
        if selected.is_absolute() || selected.components().count() > 1 {
            return super::super::executable::resolve(selected).ok().flatten();
        }
        let mut paths = std::env::var_os("PATH")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| p.is_absolute())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        paths.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);
        #[cfg(target_os = "macos")]
        paths.push(PathBuf::from("/opt/homebrew/bin"));
        paths.into_iter().take(128).find_map(|p| {
            super::super::executable::resolve(&p.join(selected))
                .ok()
                .flatten()
        })
    }
    pub fn pi_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        let Some(executable) = self.pi_executable_target() else {
            return probe_report(
                "agent_pi_default",
                AgentKindV1::Pi,
                ExecutableProbe::NotFound,
            );
        };
        let installation = super::super::pi_cli_installation(&executable);
        if installation.is_err() || !self.layout.pi_config_root.is_absolute() {
            return report_only_agent(
                "agent_pi_default",
                AgentKindV1::Pi,
                "not-probed".into(),
                AgentReportOnlyReasonV1::ConfigUnavailable,
            );
        }
        let identity = installation.expect("checked CLI identity");
        let mut outcome = resolve_agent_observation(base_observation(
            "agent_pi_default",
            AgentKindV1::Pi,
            identity.version,
        ));
        let resource_settings = super::super::filesystem_config::read_validated_config_bytes(
            &self.layout.pi_config_root.join("settings.json"),
        )
        .and_then(|observed| {
            let bytes = observed.map(|(bytes, _)| bytes).unwrap_or_default();
            let settings = settings_value(&bytes)?;
            let rules = settings
                .get("skills")
                .map(|v| v.as_array().ok_or(AgentFilesystemScanError::InvalidConfig))
                .transpose()?;
            let excluded = rules.is_some_and(|rules| {
                rules.iter().any(|rule| {
                    let Some(rule) = rule.as_str() else {
                        return true;
                    };
                    let negative = rule.starts_with('!') || rule.starts_with('-');
                    // Exact exclusions of unrelated Skills remain usable. Complex exclusion
                    // patterns cannot be statically proven here and fail closed at Save.
                    let pattern = &rule[usize::from(negative)..];
                    negative
                        && (!pattern
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-/ ".contains(&b))
                            || pattern.contains("hiroute-collaboration")
                            || pattern.ends_with("SKILL.md")
                            || pattern.trim_end_matches('/').ends_with("skills"))
                })
            });
            Ok((CanonicalDigest::of_bytes(&bytes), !excluded))
        });
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            super::super::observed_capabilities::attach_file_capabilities(
                installation,
                &executable,
                &self.pi_user_skill_target(),
            );
            let resources_allowed = resource_settings
                .as_ref()
                .is_ok_and(|(_, allowed)| *allowed);
            if let Ok((settings_digest, _)) = &resource_settings {
                installation.observation_digest =
                    CanonicalDigest::of(&(&installation.observation_digest, settings_digest))
                        .expect("serializable native observation");
                for proof in &mut installation.capability_evidence {
                    proof.dependency_digest = installation.observation_digest.clone();
                }
            }
            // Only a successful local resource/tool interface check authorizes Skill loading.
            // Model configuration does not depend on the Worker or history SDK contracts.
            let resource_contract = self.pi_collaboration_cli.lock().ok().is_some_and(|v| {
                v.as_ref()
                    .is_some_and(|(_, digest)| *digest == identity.manifest_digest)
            });
            for proof in &mut installation.capability_evidence {
                if proof.capability == AgentCapability::SkillLoading
                    && resources_allowed
                    && resource_contract
                {
                    proof.state = CapabilityState::Proven;
                    proof.reason = None;
                    proof.adapter_contract = "hiroute.pi-native-resources/v1".into();
                }
                if proof.capability == AgentCapability::TrustedCliExecution
                    && self.pi_collaboration_cli.lock().ok().is_some_and(|v| {
                        v.as_ref().is_some_and(|(cli, digest)| {
                            *digest == identity.manifest_digest
                                && super::super::executable::resolve(cli)
                                    .ok()
                                    .flatten()
                                    .as_ref()
                                    == Some(cli)
                        })
                    })
                {
                    proof.state = CapabilityState::Proven;
                    proof.reason = None;
                    proof.adapter_contract = "hiroute.pi-local-cli/v1".into();
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
    pub fn check_pi_collaboration(&self, cli: &Path) -> Result<(), AgentFilesystemScanError> {
        *self
            .pi_collaboration_cli
            .lock()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)? = None;
        let executable = self
            .pi_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let identity = super::super::pi_cli_installation(&executable)?;
        self.check_pi_capability(&executable, super::super::PiSdkCapability::Collaboration)?;
        if super::super::pi_cli_installation(&executable)?.manifest_digest
            != identity.manifest_digest
        {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        let cli = super::super::executable::resolve(cli)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        *self
            .pi_collaboration_cli
            .lock()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)? =
            Some((cli, identity.manifest_digest));
        Ok(())
    }

    #[cfg(unix)]
    pub fn check_pi_model_configuration(&self) -> Result<(), AgentFilesystemScanError> {
        let executable = self
            .pi_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        self.check_pi_capability(&executable, super::super::PiSdkCapability::Models)
    }

    #[cfg(unix)]
    fn check_pi_capability(
        &self,
        executable: &Path,
        capability: super::super::PiSdkCapability,
    ) -> Result<(), AgentFilesystemScanError> {
        let node = super::super::executable::resolve(Path::new("node"))
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        super::super::check_pi_sdk_capability(executable, &node, capability)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)
    }
}
