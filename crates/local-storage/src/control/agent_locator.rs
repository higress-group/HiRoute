//! Durable discovery hints for current Agent settings. These are never authority.
//!
//! Coverage, members and heads are installed together. A lookup must authenticate
//! the actual selected journal and its current dependencies before returning status.
use super::{agent_history_cache::AgentHistoryRow, agent_locator_input, *};
use agent_locator_input::{FacetAction, LocatorInput};

const AGENT_KINDS: &[&str] = &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"];

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-locator.coverage")
}

fn action(value: Option<FacetAction>) -> Option<&'static str> {
    match value {
        None => None,
        Some(FacetAction::Configure) => Some("configure"),
        Some(FacetAction::Restore) => Some("restore"),
    }
}

fn save_head(
    db: &Connection,
    workspace: &str,
    scope: &str,
    context: &str,
    key: &str,
    ordinal: i64,
    action: &str,
) -> PortResult<()> {
    db.execute(
        "INSERT INTO agent_locator_heads(workspace_id,scope,context_id,grant_key,ordinal,action)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(workspace_id,scope,context_id,grant_key) DO UPDATE SET
             ordinal=excluded.ordinal,action=excluded.action
         WHERE excluded.ordinal>agent_locator_heads.ordinal",
        params![workspace, scope, context, key, ordinal, action],
    )
    .map_err(|_| invalid())?;
    Ok(())
}

fn grant_key(grant: &hiroute_domain::AgentAccessGrantRefV1) -> PortResult<String> {
    grant.validate().map_err(|_| invalid())?;
    CanonicalDigest::of(grant)
        .map(|digest| digest.to_string())
        .map_err(|_| invalid())
}

fn save_settings(
    db: &Connection,
    ordinal: i64,
    row: &AgentHistoryRow,
    input: &LocatorInput,
) -> PortResult<()> {
    let grant = if input.additional_model_grant_candidate {
        match agent_locator_input::grant_reference(db, row, input) {
            Ok(reference) => Some(reference),
            Err(error)
                if matches!(
                    error.code,
                    PortErrorCode::Corrupt | PortErrorCode::InvalidData
                ) =>
            {
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let grant_unresolved = input.additional_model_grant_candidate && grant.is_none();
    let grant_json = grant
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid())?;
    db.execute(
        "INSERT INTO agent_locator_members
         (ordinal,operation_id,workspace_id,context_id,generation,accepted_digest,plan_digest,
          model_action,collaboration_action,management_action,resident_declaration,grant_ref_json,
          grant_unresolved,classification)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'settings')",
        params![
            ordinal,
            input.operation_id.as_str(),
            input.workspace_id.as_str(),
            input.context,
            input.generation,
            input.accepted_digest.as_str(),
            input.plan_bytes_digest.as_str(),
            action(input.model),
            action(input.collaboration),
            action(input.management),
            input.resident_service_declaration,
            grant_json,
            grant_unresolved
        ],
    )
    .map_err(|_| invalid())?;
    for (scope, changed) in [
        ("model", input.model),
        ("collaboration", input.collaboration),
        ("management", input.management),
    ] {
        if let Some(changed) = action(changed) {
            save_head(
                db,
                input.workspace_id.as_str(),
                scope,
                &input.context,
                "",
                ordinal,
                changed,
            )?;
        }
    }
    if input.resident_service_declaration {
        save_head(
            db,
            input.workspace_id.as_str(),
            "resident",
            "",
            "",
            ordinal,
            "declaration",
        )?;
    }
    if let Some(grant) = grant {
        save_head(
            db,
            input.workspace_id.as_str(),
            "grant",
            &input.context,
            &grant_key(&grant)?,
            ordinal,
            "grant",
        )?;
    }
    Ok(())
}

fn save_unresolved(db: &Connection, ordinal: i64, row: Option<&AgentHistoryRow>) -> PortResult<()> {
    // The SQL workspace is usable as a fence only when the retained envelope binds it.
    let workspace = row
        .filter(|row| agent_locator_input::has_bound_header(row))
        .map(|row| row.workspace_id.as_str());
    let operation = row
        .filter(|_| workspace.is_some())
        .map(|row| row.operation_id.as_str());
    db.execute(
        "INSERT INTO agent_locator_members(ordinal,operation_id,workspace_id,resident_declaration,grant_unresolved,classification)
         VALUES(?1,?2,?3,0,0,'unresolved')",
        params![ordinal, operation, workspace],
    ).map_err(|_| invalid())?;
    Ok(())
}

fn save_member(db: &Connection, ordinal: i64, row: Option<AgentHistoryRow>) -> PortResult<()> {
    let Some(row) = row else {
        return save_unresolved(db, ordinal, None);
    };
    if row.state != "succeeded" || !AGENT_KINDS.contains(&row.operation_kind.as_str()) {
        return save_unresolved(db, ordinal, Some(&row));
    }
    match agent_locator_input::classify(db, &row) {
        Ok(Some(input)) => save_settings(db, ordinal, &row, &input),
        Ok(None) => {
            db.execute("INSERT INTO agent_locator_members(ordinal,operation_id,workspace_id,generation,accepted_digest,resident_declaration,grant_unresolved,classification)
                VALUES(?1,?2,?3,?4,?5,0,0,'other')", params![ordinal, row.operation_id, row.workspace_id, row.generation, row.accepted_digest]).map_err(|_| invalid())?;
            Ok(())
        }
        Err(error)
            if matches!(
                error.code,
                PortErrorCode::Corrupt | PortErrorCode::InvalidData
            ) =>
        {
            save_unresolved(db, ordinal, Some(&row))
        }
        Err(error) => Err(error),
    }
}

/// Called only as part of the existing Control upgrade transaction, after v32 has
/// admitted its source. Optional effects are not expanded except exact Ensure receipts.
pub(crate) fn initialize(db: &Connection) -> Result<(), LocalStorageError> {
    // A version number alone cannot establish the admitted v32 dependency coverage.
    let pending = super::current_dependencies::validate_recovery_members(db)
        .map_err(|_| LocalStorageError::InvalidData)?;
    super::native_targets::validate_coverage(db).map_err(|_| LocalStorageError::InvalidData)?;
    super::artifact_heads::validate_coverage(db).map_err(|_| LocalStorageError::InvalidData)?;
    crate::migrations::routing_read_v32::validate(db)
        .map_err(|_| LocalStorageError::InvalidData)?;
    let mut query = db.prepare(
        "SELECT rowid FROM operations
         WHERE (state='succeeded' AND operation_kind IN ('ApplyAgentConnectionChange','ApplyAgentConnectionRestore'))
            OR CASE WHEN json_valid(operation_json) THEN
                json_extract(operation_json,'$.operation_id') IS NOT operation_id
                OR json_extract(operation_json,'$.workspace_id') IS NOT workspace_id
                OR json_extract(operation_json,'$.idempotency.principal') IS NOT principal
                OR json_extract(operation_json,'$.idempotency.operation_kind') IS NOT operation_kind
                OR json_extract(operation_json,'$.idempotency.key') IS NOT idempotency_key
                OR json_extract(operation_json,'$.request_digest') IS NOT request_digest
                OR json_extract(operation_json,'$.accepted_digest') IS NOT accepted_change_digest
                OR json_extract(operation_json,'$.state') IS NOT state
                OR json_extract(operation_json,'$.generation') IS NOT generation
                ELSE 1 END
         ORDER BY rowid"
    )?;
    let ordinals = query
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(query);
    let mut selected = db.prepare("SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE rowid=?1")?;
    for ordinal in ordinals {
        let row = match selected.query_row([ordinal], AgentHistoryRow::read) {
            Ok(row) => Some(row),
            Err(
                rusqlite::Error::FromSqlConversionFailure(..)
                | rusqlite::Error::IntegralValueOutOfRange(..)
                | rusqlite::Error::InvalidColumnType(..)
                | rusqlite::Error::Utf8Error(..),
            ) => None,
            Err(error) => return Err(error.into()),
        };
        save_member(db, ordinal, row).map_err(|_| LocalStorageError::InvalidData)?;
    }
    drop(selected);
    for id in pending {
        super::agent_locator_pending::initialize_member(db, &id)
            .map_err(|_| LocalStorageError::InvalidData)?;
    }
    db.execute_batch(crate::migrations::agent_locator_v33::TRIGGERS)?;
    let schema: i64 = db.query_row("PRAGMA main.schema_version", [], |r| r.get(0))?;
    let inserted = db.execute("INSERT INTO agent_locator_coverage(singleton,format_version,store_uuid,schema_version,member_count,pending_count,head_count,tampered)
        SELECT 1,1,store_uuid,?1,(SELECT count(*) FROM agent_locator_members),(SELECT count(*) FROM agent_locator_pending),(SELECT count(*) FROM agent_locator_heads),0
        FROM storage_meta WHERE singleton=1", [schema])?;
    if inserted != 1 {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

/// A missing marker, missing producer guard, schema change or projection mutation
/// is damage; none of these is evidence that a context has no saved settings.
pub(super) fn validate_coverage(db: &Connection) -> PortResult<()> {
    let temporary_shadow: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE
           name IN ('operations','operation_steps','operation_payloads','storage_meta',
                    'operation_recovery_members','operation_recovery_coverage','writer_claim',
                    'agent_locator_coverage','agent_locator_members','agent_locator_pending','agent_locator_heads','agent_locator_dirty')
           OR (type='trigger' AND tbl_name IN ('operations','operation_steps','operation_payloads',
                    'operation_recovery_members','operation_recovery_coverage','writer_claim',
                    'agent_locator_coverage','agent_locator_members','agent_locator_pending','agent_locator_heads','agent_locator_dirty'))) ",
        [], |row| row.get(0),
    ).map_err(|_| invalid())?;
    if temporary_shadow {
        return Err(invalid());
    }
    for definition in crate::migrations::agent_locator_v33::TRIGGERS
        .split("CREATE TRIGGER ")
        .skip(1)
    {
        let (name, _) = definition.split_once(' ').ok_or_else(invalid)?;
        let expected = format!("CREATE TRIGGER {}", definition.trim().trim_end_matches(';'));
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE type='trigger' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .map_err(|_| invalid())?;
        if actual != expected {
            return Err(invalid());
        }
    }
    let schema: i64 = db
        .query_row("PRAGMA main.schema_version", [], |row| row.get(0))
        .map_err(|_| invalid())?;
    let complete: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_locator_coverage c JOIN storage_meta s
           ON s.singleton=c.singleton AND s.store_uuid=c.store_uuid
         WHERE c.singleton=1 AND c.format_version=1 AND c.schema_version=?1 AND c.tampered=0
           AND c.member_count=(SELECT count(*) FROM agent_locator_members NOT INDEXED)
           AND c.pending_count=(SELECT count(*) FROM agent_locator_pending NOT INDEXED)
           AND c.pending_count=(SELECT pending_count FROM operation_recovery_coverage WHERE singleton=1)
           AND NOT EXISTS(SELECT 1 FROM operation_recovery_members r LEFT JOIN agent_locator_pending p USING(operation_id) WHERE p.operation_id IS NULL)
           AND NOT EXISTS(SELECT 1 FROM agent_locator_pending p LEFT JOIN operation_recovery_members r USING(operation_id) WHERE r.operation_id IS NULL)
           AND NOT EXISTS(SELECT 1 FROM operation_recovery_members r LEFT JOIN operations o USING(operation_id)
               WHERE o.operation_id IS NULL OR (o.state IN ('succeeded','rolled_back') AND o.finalization_pending=0))
           AND NOT EXISTS(SELECT 1 FROM writer_claim w LEFT JOIN operation_recovery_members r USING(operation_id) WHERE r.operation_id IS NULL)
           AND c.head_count=(SELECT count(*) FROM agent_locator_heads NOT INDEXED))",
            [schema],
            |row| row.get(0),
        )
        .map_err(|_| invalid())?;
    if complete { Ok(()) } else { Err(invalid()) }
}

/// A producer owns only events created after this point in its write transaction.
/// When coverage was already damaged, leave it damaged and keep the new events;
/// unrelated writes must not silently repair or erase the earlier uncertainty.
pub(super) fn begin_write(db: &Connection) -> Option<i64> {
    validate_coverage(db).ok()?;
    db.query_row(
        "SELECT coalesce(max(event_id),0) FROM agent_locator_dirty",
        [],
        |r| r.get(0),
    )
    .ok()
}

pub(super) fn finish_write(
    db: &Connection,
    operation: &OperationV1,
    before: Option<i64>,
    publish_terminal: bool,
) -> PortResult<()> {
    let Some(before) = before else {
        return Ok(());
    };
    let id = operation.operation_id.as_str();
    let old_unknown: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_locator_dirty WHERE operation_id=?1 AND event_id<=?2)",
        params![id, before], |r| r.get(0),
    ).map_err(|_| invalid())?;
    if old_unknown {
        return Ok(());
    }
    if operation.state == OperationState::Succeeded
        && AGENT_KINDS.contains(&operation.idempotency.operation_kind.as_str())
    {
        let previous: Option<(Option<u64>, Option<String>)> = db.query_row(
            "SELECT generation,accepted_digest FROM agent_locator_members WHERE operation_id=?1",
            [id], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|_| invalid())?;
        let current: (u64, String) = db
            .query_row(
                "SELECT generation,accepted_change_digest FROM operations WHERE operation_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| invalid())?;
        if let Some(previous) = &previous
            && (previous.0 != Some(current.0) || previous.1.as_deref() != Some(current.1.as_str()))
        {
            // Never let a later write erase uncertainty about an existing member.
            return Ok(());
        }
        if previous.is_none() {
            if !publish_terminal {
                // A success outside terminal commit is not an authoritative new head.
                return Ok(());
            }
            let ordinal: i64 = db
                .query_row(
                    "SELECT rowid FROM operations WHERE operation_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|_| invalid())?;
            let row = db.query_row(
                "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE rowid=?1",
                [ordinal], AgentHistoryRow::read,
            ).map_err(|_| invalid())?;
            save_member(db, ordinal, Some(row))?;
        }
    }
    if !super::agent_locator_pending::sync_member(db, id)? {
        return Ok(());
    }
    // No other connection can interleave while this transaction owns SQLite's
    // writer lock. The delete triggers flag our own mutation until this final update.
    db.execute(
        "DELETE FROM agent_locator_dirty WHERE operation_id=?1 AND event_id>?2",
        params![id, before],
    )
    .map_err(|_| invalid())?;
    db.execute("UPDATE agent_locator_coverage SET member_count=(SELECT count(*) FROM agent_locator_members NOT INDEXED),
        pending_count=(SELECT count(*) FROM agent_locator_pending NOT INDEXED),
        head_count=(SELECT count(*) FROM agent_locator_heads NOT INDEXED),tampered=0 WHERE singleton=1", []).map_err(|_| invalid())?;
    Ok(())
}
