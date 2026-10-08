use super::*;
use hiroute_domain::{
    CanonicalDigest, JudgmentSettingsV1, MaterializedBranchV1, MaterializedGroupId,
};

pub(crate) fn fixture() -> PlannerInputV1 {
    let strategy = ComplexityV1::with_branches(
        ComplexityV1::compile_with_classifier(
            [],
            CompiledClassifierKindV1::Rest,
            Some(CanonicalDigest::of_bytes(b"service-v1").to_string()),
        )
        .unwrap(),
        vec!["code".into(), "writing".into()],
        "writing".into(),
    )
    .unwrap();
    let branches = vec![
        MaterializedBranchV1 {
            id: "code".into(),
            name: "开发".into(),
            condition: "Coding work".into(),
            group: MaterializedGroupId::Branch(0),
            primary_group: Some(MaterializedGroupId::BranchPrimary(0)),
            judgment: JudgmentSettingsV1::default(),
        },
        MaterializedBranchV1 {
            id: "writing".into(),
            name: "写作".into(),
            condition: "Writing work".into(),
            group: MaterializedGroupId::Branch(1),
            primary_group: None,
            judgment: JudgmentSettingsV1::default(),
        },
    ];
    let groups = [
        ("branch_0", vec!["economy"]),
        ("branch_primary_0", vec!["primary", "expert"]),
        ("branch_1", vec!["primary"]),
    ]
    .into_iter()
    .map(|(id, candidates)| MaterializedModelGroupV1 {
        group_id: id.into(),
        policy: GroupPolicyV1::Manual,
        candidate_ids: candidates.into_iter().map(str::to_owned).collect(),
    })
    .collect();
    let mut input = input(
        request(IngressProtocol::Responses, "Fix a bug"),
        policy(
            MaterializedRouteV1::Branches {
                branches,
                default_branch_id: "writing".into(),
                reselect_on_user_message: false,
            },
            groups,
            Some(strategy),
            StaticCostPolicyV1::SubscriptionAndFree,
        ),
        ["economy", "primary", "expert"]
            .into_iter()
            .map(|id| {
                candidate(
                    id,
                    IngressProtocol::Responses,
                    Some(40),
                    None,
                    CostClassV1::Subscription,
                )
            })
            .collect(),
    );
    let decision = input.classification_decision.as_mut().unwrap();
    decision.branch_id = "code".into();
    decision.execution_group = hiroute_domain::ExecutionGroupV1::Regular;
    decision.decision_source = ComplexityDecisionSourceV1::ExternalClassifier;
    decision.complexity_score = None;
    decision.threshold = None;
    decision.classification_duration_micros = Some(10);
    decision.fallback_used = false;
    input
}

#[test]
fn group_choice_constrains_hold_and_never_revives_a_regular_model() {
    let mut input = fixture();
    let primary = &input.candidates[1];
    input.context_hold = Some(HoldPreferenceV1 {
        stable_binding_id: primary.stable_binding_id.clone(),
        candidate_id: primary.candidate_id.clone(),
        profile_digest: primary.profile_digest.clone(),
        reasoning_profile_id: "fixed".into(),
        origin_group_id: "branch_primary_0".into(),
    });
    input.previous_success_candidate_id = Some("economy".into());
    input
        .classification_decision
        .as_mut()
        .unwrap()
        .execution_group = hiroute_domain::ExecutionGroupV1::Primary;
    let output = Planner.plan(&input).unwrap();
    assert_eq!(
        output
            .ledger
            .ordered_candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["primary", "expert"]
    );
    input
        .classification_decision
        .as_mut()
        .unwrap()
        .execution_group = hiroute_domain::ExecutionGroupV1::Regular;
    let output = Planner.plan(&input).unwrap();
    assert_eq!(
        output
            .ledger
            .ordered_candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["economy", "primary", "expert"]
    );
    assert_eq!(output.ledger.ordered_candidates[0].group_id, "branch_0");
}

#[test]
fn same_candidate_in_two_groups_keeps_selected_position_and_is_only_tried_once() {
    let mut input = fixture();
    input.policy.groups[1]
        .candidate_ids
        .insert(0, "economy".into());
    input.policy = input.policy.seal().unwrap();
    input
        .classification_decision
        .as_mut()
        .unwrap()
        .execution_group = hiroute_domain::ExecutionGroupV1::Primary;
    let primary = Planner.plan(&input).unwrap();
    assert_eq!(primary.ledger.ordered_candidates[0].candidate_id, "economy");
    assert_eq!(
        primary.ledger.ordered_candidates[0].group_id,
        "branch_primary_0"
    );
    input
        .classification_decision
        .as_mut()
        .unwrap()
        .execution_group = hiroute_domain::ExecutionGroupV1::Regular;
    let regular = Planner.plan(&input).unwrap();
    assert_eq!(
        regular
            .ledger
            .ordered_candidates
            .iter()
            .filter(|c| c.candidate_id == "economy")
            .count(),
        1
    );
    assert_eq!(regular.ledger.ordered_candidates[0].group_id, "branch_0");
}

#[test]
fn custom_branch_failure_uses_only_its_published_default() {
    let mut input = fixture();
    let (mut decision, facts) = ComplexityV1::decide(
        Some("anything"),
        None,
        input.policy.complexity_strategy.as_ref().unwrap(),
    )
    .unwrap();
    decision.classification_duration_micros = Some(10);
    decision.fallback_reason = Some(ClassifierFallbackReasonV1::Timeout);
    // The default writing category has only one execution group.
    decision.execution_group = hiroute_domain::ExecutionGroupV1::Regular;
    input.classification_decision = Some(decision);
    input.classification_facts = Some(facts);
    let output = Planner.plan(&input).unwrap();
    assert_eq!(output.complexity.unwrap().branch_id, "writing");
    assert_eq!(output.ledger.ordered_candidates[0].candidate_id, "primary");
}
