//! Current native-file recovery roots. Optional historical restore points are read on use.
//! These references confer no authority: their original journal and receipt remain required.
use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct Owner {
    operation: OperationId,
    effect: String,
    receipt: CanonicalDigest,
}

pub(super) fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.artifact-heads.binding")
}

pub(super) fn file_intents(
    operation: &OperationV1,
) -> impl Iterator<Item = &ExternalEffectIntentV1> {
    operation.plan.external().iter().filter(|intent| {
        intent.kind() == OwnedEffectKind::AgentArtifact
            && matches!(
                intent.desired()["transaction"].as_str(),
                Some("apply" | "restore" | "settings")
            )
    })
}

fn head_intents(operation: &OperationV1) -> impl Iterator<Item = &ExternalEffectIntentV1> {
    // Catalog paths are content addressed. The current model's exact catalog is a child
    // dependency, not a permanent head for every catalog ever produced.
    file_intents(operation).filter(|intent| {
        intent.desired()["payload"]["schema"] != "hiroute.codex-catalog-artifact/v1"
    })
}

pub(super) fn receipt<'a>(
    operation: &'a OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Option<&'a OwnedEffectV1>> {
    if operation.state == OperationState::Succeeded
        && !operation.steps.iter().any(|step| {
            step.kind == hiroute_domain::OperationStepKind::ApplyAgentArtifacts
                && step.status == hiroute_domain::OperationStepStatus::Applied
        })
    {
        return Err(invalid());
    }
    let mut matching = operation
        .steps
        .iter()
        .filter(|step| step.kind == hiroute_domain::OperationStepKind::ApplyAgentArtifacts)
        .flat_map(|step| step.effects.iter())
        .filter(|effect| effect.effect_id == intent.effect_id());
    let found = matching.next();
    if matching.next().is_some() {
        return Err(invalid());
    }
    if let Some(effect) = found
        && (effect.kind != intent.kind()
            || effect.target != intent.target()
            || effect.before_fingerprint.as_ref() != intent.before_fingerprint()
            || effect.compensation["operation_id"] != operation.operation_id.as_str()
            || effect.compensation["effect_id"] != intent.effect_id()
            || !matches!(
                effect.compensation["schema"].as_str(),
                Some(
                    "hiroute.managed-artifact-compensation/v1"
                        | "hiroute.native-file-reference/v1"
                        | "hiroute.native-restoration-ack/v1"
                )
            ))
    {
        return Err(invalid());
    }
    Ok(found)
}

fn owner(operation: &OperationV1, intent: &ExternalEffectIntentV1) -> PortResult<Owner> {
    Ok(Owner {
        operation: operation.operation_id.clone(),
        effect: intent.effect_id().into(),
        receipt: CanonicalDigest::of(receipt(operation, intent)?.ok_or_else(invalid)?)
            .map_err(|_| invalid())?,
    })
}

fn retires(intent: &ExternalEffectIntentV1) -> bool {
    let desired = intent.desired();
    desired["transaction"] == "restore"
        || (artifact_dependencies::native_payload(intent)
            && desired["payload"]["change"]["action"] == "restore")
        || (desired["payload"]["schema"] == "hiroute.settings-skill-file/v1"
            && desired["payload"]["action"] == "remove")
}

pub(super) fn validate_coverage(db: &Connection) -> PortResult<()> {
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM artifact_head_coverage c JOIN storage_meta s
        ON c.singleton=s.singleton AND c.store_uuid=s.store_uuid WHERE c.singleton=1 AND c.format_version=1
        AND c.head_count=(SELECT count(*) FROM artifact_heads))", [], |r| r.get(0)).map_err(|_| invalid())?;
    if valid { Ok(()) } else { Err(invalid()) }
}

fn current(db: &Connection, workspace: &WorkspaceId, target: &str) -> PortResult<Option<Owner>> {
    let value: Option<(String, String)> = db
        .query_row(
            "SELECT owner_json,owner_operation_id FROM artifact_heads WHERE workspace_id=?1 AND target=?2",
            params![workspace.as_str(), target],
            |r| Ok((r.get(0)?,r.get(1)?)),
        )
        .optional()
        .map_err(|_| invalid())?;
    value
        .map(|(v, id)| {
            let owner: Owner = serde_json::from_str(&v).map_err(|_| invalid())?;
            if owner.operation.as_str() != id {
                return Err(invalid());
            }
            Ok(owner)
        })
        .transpose()
}

fn authenticate_owner(
    db: &Connection,
    workspace: &WorkspaceId,
    target: &str,
    expected: &Owner,
    cache: Option<&RefCell<completed_operation_cache::CompletedOperationCache>>,
) -> PortResult<OperationV1> {
    let operation = if let Some(cache) = cache {
        cache
            .borrow_mut()
            .read(&Default::default(), db, &expected.operation)?
    } else {
        load_operation_where(
            &Default::default(),
            db,
            "operation_id=?1",
            [expected.operation.as_str()],
        )?
    }
    .ok_or_else(invalid)?;
    if operation.workspace_id != *workspace
        || operation.state != OperationState::Succeeded
        || !journal::is_current_artifact_step(db, &operation)?
    {
        return Err(invalid());
    }
    let intent = head_intents(&operation)
        .find(|i| i.target() == target && i.effect_id() == expected.effect)
        .ok_or_else(invalid)?;
    if retires(intent) || owner(&operation, intent)? != *expected {
        return Err(invalid());
    }
    Ok(operation)
}

fn replace(
    db: &Connection,
    workspace: &WorkspaceId,
    target: &str,
    next: Option<Owner>,
) -> PortResult<()> {
    let before = current(db, workspace, target)?;
    if let Some(next) = &next {
        db.execute("INSERT INTO artifact_heads(workspace_id,target,owner_operation_id,owner_json) VALUES(?1,?2,?3,?4)
            ON CONFLICT(workspace_id,target) DO UPDATE SET owner_operation_id=excluded.owner_operation_id,owner_json=excluded.owner_json",
            params![workspace.as_str(), target, next.operation.as_str(), serde_json::to_string(next).map_err(|_| invalid())?]).map_err(|_| invalid())?;
    } else {
        db.execute(
            "DELETE FROM artifact_heads WHERE workspace_id=?1 AND target=?2",
            params![workspace.as_str(), target],
        )
        .map_err(|_| invalid())?;
    }
    db.execute(
        "UPDATE artifact_head_coverage SET head_count=head_count+?1 WHERE singleton=1",
        [i64::from(next.is_some()) - i64::from(before.is_some())],
    )
    .map_err(|_| invalid())?;
    Ok(())
}

/// Accepted target/revision checks and writer claim precede this in the same transaction.
/// The exact current owner is captured even when the user selected an older Restore point.
pub(super) fn admit(
    db: &Connection,
    operation: &OperationV1,
    cache: Option<&RefCell<completed_operation_cache::CompletedOperationCache>>,
) -> PortResult<()> {
    validate_coverage(db)?;
    for intent in head_intents(operation) {
        let before = current(db, &operation.workspace_id, intent.target())?;
        if let Some(before) = &before {
            authenticate_owner(db, &operation.workspace_id, intent.target(), before, cache)?;
        }
        db.execute("INSERT INTO artifact_head_transitions(operation_id,target,before_json) VALUES(?1,?2,?3)",
            params![operation.operation_id.as_str(), intent.target(), serde_json::to_string(&before).map_err(|_| invalid())?]).map_err(|_| invalid())?;
    }
    Ok(())
}

/// Called before terminal journal publication/releasing the writer, in that same transaction.
pub(super) fn finish(
    db: &Connection,
    operation: &OperationV1,
    cache: Option<&RefCell<completed_operation_cache::CompletedOperationCache>>,
) -> PortResult<()> {
    if !matches!(
        operation.state,
        OperationState::Succeeded | OperationState::RolledBack
    ) {
        return Ok(());
    }
    validate_coverage(db)?;
    let count: usize = db
        .query_row(
            "SELECT count(*) FROM artifact_head_transitions WHERE operation_id=?1",
            [operation.operation_id.as_str()],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if count != head_intents(operation).count() {
        return Err(invalid());
    }
    for intent in head_intents(operation) {
        let value: String = db.query_row("SELECT before_json FROM artifact_head_transitions WHERE operation_id=?1 AND target=?2",
            params![operation.operation_id.as_str(), intent.target()], |r| r.get(0)).map_err(|_| invalid())?;
        let before: Option<Owner> = serde_json::from_str(&value).map_err(|_| invalid())?;
        if current(db, &operation.workspace_id, intent.target())? != before {
            return Err(invalid());
        }
        if let Some(before) = &before {
            authenticate_owner(db, &operation.workspace_id, intent.target(), before, cache)?;
        }
        if operation.state == OperationState::Succeeded {
            // A missing/foreign receipt cannot retire an existing recovery owner.
            let next = owner(operation, intent)?;
            replace(
                db,
                &operation.workspace_id,
                intent.target(),
                (!retires(intent)).then_some(next),
            )?;
        }
    }
    db.execute(
        "DELETE FROM artifact_head_transitions WHERE operation_id=?1",
        [operation.operation_id.as_str()],
    )
    .map_err(|_| invalid())?;
    Ok(())
}

pub(super) fn remember_legacy(db: &Connection, operation: &OperationV1) -> PortResult<()> {
    if operation.state == OperationState::Succeeded {
        for intent in head_intents(operation) {
            let next = owner(operation, intent)?;
            replace(
                db,
                &operation.workspace_id,
                intent.target(),
                (!retires(intent)).then_some(next),
            )?;
        }
    }
    Ok(())
}

impl ControlStore {
    /// Current native model declarations for budget and access checks. Historical
    /// Restore menus retain their separate strict history reader. Authenticate the
    /// current file reference before selecting its typed role; then require the
    /// exact grant-step proof consumed by these active-authority checks. Optional
    /// completed service provenance is not a dependency of current budget/access reads.
    pub fn current_agent_configuration_operations(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Vec<OperationV1>> {
        use hiroute_domain::{AgentConnectionEffectRoleV1, AgentOperationRead};
        let db = self.connection.borrow();
        let snapshot = if db.is_autocommit() {
            Some(db.unchecked_transaction().map_err(|_| invalid())?)
        } else {
            None
        };
        validate_coverage(&db)?;
        let mut query = db.prepare("SELECT target,owner_operation_id,owner_json FROM artifact_heads WHERE workspace_id=?1 ORDER BY target").map_err(|_| invalid())?;
        let rows = query
            .query_map([workspace.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| invalid())?;
        let mut output = BTreeMap::new();
        for row in rows {
            let (target, id, encoded) = row.map_err(|_| invalid())?;
            let expected: Owner = serde_json::from_str(&encoded).map_err(|_| invalid())?;
            if expected.operation.as_str() != id {
                return Err(invalid());
            }
            let operation = authenticate_owner(
                &db,
                workspace,
                &target,
                &expected,
                Some(&self.completed_operation_cache),
            )?;
            let subject = operation
                .agent_input()
                .connection_subject()
                .map_err(|_| invalid())?
                .ok_or_else(invalid)?;
            let model_target = AgentConnectionEffectRoleV1::ManagedConfiguration
                .target_for(&subject)
                .map_err(|_| invalid())?;
            if target != model_target {
                continue;
            }
            if operation.idempotency.operation_kind != "ApplyAgentConnectionChange"
                || !journal::is_current_grant_step(&db, &operation)?
            {
                return Err(invalid());
            }
            output.insert(operation.operation_id.to_string(), operation);
        }
        drop(query);
        if let Some(snapshot) = snapshot {
            snapshot.commit().map_err(|_| invalid())?;
        }
        Ok(output.into_values().collect())
    }

    /// Bounded by current resources and unresolved work, not historical menu entries.
    pub(crate) fn artifact_dependencies(
        &self,
    ) -> PortResult<Vec<artifact_dependencies::Dependency>> {
        let db = self.connection.borrow();
        let snapshot = if db.is_autocommit() {
            Some(db.unchecked_transaction().map_err(|_| invalid())?)
        } else {
            None
        };
        validate_coverage(&db)?;
        let mut output = BTreeMap::new();
        let mut query = db.prepare("SELECT workspace_id,target,owner_operation_id,owner_json FROM artifact_heads ORDER BY workspace_id,target").map_err(|_| invalid())?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| invalid())?;
        for row in rows {
            let (workspace, target, id, encoded) = row.map_err(|_| invalid())?;
            let expected: Owner = serde_json::from_str(&encoded).map_err(|_| invalid())?;
            let operation = self
                .load_operation(&expected.operation)?
                .ok_or_else(invalid)?;
            if expected.operation.as_str() != id
                || operation.workspace_id.as_str() != workspace
                || operation.state != OperationState::Succeeded
                || !journal::is_current_artifact_step(&db, &operation)?
            {
                return Err(invalid());
            }
            let intent = head_intents(&operation)
                .find(|intent| intent.target() == target && intent.effect_id() == expected.effect)
                .ok_or_else(invalid)?;
            if retires(intent) || owner(&operation, intent)? != expected {
                return Err(invalid());
            }
            artifact_dependencies::add(&mut output, &operation, intent, true)?;
            if intent.desired()["payload"]["schema"] == "hiroute.settings-codex-model-file/v1" {
                for catalog in file_intents(&operation).filter(|i| {
                    i.desired()["payload"]["schema"] == "hiroute.codex-catalog-artifact/v1"
                }) {
                    artifact_dependencies::add(&mut output, &operation, catalog, true)?;
                }
            }
        }
        drop(query);
        for id in current_dependencies::validate_recovery_members(&db)? {
            let id = OperationId::parse(id).map_err(|_| invalid())?;
            let operation = self.load_operation(&id)?.ok_or_else(invalid)?;
            if !journal::is_current(&db, &operation)? {
                return Err(invalid());
            }
            // Successfully finished callbacks have already committed the heads. A rollback
            // callback can still use its own backups; neither needs an admission CAS anymore.
            for intent in file_intents(&operation) {
                artifact_dependencies::add(&mut output, &operation, intent, false)?;
                artifact_dependencies::add_predecessor(self, &mut output, &operation, intent)?;
            }
            if !matches!(
                operation.state,
                OperationState::Succeeded | OperationState::RolledBack
            ) {
                let count: usize = db
                    .query_row(
                        "SELECT count(*) FROM artifact_head_transitions WHERE operation_id=?1",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(|_| invalid())?;
                if count != head_intents(&operation).count() {
                    return Err(invalid());
                }
                for intent in head_intents(&operation) {
                    let value: String = db.query_row("SELECT before_json FROM artifact_head_transitions WHERE operation_id=?1 AND target=?2", params![id.as_str(), intent.target()], |r| r.get(0)).map_err(|_| invalid())?;
                    let before: Option<Owner> =
                        serde_json::from_str(&value).map_err(|_| invalid())?;
                    if current(&db, &operation.workspace_id, intent.target())? != before {
                        return Err(invalid());
                    }
                }
            }
        }
        let orphan: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM artifact_head_transitions t LEFT JOIN operation_recovery_members r USING(operation_id) WHERE r.operation_id IS NULL)", [], |r| r.get(0)).map_err(|_| invalid())?;
        if orphan {
            return Err(invalid());
        }
        if let Some(snapshot) = snapshot {
            snapshot.commit().map_err(|_| invalid())?;
        }
        Ok(output.into_values().collect())
    }
}
