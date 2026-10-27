//! A local routing-input revision, independent of grants used to read the service.
use rusqlite::Connection;

// These are the durable inputs read by routing_snapshot and plan_content_snapshot.
// Journal progress, observation cursors and native apply capabilities are not inputs.
const TABLES: &[&str] = &[
    "storage_meta",
    "workspace_revision_heads",
    "dependency_revisions",
    "compute_sources",
    "source_bindings",
    "source_inventory_snapshots",
    "credential_pools",
    "compute_management_sources",
    "plan_heads",
    "plan_drafts",
    "decision_services",
    "gateway_publications",
    "publication_heads",
];

fn definitions() -> impl Iterator<Item = (String, String)> {
    TABLES.iter().flat_map(|table| {
        ["INSERT", "UPDATE", "DELETE"]
            .into_iter()
            .map(move |event| {
                let name = format!("routing_read_{table}_{}", event.to_ascii_lowercase());
                let sql = format!(
                    "CREATE TRIGGER {name} AFTER {event} ON {table} BEGIN
                SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM routing_read_revision WHERE singleton=1)
                    THEN RAISE(ABORT,'missing routing input revision') END;
                UPDATE routing_read_revision SET revision=revision+1 WHERE singleton=1;
            END"
                );
                (name, sql)
            })
    })
}

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE routing_read_revision (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0)
    ); INSERT INTO routing_read_revision VALUES(1,0);",
    )?;
    for (_, sql) in definitions() {
        db.execute_batch(&sql)?;
    }
    Ok(())
}

pub(crate) fn validate(db: &Connection) -> rusqlite::Result<()> {
    for (name, sql) in definitions() {
        let actual: String = db.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [name],
            |r| r.get(0),
        )?;
        if actual != sql {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn remove_test_shape(db: &Connection) {
    for (name, _) in definitions() {
        db.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    }
    db.execute_batch("DROP TABLE routing_read_revision")
        .unwrap();
}
