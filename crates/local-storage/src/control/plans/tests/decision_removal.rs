use super::*;

const SERVICE_ID: &str = "decision/removal";

fn service() -> DecisionServiceV1 {
    DecisionServiceV1 {
        id: SERVICE_ID.into(),
        revision: 1,
        name: "Removal fixture".into(),
        connection: DecisionConnectionV1::Custom {
            endpoint: "https://decision.example.test/decide".into(),
            timeout_ms: 3000,
            auth_header: None,
        },
    }
}

fn fixture() -> (GatewayPublicationV1, PlanVersionV1) {
    let publication: GatewayPublicationV1 = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../e2e/product/fixtures/routing/current-publication.v3.json"
    )))
    .unwrap();
    let mut body = (*publication
        .plans
        .iter()
        .find(|plan| plan.agent_plan_id().as_str() == "plan/smart")
        .unwrap()
        .clone()
        .into_current()
        .unwrap()
        .body)
        .clone();
    let mode = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(service()),
    };
    let RequestOwnedRouteV1::Classified {
        classifier,
        judgment,
        native_protocol,
        ..
    } = &mut body.materialized.request_owned
    else {
        panic!("classified fixture");
    };
    *classifier = ComplexityClassifierV1::with_mode(Vec::new(), mode.clone()).unwrap();
    *judgment = None;
    *native_protocol = None;
    body.materialized_route_digest = body.materialized.route_digest().unwrap();
    let compiled = CompiledAgentPlanV1::seal_current(body).unwrap();
    let selections = |group_id| {
        compiled
            .body
            .materialized
            .attempt_owned
            .groups
            .iter()
            .find(|group| group.group_id == group_id)
            .unwrap()
            .candidates
            .iter()
            .map(|candidate| CandidateSelectionV1 {
                binding_id: candidate.binding_id.clone(),
                reasoning: match &candidate.exact_reasoning {
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
            .collect()
    };
    let configuration = AgentPlanAuthoringV2 {
        schema: PLAN_AUTHORING_SCHEMA_V2.into(),
        follow_request_reasoning: false,
        display_name: compiled.body.identity.display_name.clone(),
        purpose: compiled.body.identity.purpose.clone(),
        mode: PlanEditorMode::SmartSaving,
        requirements: CapabilityRequirementsV1::default(),
        limits: compiled.body.materialized.attempt_owned.limits.clone(),
        strategy: AgentPlanStrategyV2::SmartSaving {
            native_protocol: None,
            economy: selections(MaterializedGroupId::Economy),
            primary: selections(MaterializedGroupId::Primary),
            judgment: JudgmentSettingsV1::default(),
            reselect_on_user_message: false,
            classifier: mode,
            complex_keywords: Vec::new(),
        },
        delegation_enabled: false,
        work: None,
    };
    let version =
        PlanVersionV1::new(WorkspaceId::default(), configuration, compiled.clone()).unwrap();
    let publication = publication
        .next_with_plan(
            GatewayPublicationRevision::new(2).unwrap(),
            publication.alias_registry.clone(),
            compiled,
        )
        .unwrap();
    (publication, version)
}

fn save_service(store: &ControlStore) {
    store.connection.borrow().execute(
        "INSERT INTO decision_services(workspace_id,service_id,revision,service_json) VALUES(?1,?2,1,?3)",
        params![WorkspaceId::DEFAULT, SERVICE_ID, encode(&service()).unwrap()],
    ).unwrap();
}

fn blockers(store: &ControlStore) -> PortResult<Vec<PlanReferenceBlockerV1>> {
    store.decision_removal_references(&WorkspaceId::default(), SERVICE_ID, 1)
}

fn head(store: &ControlStore, version: &PlanVersionV1, status: PlanLifecycleV1) {
    let head = PlanHeadV1 {
        reference: version.reference.clone(),
        head_revision: 1,
        model_alias: version.compiled.model_alias().clone(),
        status,
    };
    store.connection.borrow().execute(
        "INSERT OR REPLACE INTO plan_heads(workspace_id,plan_id,head_revision,head_json) VALUES(?1,?2,1,?3)",
        params![WorkspaceId::DEFAULT, head.reference.plan_id.as_str(), encode(&head).unwrap()],
    ).unwrap();
}

#[test]
fn decision_removal_and_general_removal_share_active_version_protection() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let store = stores.control();
    let (_, version) = fixture();
    save_service(store);
    seed(store, &version);
    let bindings = vec![
        version.compiled.body.materialized.attempt_owned.groups[0].candidates[0]
            .binding_id
            .clone(),
    ];
    let general = || store.compute_management_references(&WorkspaceId::default(), &bindings);
    assert!(blockers(store).is_err());
    assert!(general().is_err());
    store
        .reconcile_plan_versions(&WorkspaceId::default(), &[], 1)
        .unwrap();
    assert!(blockers(store).unwrap().is_empty());
    assert!(general().unwrap().is_empty());
    for status in [
        PlanLifecycleV1::Enabled,
        PlanLifecycleV1::Disabled,
        PlanLifecycleV1::Deleted,
    ] {
        head(store, &version, status);
        assert_eq!(
            blockers(store).unwrap().is_empty(),
            status == PlanLifecycleV1::Deleted
        );
        assert_eq!(
            general().unwrap().is_empty(),
            status == PlanLifecycleV1::Deleted
        );
    }
    let hold = reservation(&version, "run/decision-removal");
    store.acquire_exact_plan_version(&hold).unwrap();
    assert_eq!(
        blockers(store).unwrap()[0].protection,
        PlanReferenceProtectionV1::UnresolvedHold
    );
    assert!(!general().unwrap().is_empty());
    store
        .reconcile_plan_versions(&WorkspaceId::default(), &[], 99)
        .unwrap();
    assert!(!blockers(store).unwrap().is_empty());
    store
        .release_plan_version(&WorkspaceId::default(), &hold.owner)
        .unwrap();
    store
        .connection
        .borrow()
        .execute("UPDATE plan_versions SET state='prepared'", [])
        .unwrap();
    assert_eq!(
        blockers(store).unwrap()[0].protection,
        PlanReferenceProtectionV1::PreparedVersion
    );
    assert!(!general().unwrap().is_empty());
    store
        .connection
        .borrow()
        .execute("UPDATE plan_versions SET state='published'", [])
        .unwrap();
    assert!(blockers(store).unwrap().is_empty());
    assert_eq!(
        store
            .published_plan_versions(&WorkspaceId::default())
            .unwrap(),
        vec![version]
    );
}

#[test]
fn decision_removal_checks_publications_with_empty_index_and_no_local_owner() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let store = stores.control();
    save_service(store);
    let (publication, version) = fixture();
    let bytes = publication.canonical_bytes().unwrap();
    let binding = vec![
        version.compiled.body.materialized.attempt_owned.groups[0].candidates[0]
            .binding_id
            .clone(),
    ];
    store.connection.borrow().execute(
        "INSERT INTO gateway_publications(workspace_id,publication_revision,digest,publication_bytes,state,created_at,updated_at) VALUES(?1,2,?2,?3,'active',1,1)",
        params![WorkspaceId::DEFAULT, CanonicalDigest::of_bytes(&bytes).as_str(), bytes],
    ).unwrap();
    for (state, protection) in [
        ("active", PlanReferenceProtectionV1::ActivePublication),
        ("prepared", PlanReferenceProtectionV1::PreparedPublication),
        ("lkg", PlanReferenceProtectionV1::LkgPublication),
    ] {
        store
            .connection
            .borrow()
            .execute("UPDATE gateway_publications SET state=?1", [state])
            .unwrap();
        let result = blockers(store).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].protection, protection);
        assert_eq!(result[0].plan_id.as_deref(), Some("plan/smart"));
        assert!(
            !store
                .compute_management_references(&WorkspaceId::default(), &binding)
                .unwrap()
                .is_empty()
        );
        let change = serde_json::json!({"decision_service_change": {"id":SERVICE_ID,"expected_revision":1,"service":null,"input_slot":null}});
        assert!(
            crate::control::decision_services::stage(
                &store.connection.borrow(),
                &WorkspaceId::default(),
                &change
            )
            .is_err()
        );
    }
    store
        .connection
        .borrow()
        .execute(
            "UPDATE gateway_publications SET publication_bytes=X'00'",
            [],
        )
        .unwrap();
    assert!(blockers(store).is_err());
    assert!(
        store
            .compute_management_references(&WorkspaceId::default(), &binding)
            .is_err()
    );
}

#[test]
fn decision_removal_v1_activation_rejects_a_post_stage_draft_and_compensates() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let store = stores.control();
    save_service(store);
    let workspace = WorkspaceId::default();
    let desired = serde_json::json!({"decision_service_change": {"id":SERVICE_ID,"expected_revision":1,"service":null,"input_slot":null}});
    let effect = store
        .apply_control(
            &OperationId::parse("operation/decision-removal-race").unwrap(),
            &workspace,
            store.current_revisions(&workspace).unwrap().target,
            &desired,
        )
        .unwrap();
    let mut draft = draft();
    draft.editor.smart.classifier = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(service()),
    };
    store.save_plan_draft(&draft, None).unwrap();
    assert_eq!(
        blockers(store).unwrap()[0].protection,
        PlanReferenceProtectionV1::Draft
    );
    let error = store.activate_control(&effect).unwrap_err();
    assert_eq!(error.context, "decision_service.in_use");
    assert_eq!(
        store.compensate_control(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(
        store.decision_service(&workspace, SERVICE_ID, 1).unwrap(),
        Some(service())
    );
    assert_eq!(
        store.plan_draft(&workspace, &draft.draft_id).unwrap(),
        Some(draft)
    );
}

#[test]
fn decision_removal_preview_is_read_only_and_fails_closed_on_unknown_owners() {
    let root = tempdir().unwrap();
    let stores = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let store = stores.control();
    save_service(store);
    let before = store.connection.borrow().total_changes();
    assert!(blockers(store).unwrap().is_empty());
    assert_eq!(store.connection.borrow().total_changes(), before);
    assert!(
        store
            .decision_removal_references(&WorkspaceId::default(), SERVICE_ID, 2)
            .is_err()
    );
    for index in 0..1000 {
        store.connection.borrow().execute(
            "INSERT INTO plan_drafts(workspace_id,draft_id,revision,draft_json) VALUES(?1,?2,1,'{}')",
            params![WorkspaceId::DEFAULT, format!("draft/unrelated-{index}")],
        ).unwrap();
    }
    assert!(blockers(store).unwrap().is_empty());
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO decision_plan_references VALUES(?1,?2,'unknown','orphan',1)",
            params![WorkspaceId::DEFAULT, SERVICE_ID],
        )
        .unwrap();
    assert!(blockers(store).is_err());
}
