//! Authoritative, append-only execution-fact event storage.

use hiroute_domain::{
    CanonicalDigest, ExecutionFactEnvelopeV1, LogicalRequestId, SessionId, WorkspaceId,
};
use rusqlite::{Connection, Transaction, params};

use crate::writer::ObservationStoreError;

pub(crate) fn insert(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
    digest: &CanonicalDigest,
) -> Result<(), ObservationStoreError> {
    let body = serde_json::to_string(envelope).map_err(|_| ObservationStoreError::Corrupt)?;
    let body = super::sensitive::put(
        transaction,
        "fact",
        digest.as_str(),
        envelope.correlation.workspace_id.as_str(),
        envelope.correlation.conversation_id.as_str(),
        envelope.correlation.request_id.as_str(),
        envelope
            .occurred_at_ms()
            .map_err(|_| ObservationStoreError::Corrupt)?,
        &body,
    )?;
    super::sensitive::project_model(transaction, envelope)?;
    super::safe_facts::project(transaction, envelope, digest)?;
    transaction
        .execute(
            "INSERT INTO execution_fact_events
             (workspace_id, session_id, turn_id, request_id, producer_id, producer_epoch,
              stream_id, sequence, event_id, envelope_json, envelope_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                envelope.correlation.turn_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.producer.stream.producer_id.as_str(),
                envelope.producer.stream.producer_epoch.as_str(),
                envelope.producer.stream.stream_id.as_str(),
                sql_u64(envelope.sequence)?,
                envelope.event_id.as_str(),
                body,
                digest.as_str(),
            ],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    crate::valuation::ingest(transaction, envelope, digest)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

pub(crate) fn load_request(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    request_id: &LogicalRequestId,
) -> Result<Vec<ExecutionFactEnvelopeV1>, ObservationStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT session_id, turn_id, producer_id, producer_epoch, stream_id, sequence,
                    event_id, envelope_json, envelope_digest
             FROM execution_fact_events WHERE workspace_id=?1 AND request_id=?2
             ORDER BY sequence",
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    let rows = statement
        .query_map(params![workspace_id.as_str(), request_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    let mut facts = Vec::new();
    for row in rows {
        let row = row.map_err(|_| ObservationStoreError::Corrupt)?;
        let original = super::sensitive::hydrate(connection, "fact", &row.8, &row.7)?;
        let envelope: ExecutionFactEnvelopeV1 =
            serde_json::from_str(&original).map_err(|_| ObservationStoreError::Corrupt)?;
        envelope
            .validate_persisted_contract()
            .map_err(|_| ObservationStoreError::Corrupt)?;
        let digest = CanonicalDigest::of(&envelope).map_err(|_| ObservationStoreError::Corrupt)?;
        if digest.as_str() != row.8
            || envelope.correlation.workspace_id != *workspace_id
            || envelope.correlation.request_id != *request_id
            || envelope.correlation.conversation_id.as_str() != row.0
            || envelope.correlation.turn_id.as_str() != row.1
            || envelope.producer.stream.producer_id.as_str() != row.2
            || envelope.producer.stream.producer_epoch.as_str() != row.3
            || envelope.producer.stream.stream_id.as_str() != row.4
            || i64::try_from(envelope.sequence).ok() != Some(row.5)
            || envelope.event_id.as_str() != row.6
        {
            return Err(ObservationStoreError::Corrupt);
        }
        facts.push(envelope);
    }
    Ok(facts)
}

pub(crate) fn load_session(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    session_id: &SessionId,
) -> Result<Vec<ExecutionFactEnvelopeV1>, ObservationStoreError> {
    #[cfg(test)]
    SESSION_LOADS.with(|count| count.set(count.get() + 1));
    let request_ids = {
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT request_id FROM execution_fact_events
                 WHERE workspace_id=?1 AND session_id=?2 ORDER BY request_id",
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        statement
            .query_map(params![workspace_id.as_str(), session_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationStoreError::Corrupt)?
    };
    let mut facts = Vec::new();
    for request_id in request_ids {
        let request_id =
            LogicalRequestId::parse(request_id).map_err(|_| ObservationStoreError::Corrupt)?;
        let request_facts = load_request(connection, workspace_id, &request_id)?;
        if request_facts
            .iter()
            .any(|fact| fact.correlation.conversation_id != *session_id)
        {
            return Err(ObservationStoreError::Corrupt);
        }
        facts.extend(request_facts);
    }
    Ok(facts)
}

#[cfg(test)]
thread_local! {
    pub(crate) static SESSION_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn sql_u64(value: u64) -> Result<i64, ObservationStoreError> {
    value.try_into().map_err(|_| ObservationStoreError::Corrupt)
}
