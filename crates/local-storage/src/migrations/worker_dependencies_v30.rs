//! Admit OpenCode native selections, preserving all existing row and journal bytes.

pub(super) const CONTROL: &str = r#"
CREATE TABLE worker_dependency_selections_v30 (
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli','pi','deepseek_harness','opencode')),
    revision INTEGER NOT NULL
        CHECK(typeof(revision) = 'integer' AND revision > 0),
    selection_json TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, harness)
);

CREATE TABLE worker_dependency_selection_effects_v30 (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli','pi','deepseek_harness','opencode')),
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

INSERT INTO worker_dependency_selections_v30
    SELECT workspace_id, harness, revision, selection_json, owner_operation_id, updated_at
    FROM worker_dependency_selections;
INSERT INTO worker_dependency_selection_effects_v30
    SELECT operation_id, workspace_id, harness, before_revision, before_json,
           before_owner_operation_id, after_revision, after_json, activated, compensated
    FROM worker_dependency_selection_effects;

DROP TABLE worker_dependency_selection_effects;
DROP TABLE worker_dependency_selections;
ALTER TABLE worker_dependency_selections_v30 RENAME TO worker_dependency_selections;
ALTER TABLE worker_dependency_selection_effects_v30 RENAME TO worker_dependency_selection_effects;
CREATE INDEX worker_dependency_selection_effects_state
    ON worker_dependency_selection_effects(activated, compensated, operation_id);
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn opencode_selection_constraint_extends_existing_tables_without_changing_raw_rows() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE operations(operation_id TEXT PRIMARY KEY); INSERT INTO operations VALUES('accepted');
            CREATE TABLE worker_dependency_selections(workspace_id TEXT,harness TEXT,revision INTEGER,selection_json TEXT,owner_operation_id TEXT,updated_at INTEGER);
            CREATE TABLE worker_dependency_selection_effects(operation_id TEXT,workspace_id TEXT,harness TEXT,before_revision INTEGER,before_json TEXT,before_owner_operation_id TEXT,after_revision INTEGER,after_json TEXT,activated INTEGER,compensated INTEGER);
            CREATE INDEX worker_dependency_selection_effects_state ON worker_dependency_selection_effects(activated,compensated,operation_id);").unwrap();
        let raw = "{  \"harness\": \"pi\", \"cli_path\": \"/selected/pi\", \"node_path\":\"/selected/node\" }";
        db.execute(
            "INSERT INTO worker_dependency_selections VALUES('w','pi',7,?1,'accepted',99)",
            [raw],
        )
        .unwrap();
        db.execute("INSERT INTO worker_dependency_selection_effects VALUES('accepted','w','pi',6,?1,'previous',7,?1,1,0)",[raw]).unwrap();
        db.execute_batch(super::CONTROL).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT selection_json FROM worker_dependency_selections WHERE harness='pi'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            raw
        );
        assert_eq!(db.query_row("SELECT before_json,after_json,activated,compensated FROM worker_dependency_selection_effects",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap(),(raw.into(),raw.into(),1,0));
        db.execute(
            "INSERT INTO worker_dependency_selections VALUES('w','opencode',1,'{}','accepted',100)",
            [],
        )
        .unwrap();
        assert!(db.execute("INSERT INTO worker_dependency_selections VALUES('w','unregistered',1,'{}','accepted',100)",[]).is_err());
    }
}
