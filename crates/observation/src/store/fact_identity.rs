//! Minimal immutable business identity after detail retirement. No request,
//! session, envelope or historical reply is retained by this index.
use crate::writer::ObservationStoreError as Error;
use rusqlite::{Connection, params};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS retired_execution_identities_v6(
 producer_id TEXT NOT NULL,producer_epoch TEXT NOT NULL,stream_id TEXT NOT NULL,
 event_id TEXT NOT NULL,sequence INTEGER NOT NULL,payload_digest TEXT NOT NULL,
 PRIMARY KEY(producer_id,producer_epoch,stream_id,event_id));";

pub(super) fn retire_request(
    db: &Connection,
    workspace: &str,
    request: &str,
    limit: Option<usize>,
) -> Result<(), Error> {
    let limit = limit.map_or(-1, |n| n as i64);
    db.execute("INSERT OR IGNORE INTO retired_execution_identities_v6
        SELECT producer_id,producer_epoch,stream_id,event_id,sequence,envelope_digest
        FROM execution_fact_events WHERE workspace_id=?1 AND request_id=?2 ORDER BY sequence,rowid LIMIT ?3",
        params![workspace,request,limit]).map_err(|_|Error::ActivityUnavailable)?;
    Ok(())
}

pub(super) fn retire_session(db: &Connection, workspace: &str, session: &str) -> Result<(), Error> {
    db.execute(
        "INSERT OR IGNORE INTO retired_execution_identities_v6
        SELECT producer_id,producer_epoch,stream_id,event_id,sequence,envelope_digest
        FROM execution_fact_events WHERE workspace_id=?1 AND session_id=?2",
        params![workspace, session],
    )
    .map_err(|_| Error::ActivityUnavailable)?;
    Ok(())
}
