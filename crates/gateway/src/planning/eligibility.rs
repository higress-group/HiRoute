use crate::server::core_runtime::model_ir::{ModelRequestIRV1, RequestCapabilityRequirementsV1};
use crate::server::core_runtime::profiles::{
    CandidateContextDemand, CandidateProtocolProfile, ContextProjectionError, ContextProjector,
};
use crate::server::request_plan::IngressProtocol;

use super::{
    CostClassV1, ExclusionReasonCodeV1, PaidBudgetQuoteFactV1, PlannerCandidateFactsV1,
    RequestOwnedLimitsV1, StaticCostPolicyV1,
};

#[derive(Clone, Debug)]
pub(crate) struct EligibleProjection {
    pub reasoning_profile_id: String,
    pub context: Option<CandidateContextDemand>,
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
    let reasoning = reasoning_gate(&candidate.protocol_profile)?;
    let context = context_gate(request, candidate, reasoning)?;
    if context.is_none()
        && cost_policy == StaticCostPolicyV1::BudgetedPaid
        && candidate.cost_class == CostClassV1::Paid
    {
        return Err(ExclusionReasonCodeV1::PaidBudgetQuoteUnavailable);
    }
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
    profile
        .validate(requirements)
        .map_err(|error| match error {
            crate::server::core_runtime::profiles::CapabilityError::ReasoningProfileMismatch
            | crate::server::core_runtime::profiles::CapabilityError::ReasoningProfileUnknown => {
                ExclusionReasonCodeV1::ReasoningProfileMismatch
            }
            _ => ExclusionReasonCodeV1::ProtocolPathUnavailable,
        })?;
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
    request: &ModelRequestIRV1,
    candidate: &PlannerCandidateFactsV1,
    reasoning: &crate::server::core_runtime::profiles::ReasoningProfileCapability,
) -> Result<Option<CandidateContextDemand>, ExclusionReasonCodeV1> {
    let limits = candidate
        .protocol_profile
        .capability
        .context
        .with_requested_output(request.requested_max_output_tokens);
    ContextProjector::project_serialized_len(candidate.target_serialized_bytes, &limits, reasoning)
        .map_err(|error| match error {
            ContextProjectionError::UnknownLimit(_)
            | ContextProjectionError::UnknownEstimator
            | ContextProjectionError::ArithmeticOverflow => {
                ExclusionReasonCodeV1::ContextLimitUnknown
            }
        })
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
