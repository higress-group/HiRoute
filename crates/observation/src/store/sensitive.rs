//! Original wire bytes are content-bearing, even when their container is named
//! an execution fact or receipt. Markers retain the original digest; deletion
//! removes the carrier without manufacturing a replacement canonical envelope.
use crate::writer::ObservationStoreError as Error;
use hiroute_domain::{ExecutionFactEnvelopeV1, ExecutionFactV1};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    managed_sensitive_ref: String,
}

pub(crate) fn migrate(transaction: &Connection) -> Result<(), Error> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS observation_safe_facts_v2(workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,original_digest TEXT NOT NULL,body_json TEXT NOT NULL,PRIMARY KEY(workspace_id,request_id,original_digest));
         CREATE INDEX IF NOT EXISTS observation_safe_route_decisions ON observation_safe_facts_v2(workspace_id)
           WHERE CASE WHEN json_valid(body_json) THEN json_extract(body_json,'$.event_kind') ELSE 'route_decision' END='route_decision';
         CREATE TABLE IF NOT EXISTS observation_request_tombstones_v2(workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,deleted_ms INTEGER NOT NULL,PRIMARY KEY(workspace_id,request_id));
         CREATE TABLE IF NOT EXISTS observation_sensitive_payloads_v2(
            id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,
            request_id TEXT NOT NULL,created_ms INTEGER NOT NULL,body_json TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS observation_sensitive_session_v2 ON observation_sensitive_payloads_v2(workspace_id,session_id);
         CREATE TABLE IF NOT EXISTS observation_attempt_models_v2(
            workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,ordinal INTEGER NOT NULL,model_id TEXT NOT NULL,
            PRIMARY KEY(workspace_id,request_id,ordinal));"
    ).map_err(|_|Error::ActivityUnavailable)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)] // One immutable carrier row; grouping would obscure its key.
pub(crate) fn put(
    transaction: &Transaction<'_>,
    kind: &str,
    digest: &str,
    workspace: &str,
    session: &str,
    request: &str,
    created_ms: i64,
    body: &str,
) -> Result<String, Error> {
    let id = format!("{kind}:{digest}");
    transaction.execute(
        "INSERT OR IGNORE INTO observation_sensitive_payloads_v2(id,workspace_id,session_id,request_id,created_ms,body_json) VALUES(?1,?2,?3,?4,?5,?6)",
        params![id,workspace,session,request,created_ms,body],
    ).map_err(|_|Error::ActivityUnavailable)?;
    serde_json::to_string(&Marker {
        managed_sensitive_ref: id,
    })
    .map_err(|_| Error::Corrupt)
}

pub(crate) fn hydrate(
    connection: &Connection,
    kind: &str,
    digest: &str,
    stored: &str,
) -> Result<String, Error> {
    if !stored.contains("\"managed_sensitive_ref\"") {
        return Ok(stored.into());
    }
    let marker: Marker = serde_json::from_str(stored).map_err(|_| Error::Corrupt)?;
    if marker.managed_sensitive_ref != format!("{kind}:{digest}") {
        return Err(Error::Corrupt);
    }
    connection
        .query_row(
            "SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id=?1",
            [marker.managed_sensitive_ref],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Error::ActivityUnavailable)?
        .ok_or(Error::ContentUnavailable)
}

pub(crate) fn project_model(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), Error> {
    if let ExecutionFactV1::AttemptStarted {
        ordinal,
        request_model,
        ..
    } = &envelope.fact
    {
        transaction.execute("INSERT OR IGNORE INTO observation_attempt_models_v2(workspace_id,request_id,ordinal,model_id) VALUES(?1,?2,?3,?4)",
            params![envelope.correlation.workspace_id.as_str(),envelope.correlation.request_id.as_str(),ordinal,request_model],
        ).map_err(|_|Error::ActivityUnavailable)?;
    }
    Ok(())
}

pub(crate) fn hydrate_bounded(
    connection: &Connection,
    kind: &str,
    digest: &str,
    stored: &str,
    max_bytes: usize,
) -> Result<String, Error> {
    if stored.len() > max_bytes {
        return Err(Error::ContentUnavailable);
    }
    if !stored.contains("\"managed_sensitive_ref\"") {
        return Ok(stored.into());
    }
    let marker: Marker = serde_json::from_str(stored).map_err(|_| Error::Corrupt)?;
    if marker.managed_sensitive_ref != format!("{kind}:{digest}") {
        return Err(Error::Corrupt);
    }
    connection.query_row("SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id=?1 AND length(body_json)<=?2",params![marker.managed_sensitive_ref,max_bytes],|row|row.get(0))
        .optional().map_err(|_|Error::ActivityUnavailable)?.ok_or(Error::ContentUnavailable)
}
