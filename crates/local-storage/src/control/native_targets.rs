//! Current explicit discovery targets, never execution authority or a historical scan.
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static TARGET_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.native-targets.current")
}

fn target(
    operation: &OperationV1,
) -> PortResult<Option<(String, hiroute_domain::AgentNativeTargetV1)>> {
    let plan = operation.plan.spec();
    if plan.command_id != "agents.settings.apply" {
        return Ok(None);
    }
    let spec: hiroute_domain::AgentSettingsSpecV2 =
        serde_json::from_value(plan.desired_state.clone()).map_err(|_| invalid())?;
    let Some(target) = spec.native_target else {
        return Ok(None);
    };
    target.validate().map_err(|_| invalid())?;
    let context = format!(
        "agent-context/hermes/{}",
        CanonicalDigest::of_bytes(target.config_root().as_bytes()).as_str()
    );
    if spec.context_id != context || plan.resource_id.as_deref() != Some(context.as_str()) {
        return Err(invalid());
    }
    Ok(Some((context, target)))
}

fn validate_coverage(db: &Connection) -> PortResult<()> {
    let complete: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_native_target_coverage c
        JOIN storage_meta s ON c.singleton=s.singleton AND c.store_uuid=s.store_uuid
        WHERE c.singleton=1 AND c.format_version=1
        AND c.target_count=(SELECT count(*) FROM agent_native_target_heads))",
            [],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if !complete {
        return Err(invalid());
    }
    Ok(())
}

/// Called in the journal admission or first-format conversion transaction. Even a
/// later rollback retains the explicitly selected path as a discovery hint, just
/// as the prior history reader did. Only the latest input for that path is needed.
pub(super) fn remember(db: &Connection, operation: &OperationV1) -> PortResult<()> {
    let Some((context, _)) = target(operation)? else {
        return Ok(());
    };
    validate_coverage(db)?;
    let inserted = db
        .execute(
            "INSERT INTO agent_native_target_heads VALUES(?1,?2,?3)
        ON CONFLICT(workspace_id,context_id) DO NOTHING",
            params![
                operation.workspace_id.as_str(),
                context,
                operation.operation_id.as_str()
            ],
        )
        .map_err(|_| invalid())?;
    db.execute(
        "UPDATE agent_native_target_heads SET owner_operation_id=?3
        WHERE workspace_id=?1 AND context_id=?2",
        params![
            operation.workspace_id.as_str(),
            context,
            operation.operation_id.as_str()
        ],
    )
    .map_err(|_| invalid())?;
    db.execute(
        "UPDATE agent_native_target_coverage SET target_count=target_count+?1 WHERE singleton=1",
        [inserted],
    )
    .map_err(|_| invalid())?;
    Ok(())
}

impl ControlStore {
    pub(crate) fn validate_native_target_heads(&self) -> PortResult<()> {
        self.read_native_targets(None).map(|_| ())
    }

    pub fn agent_settings_native_targets(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Vec<hiroute_domain::AgentNativeTargetV1>> {
        self.read_native_targets(Some(workspace))
    }

    fn read_native_targets(
        &self,
        workspace: Option<&WorkspaceId>,
    ) -> PortResult<Vec<hiroute_domain::AgentNativeTargetV1>> {
        let db = self.connection.borrow();
        let transaction = if db.is_autocommit() {
            Some(db.unchecked_transaction().map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.native-targets.snapshot",
                )
            })?)
        } else {
            None
        };
        validate_coverage(&db)?;
        let mut query = db.prepare("SELECT h.workspace_id,h.context_id,h.owner_operation_id,o.state,o.generation
            FROM agent_native_target_heads h LEFT JOIN operations o ON o.operation_id=h.owner_operation_id
            WHERE ?1 IS NULL OR h.workspace_id=?1 ORDER BY h.workspace_id,h.context_id").map_err(|_| invalid())?;
        let rows = query
            .query_map([workspace.map(WorkspaceId::as_str)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(|_| invalid())?;
        let mut targets = Vec::new();
        for row in rows {
            let (workspace, context, id, state, generation) = row.map_err(|_| invalid())?;
            let id = OperationId::parse(id).map_err(|_| invalid())?;
            #[cfg(test)]
            TARGET_READS.with(|n| n.set(n.get() + 1));
            let operation = self.load_operation(&id)?.ok_or_else(invalid)?;
            if operation.operation_id != id
                || operation.workspace_id.as_str() != workspace
                || operation.state.as_str() != state
                || operation.generation != generation
            {
                return Err(invalid());
            }
            let (actual, target) = target(&operation)?.ok_or_else(invalid)?;
            if actual != context {
                return Err(invalid());
            }
            targets.push(target);
        }
        drop(query);
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.native-targets.snapshot",
                )
            })?;
        }
        Ok(targets)
    }
}
