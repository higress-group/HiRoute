//! Operations prepare immutable content; only their successful installed result exposes heads.
use super::*;
use hiroute_domain::{
    ControlRepositoryPort, OperationId, OperationState, OperationV1, PlanLifecycleV1,
};

impl ControlStore {
    pub fn stage_plan_version(
        &self,
        operation: &OperationV1,
        version: &PlanVersionV1,
    ) -> Result<(), PlanVersionError> {
        version.validate()?;
        let operation_id = &operation.operation_id;
        if operation.workspace_id != version.reference.workspace_id
            || operation.plan.control().get("plan_version")
                != Some(&serde_json::to_value(version).map_err(|_| PlanVersionError::Invalid)?)
        {
            return Err(PlanVersionError::Conflict);
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if !super::super::journal::is_current(&tx, operation)
            .map_err(|_| PlanVersionError::Conflict)?
        {
            return Err(PlanVersionError::Conflict);
        }

        if let Some(content) = operation
            .plan
            .plan_content_control()
            .map_err(|_| PlanVersionError::Invalid)?
            && let Some(legacy) = content.legacy_source
        {
            stage_version_in(&tx, operation_id, &legacy)?;
        }
        stage_version_in(&tx, operation_id, version)?;
        tx.commit().map_err(storage)
    }

    /// Called under the same admission Guard after the existing Operation succeeded and its
    /// Gateway identity was verified. The exact sealed control intent is the write authority.
    pub fn commit_plan_head(
        &self,
        operation: &OperationV1,
        head: &PlanHeadV1,
        expected_head_revision: Option<u64>,
    ) -> Result<(), PlanVersionError> {
        head.validate()?;
        if operation.state != OperationState::Succeeded
            || operation.workspace_id != head.reference.workspace_id
            || operation.plan.control().get("plan_head")
                != Some(&serde_json::to_value(head).map_err(|_| PlanVersionError::Invalid)?)
        {
            return Err(PlanVersionError::Conflict);
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if !super::super::journal::is_current(&tx, operation)
            .map_err(|_| PlanVersionError::Conflict)?
        {
            return Err(PlanVersionError::Conflict);
        }

        commit_head_in(&tx, operation, head, expected_head_revision)?;
        tx.commit().map_err(storage)
    }

    pub fn discard_prepared_plan_version(
        &self,
        operation_id: &OperationId,
    ) -> Result<(), PlanVersionError> {
        let operation = self
            .load_operation(operation_id)
            .map_err(|_| PlanVersionError::StorageUnavailable)?
            .ok_or(PlanVersionError::Conflict)?;
        if operation.state != OperationState::RolledBack {
            return Err(PlanVersionError::Conflict);
        }
        self.connection
            .borrow()
            .execute(
                "DELETE FROM plan_versions WHERE state='prepared' AND owner_operation_id=?1",
                [operation_id.as_str()],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Explicit bounded reclamation, never a broad TTL delete. Current, prepared, LKG and
    /// unresolved ownership all protect bytes; an expired hold is still a hold until reconciled.
    pub fn reclaim_plan_version(
        &self,
        reference: &PlanExecutionRef,
    ) -> Result<(), PlanVersionError> {
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        holds::require_ready(&tx, &reference.workspace_id)?;
        lookup_in(&tx, reference)?;
        if head_in(&tx, &reference.workspace_id, &reference.plan_id)?
            .is_some_and(|h| h.status != PlanLifecycleV1::Deleted && h.reference == *reference)
        {
            return Err(PlanVersionError::Retained);
        }
        let held: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_version_holds WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3)", params![reference.workspace_id.as_str(),reference.plan_id.as_str(),reference.content_revision], |r| r.get(0)).map_err(storage)?;
        if held {
            return Err(PlanVersionError::Retained);
        }
        {
            let mut stmt = tx.prepare("SELECT publication_bytes FROM gateway_publications WHERE workspace_id=?1 AND state IN ('prepared','active','lkg')").map_err(storage)?;
            let rows = stmt
                .query_map([reference.workspace_id.as_str()], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .map_err(storage)?;
            for row in rows {
                let publication =
                    hiroute_domain::GatewayPublicationV1::decode_persisted(&row.map_err(storage)?)
                        .map_err(|_| PlanVersionError::Invalid)?;
                if publication.plans.iter().any(|p| {
                    p.agent_plan_id() == &reference.plan_id
                        && p.body.agent_plan_revision == reference.content_revision
                }) {
                    return Err(PlanVersionError::Retained);
                }
            }
        }
        tx.execute("DELETE FROM plan_versions WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3", params![reference.workspace_id.as_str(),reference.plan_id.as_str(),reference.content_revision]).map_err(storage)?;
        tx.commit().map_err(storage)
    }
}

fn stage_version_in(
    tx: &Connection,
    operation_id: &OperationId,
    version: &PlanVersionV1,
) -> Result<(), PlanVersionError> {
    let reference = &version.reference;
    let old: Option<(String,String,String)> = tx.query_row("SELECT content_digest,version_json,owner_operation_id FROM plan_versions WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3", params![reference.workspace_id.as_str(),reference.plan_id.as_str(),reference.content_revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
    let bytes = encode(version)?;
    if let Some((digest, existing, owner)) = old {
        if digest != reference.content_digest.as_str()
            || existing != bytes
            || owner != operation_id.as_str()
        {
            return Err(PlanVersionError::Conflict);
        }
    } else {
        tx.execute("INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES(?1,?2,?3,?4,?5,'prepared',?6)", params![reference.workspace_id.as_str(),reference.plan_id.as_str(),reference.content_revision,reference.content_digest.as_str(),bytes,operation_id.as_str()]).map_err(storage)?;
    }
    Ok(())
}

/// Local head, version state and consumed draft share the terminal journal transaction.
/// The outer writer/admission owner has verified installation; the journal generation
/// CAS in the same transaction prevents a detached intent from committing this head.
pub(in crate::control) fn finish_plan_operation(
    tx: &Connection,
    operation: &OperationV1,
) -> Result<(), PlanVersionError> {
    let Some(content) = operation
        .plan
        .plan_content_control()
        .map_err(|_| PlanVersionError::Invalid)?
    else {
        return Ok(());
    };
    match operation.state {
        OperationState::Succeeded => commit_head_in(
            tx,
            operation,
            &content.plan_head,
            content.before_head.as_ref().map(|head| head.head_revision),
        ),
        OperationState::RolledBack => {
            tx.execute(
                "DELETE FROM plan_versions WHERE state='prepared' AND owner_operation_id=?1",
                [operation.operation_id.as_str()],
            )
            .map_err(storage)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn commit_head_in(
    tx: &Connection,
    operation: &OperationV1,
    head: &PlanHeadV1,
    expected_head_revision: Option<u64>,
) -> Result<(), PlanVersionError> {
    head.validate()?;
    if operation.state != OperationState::Succeeded
        || operation.workspace_id != head.reference.workspace_id
        || operation.plan.control().get("plan_head")
            != Some(&serde_json::to_value(head).map_err(|_| PlanVersionError::Invalid)?)
    {
        return Err(PlanVersionError::Conflict);
    }
    let operation_id = &operation.operation_id;
    let mut old = head_in(tx, &head.reference.workspace_id, &head.reference.plan_id)?;
    if old.is_none() && expected_head_revision.is_some() {
        let content = operation
            .plan
            .plan_content_control()
            .map_err(|_| PlanVersionError::Invalid)?
            .ok_or(PlanVersionError::Conflict)?;
        let legacy = content.legacy_source.ok_or(PlanVersionError::Conflict)?;
        let before = content.before_head.ok_or(PlanVersionError::Conflict)?;
        if before.reference != legacy.reference {
            return Err(PlanVersionError::Conflict);
        }
        tx.execute("UPDATE plan_versions SET state='published' WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3 AND content_digest=?4 AND owner_operation_id=?5 AND state='prepared'",
                params![legacy.reference.workspace_id.as_str(), legacy.reference.plan_id.as_str(), legacy.reference.content_revision,
                    legacy.reference.content_digest.as_str(), operation_id.as_str()]).map_err(storage)?;
        if lookup_in(tx, &legacy.reference)? != legacy {
            return Err(PlanVersionError::Conflict);
        }
        old = Some(before);
    }
    if old.as_ref() == Some(head) {
        return Ok(());
    }
    if old.as_ref().map(|h| h.head_revision) != expected_head_revision
        || expected_head_revision.unwrap_or(0).checked_add(1) != Some(head.head_revision)
    {
        return Err(PlanVersionError::Conflict);
    }
    if let Some(old) = old {
        if old.status == PlanLifecycleV1::Deleted || old.model_alias != head.model_alias {
            return Err(PlanVersionError::Conflict);
        }
        if old.reference != head.reference
            && old.reference.content_revision.checked_add(1)
                != Some(head.reference.content_revision)
        {
            return Err(PlanVersionError::Conflict);
        }
    } else if head.reference.content_revision != 1 || head.status != PlanLifecycleV1::Enabled {
        return Err(PlanVersionError::Conflict);
    }
    tx.execute("UPDATE plan_versions SET state='published' WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3 AND content_digest=?4 AND owner_operation_id=?5", params![head.reference.workspace_id.as_str(),head.reference.plan_id.as_str(),head.reference.content_revision,head.reference.content_digest.as_str(),operation_id.as_str()]).map_err(storage)?;
    let version = lookup_in(tx, &head.reference)?;
    if version.compiled.model_alias() != &head.model_alias {
        return Err(PlanVersionError::Conflict);
    }
    tx.execute("INSERT INTO plan_heads(workspace_id,plan_id,head_revision,head_json) VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,plan_id) DO UPDATE SET head_revision=excluded.head_revision,head_json=excluded.head_json", params![head.reference.workspace_id.as_str(),head.reference.plan_id.as_str(),head.head_revision,encode(head)?]).map_err(storage)?;
    // Consume only the draft revision sealed into this operation. A newer saved draft lives.
    if let Some(draft) = operation.plan.control().get("consumed_draft") {
        let id = draft
            .get("draft_id")
            .and_then(|v| v.as_str())
            .ok_or(PlanVersionError::Invalid)?;
        let revision = draft
            .get("revision")
            .and_then(|v| v.as_u64())
            .ok_or(PlanVersionError::Invalid)?;
        tx.execute(
            "DELETE FROM plan_drafts WHERE workspace_id=?1 AND draft_id=?2 AND revision=?3",
            params![head.reference.workspace_id.as_str(), id, revision],
        )
        .map_err(storage)?;
    }
    Ok(())
}

/// The old producer committed the head/version/draft in one local transaction after
/// the terminal journal. Discover its actual completion, never infer it from Succeeded.
/// Called only after the initializer authenticates the complete original journal.
/// Current Gateway installation and grants still pass the independent startup gate.
pub(in crate::control) fn legacy_plan_finalization_required(
    db: &Connection,
    operation: &OperationV1,
) -> Result<bool, PlanVersionError> {
    if !matches!(
        operation.state,
        OperationState::Succeeded | OperationState::RolledBack
    ) {
        return Ok(false);
    }
    let Some(content) = operation
        .plan
        .plan_content_control()
        .map_err(|_| PlanVersionError::Invalid)?
    else {
        return Ok(false);
    };
    // Keep rollback and superseded/different heads on the established recovery path.
    if operation.state != OperationState::Succeeded
        || head_in(
            db,
            &content.plan_head.reference.workspace_id,
            &content.plan_head.reference.plan_id,
        )?
        .as_ref()
            != Some(&content.plan_head)
    {
        return Ok(true);
    }
    let indexed_head: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM plan_heads WHERE workspace_id=?1 AND plan_id=?2 AND head_revision=?3)",
        params![content.plan_head.reference.workspace_id.as_str(), content.plan_head.reference.plan_id.as_str(), content.plan_head.head_revision],
        |r| r.get(0),
    ).map_err(storage)?;
    if !indexed_head {
        return Ok(true);
    }
    let prepared: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM plan_versions WHERE owner_operation_id=?1 AND state='prepared')",
        [operation.operation_id.as_str()], |r| r.get(0),
    ).map_err(storage)?;
    if prepared {
        return Ok(true);
    }
    // A head-only lifecycle retains the original version owner. New content must
    // additionally bind the published version to this exact accepted operation.
    if content
        .before_head
        .as_ref()
        .is_none_or(|before| before.reference != content.plan_head.reference)
    {
        let owned: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM plan_versions WHERE workspace_id=?1 AND plan_id=?2
             AND content_revision=?3 AND owner_operation_id=?4 AND state='published')",
                params![
                    content.plan_head.reference.workspace_id.as_str(),
                    content.plan_head.reference.plan_id.as_str(),
                    content.plan_head.reference.content_revision,
                    operation.operation_id.as_str()
                ],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if !owned {
            return Ok(true);
        }
    }
    match lookup_in(db, &content.plan_head.reference) {
        Ok(version) => Ok(version != content.plan_version),
        Err(PlanVersionError::Unavailable) => Ok(true),
        Err(error) => Err(error),
    }
}
