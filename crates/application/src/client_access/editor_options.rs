use super::*;
use crate::compiler::{AgentPlanCompilerError, compile_agent_plan_v2};
use hiroute_domain::{AgentPlanId, AgentPlanIdentityV1, ModelAlias, PlanEditorStateV2};

pub(super) fn dispatch(
    ports: &crate::control::ApplicationPorts,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    let payload: PlanEditorOptionsRequestV1 = match serde_json::from_value(request.payload) {
        Ok(value) => value,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let Some(port) = &ports.routing else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if payload.rating_items.len() > MAX_RATING_QUERY_ITEMS
        || payload.native_selections.len() > MAX_RATING_QUERY_ITEMS
    {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let result = (|| {
        let state = port
            .routing_compilation_snapshot(&WorkspaceId::default())
            .map_err(map_control_error)?;
        if state.facts.candidates.len() > MAX_RATING_QUERY_ITEMS {
            return Err(ErrorCode::InvalidArguments);
        }
        let free_suggestions = if payload.suggest_free {
            Some(
                port.free_plan_suggestions(
                    &WorkspaceId::default(),
                    &payload.requirements,
                    &payload.native_selections,
                    RatingSnapshotSelectionV1::Latest,
                )
                .map_err(map_control_error)?,
            )
        } else {
            None
        };
        let ratings = if payload.rating_items.is_empty() {
            None
        } else {
            match port.resolve_model_ratings(&ResolveModelRatingsV1 {
                snapshot: RatingSnapshotSelectionV1::Latest,
                items: payload.rating_items,
            }) {
                Ok(result) => Some(result),
                Err(crate::model_catalog::RatingQueryError::SnapshotUnavailable) => None,
                Err(_) => return Err(ErrorCode::InvalidArguments),
            }
        };
        let suggested_alias = payload
            .display_name
            .as_ref()
            .map(|name| {
                state
                    .active_publication
                    .as_ref()
                    .map(|p| p.alias_registry.clone())
                    .unwrap_or_default()
                    .suggest_alias(name)
                    .map_err(|_| ErrorCode::InvalidArguments)
            })
            .transpose()?;
        if payload.published_plan.is_some() && payload.editor.is_some() {
            return Err(ErrorCode::InvalidArguments);
        }
        let (codex_capabilities, claude_capabilities, context_window) = if let Some(reference) =
            &payload.published_plan
        {
            let publication = state
                .active_publication
                .as_ref()
                .ok_or(ErrorCode::ResourceNotFound)?;
            let plan = published_preview_plan(publication, reference)?;
            let upper = plan
                .body
                .materialized
                .context_window_upper_bound()
                .map_err(|_| ErrorCode::CapabilityDenied)?;
            (
                Some(
                    port.codex_client_capability_preview(&plan)
                        .map_err(map_control_error)?,
                ),
                Some(
                    port.claude_client_capability_preview(&plan)
                        .map_err(map_control_error)?,
                ),
                Some(PlanContextWindowPreviewV1 {
                    maximum_tokens: upper,
                    default_tokens: upper.min(hiroute_domain::DEFAULT_PLAN_CONTEXT_WINDOW_TOKENS),
                }),
            )
        } else {
            (
                codex_capabilities(payload.editor.as_ref(), &state.facts, port.as_ref())?,
                claude_capabilities(payload.editor.as_ref(), &state.facts, port.as_ref()),
                context_window_preview(payload.editor.as_ref(), &state.facts),
            )
        };
        let candidates = state
            .facts
            .candidates
            .iter()
            .map(|fact| PlanCandidateOptionV1 {
                binding_id: fact.binding.binding_id.clone(),
                model_configuration_id: fact.model.model_configuration_id.clone(),
                display_name: fact.model.display_name.clone(),
                reasoning: fact.reasoning.clone(),
                billing_class: fact.binding.billing_class,
                routable: fact.is_routable(),
                ingress_protocols: fact
                    .protocol_profiles
                    .iter()
                    .map(|p| p.ingress_protocol)
                    .collect(),
            })
            .collect();
        Ok(PlanEditorOptionsV1 {
            claude_capabilities,
            context_window,
            suggested_alias,
            candidates,
            ratings,
            free_suggestions,
            codex_capabilities,
            revisions: state.expected_revisions,
        })
    })();
    match result {
        Ok(value) => succeeded(value, request.request_id),
        Err(error) => failed(error, request.request_id),
    }
}

fn codex_capabilities(
    editor: Option<&PlanEditorStateV2>,
    facts: &crate::compiler::AgentPlanCompilationFactsV1,
    port: &dyn crate::control::RoutingFactsPort,
) -> Result<Option<CodexClientCapabilityPreviewV1>, ErrorCode> {
    let Some(editor) = editor else {
        return Ok(None);
    };
    let Ok(configuration) = editor.effective() else {
        // Incomplete draft rows are ordinary while editing and do not claim a capability state.
        return Ok(None);
    };
    let identity = AgentPlanIdentityV1 {
        agent_plan_id: AgentPlanId::parse("plan/codex-capability-preview")
            .map_err(|_| ErrorCode::Internal)?,
        model_alias: ModelAlias::parse("hiroute-codex-capability-preview")
            .map_err(|_| ErrorCode::Internal)?,
        display_name: configuration.display_name.clone(),
        purpose: configuration.purpose.clone(),
    };
    let plan = match compile_agent_plan_v2(identity, 1, &configuration, facts) {
        Ok(plan) => plan,
        Err(error) => return Ok(Some(compilation_unavailable(&error))),
    };
    port.codex_client_capability_preview(&plan)
        .map(Some)
        .map_err(map_control_error)
}

fn context_window_preview(
    editor: Option<&PlanEditorStateV2>,
    facts: &crate::compiler::AgentPlanCompilationFactsV1,
) -> Option<PlanContextWindowPreviewV1> {
    let mut editor = editor?.clone();
    // Inspect the capability ceiling even when the custom setting exceeds new candidates.
    editor.limits.context_window_tokens = None;
    let configuration = editor.effective().ok()?;
    let identity = AgentPlanIdentityV1 {
        agent_plan_id: AgentPlanId::parse("plan/context-window-preview").ok()?,
        model_alias: ModelAlias::parse("hiroute-context-window-preview").ok()?,
        display_name: configuration.display_name.clone(),
        purpose: configuration.purpose.clone(),
    };
    let plan = compile_agent_plan_v2(identity, 1, &configuration, facts).ok()?;
    let maximum_tokens = plan.body.materialized.context_window_upper_bound().ok()?;
    Some(PlanContextWindowPreviewV1 {
        maximum_tokens,
        default_tokens: maximum_tokens.min(hiroute_domain::DEFAULT_PLAN_CONTEXT_WINDOW_TOKENS),
    })
}

fn compilation_unavailable(error: &AgentPlanCompilerError) -> CodexClientCapabilityPreviewV1 {
    let binding_id = match error {
        AgentPlanCompilerError::UnknownBinding(binding_id)
        | AgentPlanCompilerError::CandidateNotRoutable(binding_id)
        | AgentPlanCompilerError::CapabilityUnqualified(binding_id) => Some(binding_id.clone()),
        _ => None,
    };
    CodexClientCapabilityPreviewV1::Unavailable {
        issues: vec![CodexCapabilityIssueV1 {
            kind: CodexCapabilityIssueKindV1::PlanCompilation,
            binding_id,
        }],
    }
}

fn claude_capabilities(
    editor: Option<&PlanEditorStateV2>,
    facts: &crate::compiler::AgentPlanCompilationFactsV1,
    port: &dyn crate::control::RoutingFactsPort,
) -> Option<ClaudeClientCapabilityPreviewV1> {
    let editor = editor?;
    let unavailable = |reason: &str| {
        Some(ClaudeClientCapabilityPreviewV1::Unavailable {
            reason: reason.into(),
        })
    };
    let Ok(configuration) = editor.effective() else {
        return None;
    };
    let identity = AgentPlanIdentityV1 {
        agent_plan_id: AgentPlanId::parse("plan/claude-capability-preview").ok()?,
        model_alias: ModelAlias::parse("hiroute-claude-capability-preview").ok()?,
        display_name: configuration.display_name.clone(),
        purpose: configuration.purpose.clone(),
    };
    let Ok(plan) = compile_agent_plan_v2(identity, 1, &configuration, facts) else {
        return unavailable("plan_compilation");
    };
    Some(
        port.claude_client_capability_preview(&plan)
            .unwrap_or_else(|_| ClaudeClientCapabilityPreviewV1::Unavailable {
                reason: "request_capabilities".into(),
            }),
    )
}

fn published_preview_plan(
    publication: &hiroute_domain::GatewayPublicationV1,
    reference: &PublishedPlanCapabilityRequestV1,
) -> Result<hiroute_domain::CompiledAgentPlanV1, ErrorCode> {
    if !publication
        .published_agent_plans()
        .map_err(|_| ErrorCode::Internal)?
        .iter()
        .any(|plan| plan.agent_plan_id == reference.plan_id && plan.active)
    {
        return Err(ErrorCode::ResourceNotFound);
    }
    let plan = publication
        .plans
        .iter()
        .find(|plan| plan.agent_plan_id() == &reference.plan_id)
        .ok_or(ErrorCode::ResourceNotFound)?;
    if plan.body.agent_plan_revision != reference.revision {
        return Err(ErrorCode::RevisionConflict);
    }
    plan.clone().into_current().map_err(|_| ErrorCode::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::test_fixtures::{compilation_facts, custom_desired};
    use crate::control::{ControlReadError, RoutingCompilationSnapshotV1};
    use hiroute_domain::{
        AgentPlanStrategyV1, ComplexityClassifierModeV1, FreeEditorV2, PLAN_EDITOR_SCHEMA_V2,
        PlanEditorMode, SmartEditorV2, WorkspaceId,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct PreviewPort {
        calls: AtomicUsize,
    }

    impl crate::control::RoutingFactsPort for PreviewPort {
        fn codex_client_capability_preview(
            &self,
            _plan: &hiroute_domain::CompiledAgentPlanV1,
        ) -> Result<CodexClientCapabilityPreviewV1, ControlReadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CodexClientCapabilityPreviewV1::Available {
                context_window: 64_000,
                input_modalities: vec![CodexInputModalityV1::Text],
                reasoning: CodexReasoningControlV1::RouteConfiguration,
                limitations: Vec::new(),
                fixed_limits: vec![CodexFixedCapabilityLimitV1::ParallelToolCallsDisabled],
            })
        }

        fn routing_compilation_snapshot(
            &self,
            _workspace_id: &WorkspaceId,
        ) -> Result<RoutingCompilationSnapshotV1, ControlReadError> {
            Err(ControlReadError::Unavailable)
        }
    }

    fn editor() -> PlanEditorStateV2 {
        let desired = custom_desired();
        let AgentPlanStrategyV1::Custom { candidates } = desired.strategy else {
            unreachable!()
        };
        PlanEditorStateV2 {
            schema: PLAN_EDITOR_SCHEMA_V2.into(),
            display_name: desired.display_name.as_str().to_owned(),
            purpose: desired.purpose.as_str().to_owned(),
            custom_alias: None,
            mode: PlanEditorMode::FixedModel,
            candidates,
            smart: SmartEditorV2 {
                economy: Vec::new(),
                primary: Vec::new(),
                primary_fallback: false,
                classifier: ComplexityClassifierModeV1::LocalRules,
                complex_keywords: Vec::new(),
            },
            free: FreeEditorV2 {
                candidates: Vec::new(),
                primary: Vec::new(),
                primary_fallback: false,
            },
            delegation_enabled: false,
            work: None,
            requirements: desired.requirements,
            limits: desired.limits,
        }
    }

    #[test]
    fn claude_deployment_preview_reads_the_published_revision_not_current_candidate_facts() {
        let publication = crate::compiler::test_fixtures::compiled_publication(1);
        let active = publication
            .published_agent_plans()
            .unwrap()
            .into_iter()
            .find(|plan| plan.active)
            .unwrap();
        let mut reference = PublishedPlanCapabilityRequestV1 {
            plan_id: active.agent_plan_id.clone(),
            revision: active.agent_plan_revision,
        };
        let exact = published_preview_plan(&publication, &reference).unwrap();
        let original = publication
            .plans
            .iter()
            .find(|plan| plan.agent_plan_id() == &reference.plan_id)
            .unwrap()
            .clone()
            .into_current()
            .unwrap();
        assert_eq!(exact, original);
        // A newer draft or changed model metadata cannot be mistaken for a saved revision.
        reference.revision += 1;
        assert!(matches!(
            published_preview_plan(&publication, &reference),
            Err(ErrorCode::RevisionConflict)
        ));
    }

    #[test]
    fn context_bound_survives_a_custom_value_invalidated_by_candidates() {
        let mut value = editor();
        value.limits.context_window_tokens = Some(1_050_000);
        let bounds = context_window_preview(Some(&value), &compilation_facts()).unwrap();
        assert!(bounds.maximum_tokens < 1_050_000);
        assert_eq!(bounds.default_tokens, bounds.maximum_tokens.min(272_000));
        assert!(matches!(
            codex_capabilities(Some(&value), &compilation_facts(), &PreviewPort::default())
                .unwrap(),
            Some(CodexClientCapabilityPreviewV1::Unavailable { .. })
        ));
    }

    #[test]
    fn complete_editor_compiles_with_current_facts_before_capability_analysis() {
        let port = PreviewPort::default();
        let preview = codex_capabilities(Some(&editor()), &compilation_facts(), &port).unwrap();
        assert!(matches!(
            preview,
            Some(CodexClientCapabilityPreviewV1::Available {
                context_window: 64_000,
                ..
            })
        ));
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn incomplete_editor_has_no_premature_capability_warning() {
        let port = PreviewPort::default();
        let mut editor = editor();
        editor.display_name.clear();
        assert_eq!(
            codex_capabilities(Some(&editor), &compilation_facts(), &port).unwrap(),
            None
        );
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn compilation_failure_identifies_the_selected_binding_without_calling_integration() {
        let port = PreviewPort::default();
        let mut editor = editor();
        editor.candidates[0].binding_id = "binding/missing".into();
        assert_eq!(
            codex_capabilities(Some(&editor), &compilation_facts(), &port).unwrap(),
            Some(CodexClientCapabilityPreviewV1::Unavailable {
                issues: vec![CodexCapabilityIssueV1 {
                    kind: CodexCapabilityIssueKindV1::PlanCompilation,
                    binding_id: Some("binding/missing".into()),
                }],
            })
        );
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }
}
