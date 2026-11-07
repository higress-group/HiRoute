//! Preserve budget authority when decision operations own a shared source.
use super::*;
use std::collections::BTreeSet;

pub(super) const CONTROL: &str =
    "ALTER TABLE decision_source_effects ADD COLUMN key_budget_owner_operation_id TEXT;
ALTER TABLE decision_source_effects ADD COLUMN key_budget_witness_digest TEXT;";

pub(super) fn initialize(tx: &rusqlite::Transaction<'_>) -> Result<(), LocalStorageError> {
    let mut sources = tx.prepare(
        "SELECT workspace_id,source_id,owner_operation_id FROM compute_management_sources",
    )?;
    for row in sources.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (workspace, source, mut owner) = row?;
        let mut seen = BTreeSet::new();
        let mut decisions = Vec::new();
        loop {
            if !seen.insert(owner.clone()) {
                return Err(LocalStorageError::InvalidData);
            }
            let before: Option<String> = tx.query_row("SELECT before_owner_operation_id FROM decision_source_effects WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3", params![owner,workspace,source], |r| r.get(0)).optional()?;
            let Some(before) = before else {
                break;
            };
            decisions.push(owner);
            owner = before;
        }
        let digest: Option<String> = tx.query_row("SELECT key_budget_witness_digest FROM compute_management_effects WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3", params![owner,workspace,source], |r| r.get(0)).optional()?.flatten();
        for decision in decisions {
            tx.execute("UPDATE decision_source_effects SET key_budget_owner_operation_id=?4,key_budget_witness_digest=?5 WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3", params![decision,workspace,source,owner,digest])?;
        }
    }
    super::decision_sources_v34::refresh_locator(tx)
}
