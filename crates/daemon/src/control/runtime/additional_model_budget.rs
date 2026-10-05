//! Installed native catalog budgets constrain subsequent publication updates.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    AdditionalModelFileAction, settings_additional_model_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentModelRouteV2, ControlRepositoryPort, OperationStepKind, PortError,
    PortErrorCode, PortResult, PublicationRecordV1, SecretStorePort, WorkspaceId,
    is_agent_access_grant_effect,
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
    /// otherwise leave Qoder sending requests under a larger persisted native declaration.
    pub(super) fn validate_additional_installed_model_budgets(
        &self,
        record: &PublicationRecordV1,
    ) -> PortResult<()> {
        let next = record.verify().map_err(|_| invalid())?;
        let stores = self.stores_lock()?;
        for operation in stores.control().succeeded_agent_operations_for_kind(
            &WorkspaceId::default(),
            "ApplyAgentConnectionChange",
        )? {
            let Some(intent) = operation.plan.external().iter().find(|intent| {
                super::native_additional_model::is_settings_additional_model(intent)
            }) else {
                continue;
            };
            let payload = settings_additional_model_file_for_operation(&operation, intent)?;
            let kind = hiroute_application::agent_connection::additional_model_kind(intent)?;
            let AdditionalModelFileAction::Configure { models, .. } = payload.change else {
                continue;
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
                let plan = next
                    .plans
                    .iter()
                    .find(|plan| plan.agent_plan_id() == plan_id)
                    .ok_or_else(shrinking)?
                    .clone()
                    .into_current()
                    .map_err(|_| shrinking())?;
                let budget = (if kind == hiroute_domain::AgentKindV1::Pi {
                    hiroute_integrations::pi_plan_token_budget(&plan.body.materialized)
                } else {
                    hiroute_integrations::qoder_plan_token_budget(&plan.body.materialized)
                })
                .map_err(|_| shrinking())?;
                if budget.context_window_tokens < model.context_window_tokens
                    || budget.max_output_tokens < model.max_output_tokens
                {
                    return Err(shrinking());
                }
            }
        }
        Ok(())
    }
}

/// The caller holds the same store lock through admission, so a file tail cannot
/// become parked between this check and claiming the next Operation.
pub(super) fn guard_pending_model_change(
    control: &dyn ControlRepositoryPort,
    same_operation: Option<&hiroute_domain::OperationId>,
) -> PortResult<()> {
    for operation in control.recoverable_operations()? {
        if same_operation == Some(&operation.operation_id) {
            continue;
        }
        if operation
            .step(OperationStepKind::Activate)
            .terminal_result
            .as_deref()
            .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
            .is_some()
            && operation
                .plan
                .external()
                .iter()
                .any(super::native_additional_model::is_settings_additional_model)
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
fn shrinking() -> PortError {
    PortError::new(PortErrorCode::Conflict, "qoder.model.budget.shrink")
}
