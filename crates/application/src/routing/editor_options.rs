//! Shared authoring policy. Options are drafts, never publication or admission authority.
use hiroute_application_api::*;
use hiroute_domain::*;

pub(crate) fn prepare_authoring(
    input: &PlanAuthoringRequestV1,
    current: Option<&AgentPlanStatusV2>,
    candidates: &[PlanCandidateOptionV1],
) -> Result<PlanAuthoringOptionsV1, ErrorCode> {
    if input.plan_id.as_ref() != current.map(|plan| &plan.agent_plan_id) {
        return Err(ErrorCode::ResourceNotFound);
    }
    let mut editor = match current {
        Some(plan) => reopen(plan)?,
        None => initial_editor(),
    };
    let before = editor.clone();
    if let Some(mode) = input.mode {
        editor.mode = mode;
        if current.is_none() {
            editor.follow_request_reasoning = mode == PlanEditorMode::FixedModel;
        }
    }
    if let Some(name) = &input.display_name {
        editor.display_name = name.clone();
    }
    if let Some(purpose) = &input.purpose {
        editor.purpose = purpose.clone();
    }
    if let Some(classifier) = &input.classifier {
        editor.smart.classifier = classifier.clone();
    }
    let mut choices = Vec::new();
    for (requested, old, target, field, economy) in [
        (
            &input.candidates,
            &before.candidates,
            &mut editor.candidates,
            "candidates",
            false,
        ),
        (
            &input.economy,
            &before.smart.economy,
            &mut editor.smart.economy,
            "smart.economy",
            true,
        ),
        (
            &input.primary,
            &before.smart.primary,
            &mut editor.smart.primary,
            "smart.primary",
            false,
        ),
    ] {
        if let Some(values) = requested {
            if values.len() > 128 {
                return Err(ErrorCode::InvalidArguments);
            }
            *target = values
                .iter()
                .map(|selection| {
                    let mut selection = selection.clone();
                    // A repeated existing binding keeps its custom reasoning unless explicitly changed.
                    if selection.reasoning.is_none() {
                        selection.reasoning = old
                            .iter()
                            .find(|v| v.binding_id == selection.binding_id)
                            .and_then(|v| v.reasoning.clone())
                            .or_else(|| {
                                candidates
                                    .iter()
                                    .find(|v| v.binding_id == selection.binding_id)
                                    .and_then(|v| {
                                        if editor.follow_request_reasoning {
                                            if let NativeReasoningCapabilityV1::Discrete {
                                                profiles,
                                                default_profile: Some(default),
                                                ..
                                            } = &v.reasoning
                                            {
                                                if profiles.contains(default) {
                                                    return Some(ReasoningSelectionV1::Profile {
                                                        profile: default.clone(),
                                                    });
                                                }
                                            }
                                        }
                                        default_reasoning(&v.reasoning, economy)
                                    })
                            });
                    }
                    selection
                })
                .collect();
        }
        let active = match field {
            "candidates" => editor.mode == PlanEditorMode::FixedModel,
            _ => editor.mode == PlanEditorMode::SmartSaving,
        };
        if active {
            group_choices(target, field, candidates, &mut choices);
        }
    }
    if editor.display_name.trim().is_empty() {
        required(
            &mut choices,
            "display_name",
            PlanAuthoringChoiceReasonV1::Required,
        );
    }
    if editor.delegation_enabled && editor.purpose.trim().is_empty() {
        required(
            &mut choices,
            "purpose",
            PlanAuthoringChoiceReasonV1::Required,
        );
    }
    if editor.mode == PlanEditorMode::SmartSaving && editor.smart.classifier.validate().is_err() {
        required(
            &mut choices,
            "smart.classifier",
            PlanAuthoringChoiceReasonV1::Required,
        );
    }
    editor
        .validate_draft()
        .map_err(|_| ErrorCode::InvalidArguments)?;
    Ok(PlanAuthoringOptionsV1 {
        editor,
        plan_id: input.plan_id.clone(),
        expected_head_revision: current.map(|plan| plan.head.head_revision),
        required_choices: choices,
        candidate_defaults: candidates
            .iter()
            .map(|candidate| PlanCandidateDefaultsV1 {
                binding_id: candidate.binding_id.clone(),
                economy: default_reasoning(&candidate.reasoning, true),
                primary: default_reasoning(&candidate.reasoning, false),
            })
            .collect(),
    })
}

fn initial_editor() -> PlanEditorStateV2 {
    PlanEditorStateV2 {
        follow_request_reasoning: true,
        schema: PLAN_EDITOR_SCHEMA_V2.into(),
        display_name: String::new(),
        purpose: String::new(),
        custom_alias: None,
        mode: PlanEditorMode::FixedModel,
        candidates: Vec::new(),
        smart: SmartEditorV2 {
            economy: Vec::new(),
            primary: Vec::new(),
            judgment: JudgmentSettingsV1::default(),
            reselect_on_user_message: true,
            // An unfinished selector, not a usable service or an implicit local-rules choice.
            classifier: ComplexityClassifierModeV1::DecisionService {
                service: Box::new(DecisionServiceV1 {
                    id: String::new(),
                    revision: 0,
                    name: String::new(),
                    connection: DecisionConnectionV1::Custom {
                        endpoint: String::new(),
                        timeout_ms: 10_000,
                        auth_header: None,
                    },
                }),
            },
            complex_keywords: Vec::new(),
        },
        free: FreeEditorV2 {
            candidates: Vec::new(),
            primary: Vec::new(),
            primary_fallback: false,
        },
        branch_routing: None,
        delegation_enabled: false,
        work: None,
        requirements: CapabilityRequirementsV1::default(),
        limits: RoutingLimitsV1 {
            context_window_tokens: None,
            maximum_attempts: 6,
            request_timeout_ms: 3_600_000,
            attempt_timeout_ms: 30_000,
        },
    }
}

fn reopen(plan: &AgentPlanStatusV2) -> Result<PlanEditorStateV2, ErrorCode> {
    let mut editor = plan
        .desired
        .editor(Some(plan.model_alias.as_str().to_owned()))
        .map_err(|_| ErrorCode::Internal)?;
    // Domain reopening preserves all effective settings. Inactive modes use the shared seed.
    if editor.mode != PlanEditorMode::SmartSaving {
        editor.smart = initial_editor().smart;
    }
    Ok(editor)
}

fn default_reasoning(
    native: &NativeReasoningCapabilityV1,
    economy: bool,
) -> Option<ReasoningSelectionV1> {
    if native.validate().is_err() {
        return None;
    }
    match native {
        NativeReasoningCapabilityV1::Fixed { .. } | NativeReasoningCapabilityV1::Budget { .. } => {
            None
        }
        NativeReasoningCapabilityV1::Toggle { .. } => {
            Some(ReasoningSelectionV1::Toggle { enabled: !economy })
        }
        NativeReasoningCapabilityV1::Discrete { profiles, .. } => {
            // Native ordering is authoritative, including the existing all-disabled fallback.
            let enabled: Vec<_> = profiles
                .iter()
                .filter(|p| !matches!(p.to_ascii_lowercase().as_str(), "none" | "off" | "disabled"))
                .collect();
            let available: Vec<_> = if enabled.is_empty() {
                profiles.iter().collect()
            } else {
                enabled
            };
            let profile = if economy {
                available.first()
            } else {
                available.last()
            }?;
            Some(ReasoningSelectionV1::Profile {
                profile: (*profile).clone(),
            })
        }
    }
}

fn group_choices(
    values: &[CandidateSelectionV1],
    field: &str,
    candidates: &[PlanCandidateOptionV1],
    choices: &mut Vec<PlanAuthoringChoiceV1>,
) {
    if values.is_empty() {
        required(choices, field, PlanAuthoringChoiceReasonV1::Required);
    }
    for (index, selection) in values.iter().enumerate() {
        let path = format!("{field}[{index}]");
        let Some(candidate) = candidates
            .iter()
            .find(|v| v.binding_id == selection.binding_id && v.routable)
        else {
            required(
                choices,
                &path,
                PlanAuthoringChoiceReasonV1::CandidateUnavailable,
            );
            continue;
        };
        if candidate
            .reasoning
            .resolve(
                selection.reasoning.as_ref(),
                DiscreteReasoningDefault::RequireExplicit,
            )
            .is_err()
        {
            required(
                choices,
                &format!("{path}.reasoning"),
                if selection.reasoning.is_none() {
                    PlanAuthoringChoiceReasonV1::ReasoningRequired
                } else {
                    PlanAuthoringChoiceReasonV1::InvalidSelection
                },
            );
        }
    }
}

fn required(
    choices: &mut Vec<PlanAuthoringChoiceV1>,
    field: &str,
    reason: PlanAuthoringChoiceReasonV1,
) {
    choices.push(PlanAuthoringChoiceV1 {
        field: field.into(),
        reason,
    });
}

#[cfg(test)]
mod tests;
