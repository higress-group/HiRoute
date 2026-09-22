use std::collections::BTreeMap;
use std::path::PathBuf;

use hiroute_gateway::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, CanonicalTool, ContentPart, ImageSource,
    InstructionRole, MODEL_REQUEST_IR_SCHEMA, MessageRole, ModelRequestIRV1,
    RequestedReasoningControl, ToolChoice, ToolKindV1, ToolOutput, ToolResultStatusV1,
};
use hiroute_gateway::server::core_runtime::profiles::planner::*;
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, CriticalFact, Fidelity, fixed_reasoning,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComplexityCorpus {
    schema_version: String,
    complex_phrases: Vec<ComplexPhrase>,
    cases: Vec<CorpusCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComplexPhrase {
    phrase_id: String,
    phrase: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusCase {
    id: String,
    equivalence_key: Option<String>,
    protocol: String,
    latest_human: Option<String>,
    system_text: Option<String>,
    assistant_history: Option<String>,
    tool_schema_text: Option<String>,
    correlation: Option<CorpusCorrelation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusCorrelation {
    kind: String,
    prior_case_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComplexityGolden {
    schema_version: String,
    strategy_digest: String,
    planner_input_digest: String,
    planner_output_digest: String,
    cases: Vec<GoldenCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCase {
    id: String,
    branch: String,
    score: u8,
    source: String,
    reasons: Vec<String>,
    matched_user_phrase_ids: Vec<String>,
}

fn e2e_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load_json<T: for<'de> Deserialize<'de>>(relative: &str) -> T {
    let bytes = std::fs::read(e2e_path(relative)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn protocol(value: &str) -> IngressProtocol {
    match value {
        "responses" => IngressProtocol::Responses,
        "chat_completions" => IngressProtocol::ChatCompletions,
        "messages" => IngressProtocol::Messages,
        other => panic!("unknown frozen protocol {other}"),
    }
}

fn corpus_request(case: &CorpusCase) -> ModelRequestIRV1 {
    let mut messages = Vec::new();
    if let Some(history) = &case.assistant_history {
        messages.push(CanonicalMessage {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Text {
                text: history.clone(),
            }],
            name: None,
        });
    }
    if let Some(human) = &case.latest_human {
        messages.push(CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: human.clone(),
            }],
            name: None,
        });
    } else if case
        .correlation
        .as_ref()
        .is_some_and(|correlation| correlation.kind == "tool_continuation")
    {
        messages.push(CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::ToolResult {
                logical_id: "tool-1".into(),
                tool_kind: ToolKindV1::Function,
                output: ToolOutput::Text("opaque output is excluded".into()),
                status: ToolResultStatusV1::Unknown,
            }],
            name: None,
        });
    }
    ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: protocol(&case.protocol),
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: "agent/golden".into(),
        stream: false,
        instructions: case
            .system_text
            .as_ref()
            .map(|text| CanonicalInstruction {
                role: InstructionRole::System,
                content: vec![ContentPart::Text { text: text.clone() }],
            })
            .into_iter()
            .collect(),
        messages,
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tools: case
            .tool_schema_text
            .as_ref()
            .map(|text| CanonicalTool {
                kind: ToolKindV1::Function,
                name: "fixture-tool".into(),
                description: Some(text.clone()),
                input_schema: Some(json!({"description": text})),
                strict: Some(true),
                format: None,
            })
            .into_iter()
            .collect(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
    }
}

fn enum_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn planner_frozen_complexity_corpus_is_exact_across_protocols() {
    let corpus: ComplexityCorpus = load_json("e2e/corpus/p0-gateway-planner.json");
    let golden: ComplexityGolden = load_json("e2e/golden/p0-gateway-planner.json");
    assert_eq!(
        corpus.schema_version,
        "hiroute.planner-complexity-corpus/v1"
    );
    assert_eq!(
        golden.schema_version,
        "hiroute.planner-complexity-golden/v1"
    );
    let strategy = ComplexityV1::compile(
        corpus
            .complex_phrases
            .into_iter()
            .map(|phrase| (phrase.phrase_id, phrase.phrase)),
    )
    .unwrap();
    assert_eq!(
        golden.strategy_digest, strategy.payload_digest,
        "frozen strategy digest changed; actual={}",
        strategy.payload_digest
    );
    let expected = golden
        .cases
        .into_iter()
        .map(|case| (case.id.clone(), case))
        .collect::<BTreeMap<_, _>>();
    let mut decisions = BTreeMap::<String, BranchDecisionV1>::new();
    let mut equivalent = BTreeMap::<String, Vec<u8>>::new();
    for case in corpus.cases {
        let correlation = case
            .correlation
            .as_ref()
            .map(|correlation| CorrelatedBranchDecisionV1 {
                kind: match correlation.kind.as_str() {
                    "task_root" => ContinuationKindV1::TaskRoot,
                    "tool_continuation" => ContinuationKindV1::ToolContinuation,
                    other => panic!("unknown frozen correlation {other}"),
                },
                decision: decisions[correlation.prior_case_id.as_str()].clone(),
            });
        let (decision, structural_facts) = ComplexityV1::decide(
            case.latest_human.as_deref(),
            correlation.as_ref(),
            &strategy,
        )
        .unwrap();
        let golden = &expected[case.id.as_str()];
        assert_eq!(enum_name(&decision.branch_id), golden.branch, "{}", case.id);
        assert_eq!(decision.complexity_score, Some(golden.score), "{}", case.id);
        assert_eq!(
            enum_name(&decision.decision_source),
            golden.source,
            "{}",
            case.id
        );
        assert_eq!(
            decision
                .reason_codes
                .iter()
                .map(enum_name)
                .collect::<Vec<_>>(),
            golden.reasons,
            "{}",
            case.id
        );
        assert_eq!(
            decision.matched_user_phrase_ids, golden.matched_user_phrase_ids,
            "{}",
            case.id
        );
        if let Some(key) = &case.equivalence_key {
            let bytes = serde_json::to_vec(&(&decision, &structural_facts)).unwrap();
            if let Some(first) = equivalent.get(key) {
                assert_eq!(
                    &bytes, first,
                    "protocol changed structural or decision bytes for {key}"
                );
            } else {
                equivalent.insert(key.clone(), bytes);
            }
        }
        decisions.insert(case.id, decision);
    }
    assert_eq!(decisions.len(), expected.len(), "corpus/golden case drift");
}

fn planner_request(text: &str) -> ModelRequestIRV1 {
    let mut request = corpus_request(&CorpusCase {
        id: "local".into(),
        equivalence_key: None,
        protocol: "responses".into(),
        latest_human: Some(text.into()),
        system_text: None,
        assistant_history: None,
        tool_schema_text: None,
        correlation: None,
    });
    request.served_model_id = "agent/planner".into();
    request
}

fn candidate(id: &str, score: i32, class: CostClassV1) -> PlannerCandidateFactsV1 {
    let mut candidate = PlannerCandidateFactsV1::seal(
        id,
        format!("binding-{id}"),
        CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            format!("native-{id}"),
            fixed_reasoning("fixed"),
        ),
        100,
        Some(score),
        class,
        Some(if class == CostClassV1::Free { 0 } else { 10 }),
    )
    .unwrap();
    if class == CostClassV1::Paid {
        candidate.paid_budget_quote = PaidBudgetQuoteFactV1::Available {
            upper_bound_micros: 10,
        };
    }
    candidate
}

fn custom_input(
    request: ModelRequestIRV1,
    candidates: Vec<PlannerCandidateFactsV1>,
    cost_policy: StaticCostPolicyV1,
) -> PlannerInputV1 {
    let ids = candidates
        .iter()
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    let policy = CompiledPlannerPolicyV1 {
        schema_version: String::new(),
        served_model_id: "agent/planner".into(),
        identity: PlannerRouteIdentityV2::Plan {
            plan_id: "planner-golden".into(),
            revision: 1,
        },
        route: MaterializedRouteV1::Custom {
            group_id: "custom".into(),
        },
        groups: vec![MaterializedModelGroupV1 {
            group_id: "custom".into(),
            policy: GroupPolicyV1::Manual,
            candidate_ids: ids,
        }],
        complexity_strategy: None,
        cost_policy,
        limits: RequestOwnedLimitsV1 {
            max_candidate_bindings: 4,
            max_attempts: 3,
            deadline_cap_ms: 60_000,
            paid_budget_ceiling_micros: Some(1_000_000),
        },
        policy_digest: String::new(),
    }
    .seal()
    .unwrap();
    PlannerInputV1 {
        schema_version: PLANNER_INPUT_SCHEMA.into(),
        request,
        correlated_branch: None,
        classification_decision: None,
        classification_facts: None,
        context_hold: None,
        policy,
        candidates,
    }
}

#[test]
fn planner_frozen_ledger_covers_capability_context_reasoning_and_bytes() {
    let mut request = planner_request("simple image request");
    request.messages[0].content.push(ContentPart::Image {
        source: ImageSource::Url {
            url: "https://image.invalid/fixture.png".into(),
        },
    });
    let mut no_vision = candidate("no-vision", 50, CostClassV1::Paid);
    no_vision.protocol_profile.capability.request.image_url = Fidelity::Unsupported;
    no_vision
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "missing".into();
    no_vision
        .protocol_profile
        .capability
        .context
        .max_input_tokens = CriticalFact::Exact(1);
    no_vision.profile_digest = no_vision.recompute_profile_digest().unwrap();
    let capable = candidate("capable", 40, CostClassV1::Paid);
    let output = Planner
        .plan(&custom_input(
            request,
            vec![no_vision, capable],
            StaticCostPolicyV1::BudgetedPaid,
        ))
        .unwrap();
    assert_eq!(
        output.ledger.evaluations[0].first_exclusion,
        Some(ExclusionReasonCodeV1::VisionUnsupported)
    );
    assert_eq!(output.ledger.ordered_candidates[0].candidate_id, "capable");

    let mut at_n = candidate("at-n", 40, CostClassV1::Paid);
    at_n.protocol_profile.capability.context.max_input_tokens = CriticalFact::Exact(100);
    at_n.protocol_profile.capability.context.max_total_tokens = CriticalFact::Exact(Some(356));
    at_n.target_serialized_bytes = 100;
    at_n.profile_digest = at_n.recompute_profile_digest().unwrap();
    let mut at_n_plus_one = at_n.clone();
    at_n_plus_one.candidate_id = "at-n-plus-one".into();
    at_n_plus_one.stable_binding_id = "binding-at-n-plus-one".into();
    at_n_plus_one.target_serialized_bytes = 101;
    let mut reasoning_n_plus_one = at_n.clone();
    reasoning_n_plus_one.candidate_id = "reasoning-n-plus-one".into();
    reasoning_n_plus_one.stable_binding_id = "binding-reasoning-n-plus-one".into();
    reasoning_n_plus_one
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "fixed-next".into();
    reasoning_n_plus_one.profile_digest = reasoning_n_plus_one.recompute_profile_digest().unwrap();
    let input = custom_input(
        planner_request("hello"),
        vec![at_n, at_n_plus_one, reasoning_n_plus_one],
        StaticCostPolicyV1::BudgetedPaid,
    );
    let expected = Planner.plan(&input).unwrap();
    let digest_golden: ComplexityGolden = load_json("e2e/golden/p0-gateway-planner.json");
    assert_eq!(
        (&expected.input_digest, &expected.output_digest),
        (
            &digest_golden.planner_input_digest,
            &digest_golden.planner_output_digest,
        ),
        "Planner input and output digests must be resealed together"
    );
    assert_eq!(
        expected.ledger.evaluations[1].first_exclusion,
        Some(ExclusionReasonCodeV1::ContextTooLarge)
    );
    assert_eq!(
        expected.ledger.evaluations[2].first_exclusion,
        Some(ExclusionReasonCodeV1::ReasoningProfileMismatch)
    );
    let expected_bytes = expected.canonical_bytes().unwrap();
    for _ in 0..64 {
        assert_eq!(
            Planner.plan(&input).unwrap().canonical_bytes().unwrap(),
            expected_bytes
        );
    }
    let mut reordered_facts = input.clone();
    reordered_facts.candidates.reverse();
    assert_eq!(
        Planner
            .plan(&reordered_facts)
            .unwrap()
            .canonical_bytes()
            .unwrap(),
        expected_bytes,
        "candidate fact transport order must not affect the frozen ledger"
    );

    let free = candidate("free", 10, CostClassV1::Free);
    let mut free_input = custom_input(
        planner_request("architecture design entire repository"),
        vec![free],
        StaticCostPolicyV1::StrictFree,
    );
    free_input.policy.route = MaterializedRouteV1::FreeFirst {
        free_group_id: "custom".into(),
        exhaustion: FreeFirstExhaustionV1::FreeOnly,
        candidate_mode: FreeCandidateModeV1::AutomaticAllAvailable,
    };
    free_input.policy = free_input.policy.seal().unwrap();
    let free_output = Planner.plan(&free_input).unwrap();
    assert!(free_output.complexity.is_none());
    assert_eq!(
        free_output.ledger.ordered_candidates[0].candidate_id,
        "free"
    );
}

#[test]
fn planner_consumes_all_frozen_protocol_paths_without_reinterpretation() {
    let protocols = [
        ("responses", IngressProtocol::Responses),
        ("chat", IngressProtocol::ChatCompletions),
        ("messages", IngressProtocol::Messages),
    ];
    let mut covered = 0_u8;
    for &(ingress_name, ingress) in &protocols {
        for &(upstream_name, upstream) in &protocols {
            let candidate_id = format!("{ingress_name}-to-{upstream_name}");
            let candidate = PlannerCandidateFactsV1::seal(
                candidate_id.clone(),
                format!("binding-{candidate_id}"),
                CandidateProtocolProfile::exact_portable_path(
                    ingress,
                    upstream,
                    format!("native-{candidate_id}"),
                    fixed_reasoning("fixed"),
                ),
                100,
                Some(40),
                CostClassV1::Subscription,
                Some(10),
            )
            .unwrap();
            let mut request = planner_request("hello");
            request.ingress_protocol = ingress;
            let output = Planner
                .plan(&custom_input(
                    request,
                    vec![candidate],
                    StaticCostPolicyV1::SubscriptionAndFree,
                ))
                .unwrap();
            assert_eq!(output.ledger.ordered_candidates.len(), 1);
            assert_eq!(
                output.ledger.ordered_candidates[0].upstream_protocol,
                upstream
            );
            covered += 1;
        }
    }
    assert_eq!(covered, 9);
}
