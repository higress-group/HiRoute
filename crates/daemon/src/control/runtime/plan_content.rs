//! Short scoped admission barriers span the existing publication Operation without holding the
//! management gate over installer I/O. The ordinary Gateway request path does not take this gate.
use super::LocalControlAdapter;
use hiroute_application::publication::admission::{AdmissionAction, AdmissionSubject};
use hiroute_diagnostics::publication::{PublicationStage, measure};
use hiroute_domain::{
    OperationState, OperationV1, PortError, PortErrorCode, PortResult, PublicationRepositoryPort,
};
use std::collections::BTreeSet;

fn unavailable() -> PortError {
    PortError::new(PortErrorCode::Unavailable, "plan.content.recovery-required")
}

impl LocalControlAdapter {
    pub(super) fn begin_plan_content_activation(&self, operation: &OperationV1) -> PortResult<()> {
        let Some(content) = operation
            .plan
            .plan_content_control()
            .map_err(|_| unavailable())?
        else {
            return Ok(());
        };
        let subjects = BTreeSet::from([AdmissionSubject::Plan(
            content.plan_head.reference.plan_id.clone(),
        )]);
        let mut guard = self
            .plan_admission
            .enter(
                &operation.workspace_id,
                &subjects,
                AdmissionAction::Recovery,
                operation.operation_id.as_str(),
            )
            .map_err(|_| unavailable())?;
        guard.mark_recovery_required().map_err(|_| unavailable())?;
        if operation
            .plan
            .spec()
            .desired_state
            .get("schema")
            .and_then(serde_json::Value::as_str)
            == Some(hiroute_application_api::PLAN_LIFECYCLE_CHANGE_SCHEMA_V1)
        {
            let expected = operation
                .plan
                .external()
                .iter()
                .map(hiroute_domain::routing_publication_record)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| unavailable())?
                .into_iter()
                .flatten()
                .next()
                .ok_or_else(unavailable)?;
            let current = self
                .stores_lock()?
                .control()
                .active_publication(&operation.workspace_id)?;
            if current.as_ref() != Some(&expected) {
                use hiroute_application::control::RoutingFactsPort;
                let change = serde_json::from_value(operation.plan.spec().desired_state.clone())
                    .map_err(|_| unavailable())?;
                let snapshot = self
                    .plan_lifecycle_snapshot(&operation.workspace_id, &change)
                    .map_err(|_| unavailable())?;
                let preview =
                    hiroute_application::routing::preview_plan_lifecycle(&change, &snapshot)
                        .map_err(|_| unavailable())?;
                if preview.change_digest != operation.accepted_digest {
                    return Err(unavailable());
                }
            }
        }
        drop(guard);
        if content
            .before_head
            .as_ref()
            .is_some_and(|before| before.reference == content.plan_head.reference)
        {
            // Head-only lifecycle keeps the existing immutable version and its original owner.
            let version = self
                .stores_lock()?
                .control()
                .lookup_exact_plan_version(&content.plan_head.reference)
                .map_err(|_| unavailable())?;
            if version != content.plan_version {
                return Err(unavailable());
            }
            Ok(())
        } else {
            let diagnostics = self
                .publication_diagnostics
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
            measure(
                &diagnostics,
                PublicationStage::PlanVersionStage,
                Some(operation.operation_id.as_str()),
                None,
                || {
                    self.stores_lock()?
                        .control()
                        .stage_plan_version(operation, &content.plan_version)
                        .map_err(|_| unavailable())
                },
            )
        }
    }

    pub(super) fn finish_plan_content_activation(&self, operation: &OperationV1) -> PortResult<()> {
        let Some(content) = operation
            .plan
            .plan_content_control()
            .map_err(|_| unavailable())?
        else {
            return Ok(());
        };
        if !matches!(
            operation.state,
            OperationState::Succeeded | OperationState::RolledBack
        ) {
            return Ok(());
        }
        let subjects = BTreeSet::from([AdmissionSubject::Plan(
            content.plan_head.reference.plan_id.clone(),
        )]);
        let mut guard = self
            .plan_admission
            .enter(
                &operation.workspace_id,
                &subjects,
                AdmissionAction::Recovery,
                operation.operation_id.as_str(),
            )
            .map_err(|_| unavailable())?;
        let stores = self.stores_lock()?;
        match operation.state {
            OperationState::Succeeded => {
                let current = stores
                    .control()
                    .plan_head(
                        &operation.workspace_id,
                        &content.plan_head.reference.plan_id,
                    )
                    .map_err(|_| unavailable())?;
                // Startup replays immutable succeeded intents in revision order; a later durable
                // head already covers this earlier intent and must never be rolled backwards.
                if !current
                    .as_ref()
                    .is_some_and(|h| h.head_revision > content.plan_head.head_revision)
                {
                    let diagnostics = self
                        .publication_diagnostics
                        .lock()
                        .map(|p| p.clone())
                        .unwrap_or_default();
                    measure(
                        &diagnostics,
                        PublicationStage::PlanHeadCommit,
                        Some(operation.operation_id.as_str()),
                        None,
                        || {
                            stores
                                .control()
                                .commit_plan_head(
                                    operation,
                                    &content.plan_head,
                                    content.before_head.as_ref().map(|h| h.head_revision),
                                )
                                .map_err(|_| unavailable())
                        },
                    )?;
                }
            }
            OperationState::RolledBack => stores
                .control()
                .discard_prepared_plan_version(&operation.operation_id)
                .map_err(|_| unavailable())?,
            _ => unreachable!(),
        }
        drop(stores);
        guard.complete_recovery().map_err(|_| unavailable())
    }

    pub(super) fn reconcile_pending_operation_finalization(&self) -> PortResult<()> {
        let mut operations = self
            .stores_lock()?
            .control()
            .pending_finalization_operations()?;
        operations.sort_by_key(|operation| {
            operation
                .plan
                .control()
                .get("plan_head")
                .and_then(|head| head.get("head_revision"))
                .and_then(|revision| revision.as_u64())
                .unwrap_or(0)
        });
        for operation in operations {
            <Self as hiroute_domain::ExternalEffectPort>::finish_publication_activation(
                self, &operation,
            )?;
            self.stores_lock()?
                .control()
                .finish_operation_finalization(&operation)?;
        }
        Ok(())
    }
}
