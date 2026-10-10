use super::*;
use hiroute_application::compute_management::ComputeSubscriptionMaintenanceScopeV1;
use hiroute_application_api::{ComputeValidationRefV2, OperationReferenceV1};
use hiroute_domain::ComputeManagementRepositoryPort;

fn subscription(candidate_ref: &str) -> ComputeCandidateFactsV2 {
    let mut facts = candidate(candidate_ref, "unused-subscription-input");
    let digest = hiroute_domain::CanonicalDigest::of_bytes(candidate_ref.as_bytes());
    let validation = ComputeValidationRefV2 {
        approval_operation: OperationReferenceV1 {
            operation_id: format!("op_{}", &digest.as_str()[7..39]),
            state: "succeeded".into(),
            sequence: 1,
            cancellable: false,
        },
        validation_ref: "validation/subscription".into(),
        validation_revision: 1,
    };
    facts.producer = ComputeCandidateProducerV2::Cpa;
    facts.lineage_ref = "lineage/subscription".into();
    facts.provenance = ComputeCandidateProvenanceV2::ConnectorOwned {
        connector_id: "connector/cpa".into(),
        account_ref: "account/subscription".into(),
    };
    facts.authentication = Some(GatewayAuthenticationSemanticsV1::Bearer);
    facts.native_recheck = None;
    facts.credential_binding = ComputeCredentialBindingV2::CpaOwned {
        account_ref: "account/subscription".into(),
        validation: validation.clone(),
    };
    facts.validation = Some(validation);
    for model in &mut facts.models {
        model.membership = ComputeModelMembershipV2::Observed;
        model.capabilities.tool.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.vision.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.streaming.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.context_tokens.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.max_output_tokens.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.native_reasoning.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
    }
    facts
}

fn change(
    stores: &LocalStorageSet,
    facts: &ComputeCandidateFactsV2,
    selected: &[&str],
) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        edit: None,
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: facts.candidate.clone(),
        },
        expected_revisions: stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .revisions,
        selected_model_refs: selected.iter().map(|model| (*model).into()).collect(),
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: facts.validation.clone(),
    }
}

fn register_verified_subscription(
    stores: &LocalStorageSet,
    registry: &TrustedComputeCandidateRegistry,
    facts: &ComputeCandidateFactsV2,
    key: &str,
) {
    // This storage test starts at verified subscription facts. Admission still uses
    // the real durable handoff and transaction; the daemon tests cover the check.
    let validation = facts.validation.as_ref().unwrap();
    stores.control().with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO operations(
                    operation_id,workspace_id,principal,operation_kind,idempotency_key,
                    request_digest,accepted_change_digest,state,generation,operation_json,
                    created_at,updated_at
                 ) VALUES(?1,'personal/default','local-control','ApplySubscriptionCheck',?2,
                          'sha256:request','sha256:accepted','succeeded',1,'{}',1,1)",
                rusqlite::params![validation.approval_operation.operation_id, key],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO compute_subscription_validations(
                    operation_id,candidate_ref,candidate_revision,record_json,state,updated_at
                 ) VALUES(?1,?2,?3,?4,'verified',1)",
                rusqlite::params![
                    validation.approval_operation.operation_id,
                    facts.candidate.candidate_ref,
                    facts.candidate.candidate_revision,
                    serde_json::json!({ "validation": validation }).to_string(),
                ],
            )
            .unwrap();
    });
    registry.register_compute_candidate(facts.clone()).unwrap();
}

fn save(
    stores: &LocalStorageSet,
    registry: &TrustedComputeCandidateRegistry,
    facts: &ComputeCandidateFactsV2,
    selected: &[&str],
    key: &str,
) -> hiroute_domain::ComputeManagementSourceV2 {
    register_verified_subscription(stores, registry, facts, key);
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(registry, stores.control(), stores.secrets(), &input);
    let preview = planner.preview(change(stores, facts, selected)).unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        key,
    );
    let snapshot = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    snapshot.sources.into_iter().next().unwrap()
}

#[test]
fn subscription_explicit_selection_adds_and_removes_models_on_the_same_source() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut edited = subscription("candidate/add");
    edited.existing_source_id = Some(first.source_id.clone());
    let added = save(
        &stores,
        &registry,
        &edited,
        &["model/one", "model/two"],
        "add",
    );
    assert_eq!(added.source_id, first.source_id);
    assert_eq!(added.lineage_digest, first.lineage_digest);
    assert_eq!(added.provenance, first.provenance);
    assert_eq!(added.revision, first.revision + 1);
    assert_eq!(added.models.len(), 2);
    assert_eq!(added.models[0], first.models[0]);
    assert!(added.models.iter().all(|model| model.execution_eligible));
    assert!(added.credentials.is_empty());
    let second_binding = added.models[1].binding_id.clone();

    let mut edited = subscription("candidate/remove");
    edited.existing_source_id = Some(first.source_id.clone());
    let removed = save(&stores, &registry, &edited, &["model/two"], "remove");
    assert_eq!(removed.source_id, first.source_id);
    assert_eq!(removed.revision, added.revision + 1);
    assert_eq!(removed.models.len(), 1);
    assert_eq!(removed.models[0].model_ref, "model/two");
    assert_eq!(removed.models[0].binding_id, second_binding);

    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let persisted = reopened
        .control()
        .compute_management_source(&removed.source_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, removed);
}

#[test]
fn subscription_explicit_selection_rejects_ineligible_and_unknown_models_without_writes() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut edited = subscription("candidate/invalid-add");
    edited.existing_source_id = Some(first.source_id.clone());
    edited.models[1].selectable = false;
    edited.models[1].reason = Some("model_not_allowed".into());
    registry.register_compute_candidate(edited.clone()).unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let before = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    for selected in [
        vec!["model/one", "model/two"],
        vec!["model/unknown"],
        vec!["model/one", "model/one"],
    ] {
        assert!(matches!(
            planner.preview(change(&stores, &edited, &selected)),
            Err(ComputeManagementPlanningErrorV2::ModelNotSelectable)
        ));
    }
    let after = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn saved_subscription_edit_retains_handoff_owner_and_rejects_reuse() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let facts = subscription("candidate/retained-edit");
    let source = save(
        &stores,
        &registry,
        &facts,
        &["model/one"],
        "retained-initial",
    );
    let approval_id = hiroute_domain::OperationId::parse(
        &facts
            .validation
            .as_ref()
            .unwrap()
            .approval_operation
            .operation_id,
    )
    .unwrap();
    let receipt = || {
        stores
            .control()
            .compute_subscription_validation(&approval_id)
            .unwrap()
            .unwrap()
    };
    let before_receipt = receipt();
    assert_eq!(
        before_receipt.state,
        crate::ComputeSubscriptionValidationStateV1::Retained
    );
    assert!(before_receipt.save_operation_id.is_some());

    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let mut edit = change(&stores, &facts, &["model/one"]);
    edit.subject = ComputeManagementSubjectV2::SavedSource {
        source_id: source.source_id.clone(),
    };
    edit.intent = ComputeManagementIntentV2::SaveDisabled;
    let mut foreign = facts.validation.clone().unwrap();
    foreign.validation_ref = "validation/foreign".into();
    for validation in [None, Some(foreign)] {
        let mut invalid = edit.clone();
        invalid.validation = validation;
        assert!(matches!(
            planner.preview(invalid),
            Err(ComputeManagementPlanningErrorV2::ValidationConflict)
        ));
    }
    let preview = planner.preview(edit).unwrap();
    let stale = ComputeConnectionApplyRequestV1 {
        spec: preview.result.spec.clone(),
        accept_digest: preview.result.accept_digest.clone(),
        expected_revisions: preview.result.expected_revisions.clone(),
        idempotency_key: "retained-stale-edit".into(),
    };
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        "retained-disable",
    );
    let disabled = stores
        .control()
        .compute_management_source(&source.source_id)
        .unwrap()
        .unwrap();
    assert_eq!(disabled.state, MaterializationState::Disabled);
    assert_eq!(disabled.revision, source.revision + 1);
    assert_eq!(disabled.validation, source.validation);
    assert_eq!(disabled.models, source.models);
    assert!(matches!(
        planner.prepare_apply(stale),
        Err(ComputeManagementPlanningErrorV2::RevisionConflict)
    ));

    // Candidate handoff remains one-shot even though SavedSource can edit its retained source.
    let reused = planner
        .preview(change(&stores, &facts, &["model/one"]))
        .unwrap();
    let prepared = planner
        .prepare_apply(ComputeConnectionApplyRequestV1 {
            spec: reused.result.spec,
            accept_digest: reused.result.accept_digest,
            expected_revisions: reused.result.expected_revisions,
            idempotency_key: "retained-double-consume".into(),
        })
        .unwrap();
    let rejection = coordinator
        .accept_prepared(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            prepared,
        )
        .err()
        .expect("a retained Candidate receipt must not be admitted for a second save");
    // The coordinator maps storage admission Conflict to its public stale-preview error.
    assert!(
        matches!(rejection, TransactionError::ChangePreviewStale),
        "unexpected double-consumption rejection: {rejection:?}"
    );
    let after_receipt = receipt();
    assert_eq!(after_receipt.state, before_receipt.state);
    assert_eq!(
        after_receipt.save_operation_id,
        before_receipt.save_operation_id
    );
    assert_eq!(after_receipt.record_json, before_receipt.record_json);
    assert_eq!(
        stores
            .control()
            .compute_management_source(&source.source_id)
            .unwrap()
            .unwrap(),
        disabled
    );
}

#[test]
fn subscription_recheck_and_background_refresh_retain_membership_when_rights_change() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut refreshed = subscription("candidate/refresh");
    refreshed.existing_source_id = Some(first.source_id.clone());
    refreshed.models[0].selectable = false;
    refreshed.models[0].reason = Some("model_not_allowed".into());
    registry
        .register_compute_candidate(refreshed.clone())
        .unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let scope = ComputeSubscriptionMaintenanceScopeV1 {
        source_id: first.source_id.clone(),
        expected_source_revision: first.revision,
    };
    assert!(
        planner
            .preview_subscription_maintenance(change(&stores, &refreshed, &["model/one"]), &scope)
            .is_ok()
    );
    for selected in [vec!["model/one", "model/two"], vec!["model/two"]] {
        assert!(matches!(
            planner
                .preview_subscription_maintenance(change(&stores, &refreshed, &selected), &scope),
            Err(ComputeManagementPlanningErrorV2::ModelNotSelectable)
        ));
    }
    let retained = save(&stores, &registry, &refreshed, &["model/one"], "recheck");
    assert_eq!(retained.source_id, first.source_id);
    assert_eq!(retained.models.len(), 1);
    assert_eq!(retained.models[0].binding_id, first.models[0].binding_id);
    assert!(!retained.models[0].execution_eligible);
    assert!(
        compile_compute_management_source(&retained)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn removed_subscription_cannot_be_resurrected_by_late_maintenance_or_checked_candidate() {
    let root = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let source = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut checked = subscription("candidate/late");
    checked.existing_source_id = Some(source.source_id.clone());
    registry
        .register_compute_candidate(checked.clone())
        .unwrap();
    let old = change(&stores, &checked, &["model/one"]);
    // Native login files belong to the connector. Management deletion has only Control
    // and owned Secret effects; it has no External or Runtime mutation authority.
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let deletion = super::lifecycle::edit(
        &stores,
        &source,
        hiroute_application_api::ComputeManagementEditV1::Delete,
        &[],
    );
    let preview = planner.preview(deletion.clone()).unwrap();
    assert!(preview.plan().secrets().is_empty());
    assert!(preview.plan().external().is_empty());
    assert!(preview.plan().runtime().is_empty());
    super::lifecycle::execute(&stores, &registry, deletion, "delete-subscription");
    let scope = ComputeSubscriptionMaintenanceScopeV1 {
        source_id: source.source_id,
        expected_source_revision: source.revision,
    };
    assert!(planner.preview(old).is_err());
    let refreshed = change(&stores, &checked, &["model/one"]);
    assert!(matches!(
        planner.preview(refreshed.clone()),
        Err(ComputeManagementPlanningErrorV2::SourceNotFound)
    ));
    assert!(
        planner
            .preview_subscription_maintenance(refreshed, &scope)
            .is_err()
    );
    assert!(
        stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .sources
            .is_empty()
    );
}

#[test]
fn subscription_v3_mode_tracks_lifecycle_revisions_and_never_revives_after_delete() {
    use super::lifecycle::{edit, execute, saved};
    use hiroute_application::compute_management::{
        query_compute_management, query_compute_management_v3,
    };
    use hiroute_application_api::{
        ComputeManagementEditV1, ComputeManagementQueryV2, ComputeSubscriptionModeV1,
    };

    for managed in [false, true] {
        let root = tempdir().unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let registry = TrustedComputeCandidateRegistry::new();
        let expected = if managed {
            ComputeSubscriptionModeV1::CpaManaged
        } else {
            ComputeSubscriptionModeV1::NativeBorrowed
        };
        let facts_for = |label: &str| {
            let mode = if managed { "managed/" } else { "" };
            let mut facts = subscription(&format!("candidate/cpa/codex/{mode}{label}"));
            facts.provenance = ComputeCandidateProvenanceV2::ConnectorOwned {
                connector_id: "connector.cpa.codex".into(),
                account_ref: "account/subscription".into(),
            };
            facts
        };
        let assert_mode = |source: &hiroute_domain::ComputeManagementSourceV2| {
            let query = ComputeManagementQueryV2::default();
            let v3 = query_compute_management_v3(
                stores.control(),
                stores.runtime(),
                &WorkspaceId::default(),
                &query,
                None,
            )
            .unwrap();
            assert_eq!(v3.subscription_modes.len(), 1);
            let mode = &v3.subscription_modes[0];
            assert_eq!(mode.source_id, source.source_id);
            assert_eq!(mode.source_revision, source.revision);
            assert_eq!(mode.mode, expected);
            let v2 = query_compute_management(
                stores.control(),
                stores.runtime(),
                &WorkspaceId::default(),
                &query,
            )
            .unwrap();
            assert_eq!(v3.into_v2(), v2);
            assert!(
                serde_json::to_value(v2)
                    .unwrap()
                    .get("subscription_modes")
                    .is_none()
            );
        };
        let original = save(
            &stores,
            &registry,
            &facts_for("first"),
            &["model/one"],
            "first-mode",
        );
        assert_mode(&original);
        execute(
            &stores,
            &registry,
            edit(
                &stores,
                &original,
                ComputeManagementEditV1::Rename {
                    display_name: "Subscription team".into(),
                },
                &[],
            ),
            "rename-mode",
        );
        let named = saved(&stores);
        assert!(named.revision > original.revision);
        assert_eq!(named.source_id, original.source_id);
        assert_eq!(named.models, original.models);
        assert_mode(&named);

        // These are verified-check storage fixtures, not native OAuth or inference evidence.
        for (label, model_ref, upstream, disabled) in [
            ("ready-append", "model/two", "upstream-two", false),
            ("disabled-append", "model/three", "upstream-three", true),
        ] {
            let before = saved(&stores);
            if disabled {
                let mut disable = edit(&stores, &before, ComputeManagementEditV1::Delete, &[]);
                disable.edit = None;
                disable.intent = ComputeManagementIntentV2::SaveDisabled;
                disable.selected_model_refs =
                    before.models.iter().map(|m| m.model_ref.clone()).collect();
                execute(&stores, &registry, disable, "disable-mode");
            }
            let before = saved(&stores);
            let mut facts = facts_for(label);
            facts.existing_source_id = Some(before.source_id.clone());
            let mut extra = facts.models[0].clone();
            extra.model_ref = model_ref.into();
            extra.upstream_model_id = upstream.into();
            facts.models.retain(|model| model.model_ref != model_ref);
            facts.models.push(extra);
            register_verified_subscription(&stores, &registry, &facts, label);
            let mut append = change(&stores, &facts, &[model_ref]);
            append.edit = Some(ComputeManagementEditV1::AppendModels);
            execute(&stores, &registry, append, label);
            let after = saved(&stores);
            assert_eq!(after.source_id, before.source_id);
            assert_eq!(after.display_name, named.display_name);
            assert_eq!(after.state, before.state);
            assert_eq!(after.credentials, before.credentials);
            assert_eq!(&after.models[..before.models.len()], &before.models);
            assert_eq!(after.models.len(), before.models.len() + 1);
            assert_mode(&after);
        }
        let disabled = saved(&stores);
        assert_eq!(disabled.state, MaterializationState::Disabled);
        let mut late = facts_for("late-maintenance");
        late.existing_source_id = Some(disabled.source_id.clone());
        register_verified_subscription(&stores, &registry, &late, "late-mode");
        let scope = ComputeSubscriptionMaintenanceScopeV1 {
            source_id: disabled.source_id.clone(),
            expected_source_revision: disabled.revision,
        };
        let input = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        assert!(matches!(
            planner
                .preview_subscription_maintenance(change(&stores, &late, &["model/one"]), &scope),
            Err(ComputeManagementPlanningErrorV2::RevisionConflict)
        ));
        assert_eq!(saved(&stores), disabled);
        let captured = change(&stores, &late, &["model/one"]);
        execute(
            &stores,
            &registry,
            edit(&stores, &disabled, ComputeManagementEditV1::Delete, &[]),
            "delete-mode",
        );
        assert!(planner.preview(captured).is_err());
        assert!(matches!(
            planner
                .preview_subscription_maintenance(change(&stores, &late, &["model/one"]), &scope),
            Err(ComputeManagementPlanningErrorV2::SourceNotFound)
        ));
        let v3 = query_compute_management_v3(
            stores.control(),
            stores.runtime(),
            &WorkspaceId::default(),
            &ComputeManagementQueryV2::default(),
            None,
        )
        .unwrap();
        assert!(v3.sources.is_empty());
        assert!(v3.subscription_modes.is_empty());
        drop(stores);
        let reopened = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let v3 = query_compute_management_v3(
            reopened.control(),
            reopened.runtime(),
            &WorkspaceId::default(),
            &ComputeManagementQueryV2::default(),
            None,
        )
        .unwrap();
        assert!(v3.sources.is_empty());
        assert!(v3.subscription_modes.is_empty());
    }
}
