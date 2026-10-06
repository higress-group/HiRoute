//! Extend Native DSH Worker installation support without decoding or reserializing stored journals.

pub(super) const CONTROL: &str = r#"
CREATE TABLE worker_dependency_selections_v27 (
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli','pi','deepseek_harness')),
    revision INTEGER NOT NULL
        CHECK(typeof(revision) = 'integer' AND revision > 0),
    selection_json TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, harness)
);

CREATE TABLE worker_dependency_selection_effects_v27 (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli','pi','deepseek_harness')),
    before_revision INTEGER NOT NULL
        CHECK(typeof(before_revision) = 'integer' AND before_revision >= 0),
    before_json TEXT,
    before_owner_operation_id TEXT,
    after_revision INTEGER NOT NULL
        CHECK(typeof(after_revision) = 'integer' AND after_revision > 0),
    after_json TEXT NOT NULL,
    activated INTEGER NOT NULL DEFAULT 0 CHECK(activated IN (0,1)),
    compensated INTEGER NOT NULL DEFAULT 0 CHECK(compensated IN (0,1)),
    UNIQUE(workspace_id, harness, after_revision)
);

INSERT INTO worker_dependency_selections_v27
    SELECT workspace_id, harness, revision, selection_json, owner_operation_id, updated_at
    FROM worker_dependency_selections;
INSERT INTO worker_dependency_selection_effects_v27
    SELECT operation_id, workspace_id, harness, before_revision, before_json,
           before_owner_operation_id, after_revision, after_json, activated, compensated
    FROM worker_dependency_selection_effects;

DROP TABLE worker_dependency_selection_effects;
DROP TABLE worker_dependency_selections;
ALTER TABLE worker_dependency_selections_v27 RENAME TO worker_dependency_selections;
ALTER TABLE worker_dependency_selection_effects_v27 RENAME TO worker_dependency_selection_effects;
CREATE INDEX worker_dependency_selection_effects_state
    ON worker_dependency_selection_effects(activated, compensated, operation_id);
"#;

#[cfg(test)]
mod tests {
    use super::super::*;
    #[test]
    fn dsh_upgrade_preserves_existing_selection_and_effect_bytes() {
        let connection = Connection::open_in_memory().unwrap();
        for version in 1..=26 {
            connection
                .execute_batch(migration_sql(DatabaseKind::Control, version).unwrap())
                .unwrap();
        }
        let before = " { \"historical\" : [ 1, 2 ] }\n";
        let after = "{\"opaque\":true, \"ordering\" : 42}\n";
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        // This SQL migration fixture needs real parent rows. Native journal replay is
        // covered by the frozen schema-23 product fixtures, not these opaque payloads.
        for owner in [
            "operation-codex_cli",
            "operation-claude_code",
            "operation-qoder_cli",
            "operation-pi",
            "new-owner",
        ] {
            connection.execute(
                "INSERT INTO operations VALUES (?1,'personal/default','fixture-owner','worker_dependency_selection',?1,'request','accepted','succeeded',1,'{}',123,123)",
                [owner],
            ).unwrap();
        }
        for harness in ["codex_cli", "claude_code", "qoder_cli", "pi"] {
            connection.execute("INSERT INTO worker_dependency_selections VALUES ('personal/default',?1,7,?2,?3,123)",params![harness,after,format!("operation-{harness}")]).unwrap();
            connection.execute("INSERT INTO worker_dependency_selection_effects VALUES (?1,'personal/default',?2,6,?3,'previous-owner',7,?4,1,0)",params![format!("operation-{harness}"),harness,before,after]).unwrap();
        }
        connection.execute_batch(super::CONTROL).unwrap();
        let selections: Vec<(String, String)> = connection
            .prepare(
                "SELECT harness,selection_json FROM worker_dependency_selections ORDER BY harness",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(selections.len(), 4);
        assert!(selections.iter().all(|(_, raw)| raw == after));
        let effects:Vec<(String,String,i64,i64)> = connection.prepare("SELECT before_json,after_json,activated,compensated FROM worker_dependency_selection_effects").unwrap().query_map([],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(effects, vec![(before.into(), after.into(), 1, 0); 4]);
        connection.execute("INSERT INTO worker_dependency_selections VALUES ('personal/default','deepseek_harness',1,'{}','new-owner',124)",[]).unwrap();
        assert!(connection.execute("INSERT INTO worker_dependency_selections VALUES ('personal/default','unknown',1,'{}','new-owner',124)",[]).is_err());
        assert!(
            connection
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none()
        );
    }
}
