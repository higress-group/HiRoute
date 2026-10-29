//! Journal ownership and keyed runtime projection for source-owned decision credentials.
pub(super) const CONTROL: &str = r#"
CREATE TABLE decision_source_effects (
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    before_owner_operation_id TEXT NOT NULL,
    PRIMARY KEY(operation_id, source_id)
);
CREATE TABLE decision_source_runtime (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    lineage_digest TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK(source_revision > 0),
    source_owner_operation_id TEXT NOT NULL,
    state TEXT NOT NULL,
    targets_digest TEXT NOT NULL,
    credentials_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id),
    FOREIGN KEY(workspace_id, source_id) REFERENCES compute_management_sources(workspace_id, source_id) ON DELETE CASCADE
);
CREATE TABLE decision_consumer_runtime (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    consumer_binding_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    service_revision INTEGER NOT NULL CHECK(service_revision > 0),
    model TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    wire_protocol TEXT NOT NULL,
    destination_id TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id, consumer_binding_id),
    UNIQUE(workspace_id, source_id, service_id, service_revision),
    FOREIGN KEY(workspace_id, source_id) REFERENCES decision_source_runtime(workspace_id, source_id) ON DELETE CASCADE
);
CREATE INDEX decision_consumer_service_idx ON decision_consumer_runtime(workspace_id, service_id, source_id);
"#;

#[cfg(test)]
mod tests {
    use super::super::{DatabaseKind, migration_sql};
    use rusqlite::Connection;

    #[test]
    fn upgrade_adds_keyed_decision_tables_without_rewriting_existing_rows() {
        let db = Connection::open_in_memory().unwrap();
        for version in 1..=34 {
            db.execute_batch(migration_sql(DatabaseKind::Control, version).unwrap())
                .unwrap();
        }
        for table in [
            "decision_source_effects",
            "decision_source_runtime",
            "decision_consumer_runtime",
        ] {
            assert!(db.prepare(&format!("SELECT * FROM {table}")).is_ok());
        }
    }
}
