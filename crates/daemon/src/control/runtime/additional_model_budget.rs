//! Installed native catalog budgets constrain subsequent publication updates.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    AdditionalModelFileAction, settings_additional_model_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentModelBudgetBlockerV1, AgentModelBudgetConflictV1,
    AgentModelBudgetReasonV1, AgentModelRouteV2, AgentModelTokenBudgetV1, ControlRepositoryPort,
    ModelAlias, OperationStepKind, PortError, PortErrorCode, PortResult, PublicationRecordV1,
    SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};

impl LocalControlAdapter {
    /// A completed service segment still owns its native tail until exact recovery finishes.
    /// Do not reinterpret a pending generation as an unconfigured native catalog.
    pub(super) fn guard_additional_pending_model_change(
        &self,
        same_operation: Option<&hiroute_domain::OperationId>,
    ) -> PortResult<()> {
        guard_pending_model_change(self.stores_lock()?.control(), same_operation)
    }

    /// Run at admission and again before sealing Install. A published smaller budget would
    /// otherwise leave installed clients sending requests under a larger persisted native declaration.
    pub(super) fn validate_additional_installed_model_budgets(
        &self,
        record: &PublicationRecordV1,
    ) -> PortResult<()> {
        let next = record.verify().map_err(|_| invalid())?;
        let stores = self.stores_lock()?;
        let mut blockers = Vec::new();
        for operation in stores
            .control()
            .current_agent_configuration_operations(&WorkspaceId::default())?
        {
            let Some(intent) = operation.plan.external().iter().find(|intent| {
                super::native_additional_model::is_settings_additional_model(intent)
                    || hiroute_application::agent_connection::is_settings_hermes_model(intent)
            }) else {
                continue;
            };
            let (context_id, kind, models) =
                if hiroute_application::agent_connection::is_settings_hermes_model(intent) {
                    let payload = hiroute_application::agent_connection::settings_hermes_model_file_for_operation(&operation, intent)?;
                    let hiroute_application::agent_connection::HermesModelFileAction::Configure {
                        model,
                        ..
                    } = payload.change
                    else {
                        continue;
                    };
                    (
                        payload.context_id,
                        hiroute_domain::AgentKindV1::Hermes,
                        vec![model],
                    )
                } else {
                    let payload = settings_additional_model_file_for_operation(&operation, intent)?;
                    let kind =
                        hiroute_application::agent_connection::additional_model_kind(intent)?;
                    let AdditionalModelFileAction::Configure { models, .. } = payload.change else {
                        continue;
                    };
                    (payload.context_id, kind, models)
                };
            let [mutation] = operation.plan.agent_access_grants() else {
                return Err(invalid());
            };
            let effect = operation
                .step(OperationStepKind::ApplySecrets)
                .effects
                .iter()
                .find(|effect| is_agent_access_grant_effect(effect))
                .ok_or_else(invalid)?;
            let owned = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                .map_err(|_| invalid())?;
            let active = stores
                .secrets()
                .inspect_agent_access_grant(WorkspaceId::DEFAULT, owned.scope().connection_id())?;
            if active.as_ref() != Some(&owned) {
                continue;
            }
            // A Restore publication explicitly removes this grant. Its file was restored
            // before service withdrawal, so its old declaration no longer constrains plans.
            if !next.grants.iter().any(|grant| {
                grant.grant_id == owned.grant_id() && grant.generation == owned.generation()
            }) {
                continue;
            }
            for model in models {
                let Some(AgentModelRouteV2::Plan { plan_id, .. }) =
                    owned.scope().model_grant().routes.get(&model.alias)
                else {
                    return Err(invalid());
                };
                let declared = AgentModelTokenBudgetV1 {
                    context_window_tokens: model.context_window_tokens,
                    max_output_tokens: model.max_output_tokens,
                };
                let projection = next
                    .plans
                    .iter()
                    .find(|plan| plan.agent_plan_id() == plan_id)
                    .ok_or(AgentModelBudgetReasonV1::PlanMissing)
                    .and_then(|plan| {
                        let plan = plan
                            .clone()
                            .into_current()
                            .map_err(|_| AgentModelBudgetReasonV1::BudgetUnavailable)?;
                        let budget = if matches!(
                            kind,
                            hiroute_domain::AgentKindV1::Pi
                                | hiroute_domain::AgentKindV1::DeepseekHarness
                                | hiroute_domain::AgentKindV1::Hermes
                        ) {
                            hiroute_integrations::pi_plan_token_budget(&plan.body.materialized)
                        } else {
                            hiroute_integrations::qoder_plan_token_budget(&plan.body.materialized)
                        }
                        .map_err(|_| AgentModelBudgetReasonV1::BudgetUnavailable)?;
                        Ok(AgentModelTokenBudgetV1 {
                            context_window_tokens: budget.context_window_tokens,
                            max_output_tokens: budget.max_output_tokens,
                        })
                    });
                let (reason, proposed) = match projection {
                    Ok(proposed)
                        if proposed.context_window_tokens >= declared.context_window_tokens
                            && (kind == hiroute_domain::AgentKindV1::Hermes
                                || proposed.max_output_tokens >= declared.max_output_tokens) =>
                    {
                        continue;
                    }
                    Ok(proposed) => (AgentModelBudgetReasonV1::CapacityReduced, Some(proposed)),
                    Err(reason) => (reason, None),
                };
                blockers.push(AgentModelBudgetBlockerV1 {
                    agent_kind: kind,
                    context_id: context_id.clone(),
                    plan_id: plan_id.clone(),
                    model_alias: ModelAlias::parse(model.alias).map_err(|_| invalid())?,
                    reason,
                    declared,
                    proposed,
                });
            }
        }
        if blockers.is_empty() {
            return Ok(());
        }
        let conflict = AgentModelBudgetConflictV1::new(blockers);
        if !conflict.is_valid() {
            return Err(invalid());
        }
        let context = if conflict.safe_code() == "QODER_MODEL_BUDGET_CONFLICT" {
            "qoder.model.budget.shrink"
        } else {
            "agent.model.budget.conflict"
        };
        Err(PortError::new(PortErrorCode::Conflict, context).with_model_budget_conflict(conflict))
    }
}

/// The caller holds the same store lock through admission, so a file tail cannot
/// become parked between this check and claiming the next Operation.
pub(super) fn guard_pending_model_change(
    control: &dyn ControlRepositoryPort,
    same_operation: Option<&hiroute_domain::OperationId>,
) -> PortResult<()> {
    check_pending_model_change(&control.recoverable_operations()?, same_operation)
}

pub(super) fn check_pending_model_change(
    operations: &[hiroute_domain::OperationV1],
    same_operation: Option<&hiroute_domain::OperationId>,
) -> PortResult<()> {
    for operation in operations {
        if same_operation == Some(&operation.operation_id) {
            continue;
        }
        if operation
            .step(OperationStepKind::Activate)
            .terminal_result
            .as_deref()
            .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
            .is_some()
            && operation.plan.external().iter().any(|intent| {
                super::native_additional_model::is_settings_additional_model(intent)
                    || hiroute_application::agent_connection::is_settings_hermes_model(intent)
            })
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "qoder.pending_file_tail.change",
            ));
        }
    }
    Ok(())
}
fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "qoder.model.budget.binding")
}
