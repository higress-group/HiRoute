//! Complete unfinished work and exact grant-proof references, not a best-effort history cache.
use super::*;
use crate::LocalStorageError;
use hiroute_domain::{GatewayPublicationV1, PublicationRecordV1};

pub(crate) fn initialize(tx: &Connection) -> Result<(), LocalStorageError> {
    tx.execute("INSERT INTO artifact_head_coverage VALUES (1,1,(SELECT store_uuid FROM storage_meta WHERE singleton=1),0)", [])?;
    tx.execute(
        "INSERT INTO agent_native_target_coverage VALUES
        (1,1,(SELECT store_uuid FROM storage_meta WHERE singleton=1),0)",
        [],
    )?;
    // The legacy format had no durable finalize bit. Visit it once, under the upgrade
    // transaction, before publishing coverage. In particular, Succeeded does not prove
    // that a Plan head was committed by the old post-terminal callback.
    let mut query = tx.prepare("SELECT operation_json FROM operations ORDER BY rowid")?;
    let rows = query.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let operation = decode_operation(&Default::default(), &row?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        if !journal::is_current(tx, &operation).map_err(|_| LocalStorageError::InvalidData)? {
            return Err(LocalStorageError::InvalidData);
        }
        super::native_targets::remember(tx, &operation)
            .map_err(|_| LocalStorageError::InvalidData)?;
        super::artifact_heads::remember_legacy(tx, &operation)
            .map_err(|_| LocalStorageError::InvalidData)?;
        let finalization = plans::legacy_plan_finalization_required(tx, &operation)
            .map_err(|_| LocalStorageError::InvalidData)?;
        tx.execute(
            "UPDATE operations SET finalization_pending=?2 WHERE operation_id=?1",
            params![operation.operation_id.as_str(), finalization],
        )?;
    }
    drop(query);
    // Only unresolved work still needs a compare-and-swap basis. Terminal callbacks
    // use the already committed owner projection, never regenerate it after restart.
    let mut query = tx.prepare("SELECT operation_json FROM operations WHERE state NOT IN ('succeeded','rolled_back') ORDER BY rowid")?;
    for row in query.query_map([], |r| r.get::<_, String>(0))? {
        let operation = decode_operation(&Default::default(), &row?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        super::artifact_heads::admit(tx, &operation, None)
            .map_err(|_| LocalStorageError::InvalidData)?;
    }
    drop(query);
    // Rows which were already nonterminal need membership even if an UPDATE above
    // happens to retain their state. The same trigger also preserves parked/attention rows.
    tx.execute(
        "INSERT INTO operation_recovery_members SELECT operation_id FROM operations
        WHERE state NOT IN ('succeeded','rolled_back') OR finalization_pending=1
        ON CONFLICT(operation_id) DO NOTHING",
        [],
    )?;
    tx.execute("INSERT INTO operation_recovery_coverage VALUES
        (1,1,(SELECT store_uuid FROM storage_meta WHERE singleton=1),(SELECT count(*) FROM operation_recovery_members))", [])?;
    let mut query = tx.prepare("SELECT workspace_id,publication_revision,digest,publication_bytes FROM gateway_publications ORDER BY publication_revision")?;
    for row in query.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })? {
        let (workspace, revision, digest, bytes) = row?;
        let record = PublicationRecordV1::from_parts(
            WorkspaceId::parse(workspace).map_err(|_| LocalStorageError::InvalidData)?,
            hiroute_domain::GatewayPublicationRevision::new(revision)
                .map_err(|_| LocalStorageError::InvalidData)?,
            CanonicalDigest::parse(digest).map_err(|_| LocalStorageError::InvalidData)?,
            bytes,
        )
        .map_err(|_| LocalStorageError::InvalidData)?;
        let publication = record
            .verify()
            .map_err(|_| LocalStorageError::InvalidData)?;
        record_publication_proofs(tx, &publication).map_err(|_| LocalStorageError::InvalidData)?;
    }
    Ok(())
}

pub(crate) fn record_publication_proofs(
    db: &Connection,
    publication: &GatewayPublicationV1,
) -> PortResult<()> {
    for grant in &publication.grants {
        db.execute(
            "INSERT INTO publication_grant_proofs
            (workspace_id,grant_id,generation,material_sha256,publication_revision)
            VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
            params![
                publication.workspace_id.as_str(),
                grant.grant_id,
                grant.generation,
                grant.bearer_token_sha256.as_str(),
                publication.publication_revision.get()
            ],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "publication.proof.write"))?;
    }
    Ok(())
}

impl ControlStore {
    pub fn operation_requires_finalization(&self, operation: &OperationV1) -> PortResult<bool> {
        let invalid = || port(PortErrorCode::Corrupt, "control.finalize.obligation");
        if !matches!(
            operation.state,
            OperationState::Succeeded | OperationState::RolledBack
        ) {
            return Err(invalid());
        }
        // One read transaction binds the exact journal, membership and completion bit.
        // A terminal label, absent writer or a stale caller is never completion proof.
        let mut db = self.connection.borrow_mut();
        let tx = db.transaction().map_err(|_| invalid())?;
        let members = validate_recovery_members(&tx)?;
        if !journal::is_current(&tx, operation)? {
            return Err(invalid());
        }
        let pending: bool = tx.query_row(
            "SELECT finalization_pending FROM operations WHERE operation_id=?1 AND generation=?2",
            params![operation.operation_id.as_str(), operation.generation], |row| row.get(0),
        ).map_err(|_| invalid())?;
        if pending
            != members
                .iter()
                .any(|id| id == operation.operation_id.as_str())
        {
            return Err(invalid());
        }
        tx.commit().map_err(|_| invalid())?;
        Ok(pending)
    }

    /// Terminal journal publication and mandatory post-commit work are different facts.
    /// The caller may settle only after the existing real activation callback succeeds.
    pub fn finish_operation_finalization(&self, operation: &OperationV1) -> PortResult<()> {
        if !matches!(
            operation.state,
            OperationState::Succeeded | OperationState::RolledBack
        ) {
            return Ok(());
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.finalize.begin"))?;
        if !journal::is_current(&tx, operation)? {
            return Err(port(PortErrorCode::Conflict, "control.finalize.current"));
        }
        tx.execute(
            "UPDATE operations SET finalization_pending=0 WHERE operation_id=?1 AND generation=?2",
            params![operation.operation_id.as_str(), operation.generation],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "control.finalize.write"))?;
        tx.commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.finalize.commit"))
    }

    pub fn pending_finalization_operations(&self) -> PortResult<Vec<OperationV1>> {
        let connection = self.connection.borrow();
        let members = validate_recovery_members(&connection)?;
        let mut result = Vec::new();
        for id in members {
            let operation = load_operation_where(
                &self.diagnostics.borrow(),
                &connection,
                "operation_id=?1 AND state IN ('succeeded','rolled_back') AND finalization_pending=1",
                [&id],
            )?;
            if let Some(operation) = operation {
                if !journal::is_current(&connection, &operation)? {
                    return Err(port(PortErrorCode::Corrupt, "control.finalize.member"));
                }
                result.push(operation);
            }
        }
        Ok(result)
    }
}

pub(crate) fn validate_recovery_members(db: &Connection) -> PortResult<Vec<String>> {
    let invalid = || port(PortErrorCode::Corrupt, "control.recovery.coverage");
    // Missing or changed producer guards cannot establish coverage, even if the
    // current member count happens to be zero. No schema repair happens on read.
    for definition in crate::migrations::current_dependencies_v32::TRIGGERS
        .split("CREATE TRIGGER ")
        .skip(1)
    {
        let (name, _) = definition.split_once(' ').ok_or_else(invalid)?;
        let expected = format!("CREATE TRIGGER {}", definition.trim().trim_end_matches(';'));
        let actual: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| invalid())?;
        if actual != expected {
            return Err(invalid());
        }
    }
    // Missing schema/coverage is damage, never an empty current set. Read the member
    // table itself, not a secondary state index which could silently omit an entry.
    let (format, expected): (u32,u64) = db.query_row(
        "SELECT c.format_version,c.pending_count FROM operation_recovery_coverage c JOIN storage_meta s ON s.singleton=c.singleton AND s.store_uuid=c.store_uuid WHERE c.singleton=1",
        [], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|_| invalid())?;
    let mut statement = db
        .prepare(
            "SELECT operation_id FROM operation_recovery_members NOT INDEXED ORDER BY operation_id",
        )
        .map_err(|_| invalid())?;
    let members = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    if format != 1 || expected != members.len() as u64 {
        return Err(invalid());
    }
    for id in &members {
        let valid: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE operation_id=?1
            AND (state NOT IN ('succeeded','rolled_back') OR finalization_pending=1))",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| invalid())?;
        if !valid {
            return Err(invalid());
        }
    }
    let missing_claim: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM writer_claim w
        LEFT JOIN operation_recovery_members m USING(operation_id) WHERE m.operation_id IS NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if missing_claim {
        return Err(invalid());
    }
    Ok(members)
}
