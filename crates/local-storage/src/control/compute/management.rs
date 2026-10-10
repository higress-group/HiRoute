//! SQLite adapter for the source-level compute management aggregate.
//!
//! Writes are intentionally private helpers called by the existing Control effect activation and
//! compensation transaction. Public methods implement only the read port.

use hiroute_domain::{
    CanonicalDigest, ComputeManagementMutationV2, ComputeManagementRepositoryPort,
    ComputeManagementSourceV2, ComputeManagementStoredSnapshotV2, OperationId, PortError,
    PortErrorCode, PortResult, WorkspaceId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::Value;

mod references;

use super::{ControlStore, decode, encode, invalid, port, validate_storage_id};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagementEnvelope {
    compute_management_mutation: ComputeManagementMutationV2,
}

pub(in crate::control) fn stage_control(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    operation_id: &OperationId,
    control: &Value,
) -> PortResult<()> {
    let Some(mutation) = mutation_from_control(control)? else {
        return Ok(());
    };
    let current = read_source(transaction, workspace, mutation.source_id())?;
    mutation
        .validate_against(current.as_ref())
        .map_err(|_| conflict("compute.management.stage.cas"))?;
    references::validate_removal(transaction, workspace, &mutation)?;
    let identity = mutation
        .desired()
        .or(mutation.expected())
        .ok_or_else(|| invalid("compute.management.identity"))?;
    let lineage_owner: Option<String> = transaction
        .query_row(
            "SELECT source_id FROM compute_management_sources
             WHERE workspace_id=?1 AND lineage_digest=?2",
            params![workspace.as_str(), identity.lineage_digest.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.management.stage.lineage"))?;
    if lineage_owner
        .as_deref()
        .is_some_and(|source_id| source_id != mutation.source_id())
    {
        return Err(conflict("compute.management.stage.lineage"));
    }
    let before_owner: Option<String> = transaction
        .query_row(
            "SELECT owner_operation_id FROM compute_management_sources
             WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), mutation.source_id()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.management.stage.owner"))?;
    transaction
        .execute(
            "INSERT INTO compute_management_effects(
                operation_id,workspace_id,source_id,expected_revision,
                before_owner_operation_id,desired_digest
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                operation_id.as_str(),
                workspace.as_str(),
                mutation.source_id(),
                mutation.expected_revision(),
                before_owner,
                mutation.desired_digest().as_str(),
            ],
        )
        .map_err(|_| port("compute.management.stage.effect"))?;
    Ok(())
}

pub(in crate::control) fn activate(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    staged_json: &str,
    operation_id: &str,
) -> PortResult<()> {
    let Some(mutation) = mutation_from_staged(staged_json)? else {
        return Ok(());
    };
    let staged: Option<(String, String, u64, String)> = transaction
        .query_row(
            "SELECT workspace_id,source_id,expected_revision,desired_digest
             FROM compute_management_effects WHERE operation_id=?1",
            params![operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|_| port("compute.management.activate.effect"))?;
    let desired_digest = mutation.desired_digest();
    if staged
        .as_ref()
        .is_none_or(|(staged_workspace, source_id, expected_revision, digest)| {
            staged_workspace != workspace.as_str()
                || source_id != mutation.source_id()
                || *expected_revision != mutation.expected_revision()
                || digest != desired_digest.as_str()
        })
    {
        return Err(corrupt("compute.management.activate.effect_binding"));
    }
    let current = read_source(transaction, workspace, mutation.source_id())?;
    mutation
        .validate_against(current.as_ref())
        .map_err(|_| conflict("compute.management.activate.cas"))?;
    references::validate_removal(transaction, workspace, &mutation)?;
    if let Some(desired) = mutation.desired() {
        write_source(transaction, workspace, desired, operation_id)
    } else {
        let deleted = transaction.execute("DELETE FROM compute_management_sources WHERE workspace_id=?1 AND source_id=?2 AND revision=?3", params![workspace.as_str(), mutation.source_id(), mutation.expected_revision()])
            .map_err(|_| port("compute.management.delete"))?;
        if deleted != 1 {
            return Err(conflict("compute.management.delete.cas"));
        }
        Ok(())
    }
}

pub(in crate::control) fn current_matches(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    staged_json: Option<&str>,
    operation_id: &OperationId,
) -> PortResult<bool> {
    let Some(staged_json) = staged_json else {
        return Ok(true);
    };
    let Some(mutation) = mutation_from_staged(staged_json)? else {
        return Ok(true);
    };
    let effect_bound: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM compute_management_effects
             WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3)",
            params![
                operation_id.as_str(),
                workspace.as_str(),
                mutation.source_id()
            ],
            |row| row.get(0),
        )
        .map_err(|_| port("compute.management.observe.effect"))?;
    if !effect_bound {
        return Err(corrupt("compute.management.observe.effect_binding"));
    }
    let current: Option<(String, String)> = connection
        .query_row(
            "SELECT source_json, owner_operation_id FROM compute_management_sources
             WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), mutation.source_id()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| port("compute.management.observe"))?;
    match mutation.desired() {
        Some(desired) => {
            let desired = encode(desired)?;
            Ok(current
                .is_some_and(|(json, owner)| json == desired && owner == operation_id.as_str()))
        }
        None => {
            Ok(current.is_none() && deletion_owned(connection, workspace, operation_id.as_str())?)
        }
    }
}

pub(in crate::control) fn compensate(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    operation_id: &str,
) -> PortResult<bool> {
    let staged_json: Option<String> = transaction
        .query_row(
            "SELECT staged_json FROM control_effects WHERE operation_id=?1",
            params![operation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.management.compensate.stage"))?
        .flatten();
    let Some(staged_json) = staged_json else {
        return Ok(true);
    };
    let Some(mutation) = mutation_from_staged(&staged_json)? else {
        return Ok(true);
    };
    let current: Option<(String, String)> = transaction
        .query_row(
            "SELECT source_json, owner_operation_id FROM compute_management_sources
             WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), mutation.source_id()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| port("compute.management.compensate.current"))?;
    let owns = match mutation.desired() {
        Some(desired) => {
            let desired = encode(desired)?;
            current.is_some_and(|(json, owner)| json == desired && owner == operation_id)
        }
        None => current.is_none() && deletion_owned(transaction, workspace, operation_id)?,
    };
    if !owns {
        return Ok(false);
    }
    if let Some(expected) = mutation.expected() {
        let before_owner: Option<String> = transaction
            .query_row(
                "SELECT before_owner_operation_id FROM compute_management_effects
                 WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3",
                params![operation_id, workspace.as_str(), mutation.source_id()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port("compute.management.compensate.owner"))?
            .flatten();
        let before_owner =
            before_owner.ok_or_else(|| corrupt("compute.management.compensate.owner_missing"))?;
        write_source(transaction, workspace, expected, &before_owner)?;
    } else {
        transaction
            .execute(
                "DELETE FROM compute_management_sources
                 WHERE workspace_id=?1 AND source_id=?2 AND owner_operation_id=?3",
                params![workspace.as_str(), mutation.source_id(), operation_id],
            )
            .map_err(|_| port("compute.management.compensate.delete"))?;
    }
    Ok(true)
}

impl ComputeManagementRepositoryPort for ControlStore {
    fn compute_management_references(
        &self,
        workspace: &WorkspaceId,
        binding_ids: &[String],
    ) -> PortResult<Vec<String>> {
        references::read(&self.connection.borrow(), workspace, binding_ids)
    }
    fn compute_management_source(
        &self,
        source_id: &str,
    ) -> PortResult<Option<ComputeManagementSourceV2>> {
        validate_storage_id(source_id)?;
        // Management source ids are globally derived from semantic lineage. The default product
        // workspace is the sole current workspace authority.
        read_source(
            &self.connection.borrow(),
            &WorkspaceId::default(),
            source_id,
        )
    }

    fn compute_management_source_by_lineage(
        &self,
        lineage_digest: &CanonicalDigest,
    ) -> PortResult<Option<ComputeManagementSourceV2>> {
        CanonicalDigest::parse(lineage_digest.as_str().to_owned())
            .map_err(|_| invalid("compute.management.lineage"))?;
        let encoded: Option<String> = self
            .connection
            .borrow()
            .query_row(
                "SELECT source_json FROM compute_management_sources
                 WHERE workspace_id=?1 AND lineage_digest=?2",
                params![WorkspaceId::default().as_str(), lineage_digest.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port("compute.management.lineage.read"))?;
        decode_source(encoded, None)
    }

    fn compute_management_snapshot(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<ComputeManagementStoredSnapshotV2> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| port("compute.management.snapshot.begin"))?;
        let revisions = super::super::read_revisions(&transaction, workspace)?;
        let mut statement = transaction
            .prepare(
                "SELECT source_id, source_json FROM compute_management_sources
                 WHERE workspace_id=?1 ORDER BY source_id",
            )
            .map_err(|_| port("compute.management.snapshot.prepare"))?;
        let rows = statement
            .query_map(params![workspace.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| port("compute.management.snapshot.query"))?;
        let mut sources = Vec::new();
        for row in rows {
            let (source_id, encoded) =
                row.map_err(|_| corrupt("compute.management.snapshot.row"))?;
            let source: ComputeManagementSourceV2 = decode(&encoded)?;
            source
                .validate()
                .map_err(|_| corrupt("compute.management.snapshot.validate"))?;
            if source.source_id != source_id {
                return Err(corrupt("compute.management.snapshot.identity"));
            }
            sources.push(source);
        }
        drop(statement);
        transaction
            .commit()
            .map_err(|_| port("compute.management.snapshot.commit"))?;
        Ok(ComputeManagementStoredSnapshotV2 { revisions, sources })
    }
}

/// Absence alone is never a deletion ownership witness. Reuse the same durable workspace
/// revision/digest/owner triple as the outer Control effect, including in helper-level recovery.
fn deletion_owned(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    operation_id: &str,
) -> PortResult<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM workspace_state w JOIN control_effects e ON e.workspace_id=w.workspace_id WHERE w.workspace_id=?1 AND e.operation_id=?2 AND w.owner_operation_id=e.operation_id AND w.target_revision=e.after_revision AND w.desired_digest=e.after_digest AND e.activated=1 AND e.compensated=0)", params![workspace.as_str(), operation_id], |row| row.get(0))
        .map_err(|_| port("compute.management.delete.owner"))
}

fn mutation_from_staged(staged_json: &str) -> PortResult<Option<ComputeManagementMutationV2>> {
    let mut envelope: Value = serde_json::from_str(staged_json)
        .map_err(|_| corrupt("compute.management.stage.decode"))?;
    if envelope.get("schema").and_then(Value::as_str) != Some("hiroute.control-desired/v1") {
        return Err(corrupt("compute.management.stage.schema"));
    }
    let control = envelope
        .as_object_mut()
        .and_then(|object| object.remove("value"))
        .ok_or_else(|| corrupt("compute.management.stage.value"))?;
    mutation_from_control(&control)
}

fn mutation_from_control(control: &Value) -> PortResult<Option<ComputeManagementMutationV2>> {
    if control.get("compute_management_mutation").is_none() {
        return Ok(None);
    }
    let envelope: ManagementEnvelope = serde_json::from_value(control.clone())
        .map_err(|_| invalid("compute.management.control"))?;
    Ok(Some(envelope.compute_management_mutation))
}

fn read_source(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source_id: &str,
) -> PortResult<Option<ComputeManagementSourceV2>> {
    let encoded: Option<String> = connection
        .query_row(
            "SELECT source_json FROM compute_management_sources
             WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), source_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.management.read"))?;
    decode_source(encoded, Some(source_id))
}

fn decode_source(
    encoded: Option<String>,
    expected_source_id: Option<&str>,
) -> PortResult<Option<ComputeManagementSourceV2>> {
    encoded
        .map(|encoded| {
            let source: ComputeManagementSourceV2 = decode(&encoded)?;
            source
                .validate()
                .map_err(|_| corrupt("compute.management.source.validate"))?;
            if expected_source_id.is_some_and(|expected| expected != source.source_id) {
                return Err(corrupt("compute.management.source.identity"));
            }
            Ok(source)
        })
        .transpose()
}

fn write_source(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    source: &ComputeManagementSourceV2,
    operation_id: &str,
) -> PortResult<()> {
    source
        .validate()
        .map_err(|_| invalid("compute.management.write.validate"))?;
    let encoded = encode(source)?;
    transaction
        .execute(
            "INSERT INTO compute_management_sources(
                workspace_id,source_id,lineage_digest,revision,source_json,
                owner_operation_id,updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,unixepoch())
             ON CONFLICT(workspace_id,source_id) DO UPDATE SET
                lineage_digest=excluded.lineage_digest,revision=excluded.revision,
                source_json=excluded.source_json,owner_operation_id=excluded.owner_operation_id,
                updated_at=excluded.updated_at",
            params![
                workspace.as_str(),
                source.source_id,
                source.lineage_digest.as_str(),
                source.revision,
                encoded,
                operation_id,
            ],
        )
        .map_err(|_| port("compute.management.write"))?;
    Ok(())
}

fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}

fn corrupt(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Corrupt, context)
}

#[cfg(test)]
mod tests;
