//! Client recovery references read metadata; executable recovery still loads the full Operation.
use super::*;
use hiroute_domain::OperationStatus;

impl ControlStore {
    pub fn operation_status_for_idempotency(
        &self,
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
    ) -> PortResult<Option<OperationStatus>> {
        let connection = self.connection.borrow();
        let row = connection.query_row(
            "SELECT operation_id, state, generation, request_digest, accepted_change_digest,
                    json_extract(operation_json, '$.safe_error_code'),
                    (SELECT json_group_array(json_extract(value, '$.terminal_result'))
                     FROM json_each(operation_json, '$.steps'))
             FROM operations WHERE workspace_id=?1 AND principal=?2 AND operation_kind=?3 AND idempotency_key=?4",
            params![workspace.as_str(), scope.principal, scope.operation_kind, scope.key],
            |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,u64>(2)?,
                row.get::<_,String>(3)?, row.get::<_,String>(4)?, row.get::<_,Option<String>>(5)?, row.get::<_,String>(6)?)),
        ).optional().map_err(|_| port(PortErrorCode::Corrupt, "control.operation.status_read"))?;
        let Some((id, state, generation, request, accepted, safe_error_code, terminal_results)) =
            row
        else {
            return Ok(None);
        };
        let invalid = || port(PortErrorCode::Corrupt, "control.operation.status_identity");
        let operation_id = OperationId::parse(id).map_err(|_| invalid())?;
        let request = CanonicalDigest::parse(request).map_err(|_| invalid())?;
        if operation_id != OperationId::derive(workspace, scope, &request) {
            return Err(invalid());
        }
        let state = serde_json::from_value(Value::String(state)).map_err(|_| invalid())?;
        let accepted_digest = CanonicalDigest::parse(accepted).map_err(|_| invalid())?;
        let model_budget_conflict = if matches!(
            safe_error_code.as_deref(),
            Some("QODER_MODEL_BUDGET_CONFLICT" | "AGENT_MODEL_BUDGET_CONFLICT")
        ) {
            let results: Vec<Option<String>> =
                serde_json::from_str(&terminal_results).map_err(|_| invalid())?;
            results
                .iter()
                .flatten()
                .find_map(|result| hiroute_domain::AgentModelBudgetConflictV1::parse(result))
        } else {
            None
        };
        let results: Vec<Option<String>> =
            serde_json::from_str(&terminal_results).map_err(|_| invalid())?;
        let failure_detail = results
            .iter()
            .flatten()
            .find_map(|result| hiroute_domain::OperationFailureV1::parse(result));
        Ok(Some(OperationStatus {
            failure_detail,
            model_budget_conflict,
            operation_id,
            state,
            generation,
            accepted_digest,
            safe_error_code,
        }))
    }
}
