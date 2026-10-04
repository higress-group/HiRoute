use hiroute_domain::{
    AgentModelSurfaceV2, AgentSurfaceCheckRecordV1, AgentSurfaceCheckStateV1, CanonicalDigest,
    ControlRepositoryPort, GatewayPublicationRevision, PublicationRecordV1,
    PublicationRepositoryPort, WorkspaceId,
};

use super::ControlStore;

fn record(
    revision: GatewayPublicationRevision,
    surface: AgentModelSurfaceV2,
    state: AgentSurfaceCheckStateV1,
) -> AgentSurfaceCheckRecordV1 {
    AgentSurfaceCheckRecordV1 {
        schema: hiroute_domain::AGENT_SURFACE_CHECK_SCHEMA.into(),
        context_id: "agent-context/surface-check".into(),
        surface,
        applied_revision: revision,
        state,
        checked_model_ids: vec!["hiroute/model.v1".into()],
        capability_scope_digest: CanonicalDigest::of_bytes(b"capability-scope"),
        check_request_digest: CanonicalDigest::of_bytes(b"check-request"),
        reason_code: None,
    }
}

fn activate(store: &ControlStore, workspace: &WorkspaceId, revision: u64) {
    let revision = GatewayPublicationRevision::new(revision).unwrap();
    let publication = hiroute_domain::GatewayPublicationV1::new(
        workspace.clone(),
        revision,
        hiroute_domain::AliasRegistryV1::default(),
        Vec::new(),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&publication).unwrap();
    let record = PublicationRecordV1::from_parts(
        workspace.clone(),
        revision,
        CanonicalDigest::of_bytes(&bytes),
        bytes,
    )
    .unwrap();
    store.prepare_publication(&record, None).unwrap();
    store
        .mark_publication_active(workspace, revision, &record.digest)
        .unwrap();
}

fn storage() -> (tempfile::TempDir, ControlStore) {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    (directory, control)
}

#[test]
fn all_supported_native_surfaces_persist_current_verification_independently() {
    let (_directory, store) = storage();
    let workspace = WorkspaceId::default();
    activate(&store, &workspace, 2);
    let revision = GatewayPublicationRevision::new(2).unwrap();
    let surfaces = [
        AgentModelSurfaceV2::CodexCli,
        AgentModelSurfaceV2::CodexDesktop,
        AgentModelSurfaceV2::ClaudeCli,
        AgentModelSurfaceV2::QoderCli,
    ];
    for surface in surfaces {
        assert!(
            store
                .save_agent_surface_check(
                    &workspace,
                    &record(revision, surface, AgentSurfaceCheckStateV1::Passed)
                )
                .unwrap()
        );
    }
    assert_eq!(
        store
            .agent_surface_checks(&workspace, "agent-context/surface-check")
            .unwrap()
            .len(),
        surfaces.len()
    );
    let failed = record(
        revision,
        AgentModelSurfaceV2::QoderCli,
        AgentSurfaceCheckStateV1::Failed,
    );
    assert!(store.save_agent_surface_check(&workspace, &failed).unwrap());
    let checks = store
        .agent_surface_checks(&workspace, "agent-context/surface-check")
        .unwrap();
    assert_eq!(checks.len(), surfaces.len());
    assert!(checks.iter().all(|check| check.state
        == if check.surface == AgentModelSurfaceV2::QoderCli {
            AgentSurfaceCheckStateV1::Failed
        } else {
            AgentSurfaceCheckStateV1::Passed
        }));
}

#[test]
fn current_revision_results_are_kept_and_overwritten_per_surface() {
    let (_directory, store) = storage();
    let workspace = WorkspaceId::default();
    activate(&store, &workspace, 2);
    let revision = GatewayPublicationRevision::new(2).unwrap();

    assert!(
        store
            .save_agent_surface_check(
                &workspace,
                &record(
                    revision,
                    AgentModelSurfaceV2::CodexCli,
                    AgentSurfaceCheckStateV1::Passed
                )
            )
            .unwrap()
    );
    assert!(
        store
            .save_agent_surface_check(
                &workspace,
                &record(
                    revision,
                    AgentModelSurfaceV2::CodexDesktop,
                    AgentSurfaceCheckStateV1::Failed
                )
            )
            .unwrap()
    );
    let checks = store
        .agent_surface_checks(&workspace, "agent-context/surface-check")
        .unwrap();
    assert_eq!(checks.len(), 2);
    assert!(
        checks
            .iter()
            .any(|check| check.surface == AgentModelSurfaceV2::CodexCli
                && check.state == AgentSurfaceCheckStateV1::Passed)
    );
    assert!(
        checks
            .iter()
            .any(|check| check.surface == AgentModelSurfaceV2::CodexDesktop
                && check.state == AgentSurfaceCheckStateV1::Failed)
    );

    assert!(
        store
            .save_agent_surface_check(
                &workspace,
                &record(
                    revision,
                    AgentModelSurfaceV2::CodexDesktop,
                    AgentSurfaceCheckStateV1::Passed
                )
            )
            .unwrap()
    );
    let checks = store
        .agent_surface_checks(&workspace, "agent-context/surface-check")
        .unwrap();
    assert_eq!(checks.len(), 2);
    assert!(
        checks
            .iter()
            .all(|check| check.state == AgentSurfaceCheckStateV1::Passed)
    );
    assert!(
        store
            .agent_surface_checks(&workspace, "agent-context/other")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_revision_results_are_dropped_instead_of_polluting_newer_publications() {
    let (_directory, store) = storage();
    let workspace = WorkspaceId::default();
    activate(&store, &workspace, 3);

    let stale = record(
        GatewayPublicationRevision::new(2).unwrap(),
        AgentModelSurfaceV2::CodexCli,
        AgentSurfaceCheckStateV1::Passed,
    );
    assert!(!store.save_agent_surface_check(&workspace, &stale).unwrap());
    assert!(
        store
            .agent_surface_checks(&workspace, "agent-context/surface-check")
            .unwrap()
            .is_empty()
    );

    let current = record(
        GatewayPublicationRevision::new(3).unwrap(),
        AgentModelSurfaceV2::CodexCli,
        AgentSurfaceCheckStateV1::Passed,
    );
    assert!(
        store
            .save_agent_surface_check(&workspace, &current)
            .unwrap()
    );
    // No active publication at all also drops the write.
    let other = WorkspaceId::parse("personal/other").unwrap();
    let missing = record(
        GatewayPublicationRevision::new(3).unwrap(),
        AgentModelSurfaceV2::CodexCli,
        AgentSurfaceCheckStateV1::Passed,
    );
    assert!(!store.save_agent_surface_check(&other, &missing).unwrap());
}

#[test]
fn invalid_check_records_fail_closed() {
    let (_directory, store) = storage();
    let workspace = WorkspaceId::default();
    let revision = GatewayPublicationRevision::new(1).unwrap();

    let mut empty_models = record(
        revision,
        AgentModelSurfaceV2::CodexCli,
        AgentSurfaceCheckStateV1::Passed,
    );
    empty_models.checked_model_ids.clear();
    assert!(
        store
            .save_agent_surface_check(&workspace, &empty_models)
            .is_err()
    );

    let mut wrong_schema = record(
        revision,
        AgentModelSurfaceV2::CodexCli,
        AgentSurfaceCheckStateV1::Passed,
    );
    wrong_schema.schema = "hiroute.agent-surface-check/v2".into();
    assert!(
        store
            .save_agent_surface_check(&workspace, &wrong_schema)
            .is_err()
    );
}
