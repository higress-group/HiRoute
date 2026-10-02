//! Each immutable boundary authenticates the candidates once, including nested digests.
use super::*;
use crate::routing::CANDIDATE_VALIDATIONS;

fn current_catalog_only(count: usize) -> GatewayPublicationV1 {
    let mut publication = serde_json::from_slice::<crate::GatewayPublicationV1>(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap()
    .into_current()
    .unwrap();
    let mut body = publication.plans[0].body.as_ref().clone();
    let candidate = body.materialized.attempt_owned.groups[0].candidates[0].clone();
    body.materialized.request_owned = RequestOwnedRouteV1::Ordered {
        cost_policy: crate::MaterializedCostPolicyV1::ApiEquivalent,
        ordered_groups: vec![MaterializedGroupId::Custom],
    };
    body.materialized.attempt_owned.groups = vec![crate::MaterializedModelGroupV1 {
        group_id: MaterializedGroupId::Custom,
        ordering_evidence: crate::MaterializedOrderingV1::ExplicitOrder,
        pinned_ratings: BTreeMap::new(),
        candidates: (0..count)
            .map(|index| {
                let mut value = candidate.clone();
                value.binding_id = format!("binding/performance-{index}");
                value
            })
            .collect(),
    }];
    body.materialized_route_digest = body.materialized.route_digest().unwrap();
    let plan = CompiledAgentPlanV1::seal_current(body).unwrap();
    publication
        .alias_registry
        .active
        .retain(|id, _| id == plan.agent_plan_id());
    publication.plans = vec![plan];
    publication.plan_heads.clear();
    publication.aliases.clear();
    publication.grants.clear();
    publication.validate_current_contract().unwrap();
    publication
}

#[test]
fn publication_decode_validates_each_candidate_once_for_1_5_10() {
    for count in [1, 5, 10] {
        let publication = current_catalog_only(count);
        let bytes = publication.canonical_bytes().unwrap();
        CANDIDATE_VALIDATIONS.with(|calls| calls.set(0));
        let decoded = GatewayPublicationV1::decode(&bytes).unwrap();
        assert_eq!(decoded, publication);
        assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), count);

        CANDIDATE_VALIDATIONS.with(|calls| calls.set(0));
        assert_eq!(decoded.into_current().unwrap(), publication);
        assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), 0);
    }
}

#[test]
fn publication_raw_boundaries_still_reject_nested_tampering_and_noncanonical_bytes() {
    let mut publication = current_catalog_only(5);
    let mut bytes = publication.canonical_bytes().unwrap();
    bytes.push(b' ');
    assert_eq!(
        GatewayPublicationV1::decode(&bytes),
        Err(PublicationError::NonCanonicalEncoding)
    );

    std::sync::Arc::make_mut(&mut publication.plans[0].body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[4]
        .protocol_profiles[0]
        .capability
        .native_model = "wrong-model".into();
    // Recompute the outer record digest: the inner protocol identity must still reject it.
    let bytes = serde_json::to_vec(&publication).unwrap();
    assert!(
        PublicationRecordV1::from_parts(
            publication.workspace_id.clone(),
            publication.publication_revision,
            CanonicalDigest::of_bytes(&bytes),
            bytes
        )
        .is_err()
    );
    assert!(publication.into_current().is_err());
}

#[test]
fn current_publication_still_checks_exact_grant_plan_binding_before_projection() {
    let mut publication = serde_json::from_slice::<crate::GatewayPublicationV1>(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap()
    .into_current()
    .unwrap();
    let grant = &mut publication.grants[0];
    let mut routes = grant.model_grant.routes.clone();
    let mut changed = false;
    for route in routes.values_mut() {
        if let crate::AgentModelRouteV2::Plan {
            semantic_digest, ..
        } = route
        {
            *semantic_digest = CanonicalDigest::of_bytes(b"different-executable-content");
            changed = true;
        }
    }
    assert!(changed);
    grant.model_grant = crate::AgentModelGrantV2::seal(grant.model_grant.protocol, routes).unwrap();
    assert_eq!(
        publication.into_current(),
        Err(PublicationError::InvalidGrant)
    );
}

#[test]
fn immutable_plan_proof_is_shared_but_never_survives_mutation_or_decode() {
    for count in [1, 5, 10] {
        let original = current_catalog_only(count).plans.remove(0);
        let bytes = serde_json::to_vec(&original).unwrap();
        CANDIDATE_VALIDATIONS.with(|calls| calls.set(0));
        let mut edited = original.clone();
        for _ in 0..10 {
            edited.validate().unwrap();
        }
        assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), 0);
        assert!(std::sync::Arc::ptr_eq(&original.body, &edited.body));
        std::sync::Arc::make_mut(&mut edited.body).agent_plan_revision += 1;
        assert!(!std::sync::Arc::ptr_eq(&original.body, &edited.body));
        assert!(edited.validate().is_err());
        original.validate().unwrap();
        let mut changed_digest = original.clone();
        changed_digest.digest = CanonicalDigest::of_bytes(b"different");
        assert!(changed_digest.validate().is_err());
        CANDIDATE_VALIDATIONS.with(|calls| calls.set(0));
        let decoded: CompiledAgentPlanV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), count);
        decoded.validate().unwrap();
        assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), count);
        assert!(!std::sync::Arc::ptr_eq(&original.body, &decoded.body));
        let mut tampered = serde_json::to_value(&original).unwrap();
        tampered["body"]["agent_plan_revision"] = serde_json::json!(99);
        assert!(serde_json::from_value::<CompiledAgentPlanV1>(tampered).is_err());
    }
}

#[test]
fn publication_record_proof_is_value_owned_and_storage_reads_authenticate_again() {
    let publication = current_catalog_only(10);
    let record =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    CANDIDATE_VALIDATIONS.with(|calls| calls.set(0));
    for _ in 0..10 {
        assert_eq!(record.clone().verify_current().unwrap(), publication);
    }
    assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), 0);
    let wire = serde_json::to_vec(&record).unwrap();
    let cold: PublicationRecordV1 = serde_json::from_slice(&wire).unwrap();
    assert_eq!(cold, record);
    assert_eq!(CANDIDATE_VALIDATIONS.with(|calls| calls.get()), 10);
    let mut changed = record.clone();
    // Recomputed outer digest must not let malformed detached bytes inherit the proof.
    std::sync::Arc::make_mut(&mut changed.bytes)[0] = b'[';
    changed.digest = CanonicalDigest::of_bytes(&changed.bytes);
    assert!(changed.verify().is_err());
    let mut changed = record.clone();
    changed.publication_revision =
        GatewayPublicationRevision::new(record.publication_revision.get() + 1).unwrap();
    assert!(changed.verify().is_err());
    let mut changed = record.clone();
    changed.workspace_id = WorkspaceId::parse("workspace/other").unwrap();
    assert!(changed.verify().is_err());
    let mut changed = record.clone();
    changed.digest = CanonicalDigest::of_bytes(b"wrong");
    assert!(changed.verify().is_err());
    assert_eq!(record.verify_current().unwrap(), publication);
}
