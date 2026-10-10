//! Single-writer, idempotent convergence of forward Secret compensation references.
use super::*;
use hiroute_domain::{CredentialRefV1, OperationState, OperationV1};

pub(in crate::control) fn reconcile(
    store: &ControlStore,
    operation: &OperationV1,
    references: &[CredentialRefV1],
) -> PortResult<()> {
    if references.is_empty() {
        return Ok(());
    }
    if !matches!(
        operation.state,
        OperationState::RollingBack | OperationState::NeedsAttention
    ) {
        return Err(conflict("compute.compensation.state"));
    }
    let mutation = mutation_from_control(operation.plan.control())?
        .ok_or_else(|| invalid("compute.compensation.mutation"))?;
    let expected = mutation
        .expected()
        .ok_or_else(|| invalid("compute.compensation.expected"))?;
    let mut repaired = expected.clone();
    let mut seen = std::collections::BTreeSet::new();
    for reference in references {
        if !seen.insert(reference.credential_id()) {
            return Err(invalid("compute.compensation.duplicate"));
        }
        let key = repaired
            .credentials
            .iter_mut()
            .find(|key| key.key_id == reference.credential_id())
            .ok_or_else(|| invalid("compute.compensation.key"))?;
        let original = &key.credential;
        let forward = original
            .generation()
            .checked_add(2)
            .ok_or_else(|| conflict("compute.compensation.generation"))?;
        let exact = CredentialRefV1::new(
            original.credential_id(),
            original.owner_scope(),
            original.subject(),
            original.purpose(),
            original.allowed_destinations().iter().cloned(),
            forward,
        )
        .map_err(|_| invalid("compute.compensation.reference"))?;
        if reference != &exact
            || !operation
                .plan
                .secrets()
                .iter()
                .any(|secret| secret.credential() == original)
        {
            return Err(conflict("compute.compensation.authority"));
        }
        key.credential = reference.clone();
    }
    // Skip the possibly activated desired revision as well as the original one.
    repaired.revision = expected
        .revision
        .checked_add(2)
        .ok_or_else(|| conflict("compute.compensation.revision"))?;
    repaired
        .validate()
        .map_err(|_| invalid("compute.compensation.source"))?;
    let desired = serde_json::json!({"compute_management_compensation": &repaired});
    let digest =
        CanonicalDigest::of(&desired).map_err(|_| invalid("compute.compensation.digest"))?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| port("compute.compensation.begin"))?;
    let owns: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM writer_claim w JOIN operations o ON o.operation_id=w.operation_id
         WHERE w.singleton=1 AND w.operation_id=?1 AND o.generation=?2 AND o.accepted_change_digest=?3
         AND o.state IN ('rolling_back','needs_attention'))",
        params![operation.operation_id.as_str(), operation.generation, operation.accepted_digest.as_str()], |row| row.get(0))
        .map_err(|_| port("compute.compensation.writer"))?;
    if !owns {
        return Err(conflict("compute.compensation.writer"));
    }
    let owner: Option<String> = transaction.query_row(
        "SELECT m.before_owner_operation_id FROM compute_management_effects m JOIN control_effects e USING(operation_id)
         WHERE m.operation_id=?1 AND m.workspace_id=?2 AND m.source_id=?3 AND e.compensated=1",
        params![operation.operation_id.as_str(), operation.workspace_id.as_str(), expected.source_id], |row| row.get(0))
        .optional().map_err(|_| port("compute.compensation.effect"))?.flatten();
    let before_owner = owner.ok_or_else(|| conflict("compute.compensation.effect"))?;
    let current = read_source(&transaction, &operation.workspace_id, &expected.source_id)?
        .ok_or_else(|| conflict("compute.compensation.missing"))?;
    let current_owner: String = transaction.query_row(
        "SELECT owner_operation_id FROM compute_management_sources WHERE workspace_id=?1 AND source_id=?2",
        params![operation.workspace_id.as_str(), expected.source_id], |row| row.get(0))
        .map_err(|_| port("compute.compensation.current_owner"))?;
    // Source and workspace were advanced together. This exact operation-owned row
    // is the durable receipt if the process stopped before the terminal journal write.
    if current == repaired && current_owner == operation.operation_id.as_str() {
        let receipt: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM workspace_state w JOIN workspace_revision_heads h USING(workspace_id) WHERE w.workspace_id=?1 AND w.owner_operation_id=?2 AND w.desired_digest=?3 AND w.target_revision=h.revision)",
            params![operation.workspace_id.as_str(), operation.operation_id.as_str(), digest.as_str()], |row| row.get(0))
            .map_err(|_| port("compute.compensation.receipt"))?;
        return if receipt {
            Ok(())
        } else {
            Err(conflict("compute.compensation.receipt"))
        };
    }
    if &current != expected || current_owner != before_owner {
        return Err(conflict("compute.compensation.cas"));
    }
    let revision = super::super::super::read_control_head(&transaction, &operation.workspace_id)?
        .checked_add(1)
        .ok_or_else(|| conflict("compute.compensation.workspace_revision"))?;
    let encoded =
        encode(&serde_json::json!({"schema":"hiroute.control-desired/v1","value":desired}))?;
    write_source(
        &transaction,
        &operation.workspace_id,
        &repaired,
        operation.operation_id.as_str(),
    )?;
    let changed = transaction.execute("UPDATE workspace_state SET desired_json=?2,target_revision=?3,desired_digest=?4,owner_operation_id=?5,updated_at=unixepoch() WHERE workspace_id=?1",
        params![operation.workspace_id.as_str(), encoded, revision, digest.as_str(), operation.operation_id.as_str()])
        .map_err(|_| port("compute.compensation.workspace"))?;
    if changed != 1 {
        return Err(conflict("compute.compensation.workspace"));
    }
    transaction
        .execute(
            "UPDATE workspace_revision_heads SET revision=?2 WHERE workspace_id=?1",
            params![operation.workspace_id.as_str(), revision],
        )
        .map_err(|_| port("compute.compensation.head"))?;
    transaction
        .commit()
        .map_err(|_| port("compute.compensation.commit"))
}
