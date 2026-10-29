//! Converge all authenticated forward Secret generations under the same writer receipt.
use std::collections::BTreeSet;

use hiroute_domain::{
    CanonicalDigest, CredentialRefV1, DecisionServiceOperationV2, OperationState, OperationV1,
    PortError, PortErrorCode, PortResult, SecretMutationKind,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::json;

use super::super::ControlStore;
use super::super::compute::management::{read_source, write_source};

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}
fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
fn unavailable(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}

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
        return Err(conflict("decision.v2.compensation_state"));
    }
    let decision: DecisionServiceOperationV2 = serde_json::from_value(
        operation
            .plan
            .control()
            .get("decision_service_operation_v2")
            .cloned()
            .ok_or_else(|| invalid("decision.v2.compensation_operation"))?,
    )
    .map_err(|_| invalid("decision.v2.compensation_operation"))?;
    let mut seen = BTreeSet::new();
    let mut repaired = Vec::new();
    for mutation in &decision.sources {
        let mut source = mutation.before.clone();
        let mut changed = false;
        for key in &mut source.credentials {
            let Some(reference) = references
                .iter()
                .find(|item| item.credential_id() == key.key_id)
            else {
                continue;
            };
            if !seen.insert(reference.credential_id()) {
                return Err(invalid("decision.v2.compensation_duplicate"));
            }
            let original = &key.credential;
            let generation = original
                .generation()
                .checked_add(2)
                .ok_or_else(|| conflict("decision.v2.compensation_generation"))?;
            let exact = CredentialRefV1::new(
                original.credential_id(),
                original.owner_scope(),
                original.subject(),
                original.purpose(),
                original.allowed_destinations().iter().cloned(),
                generation,
            )
            .map_err(|_| invalid("decision.v2.compensation_reference"))?;
            if reference != &exact
                || !operation.plan.secrets().iter().any(|secret| {
                    secret.kind() == SecretMutationKind::Rebind && secret.credential() == original
                })
            {
                return Err(conflict("decision.v2.compensation_authority"));
            }
            key.credential = reference.clone();
            changed = true;
        }
        if changed {
            source.revision = source
                .revision
                .checked_add(2)
                .ok_or_else(|| conflict("decision.v2.compensation_revision"))?;
            source
                .validate()
                .map_err(|_| invalid("decision.v2.compensation_source"))?;
            repaired.push((mutation, source));
        }
    }
    if seen.len() != references.len() || repaired.is_empty() {
        return Err(invalid("decision.v2.compensation_reference"));
    }
    let desired = json!({"decision_source_compensation": repaired.iter().map(|(_, source)| source).collect::<Vec<_>>()});
    let digest =
        CanonicalDigest::of(&desired).map_err(|_| invalid("decision.v2.compensation_digest"))?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| unavailable("decision.v2.compensation_begin"))?;
    let owns: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM writer_claim w JOIN operations o ON o.operation_id=w.operation_id WHERE w.singleton=1 AND w.operation_id=?1 AND o.generation=?2 AND o.accepted_change_digest=?3 AND o.state IN ('rolling_back','needs_attention'))",
        params![operation.operation_id.as_str(), operation.generation, operation.accepted_digest.as_str()], |row| row.get(0))
        .map_err(|_| unavailable("decision.v2.compensation_writer"))?;
    if !owns {
        return Err(conflict("decision.v2.compensation_writer"));
    }
    let mut already = 0;
    for (mutation, source) in &repaired {
        let owner: Option<String> = transaction.query_row(
            "SELECT d.before_owner_operation_id FROM decision_source_effects d JOIN control_effects e USING(operation_id) WHERE d.operation_id=?1 AND d.workspace_id=?2 AND d.source_id=?3 AND e.compensated=1",
            params![operation.operation_id.as_str(), operation.workspace_id.as_str(), mutation.before.source_id], |row| row.get(0))
            .optional().map_err(|_| unavailable("decision.v2.compensation_effect"))?;
        let owner = owner.ok_or_else(|| conflict("decision.v2.compensation_effect"))?;
        let current = read_source(
            &transaction,
            &operation.workspace_id,
            &mutation.before.source_id,
        )?
        .ok_or_else(|| conflict("decision.v2.compensation_source"))?;
        let current_owner: String = transaction.query_row("SELECT owner_operation_id FROM compute_management_sources WHERE workspace_id=?1 AND source_id=?2",
            params![operation.workspace_id.as_str(), mutation.before.source_id], |row| row.get(0))
            .map_err(|_| unavailable("decision.v2.compensation_owner"))?;
        if current == *source && current_owner == operation.operation_id.as_str() {
            already += 1;
        } else if current != mutation.before || current_owner != owner {
            return Err(conflict("decision.v2.compensation_cas"));
        }
    }
    if already != 0 {
        if already != repaired.len() {
            return Err(conflict("decision.v2.compensation_partial"));
        }
        let receipt: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM workspace_state w JOIN workspace_revision_heads h USING(workspace_id) WHERE w.workspace_id=?1 AND w.owner_operation_id=?2 AND w.desired_digest=?3 AND w.target_revision=h.revision)",
            params![operation.workspace_id.as_str(), operation.operation_id.as_str(), digest.as_str()], |row| row.get(0))
            .map_err(|_| unavailable("decision.v2.compensation_receipt"))?;
        return if receipt {
            Ok(())
        } else {
            Err(conflict("decision.v2.compensation_receipt"))
        };
    }
    let revision = super::super::read_control_head(&transaction, &operation.workspace_id)?
        .checked_add(1)
        .ok_or_else(|| conflict("decision.v2.compensation_workspace_revision"))?;
    for (_, source) in &repaired {
        write_source(
            &transaction,
            &operation.workspace_id,
            source,
            operation.operation_id.as_str(),
        )?;
    }
    let encoded =
        serde_json::to_string(&json!({"schema":"hiroute.control-desired/v1","value":desired}))
            .map_err(|_| invalid("decision.v2.compensation_encode"))?;
    let changed = transaction.execute("UPDATE workspace_state SET desired_json=?2,target_revision=?3,desired_digest=?4,owner_operation_id=?5,updated_at=unixepoch() WHERE workspace_id=?1",
        params![operation.workspace_id.as_str(), encoded, revision, digest.as_str(), operation.operation_id.as_str()])
        .map_err(|_| unavailable("decision.v2.compensation_workspace"))?;
    if changed != 1 {
        return Err(conflict("decision.v2.compensation_workspace"));
    }
    transaction
        .execute(
            "UPDATE workspace_revision_heads SET revision=?2 WHERE workspace_id=?1",
            params![operation.workspace_id.as_str(), revision],
        )
        .map_err(|_| unavailable("decision.v2.compensation_head"))?;
    transaction
        .commit()
        .map_err(|_| unavailable("decision.v2.compensation_commit"))
}
