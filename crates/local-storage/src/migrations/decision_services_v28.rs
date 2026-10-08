pub(super) const CONTROL: &str = r#"
CREATE TABLE decision_services (
    workspace_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    service_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id, service_id, revision)
);
CREATE TABLE decision_service_operations (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id)
);
"#;

#[cfg(test)]
mod tests {
    use super::super::{DatabaseKind, migration_sql};
    use rusqlite::Connection;

    #[test]
    fn decision_upgrade_follows_released_dsh_schema() {
        let connection = Connection::open_in_memory().unwrap();
        for version in 1..=27 {
            connection
                .execute_batch(migration_sql(DatabaseKind::Control, version).unwrap())
                .unwrap();
        }
        let worker_schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='worker_dependency_selections'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(worker_schema.contains("deepseek_harness"));
        assert!(
            connection
                .prepare("SELECT * FROM decision_services")
                .is_err()
        );

        connection
            .execute_batch(migration_sql(DatabaseKind::Control, 28).unwrap())
            .unwrap();
        connection
            .execute(
                "INSERT INTO decision_services VALUES ('personal/default','decision',1,'{}')",
                [],
            )
            .unwrap();
        let retained_worker_schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='worker_dependency_selections'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(worker_schema, retained_worker_schema);
        connection
            .prepare("SELECT operation_id FROM decision_service_operations")
            .unwrap();
    }
}
