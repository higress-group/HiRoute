use crate::server::core_runtime::model_ir::{
    ContentPart, ExactProviderPathV1, ModelRequestIRV1, RequestCapabilityRequirementsV1, ToolChoice,
};
use crate::server::core_runtime::profiles::{
    CandidateContextDemand, CandidateProtocolProfile, ContextProjectionError, ContextProjector,
    Fidelity, MAX_TERMINAL_CLASSIFIED_REFUSAL_BLOCKS, MAX_TERMINAL_CLASSIFIED_REFUSAL_BYTES,
    NativeProviderStateEmission, StateAffinity, StreamingRefusalSemantics,
};
use crate::server::request_plan::IngressProtocol;

use super::{
    CostClassV1, ExclusionReasonCodeV1, PaidBudgetQuoteFactV1, PlannerCandidateFactsV1,
    RequestOwnedLimitsV1, StaticCostPolicyV1,
};

#[derive(Clone, Debug)]
pub(crate) struct EligibleProjection {
    pub reasoning_profile_id: String,
    pub context: CandidateContextDemand,
    pub effective_cost_micros: Option<u64>,
}

pub(crate) fn evaluate_candidate(
    request: &ModelRequestIRV1,
    candidate: &PlannerCandidateFactsV1,
    cost_policy: StaticCostPolicyV1,
    limits: &RequestOwnedLimitsV1,
) -> Result<EligibleProjection, ExclusionReasonCodeV1> {
    let requirements = request.requirements();
    protocol_gate(request, &requirements, &candidate.protocol_profile)?;
    if let Some(exclusion) = candidate.request_projection_exclusion {
        return Err(exclusion);
    }
    tool_projection_gate(request, &candidate.protocol_profile)?;
    vision_gate(&requirements, &candidate.protocol_profile)?;
    tool_gate(&requirements, &candidate.protocol_profile)?;
    let reasoning = reasoning_gate(&candidate.protocol_profile)?;
    let context = context_gate(candidate, reasoning)?;
    streaming_gate(&requirements, &candidate.protocol_profile)?;
    state_gate(request, &requirements, &candidate.protocol_profile)?;
    static_cost_gate(candidate, cost_policy, limits)?;
    Ok(EligibleProjection {
        reasoning_profile_id: reasoning.profile_id.clone(),
        context,
        effective_cost_micros: candidate.effective_cost_micros(),
    })
}

pub(crate) fn has_exact_reasoning_profile(candidate: &PlannerCandidateFactsV1) -> bool {
    reasoning_gate(&candidate.protocol_profile).is_ok()
}

fn protocol_gate(
    _model_request: &ModelRequestIRV1,
    requirements: &RequestCapabilityRequirementsV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    let capability = &profile.capability;
    if profile.schema_version != "hiroute.candidate-protocol-profile/v1"
        || capability.schema_version != "hiroute.candidate-capability/v1"
        || profile.path_id.trim().is_empty()
        || profile.adapter_revision.trim().is_empty()
        || profile.serializer_revision.trim().is_empty()
        || profile.decoder_revision.trim().is_empty()
        || capability.capability_id.trim().is_empty()
        || capability.capability_revision.trim().is_empty()
        || capability.model_configuration_id.trim().is_empty()
        || capability.native_model.trim().is_empty()
        || requirements.ingress_protocol != profile.ingress_protocol
        || capability.upstream_protocol != profile.connector.upstream_protocol
        || profile.exact_provider_path().is_err()
        || !profile.connector.critical_facts_are_exact()
    {
        return Err(ExclusionReasonCodeV1::ProtocolPathUnavailable);
    }
    let request = &capability.request;
    if !exact_if(requirements.text, request.text)
        || !exact_if(
            requirements.initial_instructions,
            request.initial_instructions,
        )
        || !exact_if(
            requirements.mid_conversation_instructions,
            request.mid_conversation_instructions,
        )
    {
        return Err(ExclusionReasonCodeV1::ProtocolPathUnavailable);
    }
    let response = &capability.response;
    if [
        response.text,
        response.reasoning,
        response.refusal,
        response.usage,
        response.finish_reason,
        response.typed_error,
    ]
    .into_iter()
    .any(|fidelity| fidelity != Fidelity::Exact)
    {
        return Err(ExclusionReasonCodeV1::ProtocolPathUnavailable);
    }
    Ok(())
}

fn tool_projection_gate(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    if request.ingress_protocol != IngressProtocol::Responses {
        return Ok(());
    }
    match profile.capability.upstream_protocol {
        IngressProtocol::Responses => Ok(()),
        IngressProtocol::ChatCompletions => {
            crate::server::core_runtime::adapters::ChatToolProjection::for_request(request)
                .map(|_| ())
                .map_err(|_| ExclusionReasonCodeV1::ToolInterfaceUnsupported)
        }
        IngressProtocol::Messages => {
            let has_custom = request
                .tools
                .iter()
                .chain(
                    request
                        .tool_namespaces
                        .iter()
                        .flat_map(|namespace| namespace.tools.iter()),
                )
                .any(|tool| tool.kind == crate::server::core_runtime::model_ir::ToolKindV1::Custom);
            if !request.tool_namespaces.is_empty() || has_custom {
                Err(ExclusionReasonCodeV1::ToolInterfaceUnsupported)
            } else {
                Ok(())
            }
        }
    }
}

fn vision_gate(
    requirements: &RequestCapabilityRequirementsV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    let request = &profile.capability.request;
    if !exact_if(requirements.image_url, request.image_url)
        || !exact_if(requirements.image_base64, request.image_base64)
    {
        return Err(ExclusionReasonCodeV1::VisionUnsupported);
    }
    if requirements.image_base64 {
        let Some(media_types) = request.image_base64_media_types.exact() else {
            return Err(ExclusionReasonCodeV1::ImageSourceUnsupported);
        };
        if requirements.image_media_types.iter().any(|required| {
            !media_types
                .iter()
                .any(|supported| supported.eq_ignore_ascii_case(required))
        }) {
            return Err(ExclusionReasonCodeV1::ImageSourceUnsupported);
        }
    }
    Ok(())
}

fn tool_gate(
    requirements: &RequestCapabilityRequirementsV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    let request = &profile.capability.request;
    let response = &profile.capability.response;
    let tools_active = requirements.function_tools && requirements.tool_choice != ToolChoice::None;
    if !exact_if(tools_active, request.function_tools)
        || !exact_if(
            tools_active && requirements.strict_tools,
            request.strict_tools,
        )
        || !exact_if(
            tools_active && requirements.parallel_tools,
            request.parallel_tools,
        )
        || !exact_if(tools_active, response.tool_calls)
        || !exact_if(
            tools_active && requirements.logical_tool_id_mapping,
            response.logical_tool_id_mapping,
        )
    {
        return Err(ExclusionReasonCodeV1::ToolInterfaceUnsupported);
    }
    if tools_active {
        let choice = match requirements.tool_choice {
            ToolChoice::None => request.tool_choice_none,
            ToolChoice::Auto => request.tool_choice_auto,
            ToolChoice::RequiredAny => request.tool_choice_required_any,
            ToolChoice::RequiredNamed { .. } => request.tool_choice_required_named,
        };
        if choice != Fidelity::Exact {
            return Err(ExclusionReasonCodeV1::ToolChoiceUnsupported);
        }
    }
    if !exact_if(requirements.tool_roundtrip, request.tool_roundtrip)
        || !exact_if(requirements.tool_result_text, request.tool_result_text)
        || !exact_if(requirements.tool_result_json, request.tool_result_json)
        || !exact_if(
            requirements.logical_tool_id_mapping,
            request.logical_tool_id_mapping,
        )
    {
        return Err(ExclusionReasonCodeV1::ToolRoundtripUnsupported);
    }
    Ok(())
}

fn reasoning_gate(
    profile: &CandidateProtocolProfile,
) -> Result<&crate::server::core_runtime::profiles::ReasoningProfileCapability, ExclusionReasonCodeV1>
{
    let mut ids = std::collections::BTreeSet::new();
    if profile.capability.reasoning_profiles.is_empty()
        || profile
            .capability
            .reasoning_profiles
            .iter()
            .any(|reasoning| {
                !ids.insert(reasoning.profile_id.as_str())
                    || !reasoning.validate_for(profile.capability.upstream_protocol)
            })
    {
        return Err(ExclusionReasonCodeV1::ReasoningProfileMismatch);
    }
    profile
        .selected_reasoning()
        .map_err(|_| ExclusionReasonCodeV1::ReasoningProfileMismatch)
}

fn context_gate(
    candidate: &PlannerCandidateFactsV1,
    reasoning: &crate::server::core_runtime::profiles::ReasoningProfileCapability,
) -> Result<CandidateContextDemand, ExclusionReasonCodeV1> {
    let limits = &candidate.protocol_profile.capability.context;
    match limits.max_output_tokens.exact() {
        Some(1..) => {}
        Some(0) | None => return Err(ExclusionReasonCodeV1::MaxOutputUnsupported),
    }
    if limits.max_input_tokens.exact().is_none()
        || limits.max_total_tokens.exact().is_none()
        || limits.estimator.exact().is_none()
    {
        return Err(ExclusionReasonCodeV1::ContextLimitUnknown);
    }
    ContextProjector::project_serialized_len(candidate.target_serialized_bytes, limits, reasoning)
        .map_err(|error| match error {
            ContextProjectionError::InputTooLarge { .. }
            | ContextProjectionError::TotalTooLarge { .. } => {
                ExclusionReasonCodeV1::ContextTooLarge
            }
            ContextProjectionError::UnknownLimit(_)
            | ContextProjectionError::UnknownEstimator
            | ContextProjectionError::ArithmeticOverflow => {
                ExclusionReasonCodeV1::ContextLimitUnknown
            }
        })
}

fn streaming_gate(
    requirements: &RequestCapabilityRequirementsV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    if !requirements.streaming {
        return Ok(());
    }
    if profile.capability.native_streaming.exact() != Some(&true) {
        return Err(ExclusionReasonCodeV1::StreamFeatureUnsupported);
    }
    let response = &profile.capability.response;
    if !exact_if(requirements.stream_text, response.stream_text_delta)
        || !exact_if(
            requirements.stream_tool_arguments,
            response.stream_tool_argument_delta,
        )
        || !exact_if(
            requirements.stream_reasoning,
            response.stream_reasoning_delta,
        )
        || !exact_if(requirements.stream_usage, response.stream_usage)
        || !valid_stream_refusal(
            profile.capability.upstream_protocol,
            response.stream_refusal,
        )
    {
        return Err(ExclusionReasonCodeV1::StreamFeatureUnsupported);
    }
    Ok(())
}

fn state_gate(
    request: &ModelRequestIRV1,
    requirements: &RequestCapabilityRequirementsV1,
    profile: &CandidateProtocolProfile,
) -> Result<(), ExclusionReasonCodeV1> {
    let exact_owner = profile
        .exact_provider_path()
        .map_err(|_| ExclusionReasonCodeV1::ProviderStateAffinityMismatch)?;
    let request_profile = &profile.capability.request;
    let response_profile = &profile.capability.response;
    if requirements.provider_state
        && (request_profile.provider_state != Fidelity::Exact
            || request_profile.state_affinity != StateAffinity::ExactOwner)
    {
        return Err(ExclusionReasonCodeV1::OpaqueStateUnportable);
    }
    if state_owners(request).any(|owner| owner != &exact_owner) {
        return Err(ExclusionReasonCodeV1::ProviderStateAffinityMismatch);
    }
    match profile.capability.native_provider_state {
        NativeProviderStateEmission::Never => {}
        NativeProviderStateEmission::Unknown => {
            return Err(ExclusionReasonCodeV1::OpaqueStateUnportable);
        }
        NativeProviderStateEmission::ExactOwnerAffine => {
            if request_profile.provider_state != Fidelity::Exact
                || request_profile.state_affinity != StateAffinity::ExactOwner
                || response_profile.provider_state != Fidelity::Exact
                || response_profile.state_affinity != StateAffinity::ExactOwner
            {
                return Err(ExclusionReasonCodeV1::OpaqueStateUnportable);
            }
        }
    }
    Ok(())
}

fn static_cost_gate(
    candidate: &PlannerCandidateFactsV1,
    cost_policy: StaticCostPolicyV1,
    limits: &RequestOwnedLimitsV1,
) -> Result<(), ExclusionReasonCodeV1> {
    if !candidate.statically_enabled {
        return Err(ExclusionReasonCodeV1::StaticPlanExcluded);
    }
    let class_allowed = match cost_policy {
        StaticCostPolicyV1::StrictFree => candidate.cost_class == CostClassV1::Free,
        StaticCostPolicyV1::SubscriptionAndFree => matches!(
            candidate.cost_class,
            CostClassV1::Free | CostClassV1::Subscription
        ),
        StaticCostPolicyV1::BudgetedPaid => candidate.cost_class != CostClassV1::Unknown,
        StaticCostPolicyV1::ExplicitFixed => true,
    };
    if !class_allowed {
        return Err(ExclusionReasonCodeV1::CostPolicyExcluded);
    }
    if cost_policy == StaticCostPolicyV1::BudgetedPaid && candidate.cost_class == CostClassV1::Paid
    {
        let ceiling = limits
            .paid_budget_ceiling_micros
            .ok_or(ExclusionReasonCodeV1::PaidBudgetQuoteUnavailable)?;
        let quote = match candidate.paid_budget_quote {
            PaidBudgetQuoteFactV1::Available { upper_bound_micros } => upper_bound_micros,
            PaidBudgetQuoteFactV1::NotRequired | PaidBudgetQuoteFactV1::Unavailable => {
                return Err(ExclusionReasonCodeV1::PaidBudgetQuoteUnavailable);
            }
        };
        if quote > ceiling {
            return Err(ExclusionReasonCodeV1::PaidBudgetQuoteUnavailable);
        }
    }
    Ok(())
}

fn exact_if(required: bool, fidelity: Fidelity) -> bool {
    !required || fidelity == Fidelity::Exact
}

fn valid_stream_refusal(protocol: IngressProtocol, semantics: StreamingRefusalSemantics) -> bool {
    matches!(
        (protocol, semantics),
        (
            IngressProtocol::Messages,
            StreamingRefusalSemantics::TerminalClassified {
                max_buffered_bytes: 1..=MAX_TERMINAL_CLASSIFIED_REFUSAL_BYTES,
                max_buffered_blocks: 1..=MAX_TERMINAL_CLASSIFIED_REFUSAL_BLOCKS,
            }
        ) | (
            IngressProtocol::Responses | IngressProtocol::ChatCompletions,
            StreamingRefusalSemantics::ExactDelta
        )
    )
}

fn state_owners(request: &ModelRequestIRV1) -> impl Iterator<Item = &ExactProviderPathV1> {
    request
        .provider_state
        .iter()
        .map(|state| &state.owner)
        .chain(
            request
                .instructions
                .iter()
                .flat_map(|instruction| instruction.content.iter())
                .chain(
                    request
                        .messages
                        .iter()
                        .flat_map(|message| message.content.iter()),
                )
                .filter_map(|part| match part {
                    ContentPart::ProviderState { state } => Some(&state.owner),
                    _ => None,
                }),
        )
}
