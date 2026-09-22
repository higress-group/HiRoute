use serde_json::json;

use super::*;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, CanonicalTool, ContentPart, ImageSource,
    InstructionRole, MODEL_REQUEST_IR_SCHEMA, MessageRole, ModelRequestIRV1, OpaqueProviderState,
    RequestedReasoningControl, ToolChoice, ToolKindV1, ToolOutput, ToolResultStatusV1,
};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, CriticalFact, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;

fn request(protocol: IngressProtocol, text: &str) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: protocol,
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: "agent/test".into(),
        stream: false,
        instructions: Vec::new(),
        messages: vec![CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text { text: text.into() }],
            name: None,
        }],
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
    }
}

fn strategy() -> CompiledComplexityStrategyV1 {
    ComplexityV1::compile(Vec::<(String, String)>::new()).unwrap()
}

fn decision(text: &str) -> BranchDecisionV1 {
    ComplexityV1::decide(Some(text), None, &strategy())
        .unwrap()
        .0
}

fn candidate(
    id: &str,
    ingress: IngressProtocol,
    score: Option<i32>,
    cost: Option<u64>,
    class: CostClassV1,
) -> PlannerCandidateFactsV1 {
    let mut candidate = PlannerCandidateFactsV1::seal(
        id,
        format!("binding-{id}"),
        CandidateProtocolProfile::exact_portable_path(
            ingress,
            IngressProtocol::Responses,
            format!("native-{id}"),
            fixed_reasoning("fixed"),
        ),
        100,
        score,
        class,
        cost,
    )
    .unwrap();
    if class == CostClassV1::Paid {
        candidate.paid_budget_quote = PaidBudgetQuoteFactV1::Available {
            upper_bound_micros: cost.expect("paid fixture has a pure quote"),
        };
    }
    candidate
}

fn limits() -> RequestOwnedLimitsV1 {
    RequestOwnedLimitsV1 {
        max_candidate_bindings: 6,
        max_attempts: 4,
        deadline_cap_ms: 3_600_000,
        paid_budget_ceiling_micros: Some(1_000_000),
    }
}

fn policy(
    route: MaterializedRouteV1,
    groups: Vec<MaterializedModelGroupV1>,
    complexity_strategy: Option<CompiledComplexityStrategyV1>,
    cost_policy: StaticCostPolicyV1,
) -> CompiledPlannerPolicyV1 {
    CompiledPlannerPolicyV1 {
        schema_version: String::new(),
        served_model_id: "agent/test".into(),
        identity: PlannerRouteIdentityV2::Plan {
            plan_id: "plan-test".into(),
            revision: 7,
        },
        route,
        groups,
        complexity_strategy,
        cost_policy,
        limits: limits(),
        policy_digest: String::new(),
    }
    .seal()
    .unwrap()
}

fn input(
    request: ModelRequestIRV1,
    policy: CompiledPlannerPolicyV1,
    candidates: Vec<PlannerCandidateFactsV1>,
) -> PlannerInputV1 {
    let classified = policy.complexity_strategy.as_ref().map(|strategy| {
        let latest_user = request.messages.iter().rev().find_map(|message| {
            (message.role == MessageRole::User).then(|| {
                message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        });
        ComplexityV1::decide(latest_user.as_deref(), None, strategy).unwrap()
    });
    PlannerInputV1 {
        schema_version: PLANNER_INPUT_SCHEMA.into(),
        request,
        correlated_branch: None,
        classification_decision: classified.as_ref().map(|(decision, _)| decision.clone()),
        classification_facts: classified.map(|(_, facts)| facts),
        context_hold: None,
        policy,
        candidates,
    }
}

fn custom_policy(ids: &[&str], cost_policy: StaticCostPolicyV1) -> CompiledPlannerPolicyV1 {
    policy(
        MaterializedRouteV1::Custom {
            group_id: "custom".into(),
        },
        vec![MaterializedModelGroupV1 {
            group_id: "custom".into(),
            policy: GroupPolicyV1::Manual,
            candidate_ids: ids.iter().map(|id| (*id).into()).collect(),
        }],
        None,
        cost_policy,
    )
}

#[test]
fn fixed_route_has_no_plan_identity_and_rejects_multiple_candidates() {
    let mut policy = custom_policy(&["fixed"], StaticCostPolicyV1::SubscriptionAndFree);
    policy.identity = PlannerRouteIdentityV2::Fixed {
        binding_digest: hiroute_domain::CanonicalDigest::of_bytes(b"confirmed-fixed-binding"),
    };
    policy.limits.max_candidate_bindings = 1;
    policy.cost_policy = StaticCostPolicyV1::ExplicitFixed;
    let policy = policy.seal().unwrap();
    let mut input = input(
        request(IngressProtocol::Responses, "hello"),
        policy,
        vec![candidate(
            "fixed",
            IngressProtocol::Responses,
            None,
            None,
            CostClassV1::Subscription,
        )],
    );
    let output = Planner.plan(&input).unwrap();
    assert_eq!(output.identity, input.policy.identity);
    assert_eq!(output.ledger.ordered_candidates.len(), 1);
    let encoded = serde_json::to_value(&output).unwrap();
    assert!(encoded.get("plan_id").is_none());
    assert!(encoded.get("plan_revision").is_none());
    input.policy.groups[0].candidate_ids.push("other".into());
    input.policy = input.policy.seal().unwrap();
    assert!(matches!(
        Planner.plan(&input),
        Err(PlannerError::InvalidPolicy(_))
    ));
}

fn evaluation<'a>(output: &'a PlannerOutputV1, id: &str) -> &'a CandidateEvaluationV1 {
    output
        .ledger
        .evaluations
        .iter()
        .find(|entry| entry.candidate_id == id)
        .unwrap()
}

fn refresh_profile(candidate: &mut PlannerCandidateFactsV1) {
    candidate.profile_digest = candidate.recompute_profile_digest().unwrap();
}

#[test]
fn planner_complexity_v1_matches_bilingual_and_structural_contract() {
    let readme = decision("把 README.md 标题改成 HiRoute");
    assert_eq!(
        readme.branch_id,
        hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
    );
    assert_eq!(readme.complexity_score, Some(1));
    assert_eq!(
        readme.reason_codes,
        [ComplexityReasonCodeV1::ImplementationAction]
    );

    let complex = decision("修复并发状态机，涉及 a.rs、b.rs，并补两个验收场景：1. 正常；2. 失败");
    assert_eq!(
        complex.branch_id,
        hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    assert_eq!(complex.complexity_score, Some(6));
    assert_eq!(
        complex.reason_codes,
        [
            ComplexityReasonCodeV1::DeepReasoning,
            ComplexityReasonCodeV1::MultiFileScope,
            ComplexityReasonCodeV1::MultiConstraint,
            ComplexityReasonCodeV1::ImplementationAction,
        ]
    );

    let constraints = decision("两个要求：1. 分析生产竞态根因；2. 给出迁移方案");
    assert_eq!(constraints.complexity_score, Some(3));
    assert_eq!(
        constraints.branch_id,
        hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    assert_eq!(
        constraints.reason_codes,
        [
            ComplexityReasonCodeV1::DeepReasoning,
            ComplexityReasonCodeV1::MultiConstraint,
        ]
    );

    for simple in [
        "What is the capital of France?",
        "人类为什么需要睡眠",
        "Explain this function:\n```rust\nfn performance() {}\n```",
        "解释这些引用：\n```text\na.rs b.rs\n```",
    ] {
        assert_eq!(
            decision(simple).branch_id,
            hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
        );
    }
    let bullets = decision("- architecture design\n- implement the change");
    assert_eq!(
        bullets.branch_id,
        hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    assert_eq!(bullets.complexity_score, Some(4));

    let (single_file_diff, facts) = ComplexityV1::decide(
        Some(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new",
        ),
        None,
        &strategy(),
    )
    .unwrap();
    assert_eq!(facts.distinct_file_or_module_refs, 1);
    assert_eq!(single_file_diff.complexity_score, Some(1));
    assert_eq!(
        single_file_diff.branch_id,
        hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
    );

    let (_, diff_with_requirements) = ComplexityV1::decide(
        Some(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n\nRequirements:\n- update docs\n- run checks",
        ),
        None,
        &strategy(),
    )
    .unwrap();
    assert_eq!(diff_with_requirements.numbered_requirement_count, 2);

    let (_, contact_facts) = ComplexityV1::decide(
        Some("Contact dev@example.com, inspect https://example.com/a.rs, and edit README.md"),
        None,
        &strategy(),
    )
    .unwrap();
    assert_eq!(contact_facts.distinct_file_or_module_refs, 1);
}

#[test]
fn planner_complexity_uses_only_latest_human_not_history_tools_or_request_effort() {
    let mut value = request(IngressProtocol::Responses, "谢谢");
    value.instructions = vec![CanonicalInstruction {
        role: InstructionRole::System,
        content: vec![ContentPart::Text {
            text: "architecture design security audit ".repeat(10_000),
        }],
    }];
    value.messages.insert(
        0,
        CanonicalMessage {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Text {
                text: "整个仓库并发状态机".repeat(1_000),
            }],
            name: None,
        },
    );
    value.tools = vec![CanonicalTool {
        kind: ToolKindV1::Function,
        name: "dangerous_architecture_tool".into(),
        description: Some("security audit".repeat(10_000)),
        input_schema: Some(json!({"description": "migration plan".repeat(10_000)})),
        strict: Some(true),
        format: None,
    }];
    value.tool_choice = ToolChoice::None;
    value.requested_reasoning = RequestedReasoningControl::overridden(json!("high"));
    let decided = ComplexityV1::decide(Some("谢谢"), None, &strategy())
        .unwrap()
        .0;
    assert_eq!(
        decided.branch_id,
        hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
    );
    assert_eq!(decided.complexity_score, Some(0));
    assert!(decided.reason_codes.is_empty());
}

#[test]
fn planner_user_phrase_and_continuations_have_fixed_precedence() {
    let configured = ComplexityV1::compile(vec![
        (
            "phrase-local-7".into(),
            "ＳＥＣＵＲＩＴＹ   ＡＵＤＩＴ".into(),
        ),
        ("phrase-case-fold".into(), "Maße".into()),
    ])
    .unwrap();
    let custom = ComplexityV1::decide(Some("Please SECURITY audit this"), None, &configured)
        .unwrap()
        .0;
    assert_eq!(
        custom.decision_source,
        ComplexityDecisionSourceV1::UserPhrase
    );
    assert_eq!(custom.matched_user_phrase_ids, ["phrase-local-7"]);

    let full_case_fold = ComplexityV1::decide(Some("Please review MASSE"), None, &configured)
        .unwrap()
        .0;
    assert_eq!(full_case_fold.matched_user_phrase_ids, ["phrase-case-fold"]);

    let token_boundary =
        ComplexityV1::compile(vec![("phrase-token-boundary".into(), "alpha beta".into())]).unwrap();
    let embedded_token = ComplexityV1::decide(Some("alpha_beta"), None, &token_boundary)
        .unwrap()
        .0;
    assert!(embedded_token.matched_user_phrase_ids.is_empty());
    assert_eq!(
        embedded_token.branch_id,
        hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
    );

    let original = decision("把 README.md 标题改成 HiRoute");
    let mut tool_request = request(IngressProtocol::Responses, "unused");
    tool_request.messages = vec![CanonicalMessage {
        role: MessageRole::User,
        content: vec![ContentPart::ToolResult {
            logical_id: "call-1".into(),
            tool_kind: ToolKindV1::Function,
            output: ToolOutput::Text("opaque Tool output".into()),
            status: ToolResultStatusV1::Unknown,
        }],
        name: None,
    }];
    let inherited = ComplexityV1::decide(
        None,
        Some(&CorrelatedBranchDecisionV1 {
            kind: ContinuationKindV1::ToolContinuation,
            decision: original.clone(),
        }),
        &strategy(),
    )
    .unwrap()
    .0;
    assert_eq!(inherited.branch_id, original.branch_id);
    assert_eq!(inherited.complexity_score, original.complexity_score);
    assert_eq!(
        inherited.reason_codes[0],
        ComplexityReasonCodeV1::InheritedToolContinuation
    );

    let root = ComplexityV1::decide(
        Some("now implement the additional validation"),
        Some(&CorrelatedBranchDecisionV1 {
            kind: ContinuationKindV1::TaskRoot,
            decision: original.clone(),
        }),
        &strategy(),
    )
    .unwrap()
    .0;
    assert_eq!(root.decision_source, ComplexityDecisionSourceV1::Inherited);
    assert_eq!(
        root.reason_codes[0],
        ComplexityReasonCodeV1::InheritedTaskRoot
    );

    let assert_unresolved = |decision: BranchDecisionV1| {
        assert_eq!(
            decision.branch_id,
            hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
        );
        assert_eq!(decision.complexity_score, Some(COMPLEXITY_THRESHOLD));
        assert_eq!(
            decision.decision_source,
            ComplexityDecisionSourceV1::Unresolved
        );
        assert_eq!(
            decision.reason_codes,
            [ComplexityReasonCodeV1::TaskContextUnresolved]
        );
        assert!(decision.fallback_used);
    };
    for text in ["continue", "继续！"] {
        assert_unresolved(
            ComplexityV1::decide(Some(text), None, &strategy())
                .unwrap()
                .0,
        );
    }
    assert_unresolved(
        ComplexityV1::decide(
            Some("继续"),
            Some(&CorrelatedBranchDecisionV1 {
                kind: ContinuationKindV1::ToolContinuation,
                decision: original.clone(),
            }),
            &strategy(),
        )
        .unwrap()
        .0,
    );
    let mut incompatible = original;
    incompatible.payload_digest = "sha256:incompatible-task-root".into();
    assert_unresolved(
        ComplexityV1::decide(
            Some("continue"),
            Some(&CorrelatedBranchDecisionV1 {
                kind: ContinuationKindV1::TaskRoot,
                decision: incompatible,
            }),
            &strategy(),
        )
        .unwrap()
        .0,
    );
    let mut incompatible_tool = decision("把 README.md 标题改成 HiRoute");
    incompatible_tool.payload_digest = "sha256:incompatible-tool-continuation".into();
    assert_unresolved(
        ComplexityV1::decide(
            None,
            Some(&CorrelatedBranchDecisionV1 {
                kind: ContinuationKindV1::ToolContinuation,
                decision: incompatible_tool,
            }),
            &strategy(),
        )
        .unwrap()
        .0,
    );

    tool_request.messages.clear();
    let unresolved = ComplexityV1::decide(None, None, &strategy()).unwrap().0;
    assert_eq!(
        unresolved.branch_id,
        hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    assert_eq!(
        unresolved.reason_codes,
        [ComplexityReasonCodeV1::TaskContextUnresolved]
    );
}

#[test]
fn planner_smart_saving_ranks_only_inside_materialized_groups() {
    let candidates = vec![
        candidate(
            "anchor",
            IngressProtocol::Responses,
            Some(45),
            Some(100),
            CostClassV1::Paid,
        ),
        candidate(
            "bargain",
            IngressProtocol::Responses,
            Some(40),
            Some(1),
            CostClassV1::Subscription,
        ),
        candidate(
            "weak",
            IngressProtocol::Responses,
            Some(30),
            Some(0),
            CostClassV1::Free,
        ),
        candidate(
            "primary",
            IngressProtocol::Responses,
            Some(50),
            Some(200),
            CostClassV1::Paid,
        ),
    ];
    let smart = policy(
        MaterializedRouteV1::SmartSaving {
            simple_group_id: "economy".into(),
            simple_fallback_group_ids: vec!["primary".into()],
            complex_group_id: "primary".into(),
        },
        vec![
            MaterializedModelGroupV1 {
                group_id: "economy".into(),
                policy: GroupPolicyV1::CheapestWithRatingGuard {
                    quality_anchor_ref: "anchor".into(),
                    max_gap_tenths: 10,
                },
                candidate_ids: vec!["anchor".into(), "bargain".into(), "weak".into()],
            },
            MaterializedModelGroupV1 {
                group_id: "primary".into(),
                policy: GroupPolicyV1::QualityFirst,
                candidate_ids: vec!["primary".into(), "anchor".into()],
            },
        ],
        Some(strategy()),
        StaticCostPolicyV1::BudgetedPaid,
    );
    let simple = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "把 README.md 标题改成 HiRoute"),
            smart.clone(),
            candidates.clone(),
        ))
        .unwrap();
    assert_eq!(simple.branch, PlannedBranchV1::SmartSavingSimple);
    assert_eq!(
        simple
            .ledger
            .ordered_candidates
            .iter()
            .map(|entry| entry.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["bargain", "anchor", "primary"]
    );
    assert_eq!(
        evaluation(&simple, "weak").first_exclusion,
        Some(ExclusionReasonCodeV1::RatingGuardExcluded)
    );

    let mut missing_anchor_candidates = candidates.clone();
    missing_anchor_candidates
        .iter_mut()
        .find(|candidate| candidate.candidate_id == "anchor")
        .unwrap()
        .overall_score_tenths = None;
    let missing_anchor = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "把 README.md 标题改成 HiRoute"),
            smart.clone(),
            missing_anchor_candidates,
        ))
        .unwrap();
    assert_eq!(
        evaluation(&missing_anchor, "anchor").first_exclusion,
        Some(ExclusionReasonCodeV1::QualityAnchorUnavailable)
    );
    assert_eq!(
        missing_anchor
            .ledger
            .ordered_candidates
            .iter()
            .map(|entry| entry.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["primary"]
    );

    let mut unresolved_anchor_effort = candidates.clone();
    let unresolved_anchor = unresolved_anchor_effort
        .iter_mut()
        .find(|candidate| candidate.candidate_id == "anchor")
        .unwrap();
    unresolved_anchor
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "missing".into();
    refresh_profile(unresolved_anchor);
    let missing_effort = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "把 README.md 标题改成 HiRoute"),
            smart.clone(),
            unresolved_anchor_effort,
        ))
        .unwrap();
    assert_eq!(
        evaluation(&missing_effort, "bargain").first_exclusion,
        Some(ExclusionReasonCodeV1::QualityAnchorUnavailable)
    );
    assert_eq!(
        missing_effort
            .ledger
            .ordered_candidates
            .iter()
            .map(|entry| entry.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["primary"]
    );

    let complex = Planner
        .plan(&input(
            request(
                IngressProtocol::Responses,
                "Implement an architecture design for this component",
            ),
            smart,
            candidates,
        ))
        .unwrap();
    assert_eq!(complex.branch, PlannedBranchV1::SmartSavingComplex);
    assert_eq!(
        complex
            .ledger
            .ordered_candidates
            .iter()
            .map(|entry| entry.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["primary", "anchor"]
    );
}

#[test]
fn context_hold_applies_only_inside_the_current_branch() {
    let simple = candidate(
        "simple",
        IngressProtocol::Responses,
        Some(40),
        Some(10),
        CostClassV1::Paid,
    );
    let complex = candidate(
        "complex",
        IngressProtocol::Responses,
        Some(50),
        Some(20),
        CostClassV1::Paid,
    );
    let smart = policy(
        MaterializedRouteV1::SmartSaving {
            simple_group_id: "simple-group".into(),
            simple_fallback_group_ids: Vec::new(),
            complex_group_id: "complex-group".into(),
        },
        vec![
            MaterializedModelGroupV1 {
                group_id: "simple-group".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: vec!["simple".into()],
            },
            MaterializedModelGroupV1 {
                group_id: "complex-group".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: vec!["complex".into()],
            },
        ],
        Some(strategy()),
        StaticCostPolicyV1::BudgetedPaid,
    );
    let mut held = input(
        request(IngressProtocol::Responses, "把 README.md 标题改成 HiRoute"),
        smart,
        vec![simple.clone(), complex.clone()],
    );
    held.context_hold = Some(HoldPreferenceV1 {
        stable_binding_id: complex.stable_binding_id.clone(),
        candidate_id: complex.candidate_id.clone(),
        profile_digest: complex.profile_digest.clone(),
        reasoning_profile_id: "fixed".into(),
        origin_group_id: "complex-group".into(),
    });
    let output = Planner.plan(&held).unwrap();
    assert_eq!(output.branch, PlannedBranchV1::SmartSavingSimple);
    assert_eq!(
        output
            .ledger
            .ordered_candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["simple"]
    );
    assert_eq!(output.groups.len(), 1);
    assert!(output.reason_ledger.iter().any(|reason| {
        reason.code == LedgerReasonCodeV1::ContextHoldInvalidated
            && reason.group_id.as_deref() == Some("complex-group")
    }));

    let cross_branch_digest = output.input_digest;
    held.context_hold = Some(HoldPreferenceV1 {
        stable_binding_id: simple.stable_binding_id.clone(),
        candidate_id: simple.candidate_id.clone(),
        profile_digest: simple.profile_digest.clone(),
        reasoning_profile_id: "fixed".into(),
        origin_group_id: "simple-group".into(),
    });
    let same_branch = Planner.plan(&held).unwrap();
    assert_eq!(
        same_branch.ledger.ordered_candidates[0].candidate_id,
        "simple"
    );
    assert_ne!(same_branch.input_digest, cross_branch_digest);
    assert!(same_branch.reason_ledger.iter().any(|reason| {
        reason.code == LedgerReasonCodeV1::ContextHoldApplied
            && reason.group_id.as_deref() == Some("simple-group")
    }));

    let held_digest = same_branch.input_digest;
    held.context_hold.as_mut().unwrap().reasoning_profile_id = "wrong".into();
    let invalid = Planner.plan(&held).unwrap();
    assert_eq!(invalid.ledger.ordered_candidates[0].candidate_id, "simple");
    assert_ne!(invalid.input_digest, held_digest);
    assert!(
        invalid
            .reason_ledger
            .iter()
            .any(|reason| reason.code == LedgerReasonCodeV1::ContextHoldInvalidated)
    );
}

#[test]
fn external_classification_drives_existing_smart_groups_without_rule_score() {
    let strategy = ComplexityV1::compile_with_classifier(
        Vec::<(String, String)>::new(),
        CompiledClassifierKindV1::Rest,
        Some(
            hiroute_domain::CanonicalDigest::of_bytes(b"rest-classifier")
                .as_str()
                .to_owned(),
        ),
    )
    .unwrap();
    let smart = policy(
        MaterializedRouteV1::SmartSaving {
            simple_group_id: "simple-group".into(),
            simple_fallback_group_ids: Vec::new(),
            complex_group_id: "complex-group".into(),
        },
        vec![
            MaterializedModelGroupV1 {
                group_id: "simple-group".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: vec!["simple".into()],
            },
            MaterializedModelGroupV1 {
                group_id: "complex-group".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: vec!["complex".into()],
            },
        ],
        Some(strategy.clone()),
        StaticCostPolicyV1::BudgetedPaid,
    );
    let mut planner_input = input(
        request(IngressProtocol::Responses, "rename README"),
        smart,
        vec![
            candidate(
                "simple",
                IngressProtocol::Responses,
                Some(40),
                Some(10),
                CostClassV1::Subscription,
            ),
            candidate(
                "complex",
                IngressProtocol::Responses,
                Some(50),
                Some(20),
                CostClassV1::Subscription,
            ),
        ],
    );
    planner_input.classification_decision = Some(BranchDecisionV1 {
        strategy_id: strategy.strategy_id.clone(),
        schema_version: strategy.schema_version.clone(),
        payload_digest: strategy.payload_digest.clone(),
        branch_id: hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID.into(),
        complexity_score: None,
        threshold: None,
        decision_source: ComplexityDecisionSourceV1::ExternalClassifier,
        reason_codes: Vec::new(),
        matched_user_phrase_ids: Vec::new(),
        fallback_used: false,
        classification_duration_micros: Some(125),
        fallback_reason: None,
    });

    let output = Planner.plan(&planner_input).unwrap();
    assert_eq!(output.branch, PlannedBranchV1::SmartSavingComplex);
    assert_eq!(output.ledger.ordered_candidates[0].candidate_id, "complex");
    let decision = output.complexity.unwrap();
    assert_eq!(
        decision.decision_source,
        ComplexityDecisionSourceV1::ExternalClassifier
    );
    assert_eq!(decision.complexity_score, None);
    let observed: hiroute_domain::BranchDecisionV1 =
        serde_json::from_value(serde_json::to_value(&decision).unwrap()).unwrap();
    assert_eq!(
        observed.decision_source,
        hiroute_domain::ComplexityDecisionSourceV1::ExternalClassifier
    );
    assert_eq!(observed.complexity_score, None);
}

#[test]
fn external_decision_can_be_inherited_without_inventing_a_rule_threshold() {
    let strategy = ComplexityV1::compile_with_classifier(
        Vec::<(String, String)>::new(),
        CompiledClassifierKindV1::Rest,
        Some(
            hiroute_domain::CanonicalDigest::of_bytes(b"rest-classifier")
                .as_str()
                .to_owned(),
        ),
    )
    .unwrap();
    let previous = BranchDecisionV1 {
        strategy_id: strategy.strategy_id.clone(),
        schema_version: strategy.schema_version.clone(),
        payload_digest: strategy.payload_digest.clone(),
        branch_id: hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID.into(),
        complexity_score: None,
        threshold: None,
        decision_source: ComplexityDecisionSourceV1::ExternalClassifier,
        reason_codes: Vec::new(),
        matched_user_phrase_ids: Vec::new(),
        fallback_used: false,
        classification_duration_micros: Some(125),
        fallback_reason: None,
    };
    let (inherited, _) = ComplexityV1::decide(
        Some("continue"),
        Some(&CorrelatedBranchDecisionV1 {
            kind: ContinuationKindV1::TaskRoot,
            decision: previous,
        }),
        &strategy,
    )
    .unwrap();

    assert_eq!(
        inherited.decision_source,
        ComplexityDecisionSourceV1::Inherited
    );
    assert_eq!(
        inherited.branch_id,
        hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    assert_eq!(inherited.complexity_score, None);
    assert_eq!(inherited.threshold, None);
}

#[test]
fn planner_free_first_never_classifies_and_custom_preserves_exact_order() {
    let free = candidate(
        "free",
        IngressProtocol::Responses,
        Some(20),
        Some(0),
        CostClassV1::Free,
    );
    let paid = candidate(
        "paid",
        IngressProtocol::Responses,
        Some(50),
        Some(1),
        CostClassV1::Paid,
    );
    let free_policy = policy(
        MaterializedRouteV1::FreeFirst {
            free_group_id: "free".into(),
            exhaustion: FreeFirstExhaustionV1::FreeOnly,
            candidate_mode: FreeCandidateModeV1::AutomaticAllAvailable,
        },
        vec![MaterializedModelGroupV1 {
            group_id: "free".into(),
            policy: GroupPolicyV1::Manual,
            candidate_ids: vec!["free".into()],
        }],
        None,
        StaticCostPolicyV1::StrictFree,
    );
    let free_output = Planner
        .plan(&input(
            request(
                IngressProtocol::Responses,
                "security audit entire repository",
            ),
            free_policy,
            vec![free.clone()],
        ))
        .unwrap();
    assert_eq!(free_output.branch, PlannedBranchV1::FreeFirstFreeOnly);
    assert!(free_output.complexity.is_none());
    assert_eq!(
        free_output.ledger.ordered_candidates[0].candidate_id,
        "free"
    );

    let primary_fallback = policy(
        MaterializedRouteV1::FreeFirst {
            free_group_id: "free".into(),
            exhaustion: FreeFirstExhaustionV1::PrimaryFallback {
                primary_group_id: "primary".into(),
            },
            candidate_mode: FreeCandidateModeV1::Manual,
        },
        vec![
            MaterializedModelGroupV1 {
                group_id: "free".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: vec!["free".into()],
            },
            MaterializedModelGroupV1 {
                group_id: "primary".into(),
                policy: GroupPolicyV1::QualityFirst,
                candidate_ids: vec!["paid".into()],
            },
        ],
        None,
        StaticCostPolicyV1::BudgetedPaid,
    );
    let primary_fallback_output = Planner
        .plan(&input(
            request(
                IngressProtocol::Responses,
                "security audit entire repository",
            ),
            primary_fallback,
            vec![free.clone(), paid.clone()],
        ))
        .unwrap();
    assert_eq!(
        primary_fallback_output.branch,
        PlannedBranchV1::FreeFirstPrimaryFallback
    );
    assert!(primary_fallback_output.complexity.is_none());
    assert_eq!(
        primary_fallback_output
            .ledger
            .ordered_candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["free", "paid"]
    );

    let invalid_free = policy(
        MaterializedRouteV1::FreeFirst {
            free_group_id: "free".into(),
            exhaustion: FreeFirstExhaustionV1::FreeOnly,
            candidate_mode: FreeCandidateModeV1::Manual,
        },
        vec![MaterializedModelGroupV1 {
            group_id: "free".into(),
            policy: GroupPolicyV1::Manual,
            candidate_ids: vec!["paid".into()],
        }],
        None,
        StaticCostPolicyV1::StrictFree,
    );
    assert!(matches!(
        Planner.plan(&input(
            request(IngressProtocol::Responses, "hello"),
            invalid_free,
            vec![paid.clone()],
        )),
        Err(PlannerError::InvalidPolicy(_))
    ));

    let mut over_budget = paid.clone();
    over_budget.paid_budget_quote = PaidBudgetQuoteFactV1::Available {
        upper_bound_micros: 1_000_001,
    };
    let budget_output = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            custom_policy(&["paid"], StaticCostPolicyV1::BudgetedPaid),
            vec![over_budget],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&budget_output, "paid").first_exclusion,
        Some(ExclusionReasonCodeV1::PaidBudgetQuoteUnavailable)
    );

    let mut routing_price_unknown = paid.clone();
    routing_price_unknown.api_equivalent_cost_micros = None;
    let unknown_price_output = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            custom_policy(&["paid"], StaticCostPolicyV1::BudgetedPaid),
            vec![routing_price_unknown],
        ))
        .unwrap();
    assert!(evaluation(&unknown_price_output, "paid").eligible);
    assert_eq!(
        evaluation(&unknown_price_output, "paid").effective_cost_micros,
        None
    );

    let mut cheap = free;
    cheap.candidate_id = "cheap".into();
    cheap.stable_binding_id = "binding-cheap".into();
    let custom = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            custom_policy(&["paid", "cheap"], StaticCostPolicyV1::BudgetedPaid),
            vec![paid, cheap],
        ))
        .unwrap();
    assert!(custom.complexity.is_none());
    assert_eq!(
        custom
            .ledger
            .ordered_candidates
            .iter()
            .map(|entry| entry.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ["paid", "cheap"]
    );
}

#[test]
fn planner_freezes_candidates_beyond_the_runtime_binding_cap() {
    let ids = ["one", "two", "three", "four", "five"];
    let candidates = ids
        .iter()
        .map(|id| {
            candidate(
                id,
                IngressProtocol::Responses,
                Some(10),
                Some(0),
                CostClassV1::Free,
            )
        })
        .collect();
    let mut policy = custom_policy(&ids, StaticCostPolicyV1::SubscriptionAndFree);
    policy.limits.max_candidate_bindings = 4;
    let policy = policy.seal().unwrap();

    let output = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            policy,
            candidates,
        ))
        .unwrap();

    assert_eq!(output.outcome, PlannerOutcomeV1::Ready);
    assert_eq!(output.limits.max_candidate_bindings, 4);
    assert_eq!(
        output
            .ledger
            .ordered_candidates
            .iter()
            .map(|candidate| candidate.candidate_id.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(output.ledger.evaluations.len(), ids.len());
}

#[test]
fn planner_context_and_reasoning_n_n_plus_one_are_exact() {
    let mut at_n = candidate(
        "at-n",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Paid,
    );
    at_n.protocol_profile.capability.context.max_input_tokens = CriticalFact::Exact(100);
    at_n.protocol_profile.capability.context.max_total_tokens = CriticalFact::Exact(Some(356));
    at_n.target_serialized_bytes = 100;
    refresh_profile(&mut at_n);
    let mut at_n_plus_one = at_n.clone();
    at_n_plus_one.candidate_id = "at-n-plus-one".into();
    at_n_plus_one.stable_binding_id = "binding-at-n-plus-one".into();
    at_n_plus_one.target_serialized_bytes = 101;
    let mut wrong_reasoning = at_n.clone();
    wrong_reasoning.candidate_id = "wrong-reasoning".into();
    wrong_reasoning.stable_binding_id = "binding-wrong-reasoning".into();
    wrong_reasoning
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "fixed-next".into();
    refresh_profile(&mut wrong_reasoning);

    let output = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            custom_policy(
                &["at-n", "at-n-plus-one", "wrong-reasoning"],
                StaticCostPolicyV1::BudgetedPaid,
            ),
            vec![at_n, at_n_plus_one, wrong_reasoning],
        ))
        .unwrap();
    assert!(evaluation(&output, "at-n").eligible);
    assert_eq!(
        evaluation(&output, "at-n-plus-one").first_exclusion,
        Some(ExclusionReasonCodeV1::ContextTooLarge)
    );
    assert_eq!(
        evaluation(&output, "wrong-reasoning").first_exclusion,
        Some(ExclusionReasonCodeV1::ReasoningProfileMismatch)
    );
}

#[test]
fn planner_profile_policy_digests_and_output_bytes_are_frozen() {
    let candidate = candidate(
        "stable",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Paid,
    );
    let input = input(
        request(IngressProtocol::Responses, "hello"),
        custom_policy(&["stable"], StaticCostPolicyV1::BudgetedPaid),
        vec![candidate],
    );
    let expected = Planner.plan(&input).unwrap();
    let expected_bytes = expected.canonical_bytes().unwrap();
    assert_eq!(expected.recompute_digest().unwrap(), expected.output_digest);
    for _ in 0..32 {
        let actual = Planner.plan(&input).unwrap();
        assert_eq!(actual.canonical_bytes().unwrap(), expected_bytes);
        assert_eq!(actual.output_digest, expected.output_digest);
    }

    let mut policy_tampered = input.clone();
    policy_tampered.policy.limits.max_attempts += 1;
    assert!(matches!(
        Planner.plan(&policy_tampered),
        Err(PlannerError::PolicyDigestMismatch)
    ));
    let mut profile_tampered = input;
    profile_tampered.candidates[0]
        .protocol_profile
        .capability
        .native_model = "tampered".into();
    assert!(matches!(
        Planner.plan(&profile_tampered),
        Err(PlannerError::ProfileDigestMismatch(candidate)) if candidate == "stable"
    ));
}

#[test]
fn planner_opaque_state_affinity_and_cost_are_after_streaming() {
    let mut state_request = request(IngressProtocol::Responses, "continue");
    state_request.stream = true;
    let mut facts = candidate(
        "stateful",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Unknown,
    );
    let other_owner = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "other",
        fixed_reasoning("fixed"),
    )
    .exact_provider_path()
    .unwrap();
    state_request.provider_state = vec![OpaqueProviderState {
        owner: other_owner,
        block_index: None,
        kind: "previous_response_id".into(),
        value: json!("response-1"),
    }];
    facts.protocol_profile.capability.native_streaming = CriticalFact::Unknown;
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            state_request.clone(),
            custom_policy(&["stateful"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "stateful").first_exclusion,
        Some(ExclusionReasonCodeV1::StreamFeatureUnsupported)
    );

    facts.protocol_profile.capability.native_streaming = CriticalFact::Exact(true);
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            state_request,
            custom_policy(&["stateful"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "stateful").first_exclusion,
        Some(ExclusionReasonCodeV1::OpaqueStateUnportable)
    );

    let output = Planner
        .plan(&input(
            request(IngressProtocol::Responses, "hello"),
            custom_policy(&["stateful"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "stateful").first_exclusion,
        Some(ExclusionReasonCodeV1::CostPolicyExcluded)
    );
}

#[path = "namespace_tests.rs"]
mod namespace_tests;
