use serde_json::json;

use super::*;
use crate::server::core_runtime::model_ir::{
    CanonicalTool, CanonicalToolNamespaceV1, ResponsesToolOrderEntryV1, ToolChoice, ToolKindV1,
};
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, Fidelity, fixed_reasoning};
use crate::server::request_plan::IngressProtocol;

#[test]
fn planner_hard_gate_order_is_stable_and_fail_closed() {
    let mut rich_request = request(IngressProtocol::Responses, "ordinary request");
    rich_request.messages[0].content.push(ContentPart::Image {
        source: ImageSource::Url {
            url: "https://image.invalid/example.png".into(),
        },
    });
    rich_request.tools = vec![CanonicalTool {
        kind: ToolKindV1::Function,
        name: "lookup".into(),
        description: None,
        input_schema: Some(json!({"type":"object"})),
        strict: None,
        format: None,
    }];
    rich_request.tool_choice = ToolChoice::Auto;
    rich_request.stream = true;

    let mut facts = candidate(
        "candidate",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Paid,
    );
    facts.protocol_profile.capability.request.image_url = Fidelity::Unsupported;
    facts.protocol_profile.capability.request.function_tools = Fidelity::Unsupported;
    facts
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "missing".into();
    facts.protocol_profile.capability.context.max_input_tokens = CriticalFact::Exact(1);
    facts.protocol_profile.capability.native_streaming = CriticalFact::Unknown;
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            rich_request.clone(),
            custom_policy(&["candidate"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "candidate").first_exclusion,
        Some(ExclusionReasonCodeV1::VisionUnsupported)
    );

    facts.protocol_profile.capability.request.image_url = Fidelity::Exact;
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            rich_request.clone(),
            custom_policy(&["candidate"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "candidate").first_exclusion,
        Some(ExclusionReasonCodeV1::ToolInterfaceUnsupported)
    );

    facts.protocol_profile.capability.request.function_tools = Fidelity::Exact;
    facts.protocol_profile.capability.response.tool_calls = Fidelity::Exact;
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            rich_request.clone(),
            custom_policy(&["candidate"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "candidate").first_exclusion,
        Some(ExclusionReasonCodeV1::ReasoningProfileMismatch)
    );

    facts
        .protocol_profile
        .capability
        .selected_reasoning_profile_id = "fixed".into();
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            rich_request.clone(),
            custom_policy(&["candidate"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts.clone()],
        ))
        .unwrap();
    assert_eq!(
        evaluation(&output, "candidate").first_exclusion,
        Some(ExclusionReasonCodeV1::StreamFeatureUnsupported)
    );

    facts.protocol_profile.capability.native_streaming = CriticalFact::Exact(true);
    refresh_profile(&mut facts);
    let output = Planner
        .plan(&input(
            rich_request,
            custom_policy(&["candidate"], StaticCostPolicyV1::BudgetedPaid),
            vec![facts],
        ))
        .unwrap();
    assert!(evaluation(&output, "candidate").eligible);
    assert!(
        evaluation(&output, "candidate")
            .context
            .as_ref()
            .unwrap()
            .target_serialized_input_upper_bound
            > 1
    );
}

#[test]
fn planner_tool_schema_with_none_is_not_an_active_tool_requirement() {
    let mut disabled_tool_request = request(IngressProtocol::Responses, "hello");
    disabled_tool_request.tools = vec![CanonicalTool {
        kind: ToolKindV1::Function,
        name: "disabled".into(),
        description: None,
        input_schema: Some(json!({"type":"object"})),
        strict: Some(true),
        format: None,
    }];
    disabled_tool_request.tool_choice = ToolChoice::None;
    let mut no_tool = candidate(
        "no-tool",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Paid,
    );
    no_tool.protocol_profile.capability.request.function_tools = Fidelity::Unsupported;
    no_tool.protocol_profile.capability.request.strict_tools = Fidelity::Unsupported;
    no_tool.protocol_profile.capability.request.tool_choice_none = Fidelity::Unsupported;
    no_tool.protocol_profile.capability.response.tool_calls = Fidelity::Unsupported;
    refresh_profile(&mut no_tool);
    let output = Planner
        .plan(&input(
            disabled_tool_request,
            custom_policy(&["no-tool"], StaticCostPolicyV1::BudgetedPaid),
            vec![no_tool],
        ))
        .unwrap();
    assert!(evaluation(&output, "no-tool").eligible);
}

#[test]
fn planner_freezes_namespace_only_tools_with_ordinary_tool_roundtrip_gates() {
    let mut namespace_request = request(IngressProtocol::Responses, "hello");
    namespace_request.tool_namespaces = vec![CanonicalToolNamespaceV1 {
        name: "group".into(),
        description: None,
        tools: vec![CanonicalTool {
            kind: ToolKindV1::Function,
            name: "child".into(),
            description: None,
            input_schema: Some(json!({"type":"object"})),
            strict: Some(true),
            format: None,
        }],
    }];
    namespace_request.responses_tool_order =
        vec![ResponsesToolOrderEntryV1::Namespace { index: 0 }];
    namespace_request.tool_choice = ToolChoice::Auto;
    namespace_request.stream = true;
    let requirements = namespace_request.requirements();
    assert!(requirements.function_tools);
    assert!(requirements.strict_tools);
    assert!(requirements.tool_roundtrip);
    assert!(requirements.logical_tool_id_mapping);
    assert!(requirements.stream_tool_arguments);

    let same_protocol = candidate(
        "responses",
        IngressProtocol::Responses,
        Some(40),
        Some(1),
        CostClassV1::Paid,
    );
    let mut no_tool = same_protocol.clone();
    no_tool.candidate_id = "no-tool".into();
    no_tool.stable_binding_id = "binding-no-tool".into();
    no_tool.protocol_profile.capability.request.function_tools = Fidelity::Unsupported;
    refresh_profile(&mut no_tool);
    let cross_profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "native-chat",
        fixed_reasoning("fixed"),
    );
    let mut cross = PlannerCandidateFactsV1::seal(
        "chat",
        "binding-chat",
        cross_profile,
        100,
        Some(40),
        CostClassV1::Paid,
        Some(1),
    )
    .unwrap();
    cross.paid_budget_quote = PaidBudgetQuoteFactV1::Available {
        upper_bound_micros: 1,
    };
    let output = Planner
        .plan(&input(
            namespace_request,
            custom_policy(
                &["responses", "no-tool", "chat"],
                StaticCostPolicyV1::BudgetedPaid,
            ),
            vec![same_protocol, no_tool, cross],
        ))
        .unwrap();
    assert!(evaluation(&output, "responses").eligible);
    assert_eq!(
        evaluation(&output, "no-tool").first_exclusion,
        Some(ExclusionReasonCodeV1::ToolInterfaceUnsupported)
    );
    assert!(
        evaluation(&output, "chat").eligible,
        "{:#?}",
        evaluation(&output, "chat")
    );
}
