//! Journal all source memberships and the immutable decision service as one Control effect.
use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    DECISION_SERVICE_OPERATION_SCHEMA_V2, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionServiceOperationV2, DecisionServiceRecord, OperationId, OperationState, OperationV1,
    PortError, PortErrorCode, PortResult, WorkspaceId, register_decision_consumer,
    remove_decision_consumers,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;

use super::compute::management::{read_source, write_source};

mod compensation;
pub(super) use compensation::reconcile as reconcile_compensation;

fn error(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

fn parse_control(value: &Value) -> PortResult<Option<DecisionServiceOperationV2>> {
    let Some(value) = value.get("decision_service_operation_v2") else {
        return Ok(None);
    };
    let operation: DecisionServiceOperationV2 = serde_json::from_value(value.clone())
        .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.operation"))?;
    if operation.schema != DECISION_SERVICE_OPERATION_SCHEMA_V2
        || !operation.input.change.validate()
    {
        return Err(error(PortErrorCode::Corrupt, "decision.v2.operation"));
    }
    Ok(Some(operation))
}

fn parse_staged(staged: &str) -> PortResult<Option<DecisionServiceOperationV2>> {
    let value: Value = serde_json::from_str(staged)
        .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.staged"))?;
    parse_control(
        value
            .get("value")
            .ok_or_else(|| error(PortErrorCode::Corrupt, "decision.v2.staged"))?,
    )
}

fn check_service(
    connection: &Connection,
    workspace: &WorkspaceId,
    operation: &DecisionServiceOperationV2,
) -> PortResult<()> {
    let change = &operation.input.change;
    let actual: u64 = connection.query_row(
        "SELECT COALESCE(MAX(revision),0) FROM decision_services WHERE workspace_id=?1 AND service_id=?2",
        params![workspace.as_str(), change.id], |row| row.get(0),
    ).map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.service_read"))?;
    if actual != change.expected_revision {
        return Err(error(
            PortErrorCode::Conflict,
            "decision.v2.service_revision",
        ));
    }
    if change.service.is_none()
        && super::decision_services::references(connection, workspace, &change.id)?
    {
        return Err(error(PortErrorCode::Conflict, "decision_service.in_use"));
    }
    Ok(())
}

fn check_sources(
    connection: &Connection,
    workspace: &WorkspaceId,
    operation: &DecisionServiceOperationV2,
) -> PortResult<()> {
    let change = &operation.input.change;
    let expected: BTreeMap<_, _> = change
        .expected_source_revisions
        .iter()
        .map(|entry| (entry.source_id.as_str(), entry.revision))
        .collect();
    if expected.len() != operation.sources.len() {
        return Err(error(PortErrorCode::Conflict, "decision.v2.source_closure"));
    }
    let mut seen = BTreeSet::new();
    for mutation in &operation.sources {
        let before = &mutation.before;
        if !seen.insert(before.source_id.as_str())
            || expected.get(before.source_id.as_str()) != Some(&before.revision)
            || read_source(connection, workspace, &before.source_id)?.as_ref() != Some(before)
        {
            return Err(error(
                PortErrorCode::Conflict,
                "decision.v2.source_revision",
            ));
        }
        let planned = match &change.service {
            Some(service) => register_decision_consumer(before, service).map(Some),
            None => remove_decision_consumers(before, &change.id),
        }
        .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.source_transition"))?
        .ok_or_else(|| error(PortErrorCode::Corrupt, "decision.v2.source_transition"))?;
        if planned.after != mutation.after {
            return Err(error(
                PortErrorCode::Corrupt,
                "decision.v2.source_transition",
            ));
        }
    }
    if let Some(service) = &change.service {
        let source = match &service.connection {
            DecisionConnectionV2::Native {
                authentication: DecisionAuthenticationV2::SharedSource { source_id, .. },
                ..
            } => Some(source_id.as_str()),
            _ => None,
        };
        if seen != source.into_iter().collect() {
            return Err(error(PortErrorCode::Conflict, "decision.v2.source_closure"));
        }
    } else {
        // Discover only versions of this service and its indexed memberships.
        // Both views must agree before any source or Secret is changed.
        let mut retained = BTreeSet::new();
        let mut statement = connection.prepare("SELECT service_json FROM decision_services WHERE workspace_id=?1 AND service_id=?2")
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.retained"))?;
        let rows = statement
            .query_map(params![workspace.as_str(), change.id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.retained"))?;
        for row in rows {
            let record: DecisionServiceRecord = serde_json::from_str(
                &row.map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.retained"))?,
            )
            .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.retained"))?;
            if !record.validate() || record.id() != change.id {
                return Err(error(PortErrorCode::Corrupt, "decision.v2.retained"));
            }
            if let DecisionServiceRecord::V2(service) = record
                && let DecisionConnectionV2::Native {
                    authentication: DecisionAuthenticationV2::SharedSource { source_id, .. },
                    ..
                } = service.connection
            {
                retained.insert(source_id);
            }
        }
        let mut statement = connection.prepare("SELECT DISTINCT source_id FROM decision_consumer_runtime WHERE workspace_id=?1 AND service_id=?2")
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.membership"))?;
        let indexed = statement
            .query_map(params![workspace.as_str(), change.id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.membership"))?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.membership"))?;
        let supplied = seen.into_iter().map(str::to_owned).collect::<BTreeSet<_>>();
        if retained != indexed || retained != supplied {
            return Err(error(PortErrorCode::Conflict, "decision.v2.source_closure"));
        }
    }
    Ok(())
}

pub(super) fn stage(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    operation_id: &OperationId,
    desired: &Value,
) -> PortResult<()> {
    let Some(operation) = parse_control(desired)? else {
        return Ok(());
    };
    check_service(transaction, workspace, &operation)?;
    check_sources(transaction, workspace, &operation)?;
    for mutation in &operation.sources {
        let owner: String = transaction.query_row("SELECT owner_operation_id FROM compute_management_sources WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), mutation.before.source_id], |row| row.get(0))
            .map_err(|_| error(PortErrorCode::Conflict, "decision.v2.source_owner"))?;
        transaction.execute("INSERT INTO decision_source_effects(operation_id,workspace_id,source_id,before_owner_operation_id) VALUES(?1,?2,?3,?4)",
            params![operation_id.as_str(), workspace.as_str(), mutation.before.source_id, owner])
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.stage"))?;
    }
    Ok(())
}

pub(super) fn activate(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    staged: &str,
    operation_id: &str,
) -> PortResult<()> {
    let Some(operation) = parse_staged(staged)? else {
        return Ok(());
    };
    check_service(transaction, workspace, &operation)?;
    check_sources(transaction, workspace, &operation)?;
    for mutation in &operation.sources {
        let bound: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM decision_source_effects e JOIN compute_management_sources s USING(workspace_id,source_id) WHERE e.operation_id=?1 AND e.workspace_id=?2 AND e.source_id=?3 AND e.before_owner_operation_id=s.owner_operation_id)",
            params![operation_id, workspace.as_str(), mutation.before.source_id], |row| row.get(0))
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.activate_owner"))?;
        if !bound {
            return Err(error(PortErrorCode::Conflict, "decision.v2.activate_owner"));
        }
    }
    for mutation in &operation.sources {
        write_source(transaction, workspace, &mutation.after, operation_id)?;
    }
    Ok(())
}

pub(super) fn current_matches(
    connection: &Connection,
    workspace: &WorkspaceId,
    staged: Option<&str>,
    operation_id: &OperationId,
) -> PortResult<bool> {
    let Some(staged) = staged else {
        return Ok(true);
    };
    let Some(operation) = parse_staged(staged)? else {
        return Ok(true);
    };
    for mutation in &operation.sources {
        let current: Option<(String, String)> = connection.query_row(
            "SELECT s.source_json,s.owner_operation_id FROM compute_management_sources s JOIN decision_source_effects e USING(workspace_id,source_id) WHERE e.operation_id=?1 AND s.workspace_id=?2 AND s.source_id=?3",
            params![operation_id.as_str(), workspace.as_str(), mutation.after.source_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.observe"))?;
        let expected = serde_json::to_string(&mutation.after)
            .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.observe"))?;
        if !current
            .is_some_and(|(value, owner)| value == expected && owner == operation_id.as_str())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn compensate(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    operation_id: &str,
) -> PortResult<bool> {
    let staged: Option<String> = transaction
        .query_row(
            "SELECT staged_json FROM control_effects WHERE operation_id=?1",
            [operation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.compensate_stage"))?
        .flatten();
    let Some(staged) = staged else {
        return Ok(true);
    };
    let Some(operation) = parse_staged(&staged)? else {
        return Ok(true);
    };
    let parsed = OperationId::parse(operation_id)
        .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.operation_id"))?;
    if !current_matches(transaction, workspace, Some(&staged), &parsed)? {
        return Ok(false);
    }
    for mutation in &operation.sources {
        let owner: String = transaction.query_row("SELECT before_owner_operation_id FROM decision_source_effects WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3",
            params![operation_id, workspace.as_str(), mutation.before.source_id], |row| row.get(0))
            .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.compensate_owner"))?;
        write_source(transaction, workspace, &mutation.before, &owner)?;
    }
    Ok(true)
}

pub(super) fn finish(connection: &Connection, operation: &OperationV1) -> PortResult<()> {
    if operation.state != OperationState::Succeeded {
        return Ok(());
    }
    let staged: Option<String> = connection.query_row("SELECT staged_json FROM control_effects WHERE operation_id=?1 AND activated=1 AND compensated=0", [operation.operation_id.as_str()], |row| row.get(0))
        .optional().map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.finish_stage"))?.flatten();
    let Some(staged) = staged else {
        return Ok(());
    };
    let Some(change) = parse_staged(&staged)? else {
        return Ok(());
    };
    let done: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM decision_service_operations WHERE operation_id=?1)",
            [operation.operation_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.finish_receipt"))?;
    if done {
        return Ok(());
    }
    check_service(connection, &operation.workspace_id, &change)?;
    if !current_matches(
        connection,
        &operation.workspace_id,
        Some(&staged),
        &operation.operation_id,
    )? {
        return Err(error(PortErrorCode::Conflict, "decision.v2.finish_owner"));
    }
    if let Some(service) = &change.input.change.service {
        let encoded = serde_json::to_string(service)
            .map_err(|_| error(PortErrorCode::Corrupt, "decision.v2.finish_encode"))?;
        connection.execute("INSERT INTO decision_services(workspace_id,service_id,revision,service_json) VALUES(?1,?2,?3,?4)",
            params![operation.workspace_id.as_str(), service.id, service.revision, encoded])
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.finish_service"))?;
    } else {
        connection
            .execute(
                "DELETE FROM decision_services WHERE workspace_id=?1 AND service_id=?2",
                params![operation.workspace_id.as_str(), change.input.change.id],
            )
            .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.finish_delete"))?;
    }
    connection
        .execute(
            "INSERT INTO decision_service_operations(operation_id) VALUES(?1)",
            [operation.operation_id.as_str()],
        )
        .map_err(|_| error(PortErrorCode::Unavailable, "decision.v2.finish_receipt"))?;
    Ok(())
}
