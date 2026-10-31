//! Source operations preserve budget links and recover policy without rewinding expenditure.
use super::*;
use crate::control::key_budgets as ledger;
use hiroute_domain::KeyBudgetMutationV1;
use rusqlite::Connection;

fn bad() -> PortError {
    conflict("compute.key_budget.cas")
}
pub(super) fn validate_before(
    db: &Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    let Some(witness) = mutation.key_budgets() else {
        return Ok(());
    };
    if ledger::bindings(db, workspace, mutation.source_id())? != witness.bindings_before {
        return Err(bad());
    }
    for value in &witness.definitions_before {
        if ledger::definition(db, workspace, &value.budget_id)?.as_ref() != Some(value) {
            return Err(bad());
        }
    }
    for value in &witness.definitions_after {
        if !witness
            .definitions_before
            .iter()
            .any(|old| old.budget_id == value.budget_id)
            && ledger::definition(db, workspace, &value.budget_id)?.is_some()
        {
            return Err(bad());
        }
    }
    Ok(())
}
pub(super) fn stage(
    db: &Connection,
    workspace: &WorkspaceId,
    operation: &str,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    validate_before(db, workspace, mutation)?;
    let witness = if let Some(value) = mutation.key_budgets() {
        value.clone()
    } else {
        let before = ledger::bindings(db, workspace, mutation.source_id())?;
        if before.is_empty() {
            return Ok(());
        };
        let after = before
            .iter()
            .filter(|(key, _)| {
                mutation.desired().is_some_and(|source| {
                    source
                        .credentials
                        .iter()
                        .any(|credential| &credential.key_id == *key)
                })
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        KeyBudgetMutationV1 {
            bindings_before: before,
            bindings_after: after,
            definitions_before: vec![],
            definitions_after: vec![],
            initial_remaining: Default::default(),
        }
    };
    db.execute("INSERT INTO compute_key_budget_effects(operation_id,workspace_id,source_id,witness_json) VALUES (?1,?2,?3,?4)",params![operation,workspace.as_str(),mutation.source_id(),encode(&witness)?]).map_err(|_|bad())?;
    Ok(())
}
fn effect(
    db: &Connection,
    operation: &str,
) -> PortResult<Option<(KeyBudgetMutationV1, bool, bool)>> {
    let row:Option<(String,bool,bool)>=db.query_row("SELECT witness_json,applied,compensated FROM compute_key_budget_effects WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|bad())?;
    row.map(|(json, applied, compensated)| Ok((decode(&json)?, applied, compensated)))
        .transpose()
}
pub(super) fn current_matches(
    db: &Connection,
    workspace: &WorkspaceId,
    source: &str,
    operation: &str,
) -> PortResult<bool> {
    let Some((witness, applied, compensated)) = effect(db, operation)? else {
        return Ok(true);
    };
    if !applied || compensated || ledger::bindings(db, workspace, source)? != witness.bindings_after
    {
        return Ok(false);
    }
    // Sharing a budget delegates live policy to that budget. Later legitimate
    // edits from another member must not revoke this source's activation proof.
    Ok(true)
}
pub(super) fn apply(
    db: &Connection,
    workspace: &WorkspaceId,
    source: &str,
    operation: &str,
    restore: bool,
) -> PortResult<()> {
    let Some((witness, applied, compensated)) = effect(db, operation)? else {
        return Ok(());
    };
    if (restore && (!applied || compensated)) || (!restore && applied) {
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| bad())?
        .as_millis() as i64;
    let values = if restore {
        &witness.definitions_before
    } else {
        &witness.definitions_after
    };
    for value in values {
        let mut desired = value.clone();
        if restore {
            let after = witness
                .definitions_after
                .iter()
                .find(|v| v.budget_id == value.budget_id);
            let current = ledger::definition(db, workspace, &value.budget_id)?;
            if after == Some(value) || current.as_ref() != after {
                continue;
            }
            // Monotonic policy revisions prevent stale cross-source shared-budget edits
            // from acquiring authority again after compensation.
            desired.revision = ledger::definition(db, workspace, &value.budget_id)?
                .ok_or_else(bad)?
                .revision
                .checked_add(1)
                .ok_or_else(bad)?;
        } else if witness.definitions_before.iter().any(|old| old == value) {
            continue;
        }
        ledger::apply_definition(
            db,
            workspace,
            &desired,
            witness.initial_remaining.get(&value.budget_id).copied(),
            now,
        )?;
    }
    db.execute(
        "DELETE FROM compute_key_budget_bindings WHERE workspace_id=?1 AND source_id=?2",
        params![workspace.as_str(), source],
    )
    .map_err(|_| bad())?;
    let bindings = if restore {
        &witness.bindings_before
    } else {
        &witness.bindings_after
    };
    for (key, budget) in bindings {
        db.execute("INSERT INTO compute_key_budget_bindings(workspace_id,source_id,key_id,budget_id) VALUES (?1,?2,?3,?4)",params![workspace.as_str(),source,key,budget]).map_err(|_|bad())?;
    }
    db.execute(
        if restore {
            "UPDATE compute_key_budget_effects SET compensated=1 WHERE operation_id=?1"
        } else {
            "UPDATE compute_key_budget_effects SET applied=1 WHERE operation_id=?1"
        },
        [operation],
    )
    .map_err(|_| bad())?;
    Ok(())
}
