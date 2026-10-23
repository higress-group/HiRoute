//! Rebuildable analysis projections in activity.db. Shared release markers and
//! strict execution/receipt payloads are intentionally unchanged.
use rusqlite::{Connection, OptionalExtension, params};

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;

pub(super) const FORMAT: &str = "1";

pub(crate) fn available(db: &Connection) -> bool {
    format_supported(db)
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='observation_native_origins_v1')",
            [], |r|r.get::<_,bool>(0),
        ).unwrap_or(false)
}

fn format_supported(db: &Connection) -> bool {
    db.query_row(
        "SELECT value FROM observation_analysis_meta_v1 WHERE key='format'",
        [],
        |r| r.get::<_, String>(0),
    )
    .is_ok_and(|value| value == FORMAT)
}

pub(crate) fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='observation_analysis_meta_v1')", [], |r| r.get(0))?;
    if exists && !format_supported(db) {
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute_batch(DDL)?;
    let segment_column:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('observation_analysis_turns_v1') WHERE name='segment_id')",[],|r|r.get(0))?;
    if !segment_column {
        tx.execute_batch("ALTER TABLE observation_analysis_turns_v1 ADD COLUMN segment_id TEXT;
          INSERT OR IGNORE INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence)
            SELECT workspace_id,request_id,0 FROM observation_analysis_turns_v1 WHERE segment_id IS NULL;")?;
    }
    let mut rebuild_turns = false;
    for (column, declaration) in [
        ("first_request_id", "TEXT"),
        ("last_request_id", "TEXT"),
        ("membership_conflict", "INTEGER NOT NULL DEFAULT 0"),
        ("membership_revision", "INTEGER NOT NULL DEFAULT 0"),
        ("branch_id", "TEXT"),
        ("admission_digest", "TEXT"),
    ] {
        let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('observation_analysis_turns_v1') WHERE name=?1)",[column],|r|r.get(0))?;
        if !present {
            tx.execute_batch(&format!(
                "ALTER TABLE observation_analysis_turns_v1 ADD COLUMN {column} {declaration};"
            ))?;
            rebuild_turns = true;
        }
    }
    let membership_version: Option<String> = tx
        .query_row(
            "SELECT value FROM observation_analysis_meta_v1 WHERE key='turn_membership_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    rebuild_turns |= membership_version.as_deref() != Some("3");
    tx.execute("INSERT INTO observation_analysis_meta_v1(key,value) VALUES('turn_membership_version','3') ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])?;
    if rebuild_turns {
        tx.execute_batch(
            "INSERT OR IGNORE INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence)
            SELECT workspace_id,request_id,0 FROM logical_requests;
            DELETE FROM observation_analysis_turns_v1;",
        )?;
    }
    for key in [
        "dirty_sequence",
        "job_started_ms",
        "job_last_progress_ms",
        "job_processed",
        "job_total",
        "job_target",
        "job_status",
    ] {
        tx.execute(
            "INSERT OR IGNORE INTO observation_analysis_meta_v1(key,value) VALUES(?1,'0')",
            [key],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO observation_analysis_meta_v1(key,value) VALUES('format',?1)",
        [FORMAT],
    )?;
    install_dirty_triggers(&tx)?;
    let covered: Option<String> = tx
        .query_row(
            "SELECT value FROM observation_analysis_meta_v1 WHERE key='covered_u'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut needs_cuts = covered.is_none();
    if covered.is_none() {
        // First admission/real missing coverage may scan metadata. A matched
        // ordinary reopen never repopulates or rebuilds this workset.
        tx.execute("INSERT OR IGNORE INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence) SELECT workspace_id,request_id,0 FROM logical_requests",[])?;
        let missing: u64 = tx.query_row(
            "SELECT COUNT(*) FROM observation_analysis_dirty_v1",
            [],
            |r| r.get(0),
        )?;
        if missing == 0 {
            save_checkpoint(&tx)?;
        } else {
            set(&tx, "covered_u", "-1")?;
            set(&tx, "covered_v", "-1")?;
        }
    } else {
        let saved:(i64,i64)=tx.query_row("SELECT (SELECT CAST(value AS INTEGER) FROM observation_analysis_meta_v1 WHERE key='covered_u'),(SELECT CAST(value AS INTEGER) FROM observation_analysis_meta_v1 WHERE key='covered_v')",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if saved != checkpoint(&tx)? {
            needs_cuts = true;
            // Old writers with the auxiliary triggers already installed notify
            // precisely. Also recover first-generation or interrupted admission
            // without trusting a checkpoint over missing source revisions.
            tx.execute("INSERT OR IGNORE INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence)
              SELECT r.workspace_id,r.request_id,0 FROM logical_requests r
              LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
              WHERE NOT EXISTS(SELECT 1 FROM observation_analysis_requests_v1 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id
                AND a.revision=(SELECT MAX(p.revision) FROM observation_analysis_requests_v1 p WHERE p.workspace_id=r.workspace_id AND p.request_id=r.request_id)
                AND a.input_digest=COALESCE(v.input_digest,'')
                AND a.recorded_pending=NOT EXISTS(SELECT 1 FROM valuation_records_v2 vr WHERE vr.workspace_id=r.workspace_id AND vr.request_id=r.request_id AND vr.input_digest=v.input_digest))",[])?;
        }
    }
    // Retain proven historical cuts even if a compatible writer subsequently
    // clears the typed tombstone. Do not manufacture cuts from empty history.
    if needs_cuts {
        tx.execute("INSERT INTO observation_analysis_cuts_v1 SELECT workspace_id,session_id,deleted_at_ms,'full' FROM observation_tombstones WHERE delete_scope='facts_and_content'
            ON CONFLICT(workspace_id,session_id,kind) DO UPDATE SET through_ms=MAX(through_ms,excluded.through_ms)",[])?;
    }
    tx.commit()
}

fn install_dirty_triggers(db: &Connection) -> rusqlite::Result<()> {
    // These SQL-only notifications also remain executable by a compatible
    // released writer. They never require a new Rust/SQLite function, alter a
    // source row, or pretend its derived projection has been reconciled. They
    // are a retained-writer dependency: any table replacement must replace
    // these triggers in the same transaction, before core writes can resume.
    for table in [
        "logical_requests",
        "valuation_requests_v2",
        "valuation_attempt_inputs_v2",
        "valuation_records_v2",
    ] {
        for (event, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            let name = format!("analysis_dirty_{table}_{}", event.to_lowercase());
            db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS {name} AFTER {event} ON {table} BEGIN
                UPDATE observation_analysis_meta_v1 SET value=CAST(value AS INTEGER)+1 WHERE key='dirty_sequence';
                INSERT INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence) VALUES({row}.workspace_id,{row}.request_id,(SELECT CAST(value AS INTEGER) FROM observation_analysis_meta_v1 WHERE key='dirty_sequence'))
                ON CONFLICT(workspace_id,request_id) DO UPDATE SET sequence=excluded.sequence;
                END;"))?;
        }
    }
    // A newly authenticated grant or an Agent conflict changes every request's
    // attribution in this session, including already projected requests.
    db.execute_batch("CREATE TRIGGER IF NOT EXISTS analysis_dirty_session_agent_v1
        AFTER UPDATE OF agent_id ON sessions WHEN OLD.agent_id!=NEW.agent_id
        BEGIN
          UPDATE observation_analysis_meta_v1 SET value=CAST(value AS INTEGER)+1 WHERE key='dirty_sequence';
          INSERT INTO observation_analysis_dirty_v1(workspace_id,request_id,sequence)
            SELECT workspace_id,request_id,(SELECT CAST(value AS INTEGER) FROM observation_analysis_meta_v1 WHERE key='dirty_sequence')
            FROM logical_requests WHERE workspace_id=NEW.workspace_id AND session_id=NEW.session_id
            ON CONFLICT(workspace_id,request_id) DO UPDATE SET sequence=excluded.sequence;
        END;")?;
    // Preserve an actually observed typed deletion, including a later old
    // writer clearing the original tombstone. Absence is not deletion evidence.
    db.execute_batch("CREATE TRIGGER IF NOT EXISTS analysis_typed_delete_v1 AFTER INSERT ON observation_tombstones
        WHEN NEW.delete_scope='facts_and_content'
        BEGIN INSERT INTO observation_analysis_cuts_v1(workspace_id,session_id,through_ms,kind)
            VALUES(NEW.workspace_id,NEW.session_id,MIN(NEW.deleted_at_ms,COALESCE((SELECT through_ms FROM observation_session_barriers_v2 b WHERE b.workspace_id=NEW.workspace_id AND b.session_id=NEW.session_id),NEW.deleted_at_ms)),'full')
            ON CONFLICT(workspace_id,session_id,kind) DO UPDATE SET through_ms=MAX(through_ms,excluded.through_ms); END;")
}

pub(super) fn get(db: &Connection, key: &str) -> rusqlite::Result<u64> {
    Ok(db
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_analysis_meta_v1 WHERE key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}
pub(super) fn set(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute("INSERT INTO observation_analysis_meta_v1(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE observation_analysis_meta_v1.value!=excluded.value",params![key,value])?;
    Ok(())
}
pub(super) fn advance(db: &Connection, key: &str) -> rusqlite::Result<u64> {
    db.execute("INSERT INTO observation_analysis_meta_v1(key,value) VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1",[key])?;
    get(db, key)
}
pub(super) fn checkpoint(db: &Connection) -> rusqlite::Result<(i64, i64)> {
    Ok((
        db.query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='store_revision'",
            [],
            |r| r.get(0),
        )?,
        db.query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM valuation_records_v2",
            [],
            |r| r.get(0),
        )?,
    ))
}
pub(super) fn save_checkpoint(db: &Connection) -> rusqlite::Result<()> {
    let (u, v) = checkpoint(db)?;
    set(db, "covered_u", &u.to_string())?;
    set(db, "covered_v", &v.to_string())
}

const DDL: &str = r#"
CREATE INDEX IF NOT EXISTS analysis_source_guard_v1 ON logical_requests(workspace_id,request_id,session_id,started_at_ms);
CREATE INDEX IF NOT EXISTS analysis_source_plan_v1 ON valuation_requests_v2(workspace_id,request_id,plan_id);
CREATE INDEX IF NOT EXISTS analysis_source_agent_v1 ON sessions(workspace_id,session_id,agent_id);
CREATE TABLE IF NOT EXISTS observation_analysis_meta_v1(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS observation_analysis_dirty_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,sequence INTEGER NOT NULL,
 PRIMARY KEY(workspace_id,request_id));
CREATE INDEX IF NOT EXISTS analysis_dirty_sequence_v1 ON observation_analysis_dirty_v1(sequence DESC);
CREATE TABLE IF NOT EXISTS observation_analysis_cuts_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,through_ms INTEGER NOT NULL,kind TEXT NOT NULL,
 PRIMARY KEY(workspace_id,session_id,kind));
CREATE TABLE IF NOT EXISTS observation_native_origins_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,not_before_ms INTEGER NOT NULL,
 PRIMARY KEY(workspace_id,session_id));
CREATE TABLE IF NOT EXISTS observation_analysis_requests_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,revision INTEGER NOT NULL,
 session_id TEXT NOT NULL,turn_id TEXT NOT NULL,at_ms INTEGER NOT NULL,finished_ms INTEGER,
 plan_id TEXT,agent_id TEXT NOT NULL,traffic TEXT NOT NULL,terminal INTEGER NOT NULL,
 partial INTEGER NOT NULL,accepted_ordinal INTEGER,input_digest TEXT NOT NULL,recorded_pending INTEGER NOT NULL,
 native_model TEXT,model_id TEXT,reasoning_profile_id TEXT,producer_epoch TEXT,turn_ordinal INTEGER,
 PRIMARY KEY(workspace_id,request_id,revision));
CREATE INDEX IF NOT EXISTS analysis_requests_scope_v1 ON observation_analysis_requests_v1(workspace_id,at_ms,request_id,revision DESC);
CREATE INDEX IF NOT EXISTS analysis_requests_session_v1 ON observation_analysis_requests_v1(workspace_id,session_id,at_ms,request_id,revision DESC);
CREATE TABLE IF NOT EXISTS observation_analysis_attempts_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,revision INTEGER NOT NULL,ordinal INTEGER NOT NULL,
 pricing_key TEXT,input_tokens TEXT,output_tokens TEXT,cache_read TEXT,cache_write TEXT,reasoning TEXT,
 PRIMARY KEY(workspace_id,request_id,revision,ordinal));
CREATE TABLE IF NOT EXISTS observation_analysis_pricing_v1(
 key TEXT PRIMARY KEY,body TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS observation_analysis_turns_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,agent_turn_id TEXT NOT NULL,
 ordinal INTEGER NOT NULL,producer_epoch TEXT NOT NULL,partial INTEGER NOT NULL,segment_id TEXT,first_request_id TEXT,last_request_id TEXT,
 membership_conflict INTEGER NOT NULL DEFAULT 0,
 membership_revision INTEGER NOT NULL DEFAULT 0,branch_id TEXT,admission_digest TEXT,
 PRIMARY KEY(workspace_id,request_id));
CREATE TABLE IF NOT EXISTS observation_analysis_models_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,ordinal INTEGER NOT NULL,
 native_model TEXT NOT NULL,model_id TEXT NOT NULL,profile_digest TEXT NOT NULL,credential_digest TEXT NOT NULL,
 PRIMARY KEY(workspace_id,request_id,ordinal));
CREATE TABLE IF NOT EXISTS observation_analysis_reasoning_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,model_id TEXT NOT NULL,profile_digest TEXT NOT NULL,reasoning_profile_id TEXT,
 PRIMARY KEY(workspace_id,request_id,model_id,profile_digest));
CREATE TABLE IF NOT EXISTS observation_analysis_recorded_v1(
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,revision INTEGER NOT NULL,currency TEXT NOT NULL,
 valuation_kind TEXT NOT NULL,amount TEXT,coverage TEXT NOT NULL,
 PRIMARY KEY(workspace_id,request_id,revision,currency,valuation_kind));
CREATE TABLE IF NOT EXISTS observation_analysis_scopes_v1(
 id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,body TEXT NOT NULL,expires_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS analysis_scope_expiry_v1 ON observation_analysis_scopes_v1(expires_ms);
CREATE TABLE IF NOT EXISTS observation_analysis_members_v1(
 scope_id TEXT NOT NULL,session_id TEXT NOT NULL,anchor_id TEXT NOT NULL,predecessor TEXT,gap TEXT,
 PRIMARY KEY(scope_id,session_id));
CREATE INDEX IF NOT EXISTS analysis_scope_anchor_v1 ON observation_analysis_members_v1(scope_id,anchor_id,session_id);
CREATE TABLE IF NOT EXISTS observation_analysis_prices_v1(
 id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,digest TEXT NOT NULL,body TEXT NOT NULL,
 evaluated_ms INTEGER NOT NULL,expires_ms INTEGER NOT NULL,bytes INTEGER NOT NULL,
 UNIQUE(workspace_id,digest));
CREATE INDEX IF NOT EXISTS analysis_price_expiry_v1 ON observation_analysis_prices_v1(workspace_id,expires_ms);
CREATE TABLE IF NOT EXISTS observation_native_relations_v1(
 workspace_id TEXT NOT NULL,revision INTEGER NOT NULL,parent_id TEXT NOT NULL,child_id TEXT NOT NULL,
 active INTEGER NOT NULL,observer_id TEXT NOT NULL,sequence TEXT NOT NULL,observed_ms INTEGER NOT NULL,
 ingress_ms INTEGER NOT NULL,request_id TEXT NOT NULL,proof_id TEXT NOT NULL,
 PRIMARY KEY(workspace_id,revision));
CREATE INDEX IF NOT EXISTS native_relation_edge_v1 ON observation_native_relations_v1(workspace_id,parent_id,child_id,revision DESC);
CREATE INDEX IF NOT EXISTS native_relation_child_v1 ON observation_native_relations_v1(workspace_id,child_id,revision DESC);
CREATE TABLE IF NOT EXISTS observation_native_events_v1(
 workspace_id TEXT NOT NULL,event_id TEXT NOT NULL,body TEXT NOT NULL,
 PRIMARY KEY(workspace_id,event_id));
CREATE TABLE IF NOT EXISTS observation_native_presence_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,PRIMARY KEY(workspace_id,session_id));
CREATE TABLE IF NOT EXISTS observation_native_roots_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,proof_id TEXT NOT NULL,request_id TEXT NOT NULL,observed_ms INTEGER NOT NULL,
 PRIMARY KEY(workspace_id,session_id,proof_id));
CREATE INDEX IF NOT EXISTS native_roots_time_v1 ON observation_native_roots_v1(workspace_id,session_id,observed_ms DESC,proof_id);
CREATE TABLE IF NOT EXISTS observation_native_event_members_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,event_id TEXT NOT NULL,
 PRIMARY KEY(workspace_id,session_id,event_id));
CREATE INDEX IF NOT EXISTS native_event_member_event_v1 ON observation_native_event_members_v1(workspace_id,event_id);
CREATE TABLE IF NOT EXISTS observation_native_states_v1(
 workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,state TEXT NOT NULL,
 observer_id TEXT NOT NULL,sequence TEXT NOT NULL,observed_ms INTEGER NOT NULL,
 PRIMARY KEY(workspace_id,session_id));
"#;
