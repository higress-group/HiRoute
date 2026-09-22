use std::collections::BTreeSet;

use hiroute_application_api::{
    CodexCandidateCapabilityLimitKindV1, CodexCandidateCapabilityLimitV1,
    CodexCapabilityIssueKindV1, CodexCapabilityIssueV1, CodexClientCapabilityPreviewV1,
    CodexFixedCapabilityLimitV1, CodexInputModalityV1, CodexReasoningControlV1,
};
use hiroute_domain::{CompiledAgentPlanV1, UpstreamProtocol};
use hiroute_gateway::server::core_runtime::{
    model_ir::{RequestCapabilityRequirementsV1, ToolChoice},
    profiles::{CandidateProtocolProfile, CapabilityError},
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

use super::{
    CodexCatalogError, CodexCatalogMetadataSourceV1, CodexCatalogSelection, CodexDefaultPolicy,
};

fn generic_prompt() -> &'static str {
    static PROMPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROMPT.get_or_init(|| {
        serde_json::from_str(include_str!("codex_generic_prompt.json"))
            .expect("bundled generic prompt is valid JSON")
    })
}

impl CodexCatalogSelection {
    pub fn append_plans(
        &self,
        plans: &[CompiledAgentPlanV1],
        policy: CodexDefaultPolicy<'_>,
        metadata_source: CodexCatalogMetadataSourceV1,
        retained_models: Option<&BTreeSet<String>>,
    ) -> Result<Value, CodexCatalogError> {
        self.validate_schema()?;
        let priorities = self.plan_priorities(plans.len())?;
        let mut merged = self.original().clone();
        let models = merged
            .get_mut("models")
            .and_then(Value::as_array_mut)
            .ok_or(CodexCatalogError::InvalidCatalog)?;
        if let Some(retained) = retained_models {
            models.retain(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|name| retained.contains(name))
            });
        }
        if metadata_source == CodexCatalogMetadataSourceV1::TargetCache {
            for model in models.iter_mut() {
                let model = model
                    .as_object_mut()
                    .ok_or(CodexCatalogError::InvalidCatalog)?;
                model
                    .entry("supports_parallel_tool_calls")
                    .or_insert(Value::Bool(true));
            }
        }
        for (plan, priority) in plans.iter().zip(priorities) {
            models.push(plan_entry(plan, priority)?);
        }
        let updated = Self::parse(merged)?;
        updated.validate_schema()?;
        self.require_preserved_default(&updated, policy)?;
        Ok(updated.original().clone())
    }
}

pub(super) fn plan_entry(
    plan: &CompiledAgentPlanV1,
    priority: i32,
) -> Result<Value, CodexCatalogError> {
    let (context_window, input_modalities) = match codex_plan_capability_preview(plan) {
        CodexClientCapabilityPreviewV1::Available {
            context_window,
            input_modalities,
            ..
        } => (context_window, input_modalities),
        CodexClientCapabilityPreviewV1::Unavailable { .. } => {
            return Err(CodexCatalogError::CapabilityUnproven);
        }
    };
    let context_window =
        i64::try_from(context_window).map_err(|_| CodexCatalogError::CapabilityUnproven)?;
    let modalities = input_modalities
        .into_iter()
        .map(|modality| match modality {
            CodexInputModalityV1::Text => "text",
            CodexInputModalityV1::Image => "image",
        })
        .collect::<Vec<_>>();
    let description = format!(
        "HiRoute 路由计划：{}；推理强度由计划配置决定",
        plan.body.identity.purpose.as_str(),
    );
    Ok(json!({
        "slug": plan.model_alias().as_str(),
        "display_name": plan.body.identity.display_name.as_str(),
        "description": description,
        "visibility": "list", "supported_in_api": true, "priority": priority,
        "default_reasoning_level": null, "supported_reasoning_levels": [],
        "shell_type": "shell_command", "apply_patch_tool_type": null,
        "model_messages": {
            "instructions_template": generic_prompt(), "instructions_variables": null,
            "approvals": null, "collaboration_modes": null, "auto_review": null,
            "permissions": null, "token_budget": null
        },
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "context_window": context_window, "max_context_window": context_window,
        "auto_compact_token_limit": null, "effective_context_window_percent": 95,
        "supports_reasoning_summary_parameter": false, "default_reasoning_summary": "none",
        "support_verbosity": false, "default_verbosity": null,
        "input_modalities": modalities, "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "web_search_tool_type": "text", "supports_search_tool": false,
        "experimental_supported_tools": [], "use_responses_lite": false, "tool_mode": "direct",
        "include_skills_usage_instructions": false, "include_plugin_usage_instructions": false,
        "include_apps_usage_instructions": false,
        "upgrade": null, "availability_nux": null, "comp_hash": null,
        "default_service_tier": null, "auto_review_model_override": null,
        "model_specialty": null, "multi_agent_version": null,
        "service_tiers": [], "additional_speed_tiers": []
    }))
}

/// A Worker owns a fresh private CODEX_HOME, so its only native model is the exact task Plan.
/// Reuse the same plan entry as the user-target merge without importing or changing that target's
/// catalog. The run's Gateway grant, not this metadata, still authorizes model requests.
pub fn codex_private_worker_catalog(
    plan: &CompiledAgentPlanV1,
) -> Result<Vec<u8>, CodexCatalogError> {
    let catalog = json!({"models": [plan_entry(plan, 0)?]});
    CodexCatalogSelection::for_current_adapter(catalog.clone())?;
    serde_json::to_vec(&catalog).map_err(|_| CodexCatalogError::InvalidCatalog)
}

pub fn codex_plan_capability_preview(plan: &CompiledAgentPlanV1) -> CodexClientCapabilityPreviewV1 {
    if plan.validate().is_err() {
        return unavailable(CodexCapabilityIssueKindV1::InvalidCompiledPlan, None);
    }
    let mut candidates = Vec::new();
    for candidate in plan
        .body
        .materialized
        .attempt_owned
        .groups
        .iter()
        .flat_map(|group| &group.candidates)
    {
        let Some(profile) = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == UpstreamProtocol::Responses)
        else {
            return unavailable(
                CodexCapabilityIssueKindV1::ResponsesProtocol,
                Some(&candidate.binding_id),
            );
        };
        let Ok(encoded) = serde_json::to_value(profile) else {
            return unavailable(
                CodexCapabilityIssueKindV1::RequestCapabilities,
                Some(&candidate.binding_id),
            );
        };
        let Ok(executable) = serde_json::from_value::<CandidateProtocolProfile>(encoded) else {
            return unavailable(
                CodexCapabilityIssueKindV1::RequestCapabilities,
                Some(&candidate.binding_id),
            );
        };
        // Codex emits developer-role input items in normal requests. Anthropic
        // Messages has no distinct developer role, even when that item is a
        // prelude, and cannot guarantee later instruction positions either.
        if executable.capability.upstream_protocol == IngressProtocol::Messages {
            return unavailable(
                CodexCapabilityIssueKindV1::InstructionRoles,
                Some(&candidate.binding_id),
            );
        }
        if let Err(error) = executable.validate(&requirements(false)) {
            return unavailable(
                if matches!(
                    error,
                    CapabilityError::InitialInstructionsUnsupported
                        | CapabilityError::MidConversationInstructionsUnsupported
                ) {
                    CodexCapabilityIssueKindV1::InstructionRoles
                } else {
                    CodexCapabilityIssueKindV1::RequestCapabilities
                },
                Some(&candidate.binding_id),
            );
        }
        let image = executable.validate(&requirements(true)).is_ok();
        let context = &profile.capability.context;
        let Some(input) = context.max_input_tokens.exact().copied() else {
            return unavailable(
                CodexCapabilityIssueKindV1::ContextInput,
                Some(&candidate.binding_id),
            );
        };
        let Some(output) = context.max_output_tokens.exact().copied() else {
            return unavailable(
                CodexCapabilityIssueKindV1::ContextOutput,
                Some(&candidate.binding_id),
            );
        };
        let Some(total) = context.max_total_tokens.exact().copied() else {
            return unavailable(
                CodexCapabilityIssueKindV1::ContextTotal,
                Some(&candidate.binding_id),
            );
        };
        let Ok(reasoning) = profile.reasoning_profile_for(&candidate.exact_reasoning) else {
            return unavailable(
                CodexCapabilityIssueKindV1::ReasoningProfile,
                Some(&candidate.binding_id),
            );
        };
        let Some(reservation) = output.checked_add(reasoning.additional_reservation_tokens) else {
            return unavailable(
                CodexCapabilityIssueKindV1::ContextWindow,
                Some(&candidate.binding_id),
            );
        };
        let window = total.map_or(input, |total| input.min(total.saturating_sub(reservation)));
        if window == 0 || output == 0 {
            return unavailable(
                CodexCapabilityIssueKindV1::ContextWindow,
                Some(&candidate.binding_id),
            );
        }
        candidates.push((candidate.binding_id.as_str(), window, image));
    }
    let Some(context_window) = candidates.iter().map(|(_, window, _)| *window).min() else {
        return unavailable(CodexCapabilityIssueKindV1::InvalidCompiledPlan, None);
    };
    if i64::try_from(context_window).is_err() {
        return unavailable(CodexCapabilityIssueKindV1::ContextWindow, None);
    }
    let image = candidates.iter().all(|(_, _, image)| *image);
    let mut limitations = Vec::new();
    if candidates
        .iter()
        .any(|(_, window, _)| *window > context_window)
    {
        limitations.push(CodexCandidateCapabilityLimitV1 {
            kind: CodexCandidateCapabilityLimitKindV1::ContextWindow,
            binding_ids: limiting_bindings(&candidates, |(_, window, _)| *window == context_window),
        });
    }
    if candidates.iter().any(|(_, _, image)| *image)
        && candidates.iter().any(|(_, _, image)| !*image)
    {
        limitations.push(CodexCandidateCapabilityLimitV1 {
            kind: CodexCandidateCapabilityLimitKindV1::ImageInput,
            binding_ids: limiting_bindings(&candidates, |(_, _, image)| !*image),
        });
    }
    let mut input_modalities = vec![CodexInputModalityV1::Text];
    if image {
        input_modalities.push(CodexInputModalityV1::Image);
    }
    CodexClientCapabilityPreviewV1::Available {
        context_window,
        input_modalities,
        reasoning: CodexReasoningControlV1::RouteConfiguration,
        limitations,
        fixed_limits: vec![CodexFixedCapabilityLimitV1::ParallelToolCallsDisabled],
    }
}

fn limiting_bindings<'a>(
    candidates: &'a [(&'a str, u64, bool)],
    predicate: impl Fn(&(&str, u64, bool)) -> bool,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| predicate(candidate))
        .map(|(binding_id, _, _)| (*binding_id).to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn unavailable(
    kind: CodexCapabilityIssueKindV1,
    binding_id: Option<&str>,
) -> CodexClientCapabilityPreviewV1 {
    CodexClientCapabilityPreviewV1::Unavailable {
        issues: vec![CodexCapabilityIssueV1 {
            kind,
            binding_id: binding_id.map(str::to_owned),
        }],
    }
}

fn requirements(image: bool) -> RequestCapabilityRequirementsV1 {
    RequestCapabilityRequirementsV1 {
        ingress_protocol: IngressProtocol::Responses,
        text: true,
        initial_instructions: true,
        mid_conversation_instructions: true,
        image_url: image,
        image_base64: image,
        image_media_types: if image {
            ["image/jpeg", "image/png", "image/gif", "image/webp"]
                .map(str::to_owned)
                .to_vec()
        } else {
            Vec::new()
        },
        function_tools: true,
        strict_tools: false,
        tool_choice: ToolChoice::Auto,
        parallel_tools: false,
        tool_roundtrip: true,
        tool_result_text: true,
        tool_result_json: true,
        logical_tool_id_mapping: true,
        streaming: true,
        stream_text: true,
        stream_tool_arguments: true,
        stream_reasoning: false,
        stream_usage: true,
        provider_state: false,
    }
}

#[cfg(test)]
#[path = "codex_catalog_plan_tests.rs"]
mod tests;
