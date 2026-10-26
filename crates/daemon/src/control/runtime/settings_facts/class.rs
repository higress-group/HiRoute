//! Native Agent identities and resource targets; Skill ownership remains independent of model profiles.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::control::runtime) enum SettingsAgentClass {
    Codex,
    CodexProfile,
    Claude,
    Qoder,
    Pi,
    Dsh,
    DshDesktop,
    Hermes,
    OpenCode,
}

impl SettingsAgentClass {
    pub(in crate::control::runtime) fn is_codex(self) -> bool {
        matches!(self, Self::Codex | Self::CodexProfile)
    }

    pub(in crate::control::runtime) fn agent_id(self) -> &'static str {
        match self {
            Self::Codex | Self::CodexProfile => "agent_codex_default",
            Self::Claude => "agent_claude_default",
            Self::Qoder => "agent_qoder_default",
            Self::Pi => "agent_pi_default",
            Self::Dsh => "agent_dsh_default",
            Self::DshDesktop => "agent_dsh_desktop",
            Self::OpenCode => "agent_opencode_default",
            Self::Hermes => "agent_hermes_default",
        }
    }

    pub(in crate::control::runtime) fn context_segment(self) -> &'static str {
        match self {
            Self::Codex | Self::CodexProfile => "codex",
            Self::Claude => "claude",
            Self::Qoder => "qoder",
            Self::Pi => "pi",
            Self::Dsh | Self::DshDesktop => "dsh",
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
        }
    }

    pub(in crate::control::runtime) fn native_path(
        self,
        scanner: &FilesystemAgentScannerV1,
    ) -> Option<std::path::PathBuf> {
        Some(match self {
            Self::Codex => scanner.codex_user_config_target(),
            Self::CodexProfile => scanner.codex_profile_config_target(),
            Self::Claude => scanner.claude_user_settings_target(),
            Self::Qoder => scanner.qoder_user_config_target(),
            Self::Pi => scanner.pi_user_models_target(),
            Self::Dsh => scanner.dsh_user_models_target(),
            Self::DshDesktop => scanner.dsh_desktop_models_target(),
            Self::OpenCode => scanner.opencode_user_models_target(),
            Self::Hermes => return None,
        })
    }

    pub(in crate::control::runtime) fn skill_root_ref(self) -> &'static str {
        match self {
            Self::Codex | Self::CodexProfile => "skill-root/agent_codex_default",
            Self::Claude => "skill-root/agent_claude_default",
            Self::Qoder => "skill-root/agent_qoder_default",
            Self::Pi => "skill-root/agent_pi_default",
            Self::Dsh => "skill-root/agent_dsh_default",
            Self::DshDesktop => "skill-root/agent_dsh_desktop",
            Self::OpenCode => "skill-root/agent_opencode_default",
            Self::Hermes => "skill-root/hermes-explicit-target-required",
        }
    }

    pub(in crate::control::runtime) fn skill_target(
        self,
    ) -> Result<String, hiroute_domain::OperationValidationError> {
        self.skill_target_for(AgentSettingsFacet::Collaboration)
    }

    pub(in crate::control::runtime) fn skill_target_for(
        self,
        facet: AgentSettingsFacet,
    ) -> Result<String, hiroute_domain::OperationValidationError> {
        if self == Self::Hermes {
            return Err(hiroute_domain::OperationValidationError::UnregisteredEffectPlan);
        }
        let (profile_id, integration_profile_ref) = match self {
            Self::Codex | Self::CodexProfile => (
                hiroute_integrations::CODEX_PROFILE_ID_V1,
                hiroute_integrations::CODEX_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Claude => (
                hiroute_integrations::CLAUDE_PROFILE_ID_V1,
                hiroute_integrations::CLAUDE_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Pi => (
                hiroute_integrations::PI_PROFILE_ID_V1,
                hiroute_integrations::PI_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Hermes => {
                return Err(hiroute_domain::OperationValidationError::UnregisteredEffectPlan);
            }
            Self::OpenCode => (
                hiroute_integrations::OPENCODE_PROFILE_ID_V1,
                hiroute_integrations::OPENCODE_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Dsh | Self::DshDesktop => (
                hiroute_integrations::DSH_PROFILE_ID_V1,
                hiroute_integrations::DSH_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Qoder => (
                hiroute_integrations::QODER_PROFILE_ID_V1,
                hiroute_integrations::QODER_INTEGRATION_PROFILE_REF_V1,
            ),
        };
        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            self.agent_id(),
            profile_id,
            integration_profile_ref,
        )?;
        let role = match facet {
            AgentSettingsFacet::Collaboration => AgentConnectionEffectRoleV1::RoutingSkill,
            AgentSettingsFacet::ManagementSkill => AgentConnectionEffectRoleV1::ManagementSkill,
            AgentSettingsFacet::Model => {
                return Err(hiroute_domain::OperationValidationError::UnregisteredEffectPlan);
            }
        };
        role.settings_target_for(&subject)
    }
}
