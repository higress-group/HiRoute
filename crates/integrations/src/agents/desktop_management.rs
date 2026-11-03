//! Desktop-only installations can prepare their shared user management resource.
//! File preparation is not proof of live Skill loading or trusted CLI execution.
use super::*;
use hiroute_domain::{AgentCapability, CapabilityState, SupportedAgentInstallationV1};

impl FilesystemAgentScannerV1 {
    #[cfg(any(unix, windows))]
    pub fn desktop_management_installation(
        &self,
        agent_id: &str,
        target: &Path,
        configure: bool,
    ) -> Option<SupportedAgentInstallationV1> {
        let (kind, executable, proof) = match agent_id {
            "agent_codex_default" => {
                let engine = self
                    .codex_desktop_engine_path()
                    .and_then(|path| super::super::executable::resolve(&path).ok().flatten());
                if configure && engine.is_none() {
                    return None;
                }
                (AgentKindV1::Codex, engine, None)
            }
            "agent_dsh_default" => {
                let engine = self.dsh_desktop_executable_target();
                if configure && engine.is_none() {
                    return None;
                }
                let proof = if configure {
                    Some(self.check_dsh_desktop_management().ok()?)
                } else {
                    None
                };
                (AgentKindV1::DeepseekHarness, engine, proof)
            }
            _ => return None,
        };
        let AgentDiscoveryOutcomeV1::Supported { mut installation } =
            resolve_agent_observation(base_observation(agent_id, kind, "not-probed".into()))
        else {
            return None;
        };
        super::super::observed_capabilities::attach_optional_file_capabilities(
            &mut installation,
            executable.as_deref(),
            target,
        );
        let digest = CanonicalDigest::of(&(&installation.observation_digest, proof)).ok()?;
        installation.observation_digest = digest.clone();
        for evidence in &mut installation.capability_evidence {
            evidence.dependency_digest = digest.clone();
            if configure && evidence.capability == AgentCapability::SkillLoading {
                // Codex natively reads real-HOME .agents/skills; the checked DSH Desktop
                // composition retains DSH_HOME/skills. Neither uses the isolated model home.
                evidence.state = CapabilityState::Proven;
                evidence.reason = None;
                evidence.adapter_contract = "hiroute.desktop-shared-management/v1".into();
            }
        }
        Some(*installation)
    }
}
