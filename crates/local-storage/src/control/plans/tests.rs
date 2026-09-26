use super::*;
use crate::test_tempdir as tempdir;
use hiroute_domain::*;

fn draft() -> PlanDraftV1 {
    serde_json::from_value(serde_json::json!({
        "schema": PLAN_DRAFT_SCHEMA_V1, "workspace_id": WorkspaceId::DEFAULT,
        "draft_id":"draft/a", "revision":1,
        "editor": {"schema":PLAN_EDITOR_SCHEMA_V2,"display_name":"", "purpose":"",
          "mode":"fixed_model","candidates":[],"smart":{"economy":[],"primary":[],"primary_fallback":false,"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
          "free":{"candidates":[],"primary":[],"primary_fallback":false},"delegation_enabled":false,
          "requirements":{},"limits":{"maximum_attempts":6,"request_timeout_ms":60000,"attempt_timeout_ms":30000}}
    })).unwrap()
}

#[test]
fn saved_draft_cas_survives_reopen_and_cannot_overwrite_or_delete_newer_input() {
    let root = tempdir().unwrap();
    let workspace = WorkspaceId::default();
    let store = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let first = draft();
    store.control().save_plan_draft(&first, None).unwrap();
    let mut newer = first.clone();
    newer.revision = 2;
    newer.editor.purpose = "still editing".into();
    store.control().save_plan_draft(&newer, Some(1)).unwrap();
    assert_eq!(
        store.control().save_plan_draft(&first, None),
        Err(PlanVersionError::Conflict)
    );
    assert_eq!(
        store.control().discard_plan_draft(&workspace, "draft/a", 1),
        Err(PlanVersionError::Conflict)
    );
    drop(store);
    let reopened = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(
        reopened
            .control()
            .plan_draft(&workspace, "draft/a")
            .unwrap(),
        Some(newer)
    );
    assert!(
        reopened
            .control()
            .plan_heads(&workspace)
            .unwrap()
            .is_empty()
    );
    reopened
        .control()
        .discard_plan_draft(&workspace, "draft/a", 2)
        .unwrap();
    assert!(
        reopened
            .control()
            .plan_draft(&workspace, "draft/a")
            .unwrap()
            .is_none()
    );
}

fn version() -> PlanVersionV1 {
    let publication = GatewayPublicationV1::decode_persisted(
        include_str!("../../../../../e2e/product/golden/routing/compiled-publication.v2.json")
            .as_bytes(),
    )
    .unwrap();
    let compiled = publication
        .plans
        .into_iter()
        .find(|p| p.agent_plan_id().as_str() == "plan/custom")
        .unwrap();
    let candidates = compiled.body.materialized.attempt_owned.groups[0]
        .candidates
        .iter()
        .map(|c| CandidateSelectionV1 {
            binding_id: c.binding_id.clone(),
            reasoning: match &c.exact_reasoning {
                ExactNativeReasoningV1::Fixed { .. } => None,
                ExactNativeReasoningV1::Toggle { enabled, .. } => {
                    Some(ReasoningSelectionV1::Toggle { enabled: *enabled })
                }
                ExactNativeReasoningV1::Profile { profile, .. } => {
                    Some(ReasoningSelectionV1::Profile {
                        profile: profile.clone(),
                    })
                }
                ExactNativeReasoningV1::Budget { tokens, .. } => {
                    Some(ReasoningSelectionV1::Budget { tokens: *tokens })
                }
            },
        })
        .collect();
    PlanVersionV1::new(
        WorkspaceId::default(),
        AgentPlanAuthoringV2 {
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            display_name: compiled.body.identity.display_name.clone(),
            purpose: compiled.body.identity.purpose.clone(),
            mode: PlanEditorMode::FixedModel,
            requirements: CapabilityRequirementsV1::default(),
            limits: compiled.body.materialized.attempt_owned.limits.clone(),
            strategy: AgentPlanStrategyV2::Custom { candidates },
            delegation_enabled: false,
            work: None,
        },
        compiled,
    )
    .unwrap()
}

// This fixture proves storage rules only. Production publication evidence must use the sealed
// Operation/installer path, rather than treating this direct test-only seed as publication.
fn seed(store: &ControlStore, version: &PlanVersionV1) {
    store.connection.borrow().execute("INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES(?1,?2,?3,?4,?5,'published','test-only')", params![version.reference.workspace_id.as_str(),version.reference.plan_id.as_str(),version.reference.content_revision,version.reference.content_digest.as_str(),encode(version).unwrap()]).unwrap();
}

fn reservation(version: &PlanVersionV1, owner: &str) -> VersionReservationV1 {
    VersionReservationV1 {
        owner: VersionOwnerRefV1 {
            kind: VersionOwnerKindV1::Run,
            owner_id: owner.into(),
            purpose: VersionOwnerPurposeV1::Execution,
        },
        reference: version.reference.clone(),
        expires_at_unix: 100,
    }
}

#[test]
fn reservations_are_exact_idempotent_and_require_reconciliation_before_reclaim() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let store = stores.control();
    let version = version();
    seed(store, &version);
    let workspace = WorkspaceId::default();
    let hold = reservation(&version, "run/a");
    assert_eq!(
        store.acquire_exact_plan_version(&hold),
        Err(PlanVersionError::RecoveryRequired)
    );
    store.reconcile_plan_versions(&workspace, &[], 1).unwrap();
    assert_eq!(store.acquire_exact_plan_version(&hold).unwrap(), version);
    assert_eq!(store.acquire_exact_plan_version(&hold).unwrap(), version);
    let mut mismatch = hold.clone();
    mismatch.reference.content_digest = CanonicalDigest::of_bytes(b"wrong");
    assert_eq!(
        store.acquire_exact_plan_version(&mismatch),
        Err(PlanVersionError::Conflict)
    );
    assert_eq!(
        store.reclaim_plan_version(&version.reference),
        Err(PlanVersionError::Retained)
    );
    store.begin_plan_version_recovery().unwrap();
    assert_eq!(
        store.reclaim_plan_version(&version.reference),
        Err(PlanVersionError::RecoveryRequired)
    );
    // Past its reservation timeout, a proven accepted runtime owner still protects the version.
    store
        .reconcile_plan_versions(&workspace, std::slice::from_ref(&hold), 200)
        .unwrap();
    assert_eq!(
        store.reclaim_plan_version(&version.reference),
        Err(PlanVersionError::Retained)
    );
    store.release_plan_version(&workspace, &hold.owner).unwrap();
    store.release_plan_version(&workspace, &hold.owner).unwrap();
    store.reclaim_plan_version(&version.reference).unwrap();
    assert_eq!(
        store.lookup_exact_plan_version(&version.reference),
        Err(PlanVersionError::Unavailable)
    );
}

#[test]
fn crash_reservation_without_runtime_acceptance_is_reclaimed_only_after_expiry_and_reconcile() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let version = version();
    seed(stores.control(), &version);
    let workspace = WorkspaceId::default();
    let hold = reservation(&version, "run/orphan");
    stores
        .control()
        .reconcile_plan_versions(&workspace, &[], 1)
        .unwrap();
    stores.control().acquire_exact_plan_version(&hold).unwrap();
    drop(stores);
    let reopened = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    reopened.control().begin_plan_version_recovery().unwrap();
    assert_eq!(
        reopened.control().reclaim_plan_version(&version.reference),
        Err(PlanVersionError::RecoveryRequired)
    );
    reopened
        .control()
        .reconcile_plan_versions(&workspace, &[], 99)
        .unwrap();
    assert_eq!(
        reopened.control().reclaim_plan_version(&version.reference),
        Err(PlanVersionError::Retained)
    );
    reopened
        .control()
        .reconcile_plan_versions(&workspace, &[], 101)
        .unwrap();
    reopened
        .control()
        .reclaim_plan_version(&version.reference)
        .unwrap();
}
