use super::*;

fn content_plan() -> TransactionPlanV1 {
    let mut publication: crate::GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    publication = publication.into_current().unwrap();
    publication.publication_revision = crate::GatewayPublicationRevision::new(1).unwrap();
    let versions: Vec<_> = publication
        .plans
        .iter()
        .map(|compiled| {
            let legacy = PlanVersionV1::from_unversioned_compiled_recovery(
                publication.workspace_id.clone(),
                compiled.clone(),
            )
            .unwrap();
            PlanVersionV1::new(
                publication.workspace_id.clone(),
                legacy.configuration,
                legacy.compiled,
            )
            .unwrap()
        })
        .collect();
    publication.plan_heads = versions
        .iter()
        .map(|version| PlanHeadV1 {
            reference: version.reference.clone(),
            head_revision: version.reference.content_revision,
            model_alias: version.compiled.model_alias().clone(),
            status: PlanLifecycleV1::Enabled,
        })
        .collect();
    let version = versions[0].clone();
    let head = publication.plan_heads[0].clone();
    assert_eq!(head.head_revision, 1);
    let spec = ChangeSpecV1 {
        schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!("agent-plan/{}", version.reference.plan_id.as_str())),
        desired_state: json!({
            "schema":"hiroute.plan-content-change/v2",
            "target":{"intent":"create","creation_key":"cold-reconstruction"},
            "editor":version.configuration.editor(Some(head.model_alias.as_str().into())).unwrap(),
            "consumed_draft":null
        }),
    };
    let record =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    TransactionPlanV1::from_plan_content_planner(spec, version, head, None, None, record, None)
        .unwrap()
}

#[test]
fn cold_content_reconstruction_authenticates_once_and_reuses_the_joined_proof() {
    let plan = content_plan();
    let stored = StoredOperationInputV1::freeze(&plan).unwrap();
    CONTENT_DECODES.with(|count| count.set(0));
    let rebuilt = stored.build().unwrap();
    assert_eq!(rebuilt, plan);
    assert_eq!(CONTENT_DECODES.with(std::cell::Cell::get), 1);
    let warm = TransactionPlanV1::from_registered_typed_planner(
        rebuilt.spec().clone(),
        rebuilt.control().clone(),
        None,
        vec![],
        vec![],
        rebuilt.external().to_vec(),
    )
    .unwrap();
    assert_eq!(warm, rebuilt);
    assert_eq!(CONTENT_DECODES.with(std::cell::Cell::get), 1);
}

#[test]
fn reused_content_proof_still_binds_control_and_cold_publication_bytes() {
    let plan = content_plan();
    let mut control = plan.control().clone();
    control["change_spec_digest"] = json!(CanonicalDigest::of_bytes(b"changed"));
    assert!(
        TransactionPlanV1::from_registered_typed_planner(
            plan.spec().clone(),
            control,
            None,
            vec![],
            vec![],
            plan.external().to_vec()
        )
        .is_err()
    );
    let mut stored = StoredOperationInputV1::freeze(&plan).unwrap();
    let bytes = &mut stored.external[0].desired["publication_record"]["bytes"];
    let original = bytes[0].as_u64().unwrap();
    bytes[0] = json!((original + 1) % 256);
    assert!(stored.build().is_err());
}

#[test]
fn restored_content_journal_encodes_unchanged_immutable_inputs_once() {
    let plan = content_plan();
    let equivalent = plan.clone();
    let workspace = WorkspaceId::default();
    let scope = IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyAgentPlanChange",
        "restore-content",
    )
    .unwrap();
    let request = CanonicalDigest::of_bytes(b"restore-content-request");
    stored::CANONICAL_PLAN_ENCODINGS.with(|count| count.set(0));
    let mut operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request),
        workspace,
        scope,
        request,
        CanonicalDigest::of_bytes(b"restore-content-accepted"),
        RevisionSetV1 {
            target: 0,
            dependencies: std::collections::BTreeMap::new(),
        },
        plan,
    )
    .unwrap();
    let steps = operation.steps.clone();
    operation
        .restore_durable_state(OperationState::Succeeded, 7, steps.clone(), None)
        .unwrap();
    assert!(operation.journal_is_committed().unwrap());
    assert_eq!(
        stored::CANONICAL_PLAN_ENCODINGS.with(std::cell::Cell::get),
        1
    );
    let mut tampered = steps.clone();
    tampered[3].deterministic_input_digest = CanonicalDigest::of_bytes(b"tampered-step");
    assert!(
        operation
            .restore_durable_state(OperationState::Succeeded, 8, tampered, None)
            .is_err()
    );
    // A detached plan must establish its own proof, even when its values compare equal.
    operation.plan = std::sync::Arc::new(equivalent);
    operation
        .restore_durable_state(OperationState::Succeeded, 8, steps.clone(), None)
        .unwrap();
    assert_eq!(
        stored::CANONICAL_PLAN_ENCODINGS.with(std::cell::Cell::get),
        2
    );
    // The public identity fields are separately bound; they cannot inherit the checkpoint.
    operation.accepted_digest = CanonicalDigest::of_bytes(b"different-accepted-identity");
    operation
        .restore_durable_state(OperationState::Succeeded, 9, steps, None)
        .unwrap();
    assert_eq!(
        stored::CANONICAL_PLAN_ENCODINGS.with(std::cell::Cell::get),
        3
    );
}
