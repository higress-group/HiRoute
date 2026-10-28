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
    target_from_spec(operation.plan.spec())
}

fn target_from_spec(
    plan: &hiroute_domain::ChangeSpecV1,
) -> PortResult<Option<(String, hiroute_domain::AgentNativeTargetV1)>> {
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

/// The current target head is a discovery hint. Its owner journal is captured
/// in the Control snapshot and authenticated after the shared lock is released.
pub struct CapturedAgentNativeTarget {
    witness: AgentNativeTargetWitness,
    frozen: Option<super::agent_frozen_read::FrozenAgentJournal>,
}

#[derive(Clone)]
pub struct AgentNativeTargetWitness {
    workspace: WorkspaceId,
    context: String,
    owner: Option<(String, u64, String, String)>,
}

impl CapturedAgentNativeTarget {
    pub fn witness(&self) -> AgentNativeTargetWitness {
        self.witness.clone()
    }

    pub fn verify(self) -> PortResult<Option<hiroute_domain::AgentNativeTargetV1>> {
        let Some(frozen) = self.frozen else {
            return Ok(None);
        };
        let (owner, generation, accepted, state) =
            self.witness.owner.as_ref().ok_or_else(invalid)?;
        let row = frozen.row();
        if &row.operation_id != owner
            || row.workspace_id != self.witness.workspace.as_str()
            || row.generation != *generation
            || &row.accepted_digest != accepted
            || &row.state != state
        {
            return Err(invalid());
        }
        let journal = frozen.verify_pending()?;
        if journal.operation_id.as_str() != owner
            || journal.workspace != self.witness.workspace
            || journal.state.as_str() != state
        {
            return Err(invalid());
        }
        let (context, target) = target_from_spec(journal.plan.spec())?.ok_or_else(invalid)?;
        if context != self.witness.context {
            return Err(invalid());
        }
        Ok(Some(target))
    }
}

impl AgentNativeTargetWitness {
    pub fn matches_current(&self, control: &ControlStore) -> PortResult<bool> {
        let db = control.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "control.native-targets.fence",
            ));
        }
        validate_coverage(&db)?;
        let owner = db.query_row(
            "SELECT h.owner_operation_id,o.generation,o.accepted_change_digest,o.state
             FROM agent_native_target_heads h LEFT JOIN operations o ON o.operation_id=h.owner_operation_id
             WHERE h.workspace_id=?1 AND h.context_id=?2",
            params![self.workspace.as_str(),self.context],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)),
        ).optional().map_err(|_| invalid())?;
        Ok(owner == self.owner)
    }
}

pub(super) fn validate_coverage(db: &Connection) -> PortResult<()> {
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
    pub fn capture_agent_native_target(
        &self,
        workspace: &WorkspaceId,
        context: &str,
    ) -> PortResult<CapturedAgentNativeTarget> {
        let db = self.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "control.native-targets.snapshot",
            ));
        }
        validate_coverage(&db)?;
        let owner: Option<(String,u64,String,String)> = db.query_row(
            "SELECT h.owner_operation_id,o.generation,o.accepted_change_digest,o.state
             FROM agent_native_target_heads h LEFT JOIN operations o ON o.operation_id=h.owner_operation_id
             WHERE h.workspace_id=?1 AND h.context_id=?2",
            params![workspace.as_str(),context],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(|_| invalid())?;
        let frozen = if let Some((id, _, _, _)) = &owner {
            let row = db.query_row(
                "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE operation_id=?1",
                [id], super::agent_history_cache::AgentHistoryRow::read,
            ).map_err(|_| invalid())?;
            Some(super::agent_frozen_read::FrozenAgentJournal::capture(
                &db, row,
            )?)
        } else {
            None
        };
        Ok(CapturedAgentNativeTarget {
            witness: AgentNativeTargetWitness {
                workspace: workspace.clone(),
                context: context.to_owned(),
                owner,
            },
            frozen,
        })
    }

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
