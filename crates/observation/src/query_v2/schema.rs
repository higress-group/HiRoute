use crate::writer::ObservationStoreError;
use rusqlite::Connection;

pub(crate) fn migrate(transaction: &Connection) -> Result<(), ObservationStoreError> {
    ensure_logical_request_correlation_columns(transaction)?;
    transaction.execute_batch(
        "INSERT OR IGNORE INTO observation_meta(key,value) VALUES('query_visibility_generation','0');
         INSERT OR IGNORE INTO observation_meta(key,value) VALUES('plan_quality_generation','0');
         CREATE INDEX IF NOT EXISTS observation_request_time_v2 ON logical_requests(workspace_id,started_at_ms,request_id);
         CREATE TABLE IF NOT EXISTS observation_run_links(
            workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,run_id TEXT NOT NULL,
            body_json TEXT NOT NULL,body_digest TEXT NOT NULL,conflicted INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(workspace_id,request_id));
         CREATE INDEX IF NOT EXISTS observation_run_requests ON observation_run_links(workspace_id,run_id,request_id);
         CREATE TABLE IF NOT EXISTS observation_link_events(
            workspace_id TEXT NOT NULL,producer_epoch TEXT NOT NULL,event_id TEXT NOT NULL,
            request_id TEXT NOT NULL,body_digest TEXT NOT NULL,
            PRIMARY KEY(workspace_id,producer_epoch,event_id));
         CREATE TABLE IF NOT EXISTS plan_quality_segments(
            workspace_id TEXT NOT NULL,
            segment_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            plan_id TEXT NOT NULL,
            plan_revision INTEGER NOT NULL,
            selected_branch_id TEXT,
            executed_branch_id TEXT,
            model_configuration_id TEXT,
            profile_digest TEXT,
            attribution TEXT,
            first_turn_id TEXT,
            first_turn_ordinal INTEGER,
            last_observed_turn_id TEXT,
            last_observed_turn_ordinal INTEGER,
            first_at_ms INTEGER,
            last_at_ms INTEGER,
            history_partial INTEGER NOT NULL DEFAULT 0,
            first_request_id TEXT,
            last_request_id TEXT,
            execution_event_id TEXT,
            execution_sequence INTEGER,
            assessment_event_id TEXT,
            assessment_sequence INTEGER,
            assessment_trigger_request_id TEXT,
            assessed_at_ms INTEGER,
            target_from_turn_id TEXT,
            target_through_turn_id TEXT,
            target_from_ordinal INTEGER,
            target_through_ordinal INTEGER,
            score REAL,
            assessment_partial INTEGER,
            reason_present INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(workspace_id,segment_id));
         CREATE INDEX IF NOT EXISTS plan_quality_by_plan
            ON plan_quality_segments(workspace_id,plan_id,plan_revision,last_at_ms DESC,segment_id);
         CREATE INDEX IF NOT EXISTS plan_quality_by_session
            ON plan_quality_segments(workspace_id,session_id,last_at_ms DESC,segment_id);
         CREATE INDEX IF NOT EXISTS plan_quality_by_model
            ON plan_quality_segments(workspace_id,model_configuration_id,last_at_ms DESC,segment_id);"
    ).map_err(|_|ObservationStoreError::ActivityUnavailable)?;
    let columns = transaction
        .prepare("PRAGMA table_info(plan_quality_segments)")
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    for column in ["branch_execution_json", "upgrade_json"] {
        if !columns.contains(column) {
            transaction
                .execute(
                    &format!("ALTER TABLE plan_quality_segments ADD COLUMN {column} TEXT"),
                    [],
                )
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        }
    }
    Ok(())
}

fn ensure_logical_request_correlation_columns(
    transaction: &Connection,
) -> Result<(), ObservationStoreError> {
    let columns = {
        let mut statement = transaction
            .prepare("PRAGMA table_info(logical_requests)")
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?
    };
    if !columns.contains("session_scope") {
        transaction
            .execute(
                "ALTER TABLE logical_requests ADD COLUMN session_scope TEXT",
                [],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    if !columns.contains("correlation_provenance") {
        transaction
            .execute(
                "ALTER TABLE logical_requests ADD COLUMN correlation_provenance TEXT",
                [],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    Ok(())
}
