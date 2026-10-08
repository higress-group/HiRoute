use hiroute_domain::{
    ContentId, ConversationContentEnvelopeV1, ConversationContentPhaseV2,
    ObservationBlobAcknowledgementV1, TranscriptRoot,
};
use rusqlite::{OptionalExtension, Transaction, params};

use super::lifecycle::ContentProjectionError;

#[derive(Clone, Debug)]
pub(super) struct StreamState {
    pub(super) workspace_id: String,
    pub(super) conversation_id: String,
    pub(super) turn_id: String,
    pub(super) session_scope: String,
    pub(super) correlation_provenance: String,
    pub(super) attempt_id: Option<String>,
    pub(super) parent_root: Option<String>,
    pub(super) state: String,
    pub(super) next_chunk_ordinal: u32,
    pub(super) active_content_id: Option<String>,
}

pub(super) fn insert_or_verify<Insert, Select>(
    _transaction: &Transaction<'_>,
    table: &str,
    workspace: &str,
    id: &str,
    rejected: &serde_json::Value,
    insert: Insert,
    select: Select,
) -> Result<(), ContentProjectionError>
where
    Insert: FnOnce() -> rusqlite::Result<usize>,
    Select: FnOnce() -> rusqlite::Result<serde_json::Value>,
{
    match insert() {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(ref code, _))
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            let existing = select().map_err(|_| ContentProjectionError::ActivityStorage)?;
            if existing == *rejected {
                Ok(())
            } else {
                Err(ContentProjectionError::ImmutableConflict {
                    key: format!("{table}:{workspace}:{id}"),
                    existing: projection_digest(&existing),
                    rejected: projection_digest(rejected),
                })
            }
        }
        Err(_) => Err(ContentProjectionError::ActivityStorage),
    }
}

pub(super) fn insert_transcript_root(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    result: &TranscriptRoot,
) -> Result<(), ContentProjectionError> {
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO transcript_roots_v2
         (workspace_id, transcript_root, conversation_id, request_id, direction, fork_id,
          parent_transcript_root, state, acknowledged_at_unix_nanos)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                envelope.correlation.workspace_id.as_str(),
                result.as_str(),
                envelope.correlation.conversation_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id,
                envelope
                    .parent_transcript_root
                    .as_ref()
                    .map(|value| value.as_str()),
                envelope.phase.as_str(),
                i64_from_u64(envelope.occurred_at_unix_nanos)?
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if inserted == 0 {
        let existing: (String, String, String, Option<String>, String) = transaction
            .query_row(
                "SELECT conversation_id, request_id, direction, parent_transcript_root, state
             FROM transcript_roots_v2 WHERE workspace_id=?1 AND transcript_root=?2",
                params![envelope.correlation.workspace_id.as_str(), result.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|_| ContentProjectionError::ActivityStorage)?;
        let rejected = (
            envelope.correlation.conversation_id.as_str().to_owned(),
            envelope.correlation.request_id.as_str().to_owned(),
            envelope.direction.as_str().to_owned(),
            envelope
                .parent_transcript_root
                .as_ref()
                .map(ToString::to_string),
            envelope.phase.as_str().to_owned(),
        );
        if existing != rejected {
            return Err(ContentProjectionError::ImmutableConflict {
                key: format!("transcript_root:{}", result),
                existing: projection_digest(&existing),
                rejected: projection_digest(&rejected),
            });
        }
    }
    Ok(())
}

pub(super) fn ensure_scope(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<(), ContentProjectionError> {
    let occurred_ms = envelope.occurred_at_unix_nanos / 1_000_000;
    transaction
        .execute(
            "INSERT OR IGNORE INTO sessions
         (workspace_id, session_id, agent_id, correlation, started_at_ms, updated_at_ms,
          facts_completeness, content_completeness)
         VALUES (?1, ?2, '', ?3, ?4, ?4, 'unknown', 'unknown')",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                enum_json(&envelope.correlation.correlation_provenance)?,
                i64_from_u64(occurred_ms)?
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let correlation = enum_json(&envelope.correlation.correlation_provenance)?;
    let session_scope = enum_json(&envelope.correlation.session_scope)?;
    let existing_correlation: String = transaction
        .query_row(
            "SELECT correlation FROM sessions WHERE workspace_id=?1 AND session_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.conversation_id.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if existing_correlation == "unproven" && correlation != "unproven" {
        transaction
            .execute(
                "UPDATE sessions SET correlation=?3 WHERE workspace_id=?1 AND session_id=?2",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    envelope.correlation.conversation_id.as_str(),
                    correlation,
                ],
            )
            .map_err(|_| ContentProjectionError::ActivityStorage)?;
    } else if existing_correlation != correlation {
        return Err(ContentProjectionError::ImmutableConflict {
            key: format!("session:{}", envelope.correlation.conversation_id),
            existing: projection_digest(&existing_correlation),
            rejected: projection_digest(&correlation),
        });
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO turns (workspace_id, turn_id, session_id, started_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.turn_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                i64_from_u64(occurred_ms)?
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let turn_session: String = transaction
        .query_row(
            "SELECT session_id FROM turns WHERE workspace_id=?1 AND turn_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.turn_id.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if turn_session != envelope.correlation.conversation_id.as_str() {
        return Err(ContentProjectionError::ImmutableConflict {
            key: format!("turn:{}", envelope.correlation.turn_id),
            existing: projection_digest(&turn_session),
            rejected: projection_digest(&envelope.correlation.conversation_id.as_str()),
        });
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO logical_requests
         (workspace_id, request_id, session_id, turn_id, session_scope,
          correlation_provenance, traffic_kind, started_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'unknown', ?7)",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                envelope.correlation.turn_id.as_str(),
                session_scope,
                correlation,
                i64_from_u64(occurred_ms)?
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    transaction
        .execute(
            "UPDATE logical_requests SET session_scope=?3, correlation_provenance=?4
             WHERE workspace_id=?1 AND request_id=?2
               AND session_scope IS NULL AND correlation_provenance IS NULL",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str(),
                session_scope,
                correlation,
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let scope: (String, String, Option<String>, Option<String>) = transaction
        .query_row(
            "SELECT session_id, turn_id, session_scope, correlation_provenance
             FROM logical_requests WHERE workspace_id=?1 AND request_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if scope
        != (
            envelope.correlation.conversation_id.to_string(),
            envelope.correlation.turn_id.to_string(),
            Some(session_scope.clone()),
            Some(correlation.clone()),
        )
    {
        return Err(ContentProjectionError::ImmutableConflict {
            key: format!("request:{}", envelope.correlation.request_id),
            existing: projection_digest(&scope),
            rejected: projection_digest(&(
                envelope.correlation.conversation_id.as_str(),
                envelope.correlation.turn_id.as_str(),
                session_scope.as_str(),
                correlation.as_str(),
            )),
        });
    }
    Ok(())
}

pub(super) fn load_stream(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<StreamState, ContentProjectionError> {
    transaction
        .query_row(
            "SELECT workspace_id, conversation_id, turn_id, session_scope,
                correlation_provenance, attempt_id, parent_transcript_root, state,
                next_chunk_ordinal, active_content_id
         FROM conversation_content_streams_v2
         WHERE producer_id=?1 AND producer_epoch=?2 AND stream_id=?3 AND request_id=?4
           AND direction=?5 AND fork_id=?6",
            coordinate_params(envelope),
            |row| {
                let ordinal: i64 = row.get(8)?;
                Ok(StreamState {
                    workspace_id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    session_scope: row.get(3)?,
                    correlation_provenance: row.get(4)?,
                    attempt_id: row.get(5)?,
                    parent_root: row.get(6)?,
                    state: row.get(7)?,
                    next_chunk_ordinal: ordinal
                        .try_into()
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, ordinal))?,
                    active_content_id: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|_| ContentProjectionError::ActivityStorage)?
        .ok_or(ContentProjectionError::ContentStateConflict {
            expected: ConversationContentPhaseV2::Begin,
            rejected: envelope.phase,
        })
}

pub(super) fn validate_stream_coordinates(
    stream: &StreamState,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<(), ContentProjectionError> {
    let expected = (
        &stream.workspace_id,
        &stream.conversation_id,
        &stream.turn_id,
        &stream.session_scope,
        &stream.correlation_provenance,
        &stream.attempt_id,
        &stream.parent_root,
    );
    let rejected = (
        envelope.correlation.workspace_id.as_str(),
        envelope.correlation.conversation_id.as_str(),
        envelope.correlation.turn_id.as_str(),
        enum_json(&envelope.correlation.session_scope)?,
        enum_json(&envelope.correlation.correlation_provenance)?,
        &envelope.attempt_id.as_ref().map(ToString::to_string),
        &envelope
            .parent_transcript_root
            .as_ref()
            .map(ToString::to_string),
    );
    if expected.0 != rejected.0
        || expected.1 != rejected.1
        || expected.2 != rejected.2
        || expected.3.as_str() != rejected.3.as_str()
        || expected.4.as_str() != rejected.4.as_str()
        || expected.5 != rejected.5
        || expected.6 != rejected.6
    {
        return Err(ContentProjectionError::ImmutableConflict {
            key: format!(
                "content_stream:{}:{}:{}",
                envelope.correlation.request_id,
                envelope.direction.as_str(),
                envelope.fork_id
            ),
            existing: projection_digest(&expected),
            rejected: projection_digest(&rejected),
        });
    }
    Ok(())
}

pub(super) fn acknowledged_blobs(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<Vec<ObservationBlobAcknowledgementV1>, ContentProjectionError> {
    let mut statement = transaction.prepare(
        "SELECT content_id, content_blob_digest FROM content_instances_v2
         WHERE workspace_id=?1 AND request_id=?2 AND direction=?3 AND fork_id=?4 AND state='complete'
         ORDER BY content_id",
    ).map_err(|_| ContentProjectionError::ActivityStorage)?;
    statement
        .query_map(
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id
            ],
            |row| {
                Ok(ObservationBlobAcknowledgementV1 {
                    content_id: row.get(0)?,
                    digest: row.get(1)?,
                })
            },
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContentProjectionError::ActivityStorage)
}

pub(super) fn missing_blobs(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<Vec<ObservationBlobAcknowledgementV1>, ContentProjectionError> {
    let mut statement = transaction.prepare(
        "SELECT content_id, content_blob_digest FROM content_instances_v2
         WHERE workspace_id=?1 AND request_id=?2 AND direction=?3 AND fork_id=?4 AND state!='complete'
         ORDER BY content_id",
    ).map_err(|_| ContentProjectionError::ActivityStorage)?;
    statement
        .query_map(
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id
            ],
            |row| {
                Ok(ObservationBlobAcknowledgementV1 {
                    content_id: row.get(0)?,
                    digest: row.get(1)?,
                })
            },
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContentProjectionError::ActivityStorage)
}

pub(super) fn accumulated_expected(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    content: &ContentId,
) -> Result<Option<u64>, ContentProjectionError> {
    let value: Option<i64> = transaction.query_row("SELECT expected_byte_count FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2",
        params![envelope.correlation.workspace_id.as_str(), content.as_str()], |row| row.get(0)).map_err(|_| ContentProjectionError::ActivityStorage)?;
    value
        .map(|value| {
            value
                .try_into()
                .map_err(|_| ContentProjectionError::ContentCorrupt)
        })
        .transpose()
}
pub(super) fn accumulated_expected_str(
    transaction: &Transaction<'_>,
    workspace: &str,
    content: &str,
) -> Result<Option<u64>, ContentProjectionError> {
    let value: Option<i64> = transaction.query_row("SELECT expected_byte_count FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2", params![workspace, content], |row| row.get(0)).map_err(|_| ContentProjectionError::ActivityStorage)?;
    value
        .map(|value| {
            value
                .try_into()
                .map_err(|_| ContentProjectionError::ContentCorrupt)
        })
        .transpose()
}
pub(super) fn accumulated_actual_str(
    transaction: &Transaction<'_>,
    workspace: &str,
    content: &str,
) -> Result<u64, ContentProjectionError> {
    let value: i64 = transaction.query_row("SELECT accumulated_byte_count FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2", params![workspace, content], |row| row.get(0)).map_err(|_| ContentProjectionError::ActivityStorage)?;
    value
        .try_into()
        .map_err(|_| ContentProjectionError::ContentCorrupt)
}
pub(super) fn content_digest(
    transaction: &Transaction<'_>,
    workspace: &str,
    content: &str,
) -> Result<String, ContentProjectionError> {
    transaction.query_row("SELECT content_blob_digest FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2", params![workspace, content], |row| row.get(0)).map_err(|_| ContentProjectionError::ActivityStorage)
}

pub(super) fn touch_content_session(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<(), ContentProjectionError> {
    transaction
        .execute(
            "UPDATE sessions SET updated_at_ms=MAX(updated_at_ms, ?3)
         WHERE workspace_id=?1 AND session_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                i64_from_u64(envelope.occurred_at_unix_nanos / 1_000_000)?
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if envelope.phase != hiroute_domain::ConversationContentPhaseV2::Append {
        super::completeness::refresh_session(
            transaction,
            envelope.correlation.workspace_id.as_str(),
            envelope.correlation.conversation_id.as_str(),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    }
    Ok(())
}

pub(super) fn coordinate_params(envelope: &ConversationContentEnvelopeV1) -> [&str; 6] {
    [
        envelope.producer.stream.producer_id.as_str(),
        envelope.producer.stream.producer_epoch.as_str(),
        envelope.producer.stream.stream_id.as_str(),
        envelope.correlation.request_id.as_str(),
        envelope.direction.as_str(),
        envelope.fork_id.as_str(),
    ]
}
pub(super) fn enum_json<T: serde::Serialize>(value: &T) -> Result<String, ContentProjectionError> {
    serde_json::to_value(value)
        .map_err(|_| ContentProjectionError::Invalid("enum"))?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or(ContentProjectionError::Invalid("enum"))
}
pub(super) fn projection_digest<T: serde::Serialize>(value: &T) -> String {
    use sha2::{Digest as _, Sha256};
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub(super) fn event_file_key(event_id: &str) -> String {
    use sha2::{Digest as _, Sha256};
    format!("{:x}", Sha256::digest(event_id.as_bytes()))
}
pub(super) fn terminal_phase(state: &str) -> ConversationContentPhaseV2 {
    if state == "abort" {
        ConversationContentPhaseV2::Abort
    } else {
        ConversationContentPhaseV2::Finish
    }
}
pub(super) fn i64_from_u64(value: u64) -> Result<i64, ContentProjectionError> {
    value
        .try_into()
        .map_err(|_| ContentProjectionError::Invalid("integer"))
}
