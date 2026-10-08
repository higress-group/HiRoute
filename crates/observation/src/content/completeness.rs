//! Completeness is evidence for every expected direction, not the last finish.
use rusqlite::{Connection, OptionalExtension, params};

// `r` is always a request selected inside the caller's authorized scope. Roots
// prove termination; an open/aborted stream cannot borrow another fork's finish.
pub(crate) const REQUEST_STATE: &str = "CASE
    WHEN EXISTS(SELECT 1 FROM conversation_content_streams_v2 c
        WHERE c.workspace_id=r.workspace_id AND c.request_id=r.request_id
          AND (c.state='abort' OR c.completeness='partial'))
      OR EXISTS(SELECT 1 FROM transcript_roots_v2 t
        WHERE t.workspace_id=r.workspace_id AND t.request_id=r.request_id AND t.state!='finish')
      THEN 'partial'
    WHEN NOT EXISTS(SELECT 1 FROM transcript_roots_v2 t
        WHERE t.workspace_id=r.workspace_id AND t.request_id=r.request_id
          AND t.direction='request_input' AND t.state='finish')
      OR (r.outcome IN ('accepted','postcommit_partial','postcommit_transport_failed')
        AND NOT EXISTS(SELECT 1 FROM transcript_roots_v2 t
          WHERE t.workspace_id=r.workspace_id AND t.request_id=r.request_id
            AND t.direction='response_delivered' AND t.state='finish'))
      THEN CASE WHEN r.finished_at_ms IS NULL THEN 'unknown' ELSE 'partial' END
    WHEN EXISTS(SELECT 1 FROM conversation_content_streams_v2 c
        WHERE c.workspace_id=r.workspace_id AND c.request_id=r.request_id
          AND (c.state='open' OR c.completeness!='complete')) THEN 'unknown'
    ELSE 'complete' END";

pub(crate) fn request_state(
    connection: &Connection,
    workspace: &str,
    request: &str,
) -> rusqlite::Result<String> {
    connection.query_row(
        &format!("SELECT {REQUEST_STATE} FROM logical_requests r WHERE r.workspace_id=?1 AND r.request_id=?2"),
        params![workspace, request], |row| row.get(0),
    ).optional().map(|state| state.unwrap_or_else(|| "unknown".into()))
}

pub(crate) fn session_state(
    connection: &Connection,
    workspace: &str,
    session: &str,
) -> rusqlite::Result<String> {
    connection.query_row(
        &format!("WITH states AS (SELECT {REQUEST_STATE} AS state FROM logical_requests r
          WHERE r.workspace_id=?1 AND r.session_id=?2)
          SELECT CASE
            WHEN EXISTS(SELECT 1 FROM observation_gaps WHERE channel='content' AND workspace_id=?1 AND session_id=?2)
              OR MAX(state='partial')=1 THEN 'partial'
            WHEN COUNT(*)=0 OR MAX(state='unknown')=1 THEN 'unknown'
            ELSE 'complete' END FROM states"),
        params![workspace, session], |row| row.get(0),
    )
}

pub(crate) fn refresh_session(
    connection: &Connection,
    workspace: &str,
    session: &str,
) -> rusqlite::Result<()> {
    let state = session_state(connection, workspace, session)?;
    connection.execute(
        "UPDATE sessions SET content_completeness=?3 WHERE workspace_id=?1 AND session_id=?2
          AND content_completeness NOT IN ('deleted','expired')",
        params![workspace, session, state],
    )?;
    Ok(())
}
