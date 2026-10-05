use hiroute_application_api::{AgentCheckRequestV1, AgentCheckScopeV1};
use serde_json::{Value, json};

use crate::control::{AgentDiscoveryPort, ControlReadError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CheckDisposition {
    Configuration(Value),
    ModelConsentRequired,
    LiveProbeRequested,
    NativeAuthenticationRequested,
    CollaborationRequested,
}

pub(crate) fn check(
    discovery: &dyn AgentDiscoveryPort,
    request: &AgentCheckRequestV1,
) -> Result<CheckDisposition, ControlReadError> {
    if request.agent_id.is_empty() || !request.valid_target() {
        return Err(ControlReadError::Corrupt);
    }
    let agent = discovery
        .discover()?
        .into_iter()
        .find(|agent| agent.agent_id == request.agent_id)
        .ok_or(ControlReadError::NotFound)?;
    if request.scope == AgentCheckScopeV1::Configuration {
        return Ok(CheckDisposition::Configuration(json!({
            "schema": "hiroute.agent-check/v1",
            "scope": "configuration",
            "suite": request.suite,
            "agent": agent,
            "model_call": false,
            "gateway_listener": "unavailable:not_composed",
        })));
    }
    if matches!(
        request.scope,
        AgentCheckScopeV1::NativeAuthentication | AgentCheckScopeV1::Collaboration
    ) {
        if request.allow_model_call
            || request.suite != hiroute_application_api::AgentCheckSuiteV1::Quick
        {
            return Err(ControlReadError::Corrupt);
        }
        return Ok(if request.scope == AgentCheckScopeV1::Collaboration {
            CheckDisposition::CollaborationRequested
        } else {
            CheckDisposition::NativeAuthenticationRequested
        });
    }
    if request.scope == AgentCheckScopeV1::Live {
        if request.suite != hiroute_application_api::AgentCheckSuiteV1::Quick {
            return Err(ControlReadError::Corrupt);
        }
        let target = request.target.as_ref().ok_or(ControlReadError::Corrupt)?;
        if agent.context_id.as_deref() != Some(target.context_id.as_str())
            || match request.agent_id.as_str() {
                "agent_codex_default" => !matches!(
                    target.surface,
                    hiroute_application_api::AgentModelSurfaceV2::CodexCli
                        | hiroute_application_api::AgentModelSurfaceV2::CodexDesktop
                ),
                "agent_claude_default" => {
                    target.surface != hiroute_application_api::AgentModelSurfaceV2::ClaudeCli
                }
                "agent_qoder_default" => !matches!(
                    target.surface,
                    hiroute_application_api::AgentModelSurfaceV2::QoderCli
                        | hiroute_application_api::AgentModelSurfaceV2::PiCli
                ),
                "agent_pi_default" => true, // No public per-surface model probe for this integration.
                _ => true,
            }
        {
            return Err(ControlReadError::Denied);
        }
    }
    if !request.allow_model_call {
        Ok(CheckDisposition::ModelConsentRequired)
    } else {
        Ok(CheckDisposition::LiveProbeRequested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{AgentDiscoveryPort, DiscoveredAgentV1};
    use hiroute_application_api::{
        AgentCheckSuiteV1, AgentModelCheckTargetV2, AgentModelSurfaceV2,
    };

    struct Discovery;

    impl AgentDiscoveryPort for Discovery {
        fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
            Ok(vec![DiscoveredAgentV1 {
                codex_access: None,
                context_id: Some("agent-context/codex/default".into()),
                agent_id: "agent_codex_default".into(),
                profile_id: "codex-responses-v1".into(),
                version: String::new(),
                supported: true,
                configuration_state: "configured".into(),
                available_surfaces: [AgentModelSurfaceV2::CodexDesktop].into(),
                native_model_catalog: None,
                registered_configuration: None,
                discovered_credential: None,
                permission_hardening: None,
            }])
        }
    }

    fn live_request() -> AgentCheckRequestV1 {
        AgentCheckRequestV1 {
            agent_id: "agent_codex_default".into(),
            scope: AgentCheckScopeV1::Live,
            suite: AgentCheckSuiteV1::Quick,
            allow_model_call: true,
            target: Some(AgentModelCheckTargetV2 {
                context_id: "agent-context/codex/default".into(),
                surface: AgentModelSurfaceV2::CodexDesktop,
                expected_applied_revision: hiroute_domain::GatewayPublicationRevision::new(9)
                    .unwrap(),
                client_model_ids: vec!["hiroute.live".into()],
            }),
        }
    }

    #[test]
    fn live_probe_is_bound_to_discovered_context_surface_and_quick_suite() {
        assert_eq!(
            check(&Discovery, &live_request()),
            Ok(CheckDisposition::LiveProbeRequested)
        );

        let mut wrong_context = live_request();
        wrong_context.target.as_mut().unwrap().context_id = "agent-context/other".into();
        assert_eq!(
            check(&Discovery, &wrong_context),
            Err(ControlReadError::Denied)
        );

        let mut wrong_surface = live_request();
        wrong_surface.target.as_mut().unwrap().surface = AgentModelSurfaceV2::ClaudeCli;
        assert_eq!(
            check(&Discovery, &wrong_surface),
            Err(ControlReadError::Denied)
        );

        let mut wrong_suite = live_request();
        wrong_suite.suite = AgentCheckSuiteV1::Tool;
        assert_eq!(
            check(&Discovery, &wrong_suite),
            Err(ControlReadError::Corrupt)
        );
    }
}
