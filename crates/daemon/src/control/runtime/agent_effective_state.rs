//! Compose existing settings and authenticated receipts. This module neither activates Agents
//! nor sends inference requests, and deliberately does not infer identity from timestamps.
use super::settings_status::ConfiguredModelSettings;
use super::{LocalControlAdapter, settings_facts::SettingsAgentClass};
use hiroute_application::control::{ApplicationClockPort, ControlReadError};
use hiroute_application_api::*;
use hiroute_domain::{
    ExecutionFactV1, FactsCompleteness, FrozenExecutionTrustV1, ObservationQueryPort,
    ObservationReaderContext, ObservationRequestQuery, ObservedRequestSummary,
    PublicationRepositoryPort, ReceiptId, RoutingReceiptV1, WorkspaceId,
};

fn fact(state: AgentEffectiveFactStateV1, reason: &str) -> AgentEffectiveFactV1 {
    AgentEffectiveFactV1 {
        state,
        reason: reason.into(),
    }
}

impl LocalControlAdapter {
    pub(super) fn agent_effective_state(
        &self,
        status: &AgentModelSettingsStatusV2,
        join: Option<&ConfiguredModelSettings>,
        request: &AgentEffectiveStateRequestV1,
    ) -> Result<AgentEffectiveStateV1, ControlReadError> {
        use AgentEffectiveFactStateV1::*;
        if !request.valid() {
            return Err(ControlReadError::Denied);
        }
        let class = self
            .settings_agent_for_context(&status.context_id)
            .ok_or(ControlReadError::NotFound)?;
        let preparation = match status.state {
            AgentModelSettingsStateV2::Configured => fact(Known, "managed_configuration_saved"),
            AgentModelSettingsStateV2::Restored => fact(Known, "owned_configuration_restored"),
            AgentModelSettingsStateV2::NotConfigured if owned_restore_completed(status) => {
                fact(Known, "owned_configuration_restored")
            }
            AgentModelSettingsStateV2::NotConfigured => {
                fact(Pending, "model_routing_not_configured")
            }
            AgentModelSettingsStateV2::Pending => fact(Pending, "operation_pending"),
            AgentModelSettingsStateV2::Drift => fact(Blocked, "owned_configuration_drift"),
            AgentModelSettingsStateV2::NeedsAttention => fact(Blocked, "operation_needs_attention"),
        };
        let native_prerequisites = if status
            .native_adapter
            .as_ref()
            .is_some_and(|adapter| !adapter.resources_installed)
        {
            fact(Blocked, "native_adapter_resources_missing")
        } else if join.is_some() {
            fact(Known, "managed_resources_available")
        } else {
            fact(Unknown, "no_current_managed_configuration")
        };
        let mut result = AgentEffectiveStateV1 {
            schema: "hiroute.agent-effective-state/v1".into(),
            context_id: status.context_id.clone(),
            plan_id: request.plan_id.clone(),
            preparation,
            native_prerequisites,
            native_loading: fact(Unknown, "running_native_process_loading_unobserved"),
            current_selection: fact(Unknown, "current_native_session_selection_unobserved"),
            request_evidence: fact(Pending, "no_matching_request_observed"),
            observed_request: None,
            search_incomplete: false,
            next_actions: native_actions(class, status),
        };
        let Some(join) = join else {
            result.request_evidence = fact(Pending, "current_configuration_required");
            return Ok(result);
        };
        let snapshot = {
            let stores = self.stores_lock().map_err(super::map_port)?;
            let publication = stores
                .control()
                .active_publication(&WorkspaceId::default())
                .map_err(super::map_port)?
                .ok_or(ControlReadError::SnapshotChanged)?;
            if publication.digest != join.publication_digest {
                return Err(ControlReadError::SnapshotChanged);
            }
            publication
                .verify()
                .map_err(|_| ControlReadError::Corrupt)?
                .gateway_snapshot()
                .map_err(|_| ControlReadError::Corrupt)?
        };
        let expected = join
            .grant
            .scope()
            .model_grant()
            .routes
            .keys()
            .map(|model| {
                super::agent_live_check::expected_trust(&snapshot, join.grant.grant_id(), model)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|trust| trust.agent_plan_id.as_ref() == Some(&request.plan_id))
            .collect::<Vec<_>>();
        if expected.is_empty() {
            result.request_evidence = fact(Blocked, "selected_plan_not_granted_for_connection");
            return Ok(result);
        }
        let now = self.now_ms()?;
        // Same trusted local-user scope as the Local Control observation readers; content is
        // not requested. Status never opens the database or invents another observation index.
        let reader = ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "agent-effective-state".into(),
            1,
            now.saturating_add(5_000),
            false,
            false,
        )
        .map_err(|_| ControlReadError::Unavailable)?;
        let query = ObservationRequestQuery {
            from_ms: 0,
            to_ms: now.saturating_add(1),
            session_id: request.session_id.clone(),
            request_id: request.request_id.clone(),
            limit: 100,
            cursor: None,
            agent_id: None,
            plan_id: Some(request.plan_id.as_str().to_owned()),
            native_model: None,
            outcome: None,
            only_model_switch: false,
        };
        match self
            .delegation_observation
            .list_observed_requests(&reader, &query, now)
        {
            Ok(page) => {
                result.search_incomplete = page.next_cursor.is_some();
                for summary in &page.requests {
                    if summary.relation_conflicted {
                        continue;
                    }
                    let Some(receipt_id) = summary
                        .receipt_id
                        .as_ref()
                        .and_then(|id| ReceiptId::parse(id).ok())
                    else {
                        continue;
                    };
                    let receipt = match self
                        .delegation_observation
                        .get_receipt(&WorkspaceId::default(), &receipt_id)
                    {
                        Ok(receipt) => receipt,
                        Err(_) => {
                            result.search_incomplete = true;
                            continue;
                        }
                    };
                    if !receipt_matches(&receipt, summary, &expected) {
                        continue;
                    }
                    result.observed_request = Some(project_evidence(&receipt, summary));
                    result.request_evidence = fact(Known, "matching_request_recorded");
                    break;
                }
            }
            Err(_) => {
                result.request_evidence = fact(Unknown, "observation_read_unavailable");
                result.search_incomplete = true;
            }
        }
        // Reject a concurrent edit, restore, publication or grant change during observation.
        let current = self
            .configured_model_settings_join(&status.context_id)?
            .ok_or(ControlReadError::SnapshotChanged)?;
        if current.operation_id != join.operation_id
            || current.grant != join.grant
            || current.publication_digest != join.publication_digest
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(result)
    }
}

fn receipt_matches(
    receipt: &RoutingReceiptV1,
    summary: &ObservedRequestSummary,
    expected: &[FrozenExecutionTrustV1],
) -> bool {
    // Equality includes authority epoch, exact Plan route/revision, publication, grant ID and
    // generation. A provider probe, older configuration or another profile cannot pass.
    receipt.workspace_id == WorkspaceId::default()
        && receipt.request_id.as_str() == summary.request_id
        && receipt.session_id.as_str() == summary.session_id
        && summary.receipt_id.as_deref() == Some(receipt.receipt_id.as_str())
        && !summary.relation_conflicted
        && matches!(
            receipt.correlation.correlation_provenance,
            hiroute_domain::CorrelationProvenance::AgentSupplied
                | hiroute_domain::CorrelationProvenance::ProtocolState
        )
        && receipt.validate().is_ok()
        && expected.contains(&receipt.trust)
}

fn project_evidence(
    receipt: &RoutingReceiptV1,
    summary: &ObservedRequestSummary,
) -> AgentEffectiveRequestEvidenceV1 {
    let mut usage = std::collections::BTreeMap::new();
    for event in &receipt.ordered_facts {
        if let ExecutionFactV1::UsageAndCache {
            ordinal,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            ..
        } = &event.fact
        {
            usage.insert(
                *ordinal,
                AgentEffectiveUsageV1 {
                    attempt_ordinal: *ordinal,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cache_read_tokens: *cache_read_tokens,
                    cache_write_tokens: *cache_write_tokens,
                    reasoning_tokens: *reasoning_tokens,
                },
            );
        }
    }
    AgentEffectiveRequestEvidenceV1 {
        session_id: receipt.session_id.to_string(),
        request_id: receipt.request_id.to_string(),
        receipt_id: receipt.receipt_id.to_string(),
        started_at_ms: summary.started_at_ms,
        routing_context: hiroute_domain::ObservationRoutingContextV1::from_execution_trust(
            &receipt.trust,
        ),
        served_model_id: receipt.trust.served_model_id.clone(),
        actual_model: summary.final_native_model.clone(),
        outcome: receipt.outcome,
        facts_complete: receipt.facts_completeness == FactsCompleteness::Complete,
        publication_revision: receipt.trust.gateway_publication_revision.clone(),
        publication_digest: receipt.trust.gateway_publication_digest.to_string(),
        grant_id: receipt.trust.grant_id.clone(),
        grant_generation: receipt.trust.grant_generation,
        usage: usage.into_values().collect(),
        scope: "recorded_request_and_session_only; current_process_and_other_sessions_unobserved"
            .into(),
    }
}

fn native_actions(
    class: SettingsAgentClass,
    status: &AgentModelSettingsStatusV2,
) -> Vec<AgentNativeNextActionV1> {
    use AgentModelSettingsStateV2::*;
    let mut actions = Vec::new();
    let mut add =
        |action: &str, surface: &str, effects: &[&str], prerequisite: &str, guidance: &str| {
            actions.push(AgentNativeNextActionV1 {
                action: action.into(),
                actor: "human_or_native_agent_with_user_authorization".into(),
                surface: surface.into(),
                effects: effects.iter().map(|effect| (*effect).into()).collect(),
                prerequisite: prerequisite.into(),
                guidance: guidance.into(),
            });
        };
    match status.state {
        Drift | NeedsAttention => {
            add(
                "inspect_owned_operation",
                "local_control",
                &["read_only"],
                "inspect_operation_and_foreign_edits",
                "Inspect the reported Operation and restore conflict before repairing owned files. Preserve unrelated edits.",
            );
            return actions;
        }
        NotConfigured if owned_restore_completed(status) => {}
        Pending | NotConfigured => return actions,
        Configured | Restored => {}
    }
    if class == SettingsAgentClass::Hermes {
        add(
            "load_native_configuration",
            "hermes_cli_or_im",
            &["starts_new_native_process"],
            "saved_configuration; human_controls_existing_gateway_lifecycle",
            "Use a new Hermes process for this configuration root. For an existing IM gateway, let its human operator use the installed Hermes lifecycle controls; do not bypass self-restart protection. Plugin resources on disk do not prove loading. Existing conversations may retain their model selection. Compatible older versions remain allowed; >=0.21.7 is recommended for reasoning replay.",
        );
        if status
            .native_adapter
            .as_ref()
            .and_then(|a| a.known_channel_models.as_ref())
            .is_some_and(|models| !models.is_empty())
        {
            add(
                "inspect_native_model_overrides",
                "hermes_im",
                &["read_only"],
                "known_channel_model_overrides",
                "Inspect the selected channel and session model in Hermes. Recorded channel defaults are configuration facts, not proof of the active conversation's selection.",
            );
        }
    } else {
        add(
            "load_native_configuration",
            class.context_segment(),
            &["starts_new_native_process"],
            "saved_configuration; use_selected_native_profile",
            "Start a new supported native process for the selected configuration/profile. Use the existing HiRoute launch descriptor where required. Existing processes may retain previous settings.",
        );
    }
    if status.state == Configured {
        add(
            "select_native_model",
            class.context_segment(),
            &["changes_native_session_model"],
            "configuration_loaded; select_published_model_for_plan",
            "Select the intended published model in the native session. An account default or another preset does not prove that the selected Plan is active.",
        );
        add(
            "observe_native_request",
            class.context_segment(),
            &["model_call_may_incur_cost"],
            "explicit_model_call_consent_or_an_existing_matching_request",
            "After an authorized native request, query effective status for this context and Plan, optionally with its session/request ID. Inspect the recorded outcome, actual model and usage. A provider-only check is insufficient.",
        );
    } else {
        add(
            "confirm_restored_selection",
            class.context_segment(),
            &["changes_native_session_model"],
            "owned_configuration_restored",
            "After loading the restored native configuration, select the intended native model. Restoring files does not change an already running conversation.",
        );
    }
    actions
}

// The released status deliberately calls a completed model restore `not_configured`.
// Its succeeded operation locator distinguishes it from a never-configured target.
fn owned_restore_completed(status: &AgentModelSettingsStatusV2) -> bool {
    status.state == AgentModelSettingsStateV2::NotConfigured
        && status.operation_id.is_some()
        && status.operation_state.as_deref()
            == Some(hiroute_domain::OperationState::Succeeded.as_str())
}
