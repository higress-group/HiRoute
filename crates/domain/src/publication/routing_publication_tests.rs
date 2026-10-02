use serde_json::Value;

use super::*;

#[test]
fn routing_publication_unknown_schema_and_revision_fail_closed() {
    let publication = GatewayPublicationV1::new(
        WorkspaceId::default(),
        GatewayPublicationRevision::new(1).unwrap(),
        AliasRegistryV1::default(),
        Vec::new(),
    )
    .unwrap();
    let mut value =
        serde_json::from_slice::<Value>(&publication.canonical_bytes().unwrap()).unwrap();
    value["schema"] = serde_json::json!(UNSUPPORTED_GATEWAY_PUBLICATION_SCHEMA_V1);
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        GatewayPublicationV1::decode(&bytes).unwrap_err(),
        PublicationError::UnsupportedSchema
    );

    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    value["schema"] = serde_json::json!(STORED_PUBLICATION_SCHEMA);
    value["publication_revision"] = serde_json::json!(0);
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        GatewayPublicationV1::decode(&bytes).unwrap_err(),
        PublicationError::InvalidRevision
    );
}

#[test]
fn model_grant_projection_rejects_conflicting_or_future_plan_provenance() {
    let fixture: Value = serde_json::from_slice::<serde_json::Value>(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let plan =
        serde_json::from_value::<crate::CompiledAgentPlanV1>(fixture["plans"][0].clone()).unwrap();
    let plan = plan.into_current().unwrap();
    let revision = plan.body.agent_plan_revision;
    let accepted_digest = plan.body.materialized_route_digest.clone();
    let name = plan.model_alias().as_str().to_owned();
    let grant = |revision, semantic_digest| GatewayExecutableGrantV2 {
        grant_id: "test-grant".into(),
        generation: 1,
        bearer_token_sha256: CanonicalDigest::of_bytes(b"test-grant-token"),
        model_grant: crate::AgentModelGrantV2::seal(
            AgentIngressProtocolV1::Responses,
            BTreeMap::from([(
                name.clone(),
                crate::AgentModelRouteV2::Plan {
                    plan_id: plan.agent_plan_id().clone(),
                    alias: plan.model_alias().clone(),
                    revision,
                    semantic_digest,
                },
            )]),
        )
        .unwrap(),
    };
    let original_grant = grant(revision, accepted_digest.clone());
    let plans = vec![plan.clone()];
    let aliases = materialize_aliases(&plans, std::slice::from_ref(&original_grant)).unwrap();
    assert!(snapshot::project_grants(vec![original_grant.clone()], &aliases, &plans).is_ok());
    for changed in [
        grant(revision, CanonicalDigest::of_bytes(b"conflicting-route")),
        grant(revision + 1, accepted_digest),
    ] {
        assert_eq!(
            snapshot::project_grants(vec![changed], &aliases, &plans).unwrap_err(),
            PublicationError::InvalidGrant,
        );
    }
    let mut next_body = plan.body.as_ref().clone();
    next_body.agent_plan_revision += 1;
    let next_plan = CompiledAgentPlanV1::seal_current(next_body).unwrap();
    let next_plans = vec![next_plan];
    let next_aliases =
        materialize_aliases(&next_plans, std::slice::from_ref(&original_grant)).unwrap();
    let projected =
        snapshot::project_grants(vec![original_grant], &next_aliases, &next_plans).unwrap();
    assert!(matches!(
        &projected[0].routes[&name],
        GatewayModelRouteV2::Plan { revision: current, semantic_digest, .. }
            if *current == revision + 1
                && semantic_digest == &next_plans[0].body.materialized_route_digest
    ));
}

#[test]
fn routing_publication_record_is_bound_to_its_workspace() {
    let publication = GatewayPublicationV1::new(
        WorkspaceId::default(),
        GatewayPublicationRevision::new(1).unwrap(),
        AliasRegistryV1::default(),
        Vec::new(),
    )
    .unwrap();
    let other = WorkspaceId::parse("personal/other").unwrap();
    assert_eq!(
        PublicationRecordV1::from_publication(other, &publication).unwrap_err(),
        PublicationError::InvalidWorkspace
    );
}

#[test]
fn adding_first_grant_does_not_mutate_the_independent_plan_revision() {
    let granted: GatewayPublicationV1 = serde_json::from_slice::<crate::GatewayPublicationV1>(
        include_bytes!("../../../../e2e/product/fixtures/routing/current-publication.v3.json"),
    )
    .unwrap();
    let mut plan_only = granted.clone();
    plan_only.publication_revision = GatewayPublicationRevision::new(10).unwrap();
    plan_only.aliases.clear();
    plan_only.grants.clear();
    plan_only.validate().unwrap();

    granted.validate_transition_from(&plan_only).unwrap();
    let plan_only_projection = plan_only.gateway_snapshot().unwrap();
    assert_eq!(
        plan_only_projection.admission,
        GatewayAdmissionStateV1::NoNewCalls
    );
    assert!(plan_only_projection.aliases.is_empty());
    assert!(plan_only_projection.grants.is_empty());
    let projection = granted.gateway_snapshot().unwrap();
    for alias in &projection.aliases {
        let plan = granted
            .plans
            .iter()
            .find(|plan| plan.model_alias() == &alias.served_model_id)
            .unwrap();
        let attempts = ordered_candidates(plan)
            .unwrap()
            .into_iter()
            .map(|candidate| (candidate.binding_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        for projected in &alias.candidates {
            let attempt = attempts[projected.stable_target_key.as_str()];
            let pricing = projected.pricing_identity.as_ref().unwrap();
            assert_eq!(pricing.source_id, attempt.source_id);
            assert_eq!(
                pricing.source_identity_digest,
                attempt.source_identity_digest
            );
            assert_eq!(
                pricing.model_configuration_id,
                attempt.model_configuration_id
            );
            assert_eq!(pricing.actual_offer_ref, attempt.offer_ref);
        }
    }
}

#[test]
fn revoking_last_grant_preserves_the_independent_plan_publication() {
    let granted: GatewayPublicationV1 = serde_json::from_slice::<crate::GatewayPublicationV1>(
        include_bytes!("../../../../e2e/product/fixtures/routing/current-publication.v3.json"),
    )
    .unwrap();
    let first_grant_id = granted.grants[0].grant_id.clone();
    let last_grant_id = granted.grants[1].grant_id.clone();
    let one_grant = granted
        .next_without_access_grant(GatewayPublicationRevision::new(3).unwrap(), &first_grant_id)
        .unwrap();
    let revoked = one_grant
        .next_without_access_grant(GatewayPublicationRevision::new(4).unwrap(), &last_grant_id)
        .unwrap();

    assert_eq!(revoked.plans.len(), granted.plans.len());
    assert!(
        revoked
            .plans
            .iter()
            .all(|plan| plan.body.schema == AGENT_PLAN_COMPILED_SCHEMA_V3)
    );
    for (current, legacy) in revoked.plans.iter().zip(&granted.plans) {
        assert_eq!(current.agent_plan_id(), legacy.agent_plan_id());
        assert_eq!(
            current.body.materialized.request_owned,
            legacy.body.materialized.request_owned
        );
        assert_eq!(
            current.body.materialized.attempt_owned.groups.len(),
            legacy.body.materialized.attempt_owned.groups.len()
        );
    }
    assert!(revoked.grants.is_empty());
    assert!(revoked.aliases.is_empty());
    let revoked_projection = revoked.gateway_snapshot().unwrap();
    assert_eq!(
        revoked_projection.admission,
        GatewayAdmissionStateV1::NoNewCalls
    );
    assert!(revoked_projection.aliases.is_empty());
    assert!(revoked_projection.grants.is_empty());
    revoked.validate_transition_from(&one_grant).unwrap();
}
