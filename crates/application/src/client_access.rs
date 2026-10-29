//! Thin read-only client projections. Admission and business planning remain in their owners.
use crate::control::ControlReadError;
use crate::{ApplicationService, failed, map_control_error, succeeded};
use hiroute_application_api::*;
use hiroute_domain::{IdempotencyScopeV1, WorkspaceId};
use serde_json::Value;
mod editor_options;
mod plan_catalog;

pub trait ClientAccessPort: Send + Sync {
    fn service_status(&self) -> Result<ClientServiceStatusV1, ControlReadError>;
    fn plan_catalog(&self) -> Result<AgentPlanCatalogViewV2, ControlReadError>;
    fn operation_status_for_idempotency(
        &self,
        principal: PrincipalKind,
        operation_kind: &str,
        key: &str,
    ) -> Result<Option<hiroute_domain::OperationStatus>, ControlReadError>;
}

pub(crate) fn dispatch(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Some(ports) = &service.ports else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if request.operation_id == "FindOperationByIdempotency" {
        let lookup = match serde_json::from_value::<OperationIdempotencyLookupV1>(request.payload) {
            Ok(lookup)
                if !lookup.principal_kind.is_collaboration()
                    && recoverable_operation_kind(&lookup.operation_kind)
                    && IdempotencyScopeV1::new(
                        "lookup",
                        &lookup.operation_kind,
                        &lookup.idempotency_key,
                    )
                    .is_ok() =>
            {
                lookup
            }
            _ => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
        let Some(client) = &ports.client_access else {
            return failed(ErrorCode::NotImplemented, request.request_id);
        };
        return match client.operation_status_for_idempotency(
            lookup.principal_kind,
            &lookup.operation_kind,
            &lookup.idempotency_key,
        ) {
            Ok(operation) => {
                let digest_matches = operation
                    .as_ref()
                    .is_none_or(|op| op.accepted_digest == lookup.accepted_digest);
                let operation = operation.map(|op| ClientOperationViewV1 {
                    model_budget_conflict: if lookup.include_model_budget_conflict {
                        op.model_budget_conflict
                    } else {
                        None
                    },
                    operation_id: op.operation_id.to_string(),
                    state: op.state.as_str().into(),
                    sequence: op.generation,
                    cancellable: false,
                    accepted_digest: op.accepted_digest,
                    safe_error_code: op.safe_error_code,
                });
                // A lookup never grants authority. A mismatch returns the original safe reference
                // explicitly, allowing recovery without submitting a fabricated Apply grant.
                succeeded(
                    OperationIdempotencyResultV1 {
                        operation,
                        digest_matches,
                    },
                    request.request_id,
                )
            }
            Err(error) => failed(map_control_error(error), request.request_id),
        };
    }
    if request.operation_id == GET_TRIAL_SETUP_SNAPSHOT_OPERATION_V1 {
        if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let result = (|| {
            let port = ports.routing.as_ref().ok_or(ErrorCode::DaemonUnavailable)?;
            let capture = port
                .trial_routing_snapshot(&WorkspaceId::default())
                .map_err(map_control_error)?;
            let options = editor_options::build_options(
                ports,
                PlanEditorOptionsRequestV1 {
                    authoring: Some(PlanAuthoringRequestV1::default()),
                    include_unavailable: true,
                    ..Default::default()
                },
                capture.routing,
            )?;
            Ok(TrialSetupSnapshotV1 {
                schema: TRIAL_SETUP_SNAPSHOT_SCHEMA_V1.into(),
                options,
                management: capture.management,
            })
        })();
        return match result {
            Ok(value) => succeeded(value, request.request_id),
            Err(error) => failed(error, request.request_id),
        };
    }
    if request.operation_id == "GetPlanEditorOptions" {
        return editor_options::dispatch(ports, request);
    }
    let Some(client) = &ports.client_access else {
        return failed(ErrorCode::NotImplemented, request.request_id);
    };
    match request.operation_id.as_str() {
        "GetClientServiceStatus" => {
            if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
                return failed(ErrorCode::InvalidArguments, request.request_id);
            }
            match client.service_status() {
                Ok(status) => succeeded(status, request.request_id),
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        }
        "ListAgentPlanCatalog" => {
            let page = match serde_json::from_value::<AgentPlanCatalogQueryV2>(request.payload) {
                Ok(page) if (1..=128).contains(&page.limit) => page,
                _ => return failed(ErrorCode::InvalidArguments, request.request_id),
            };
            match client.plan_catalog() {
                Ok(catalog) => match plan_catalog::paginate(catalog, page) {
                    Ok(page) => succeeded(page, request.request_id),
                    Err(error) => failed(error, request.request_id),
                },
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        }
        "GetAgentPlanStatus" => {
            let lookup = match serde_json::from_value::<AgentPlanLookupV1>(request.payload) {
                Ok(lookup) if AgentPlanId::parse(lookup.agent_plan_id.as_str()).is_ok() => lookup,
                _ => return failed(ErrorCode::InvalidArguments, request.request_id),
            };
            match client.plan_catalog() {
                Ok(catalog) => match catalog
                    .plans
                    .into_iter()
                    .find(|p| p.agent_plan_id == lookup.agent_plan_id)
                {
                    Some(plan) => succeeded(plan, request.request_id),
                    None => failed(ErrorCode::ResourceNotFound, request.request_id),
                },
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        }
        _ => failed(ErrorCode::UnknownCommand, request.request_id),
    }
}

fn recoverable_operation_kind(operation_kind: &str) -> bool {
    matches!(
        operation_kind,
        APPLY_COMPUTE_SAVE_OPERATION_V2 | APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
    ) || command_by_operation(operation_kind)
        .is_some_and(|command| matches!(command.kind, CommandKind::Apply | CommandKind::Action))
}

#[cfg(test)]
mod tests {
    use super::recoverable_operation_kind;
    use hiroute_application_api::APPLY_SUBSCRIPTION_CHECK_OPERATION_V2;

    #[test]
    fn subscription_approval_operation_can_be_recovered_by_idempotency() {
        assert!(recoverable_operation_kind(
            APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
        ));
        assert!(!recoverable_operation_kind("PreviewSubscriptionCheck"));
    }
}
