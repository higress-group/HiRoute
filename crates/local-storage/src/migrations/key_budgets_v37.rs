//! Additive local spending authority; source/Key and older operation bytes stay unchanged.
pub(super) const CONTROL: &str = r#"
ALTER TABLE compute_management_effects ADD COLUMN key_budget_witness_digest TEXT;
ALTER TABLE compute_management_effects ADD COLUMN key_budget_accounting_digest TEXT;
ALTER TABLE compute_management_effects ADD COLUMN key_budget_compensation_root TEXT;
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
CREATE INDEX compute_key_spend_budget ON compute_key_spend_reservations(workspace_id,budget_id,budget_epoch,state);
CREATE INDEX compute_key_spend_pending ON compute_key_spend_reservations(state) WHERE state IN ('reserved','sent','pending');
CREATE TABLE compute_key_spend_charges (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id TEXT NOT NULL, budget_id TEXT NOT NULL,
 reservation_id TEXT NOT NULL, budget_epoch INTEGER NOT NULL, created_at_ms INTEGER NOT NULL,
 settled_at_ms INTEGER NOT NULL, charged_micros INTEGER NOT NULL CHECK(charged_micros>=0),
 UNIQUE(workspace_id,reservation_id)
);
CREATE INDEX compute_key_spend_charge_budget ON compute_key_spend_charges(workspace_id,budget_id,sequence);

CREATE TABLE compute_key_budget_effects (
 operation_id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, source_id TEXT NOT NULL,
 witness_json TEXT NOT NULL, accounting_before_json TEXT, applied INTEGER NOT NULL DEFAULT 0,
 compensated INTEGER NOT NULL DEFAULT 0
);
"#;

pub(super) fn initialize(tx: &rusqlite::Transaction<'_>) -> Result<(), crate::LocalStorageError> {
    super::decision_sources_v34::refresh_locator(tx)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn remove_test_shape(db: &rusqlite::Connection) {
    db.execute_batch(
        "DROP TABLE compute_key_budget_effects;
         DROP TABLE compute_key_spend_charges;
         DROP TABLE compute_key_spend_reservations;
         DROP TABLE compute_key_budget_bindings;
         DROP TABLE compute_key_budgets;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_witness_digest;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_accounting_digest;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_compensation_root;",
    )
    .unwrap();
}

pub(crate) fn validate(connection: &rusqlite::Connection) -> Result<(), crate::LocalStorageError> {
    connection.prepare("SELECT definition_json,balance_micros,reserved_micros,spent_micros,lifetime_spent_micros,renewal_index FROM compute_key_budgets LIMIT 0")?;
    connection.prepare("SELECT budget_id,key_id,key_generation,pricing_json,state FROM compute_key_spend_reservations LIMIT 0")?;
    connection.prepare("SELECT operation_id,witness_json,applied,compensated FROM compute_key_budget_effects LIMIT 0")?;
    connection.prepare("SELECT sequence,reservation_id,budget_epoch,charged_micros FROM compute_key_spend_charges LIMIT 0")?;
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
    let inconsistent: bool=connection.query_row("SELECT EXISTS(
        SELECT 1 FROM compute_key_budgets b WHERE b.revision<1 OR b.budget_epoch<0 OR b.renewal_index<0
        OR b.reserved_micros<0 OR b.spent_micros<0 OR b.lifetime_spent_micros<0
        OR b.reserved_micros != COALESCE((SELECT SUM(r.amount_micros) FROM compute_key_spend_reservations r
            WHERE r.workspace_id=b.workspace_id AND r.budget_id=b.budget_id AND r.budget_epoch=b.budget_epoch
                AND r.state IN ('reserved','sent','pending')),0)
        UNION ALL SELECT 1 FROM compute_key_budget_bindings k
        LEFT JOIN compute_management_sources s USING(workspace_id,source_id)
        LEFT JOIN compute_key_budgets b ON b.workspace_id=k.workspace_id AND b.budget_id=k.budget_id
        WHERE s.source_id IS NULL OR b.budget_id IS NULL OR NOT EXISTS(
            SELECT 1 FROM json_each(s.source_json,'$.credentials') c WHERE json_extract(c.value,'$.key_id')=k.key_id)
    )",[],|r|r.get(0))?;
    if inconsistent {
        return Err(crate::LocalStorageError::InvalidData);
    }
    // Only unresolved reservations remain startup dependencies; settled history
    // is immutable evidence and is not decoded on every service start.
    let mut reservations=connection.prepare("SELECT amount_micros,pricing_json FROM compute_key_spend_reservations WHERE state IN ('reserved','sent','pending')")?;
    for row in reservations.query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))? {
        let (amount, json) = row?;
        let estimate: hiroute_domain::KeySpendEstimateV1 =
            serde_json::from_str(&json).map_err(|_| crate::LocalStorageError::InvalidData)?;
        if estimate.upper_bound_micros() != Some(amount) {
            return Err(crate::LocalStorageError::InvalidData);
        }
    }
    Ok(())
}
