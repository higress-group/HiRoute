use hiroute_application_api::{CanonicalDigest, PrincipalKind, RevisionSetV1, SetupRequestV1};
use hiroute_domain::{OperationId, OperationV1, WorkspaceId};

use super::*;

struct MemoryControl;

impl ControlStatePort for MemoryControl {
    fn snapshot(
        &self,
        _workspace_id: &WorkspaceId,
    ) -> Result<ControlStateSnapshotV1, ControlReadError> {
        Ok(ControlStateSnapshotV1 {
            revisions: RevisionSetV1 {
                target: 7,
                dependencies: Default::default(),
            },
            desired_state: None,
            recoverable_operations: Vec::new(),
        })
    }

    fn operation(
        &self,
        _operation_id: &OperationId,
    ) -> Result<Option<OperationV1>, ControlReadError> {
        Ok(None)
    }

    fn validate_protected_capability(
        &self,
        _raw_capability: &str,
        _workspace_id: &WorkspaceId,
        _principal: PrincipalKind,
        _operation_kind: &str,
        _accepted_digest: &CanonicalDigest,
        _expected_revisions: &RevisionSetV1,
    ) -> Result<(), ControlReadError> {
        Err(ControlReadError::Denied)
    }
}

struct MemoryDiscovery(Vec<DiscoveredAgentV1>);

impl AgentDiscoveryPort for MemoryDiscovery {
    fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        Ok(self.0.clone())
    }
}

fn agent() -> DiscoveredAgentV1 {
    DiscoveredAgentV1 {
        codex_access: None,
        context_id: None,
        agent_id: "agent_codex_default".to_owned(),
        profile_id: "codex-responses-v1".to_owned(),
        version: "0.116.0".to_owned(),
        supported: true,
        configuration_state: "observed".to_owned(),
        available_surfaces: Default::default(),
        native_model_catalog: None,
        registered_configuration: None,
        discovered_credential: None,
        permission_hardening: None,
    }
}

#[test]
fn control_status_reports_gateway_as_a_fact_without_failing_the_query() {
    let status = system_status(
        &MemoryControl,
        &MemoryDiscovery(vec![agent()]),
        &WorkspaceId::default(),
        true,
        false,
    )
    .unwrap();
    assert_eq!(status.setup_revision, 7);
    assert_eq!(status.gateway, "unavailable:not_composed");
    assert_eq!(status.discovery, "ready:1_supported");
}

#[test]
fn role_all_status_reports_the_composed_daemon_and_gateway() {
    let status = system_status(
        &MemoryControl,
        &MemoryDiscovery(vec![agent()]),
        &WorkspaceId::default(),
        true,
        true,
    )
    .unwrap();
    assert_eq!(status.daemon, "role_all");
    assert_eq!(status.gateway, "ready");
}

#[test]
fn control_setup_preview_is_deterministic_and_reports_the_unbound_gateway() {
    let first = crate::setup::preview(
        &MemoryControl,
        &MemoryDiscovery(vec![agent()]),
        &WorkspaceId::default(),
        SetupRequestV1::default(),
    )
    .unwrap();
    let second = crate::setup::preview(
        &MemoryControl,
        &MemoryDiscovery(vec![agent()]),
        &WorkspaceId::default(),
        SetupRequestV1::default(),
    )
    .unwrap();
    assert_eq!(first, second);
    assert!(!first.applicable);
    assert_eq!(first.blockers, ["GATEWAY_RUNTIME_UNBOUND"]);
}

#[test]
fn control_capability_validation_never_grants_ambient_skill_authority() {
    assert_eq!(
        MemoryControl.validate_protected_capability(
            "ambient",
            &WorkspaceId::default(),
            PrincipalKind::Skill,
            "ApplySetup",
            &CanonicalDigest::of_bytes(b"change"),
            &RevisionSetV1 {
                target: 7,
                dependencies: Default::default(),
            },
        ),
        Err(ControlReadError::Denied)
    );
}
