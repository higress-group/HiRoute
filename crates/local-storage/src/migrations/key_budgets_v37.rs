//! Additive local spending authority; source/Key and older operation bytes stay unchanged.
pub(super) const CONTROL: &str = r#"
CREATE TABLE compute_key_budgets (
 workspace_id TEXT NOT NULL, budget_id TEXT NOT NULL, revision INTEGER NOT NULL,
 definition_json TEXT NOT NULL, balance_micros INTEGER NOT NULL, budget_epoch INTEGER NOT NULL DEFAULT 0,
 reserved_micros INTEGER NOT NULL DEFAULT 0 CHECK(reserved_micros>=0),
 spent_micros INTEGER NOT NULL DEFAULT 0 CHECK(spent_micros>=0),
 lifetime_spent_micros INTEGER NOT NULL DEFAULT 0 CHECK(lifetime_spent_micros>=0),
 renewal_index INTEGER NOT NULL DEFAULT 0 CHECK(renewal_index>=0),
 PRIMARY KEY(workspace_id,budget_id)
);
CREATE TABLE compute_key_budget_bindings (
 workspace_id TEXT NOT NULL, source_id TEXT NOT NULL, key_id TEXT NOT NULL, budget_id TEXT NOT NULL,
 PRIMARY KEY(workspace_id,key_id),
 FOREIGN KEY(workspace_id,source_id) REFERENCES compute_management_sources(workspace_id,source_id) ON DELETE CASCADE,
 FOREIGN KEY(workspace_id,budget_id) REFERENCES compute_key_budgets(workspace_id,budget_id)
);
CREATE INDEX compute_key_budget_source ON compute_key_budget_bindings(workspace_id,source_id);
CREATE INDEX compute_key_budget_members ON compute_key_budget_bindings(workspace_id,budget_id);
CREATE TABLE compute_key_spend_reservations (
 workspace_id TEXT NOT NULL, reservation_id TEXT NOT NULL, budget_id TEXT NOT NULL,
 key_id TEXT NOT NULL, key_generation INTEGER NOT NULL, budget_epoch INTEGER NOT NULL,
 amount_micros INTEGER NOT NULL CHECK(amount_micros>=0),
 pricing_json TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('reserved','sent','settled','released','pending')),
 created_at_ms INTEGER NOT NULL, settled_at_ms INTEGER, charged_micros INTEGER,
 PRIMARY KEY(workspace_id,reservation_id),
 FOREIGN KEY(workspace_id,budget_id) REFERENCES compute_key_budgets(workspace_id,budget_id)
);
CREATE INDEX compute_key_spend_budget ON compute_key_spend_reservations(workspace_id,budget_id,state);
CREATE TABLE compute_key_budget_effects (
 operation_id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, source_id TEXT NOT NULL,
 witness_json TEXT NOT NULL, applied INTEGER NOT NULL DEFAULT 0,
 compensated INTEGER NOT NULL DEFAULT 0
);
"#;

pub(super) fn initialize(tx: &rusqlite::Transaction<'_>) -> Result<(), crate::LocalStorageError> {
    super::decision_sources_v34::refresh_locator(tx)?;
    Ok(())
}

pub(crate) fn validate(connection: &rusqlite::Connection) -> Result<(), crate::LocalStorageError> {
    connection.prepare("SELECT definition_json,balance_micros,reserved_micros,spent_micros,lifetime_spent_micros,renewal_index FROM compute_key_budgets LIMIT 0")?;
    connection.prepare("SELECT budget_id,key_id,key_generation,pricing_json,state FROM compute_key_spend_reservations LIMIT 0")?;
    connection.prepare("SELECT operation_id,witness_json,applied,compensated FROM compute_key_budget_effects LIMIT 0")?;
    let mut statement =
        connection.prepare("SELECT budget_id,revision,definition_json FROM compute_key_budgets")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, revision, json) = row?;
        let definition: hiroute_domain::KeyBudgetDefinitionV1 =
            serde_json::from_str(&json).map_err(|_| crate::LocalStorageError::InvalidData)?;
        definition
            .validate()
            .map_err(|_| crate::LocalStorageError::InvalidData)?;
        if id != definition.budget_id || revision != definition.revision {
            return Err(crate::LocalStorageError::InvalidData);
        }
    }
    Ok(())
}
