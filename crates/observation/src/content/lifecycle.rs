use std::fs::{self, OpenOptions};
use std::io::Write;

use hiroute_domain::{
    ContentId, ConversationContentEnvelopeV1, ConversationContentPhaseV2,
    ObservationBlobAcknowledgementV1, ObservationDigestSubjectV1, TranscriptRoot,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::store::LocalObservationStore;

use super::projection::{
    accumulated_actual_str, accumulated_expected, accumulated_expected_str, acknowledged_blobs,
    content_digest, coordinate_params, ensure_scope, enum_json, event_file_key, i64_from_u64,
    insert_or_verify, insert_transcript_root, load_stream, missing_blobs, projection_digest,
    terminal_phase, touch_content_session, validate_stream_coordinates,
};
use super::storage::{decode_base64, finalize_content};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ContentProjectionAck {
    pub transcript_root: Option<String>,
    pub delta_parent_transcript_root: Option<String>,
    pub acknowledged_blobs: Vec<ObservationBlobAcknowledgementV1>,
    pub next_chunk_ordinal: u32,
    pub chunk_object_ref: Option<String>,
    pub chunk_byte_offset: Option<u64>,
    pub chunk_byte_count: Option<u64>,
    pub cleanup_content_ids: Vec<ContentId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContentProjectionError {
    UnknownTranscriptRoot(TranscriptRoot),
    MissingBlob(Vec<ObservationBlobAcknowledgementV1>),
    ContentStateConflict {
        expected: ConversationContentPhaseV2,
        rejected: ConversationContentPhaseV2,
    },
    ChunkOrdinalConflict {
        expected: u32,
        rejected: u32,
    },
    DigestMismatch {
        subject: ObservationDigestSubjectV1,
        subject_id: String,
        expected: String,
        rejected: String,
    },
    ImmutableConflict {
        key: String,
        existing: String,
        rejected: String,
    },
    Invalid(&'static str),
    ActivityStorage,
    ContentStorage,
    ContentCorrupt,
}

pub(crate) fn apply_content(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<ContentProjectionAck, ContentProjectionError> {
    match envelope.phase {
        ConversationContentPhaseV2::Begin => apply_begin(transaction, envelope),
        ConversationContentPhaseV2::Append => apply_append(store, transaction, envelope),
        ConversationContentPhaseV2::Finish | ConversationContentPhaseV2::Abort => {
            apply_terminal(store, transaction, envelope)
        }
    }
}

fn apply_begin(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<ContentProjectionAck, ContentProjectionError> {
    ensure_scope(transaction, envelope)?;
    if let Some(parent) = &envelope.parent_transcript_root {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM transcript_roots_v2
                 WHERE workspace_id=?1 AND transcript_root=?2 AND conversation_id=?3)",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    parent.as_str(),
                    envelope.correlation.conversation_id.as_str(),
                ],
                |row| row.get(0),
            )
            .map_err(|_| ContentProjectionError::ActivityStorage)?;
        if !exists {
            return Err(ContentProjectionError::UnknownTranscriptRoot(
                parent.clone(),
            ));
        }
    }
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO conversation_content_streams_v2
             (workspace_id, producer_id, producer_epoch, stream_id, request_id, direction,
              fork_id, conversation_id, session_scope, correlation_provenance, turn_id,
              attempt_id, parent_transcript_root, state, next_chunk_ordinal, begin_sequence,
              completeness, updated_at_unix_nanos)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     'open', 0, ?14, 'unknown', ?15)",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.producer.stream.producer_id.as_str(),
                envelope.producer.stream.producer_epoch.as_str(),
                envelope.producer.stream.stream_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id,
                envelope.correlation.conversation_id.as_str(),
                enum_json(&envelope.correlation.session_scope)?,
                enum_json(&envelope.correlation.correlation_provenance)?,
                envelope.correlation.turn_id.as_str(),
                envelope.attempt_id.as_ref().map(|value| value.as_str()),
                envelope
                    .parent_transcript_root
                    .as_ref()
                    .map(|value| value.as_str()),
                i64_from_u64(envelope.sequence)?,
                i64_from_u64(envelope.occurred_at_unix_nanos)?,
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if inserted == 0 {
        return Err(ContentProjectionError::ContentStateConflict {
            expected: ConversationContentPhaseV2::Append,
            rejected: ConversationContentPhaseV2::Begin,
        });
    }
    touch_content_session(transaction, envelope)?;
    Ok(ContentProjectionAck {
        delta_parent_transcript_root: envelope
            .parent_transcript_root
            .as_ref()
            .map(ToString::to_string),
        ..ContentProjectionAck::default()
    })
}

fn apply_append(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<ContentProjectionAck, ContentProjectionError> {
    let mut stream = load_stream(transaction, envelope)?;
    if stream.state != "open" {
        return Err(ContentProjectionError::ContentStateConflict {
            expected: terminal_phase(&stream.state),
            rejected: ConversationContentPhaseV2::Append,
        });
    }
    validate_stream_coordinates(&stream, envelope)?;
    let rejected = envelope
        .chunk_ordinal
        .ok_or(ContentProjectionError::Invalid("chunk_ordinal"))?;
    if rejected != stream.next_chunk_ordinal {
        return Err(ContentProjectionError::ChunkOrdinalConflict {
            expected: stream.next_chunk_ordinal,
            rejected,
        });
    }
    let content_id = envelope
        .content_id
        .as_ref()
        .ok_or(ContentProjectionError::Invalid("content_id"))?;
    let mut cleanup_content_ids = Vec::new();
    let mut acknowledged_blobs = Vec::new();
    if let Some(active) = stream.active_content_id.as_deref()
        && active != content_id.as_str()
    {
        let acknowledgement = finalize_content(store, transaction, envelope, active)?;
        cleanup_content_ids.push(
            ContentId::parse(active.to_owned())
                .map_err(|_| ContentProjectionError::ContentCorrupt)?,
        );
        acknowledged_blobs.push(acknowledgement);
        stream.active_content_id = None;
    }
    ensure_message_and_content(transaction, envelope)?;
    let (prior_byte_count, expected_byte_count, content_state) = transaction
        .query_row(
            "SELECT accumulated_byte_count, expected_byte_count, state
             FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                content_id.as_str()
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if content_state != "installing" {
        return Err(ContentProjectionError::ContentStateConflict {
            expected: ConversationContentPhaseV2::Finish,
            rejected: ConversationContentPhaseV2::Append,
        });
    }
    let bytes = decode_base64(
        envelope
            .canonical_bytes_base64
            .as_deref()
            .ok_or(ContentProjectionError::Invalid("canonical_bytes_base64"))?,
    )?;
    let directory = store.staging_directory(&envelope.correlation.workspace_id, content_id);
    fs::create_dir_all(&directory).map_err(|_| ContentProjectionError::ContentStorage)?;
    let path = directory.join(format!(
        "{rejected:010}-{}.chunk",
        event_file_key(envelope.event_id.as_str())
    ));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| ContentProjectionError::ContentStorage)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(&path).map_err(|_| ContentProjectionError::ContentStorage)?;
            if existing != bytes {
                return Err(ContentProjectionError::ChunkOrdinalConflict {
                    expected: rejected,
                    rejected,
                });
            }
        }
        Err(_) => return Err(ContentProjectionError::ContentStorage),
    }
    transaction
        .execute(
            "INSERT INTO content_chunks_v2
             (workspace_id, content_id, chunk_ordinal, object_path, byte_offset, byte_count,
              event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                envelope.correlation.workspace_id.as_str(),
                content_id.as_str(),
                i64::from(rejected),
                path.to_string_lossy(),
                prior_byte_count,
                i64_from_u64(bytes.len() as u64)?,
                envelope.event_id.as_str(),
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let accumulated = prior_byte_count
        .checked_add(i64_from_u64(bytes.len() as u64)?)
        .ok_or(ContentProjectionError::Invalid("canonical_bytes_base64"))?;
    if accumulated < 0 || expected_byte_count.is_some_and(|expected| accumulated > expected) {
        return Err(ContentProjectionError::DigestMismatch {
            subject: ObservationDigestSubjectV1::ContentBlob,
            subject_id: content_id.to_string(),
            expected: expected_byte_count
                .map_or_else(|| "unknown".into(), |value| value.to_string()),
            rejected: accumulated.to_string(),
        });
    }
    transaction
        .execute(
            "UPDATE content_instances_v2 SET accumulated_byte_count=?3
             WHERE workspace_id=?1 AND content_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                content_id.as_str(),
                accumulated
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let next = rejected
        .checked_add(1)
        .ok_or(ContentProjectionError::Invalid("chunk_ordinal"))?;
    transaction
        .execute(
            "UPDATE conversation_content_streams_v2
             SET next_chunk_ordinal=?7, active_content_id=?8, updated_at_unix_nanos=?9
             WHERE producer_id=?1 AND producer_epoch=?2 AND stream_id=?3 AND request_id=?4
               AND direction=?5 AND fork_id=?6",
            params![
                envelope.producer.stream.producer_id.as_str(),
                envelope.producer.stream.producer_epoch.as_str(),
                envelope.producer.stream.stream_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id,
                i64::from(next),
                content_id.as_str(),
                i64_from_u64(envelope.occurred_at_unix_nanos)?,
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let expected = accumulated_expected(transaction, envelope, content_id)?;
    if expected.is_some_and(|expected| expected == accumulated as u64) {
        acknowledged_blobs.push(finalize_content(
            store,
            transaction,
            envelope,
            content_id.as_str(),
        )?);
        cleanup_content_ids.push(content_id.clone());
        transaction
            .execute(
                "UPDATE conversation_content_streams_v2 SET active_content_id=NULL
                 WHERE producer_id=?1 AND producer_epoch=?2 AND stream_id=?3 AND request_id=?4
                   AND direction=?5 AND fork_id=?6",
                coordinate_params(envelope),
            )
            .map_err(|_| ContentProjectionError::ActivityStorage)?;
    }
    touch_content_session(transaction, envelope)?;
    Ok(ContentProjectionAck {
        delta_parent_transcript_root: stream.parent_root,
        acknowledged_blobs,
        next_chunk_ordinal: next,
        chunk_object_ref: Some(format!("content-chunk:{content_id}:{rejected}")),
        chunk_byte_offset: Some(
            prior_byte_count
                .try_into()
                .map_err(|_| ContentProjectionError::ContentCorrupt)?,
        ),
        chunk_byte_count: Some(bytes.len() as u64),
        cleanup_content_ids,
        ..ContentProjectionAck::default()
    })
}

fn apply_terminal(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<ContentProjectionAck, ContentProjectionError> {
    let stream = load_stream(transaction, envelope)?;
    if stream.state != "open" {
        return Err(ContentProjectionError::ContentStateConflict {
            expected: terminal_phase(&stream.state),
            rejected: envelope.phase,
        });
    }
    validate_stream_coordinates(&stream, envelope)?;
    let mut cleanup_content_ids = Vec::new();
    if let Some(active) = stream.active_content_id.as_deref() {
        let expected = accumulated_expected_str(transaction, &stream.workspace_id, active)?;
        let actual = accumulated_actual_str(transaction, &stream.workspace_id, active)?;
        if expected.is_some_and(|expected| expected != actual) {
            let digest = content_digest(transaction, &stream.workspace_id, active)?;
            return Err(ContentProjectionError::MissingBlob(vec![
                ObservationBlobAcknowledgementV1 {
                    content_id: active.into(),
                    digest,
                },
            ]));
        }
        finalize_content(store, transaction, envelope, active)?;
        cleanup_content_ids.push(
            ContentId::parse(active.to_owned())
                .map_err(|_| ContentProjectionError::ContentCorrupt)?,
        );
    }
    let incomplete = missing_blobs(transaction, envelope)?;
    if !incomplete.is_empty() {
        return Err(ContentProjectionError::MissingBlob(incomplete));
    }
    let result = envelope
        .result_transcript_root
        .as_ref()
        .ok_or(ContentProjectionError::Invalid("result_transcript_root"))?;
    insert_transcript_root(transaction, envelope, result)?;
    let state = envelope.phase.as_str();
    let completeness = match envelope.completeness_delta {
        Some(hiroute_domain::ContentCompletenessDeltaV2::Complete) => "complete",
        Some(hiroute_domain::ContentCompletenessDeltaV2::Partial) => "partial",
        Some(hiroute_domain::ContentCompletenessDeltaV2::Unknown) | None => "unknown",
    };
    transaction
        .execute(
            "UPDATE conversation_content_streams_v2
             SET state=?7, result_transcript_root=?8, active_content_id=NULL,
                 terminal_sequence=?9, completeness=?10, updated_at_unix_nanos=?11
             WHERE producer_id=?1 AND producer_epoch=?2 AND stream_id=?3 AND request_id=?4
               AND direction=?5 AND fork_id=?6",
            params![
                envelope.producer.stream.producer_id.as_str(),
                envelope.producer.stream.producer_epoch.as_str(),
                envelope.producer.stream.stream_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id,
                state,
                result.as_str(),
                i64_from_u64(envelope.sequence)?,
                completeness,
                i64_from_u64(envelope.occurred_at_unix_nanos)?,
            ],
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    touch_content_session(transaction, envelope)?;
    Ok(ContentProjectionAck {
        transcript_root: Some(result.to_string()),
        delta_parent_transcript_root: stream.parent_root,
        acknowledged_blobs: acknowledged_blobs(transaction, envelope)?,
        next_chunk_ordinal: stream.next_chunk_ordinal,
        cleanup_content_ids,
        ..ContentProjectionAck::default()
    })
}

fn ensure_message_and_content(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<(), ContentProjectionError> {
    let message = envelope
        .message_instance_id
        .as_ref()
        .ok_or(ContentProjectionError::Invalid("message_instance_id"))?;
    let content = envelope
        .content_id
        .as_ref()
        .ok_or(ContentProjectionError::Invalid("content_id"))?;
    let message_ordinal = envelope
        .message_ordinal
        .ok_or(ContentProjectionError::Invalid("message_ordinal"))?;
    let part_ordinal = envelope
        .part_ordinal
        .ok_or(ContentProjectionError::Invalid("part_ordinal"))?;
    let role = envelope
        .message_role
        .as_deref()
        .ok_or(ContentProjectionError::Invalid("message_role"))?;
    let kind = envelope
        .content_kind
        .as_deref()
        .ok_or(ContentProjectionError::Invalid("content_kind"))?;
    let media = envelope
        .canonical_media_type
        .as_deref()
        .ok_or(ContentProjectionError::Invalid("canonical_media_type"))?;
    let digest = envelope
        .content_blob_digest
        .as_ref()
        .ok_or(ContentProjectionError::Invalid("content_blob_digest"))?;
    let occurred_at = i64_from_u64(envelope.occurred_at_unix_nanos)?;
    let logical_message: Option<(String, String)> = transaction
        .query_row(
            "SELECT message_instance_id, message_role FROM content_message_instances_v2
         WHERE workspace_id=?1 AND request_id=?2 AND direction=?3 AND fork_id=?4
           AND message_ordinal=?5",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.request_id.as_str(),
                envelope.direction.as_str(),
                envelope.fork_id,
                i64::from(message_ordinal),
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let rejected_message = (message.to_string(), role.to_owned());
    if logical_message
        .as_ref()
        .is_some_and(|existing| existing != &rejected_message)
    {
        return Err(ContentProjectionError::ImmutableConflict {
            key: format!(
                "message_ordinal:{}:{}:{}:{}",
                envelope.correlation.request_id,
                envelope.direction.as_str(),
                envelope.fork_id,
                message_ordinal,
            ),
            existing: projection_digest(&logical_message),
            rejected: projection_digest(&rejected_message),
        });
    }
    let message_projection = serde_json::json!([
        envelope.correlation.conversation_id.as_str(),
        envelope.correlation.request_id.as_str(),
        envelope.direction.as_str(),
        envelope.fork_id,
        message_ordinal,
        role
    ]);
    insert_or_verify(
        transaction,
        "content_message_instances_v2",
        envelope.correlation.workspace_id.as_str(),
        message.as_str(),
        &message_projection,
        || {
            transaction.execute(
                "INSERT INTO content_message_instances_v2
             (workspace_id, message_instance_id, conversation_id, request_id, direction, fork_id,
              message_ordinal, message_role, occurred_at_unix_nanos)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    message.as_str(),
                    envelope.correlation.conversation_id.as_str(),
                    envelope.correlation.request_id.as_str(),
                    envelope.direction.as_str(),
                    envelope.fork_id,
                    i64::from(message_ordinal),
                    role,
                    occurred_at
                ],
            )
        },
        || {
            transaction.query_row(
            "SELECT conversation_id, request_id, direction, fork_id, message_ordinal, message_role
             FROM content_message_instances_v2 WHERE workspace_id=?1 AND message_instance_id=?2",
            params![envelope.correlation.workspace_id.as_str(), message.as_str()],
            |row| Ok(serde_json::json!([row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?])),
        )
        },
    )?;
    let expected_count = envelope.content_ref.as_ref().map(|value| value.byte_count);
    let expected_count_sql = expected_count.map(i64_from_u64).transpose()?;
    let has_content_ref = envelope.content_ref.is_some();
    let delivery = envelope
        .downstream_delivery
        .as_ref()
        .map(enum_json)
        .transpose()?;
    let content_projection = serde_json::json!([
        message.as_str(),
        envelope.correlation.request_id.as_str(),
        envelope.direction.as_str(),
        envelope.fork_id,
        part_ordinal,
        kind,
        media,
        digest.as_str(),
        expected_count,
        has_content_ref,
        envelope.transport_frame_id,
        delivery
    ]);
    insert_or_verify(
        transaction,
        "content_instances_v2",
        envelope.correlation.workspace_id.as_str(),
        content.as_str(),
        &content_projection,
        || {
            transaction.execute(
                "INSERT INTO content_instances_v2
             (workspace_id, content_id, message_instance_id, request_id, direction, fork_id,
              part_ordinal, content_kind, canonical_media_type, content_blob_digest,
              expected_byte_count, has_content_ref, transport_frame_id, downstream_delivery,
              accumulated_byte_count, state, created_at_unix_nanos)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     0, 'installing', ?15)",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    content.as_str(),
                    message.as_str(),
                    envelope.correlation.request_id.as_str(),
                    envelope.direction.as_str(),
                    envelope.fork_id,
                    i64::from(part_ordinal),
                    kind,
                    media,
                    digest.as_str(),
                    expected_count_sql,
                    has_content_ref,
                    envelope.transport_frame_id,
                    delivery,
                    occurred_at
                ],
            )
        },
        || {
            transaction.query_row(
                "SELECT message_instance_id, request_id, direction, fork_id, part_ordinal,
                    content_kind, canonical_media_type, content_blob_digest, expected_byte_count,
                    has_content_ref, transport_frame_id, downstream_delivery
             FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2",
                params![envelope.correlation.workspace_id.as_str(), content.as_str()],
                |row| {
                    Ok(serde_json::json!([
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, bool>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?
                    ]))
                },
            )
        },
    )?;
    let reference_projection =
        serde_json::json!([message.as_str(), content.as_str(), digest.as_str()]);
    let reference_key = format!(
        "{}:{}:{}:{message_ordinal}:{part_ordinal}",
        envelope.correlation.request_id,
        envelope.direction.as_str(),
        envelope.fork_id,
    );
    insert_or_verify(
        transaction,
        "request_content_refs_v2",
        envelope.correlation.workspace_id.as_str(),
        &reference_key,
        &reference_projection,
        || {
            transaction.execute(
                "INSERT INTO request_content_refs_v2
             (workspace_id, request_id, direction, fork_id, message_instance_id, content_id,
              message_ordinal, part_ordinal, blob_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    envelope.correlation.request_id.as_str(),
                    envelope.direction.as_str(),
                    envelope.fork_id,
                    message.as_str(),
                    content.as_str(),
                    i64::from(message_ordinal),
                    i64::from(part_ordinal),
                    digest.as_str()
                ],
            )
        },
        || {
            transaction.query_row(
                "SELECT message_instance_id, content_id, blob_digest FROM request_content_refs_v2
             WHERE workspace_id=?1 AND request_id=?2 AND direction=?3 AND fork_id=?4
               AND message_ordinal=?5 AND part_ordinal=?6",
                params![
                    envelope.correlation.workspace_id.as_str(),
                    envelope.correlation.request_id.as_str(),
                    envelope.direction.as_str(),
                    envelope.fork_id,
                    i64::from(message_ordinal),
                    i64::from(part_ordinal)
                ],
                |row| {
                    Ok(serde_json::json!([
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?
                    ]))
                },
            )
        },
    )?;
    Ok(())
}
