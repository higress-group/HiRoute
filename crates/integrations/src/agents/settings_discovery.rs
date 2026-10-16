//! Targeted native facts for settings writes; unrelated Agent diagnostics are not a save gate.
use super::*;

impl FilesystemAgentScannerV1 {
    /// Skill-only saves do not parse the unrelated model document. Capability evidence is
    /// still supplied by the same native resource checks used by collaboration.
    pub fn skill_settings_installation(
        &self,
        agent_id: &str,
    ) -> Option<hiroute_domain::SupportedAgentInstallationV1> {
        let (kind, selected, config) = match agent_id {
            "agent_codex_default" => (
                AgentKindV1::Codex,
                &self.layout.codex_executable,
                &self.layout.codex_user_config,
            ),
            "agent_claude_default" => (
                AgentKindV1::ClaudeCode,
                &self.layout.claude_executable,
                &self.layout.claude_user_settings,
            ),
            _ => return None,
        };
        let executable = super::super::executable::resolve(selected).ok().flatten()?;
        let AgentDiscoveryOutcomeV1::Supported { mut installation } =
            resolve_agent_observation(base_observation(agent_id, kind, "not-probed".into()))
        else {
            return None;
        };
        super::super::observed_capabilities::attach_file_capabilities(
            &mut installation,
            &executable,
            config,
        );
        #[cfg(unix)]
        if kind == AgentKindV1::Codex {
            if let Ok(cache) = self.codex_ingress.lock()
                && let Some(evidence) = cache.as_ref()
            {
                evidence.attach_collaboration(&mut installation);
            }
        } else if let Ok(cache) = self.claude_ingress.lock()
            && let Some(evidence) = cache.as_ref()
        {
            evidence.attach_collaboration(&executable, &mut installation);
        }
        Some(*installation)
    }

    pub fn codex_profile_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        // The profile owns its file capabilities; inherited root is independently sealed for
        // configure, and must not make a safe revoke depend on the current root's validity.
        let mut profile = self.clone();
        profile.layout.codex_user_config = self.codex_profile_config_target();
        profile.codex_settings_discovery(false)
    }

    pub fn codex_settings_discovery(
        &self,
        include_collaboration_evidence: bool,
    ) -> FilesystemAgentDiscoveryV1 {
        let mut discovery = self.scan_codex();
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut discovery.outcome {
            super::super::observed_capabilities::attach_target_file_capabilities(
                installation,
                &self.layout.codex_user_config,
            );
            // Only a collaboration install consumes its explicitly checked Skill/CLI proof.
            // Ordinary model saves keep stable dependencies when the short-lived probe expires.
            #[cfg(any(unix, windows))]
            if include_collaboration_evidence
                && let Ok(cache) = self.codex_ingress.lock()
                && let Some(evidence) = cache.as_ref()
            {
                evidence.attach_collaboration(installation);
            }
            #[cfg(not(any(unix, windows)))]
            let _ = include_collaboration_evidence;
        }
        discovery
    }

    /// Settings writes need the current configuration and a launchable path, not a fresh
    /// `claude --version` subprocess. The ordinary scan still reports diagnostic probe results;
    /// this path retains file, precedence, path and authentication checks.
    pub fn claude_settings_discovery(
        &self,
        include_collaboration_evidence: bool,
    ) -> FilesystemAgentDiscoveryV1 {
        match super::super::executable::resolve(&self.layout.claude_executable) {
            Ok(Some(path)) => {
                let Some(canonical_path) = path.to_str().map(str::to_owned) else {
                    return probe_report(
                        "agent_claude_default",
                        AgentKindV1::ClaudeCode,
                        ExecutableProbe::Unknown(
                            super::super::executable::ProbeFailure::Unavailable,
                        ),
                    );
                };
                let mut discovery = self.scan_claude(
                    Some(ExecutableObservationV1 {
                        version: "not-probed".into(),
                        canonical_path,
                    }),
                    None,
                    true,
                );
                if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut discovery.outcome
                {
                    super::super::observed_capabilities::attach_target_file_capabilities(
                        installation,
                        &self.layout.claude_user_settings,
                    );
                    #[cfg(any(unix, windows))]
                    if let Ok(cache) = self.claude_ingress.lock()
                        && let Some(evidence) = cache.as_ref()
                    {
                        if include_collaboration_evidence {
                            evidence.attach_collaboration(&path, installation);
                        } else {
                            evidence.attach_authentication(&path, installation);
                        }
                    }
                    #[cfg(not(any(unix, windows)))]
                    let _ = include_collaboration_evidence;
                }
                discovery
            }
            Ok(None) => probe_report(
                "agent_claude_default",
                AgentKindV1::ClaudeCode,
                ExecutableProbe::NotFound,
            ),
            Err(reason) => probe_report(
                "agent_claude_default",
                AgentKindV1::ClaudeCode,
                ExecutableProbe::Unknown(reason),
            ),
        }
    }
}
