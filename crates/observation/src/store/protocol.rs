use crate::store::cached_sql;
use hiroute_domain::{
    CONVERSATION_CONTENT_SCHEMA_V2, CanonicalDigest, ConversationContentEnvelopeV1,
    ExecutionFactEnvelopeV1, ExecutionFactError, LossNoticeV1, OBSERVATION_ACK_SCHEMA_V2,
    OBSERVATION_NACK_SCHEMA_V1, ObservationAckV2, ObservationChannel,
    ObservationContentAcknowledgementV1, ObservationEnvelopeError, ObservationEnvelopeViolationV1,
    ObservationFeedbackIdentityV1, ObservationNackDetailV1, ObservationNackV1,
    ObservationSequenceRangeV1, ObservationStreamV1, SequenceRangeV1,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::content::lifecycle::{ContentProjectionAck, ContentProjectionError};
use crate::receipt::FactProjectionError;
use crate::writer::{IngestOutcome, ObservationStoreError};

use super::{gap, increment_store_revision, record_projection, records};

#[derive(Clone, Copy, Debug)]
pub(super) struct Checkpoint {
    pub(super) contiguous: u64,
    pub(super) accounted: u64,
}

pub(super) enum Preflight {
    Ready {
        checkpoint: Checkpoint,
        had_gap: bool,
    },
    Outcome(Box<IngestOutcome>),
}

fn preflight_outcome(outcome: IngestOutcome) -> Preflight {
    Preflight::Outcome(Box::new(outcome))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn preflight(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    payload_digest: &str,
    workspace_id: &hiroute_domain::WorkspaceId,
    session_id: Option<&hiroute_domain::SessionId>,
    occurred_ms: i64,
    envelope_losses: &[LossNoticeV1],
    envelope_loss_reason: Option<&str>,
    channel_losses: &[LossNoticeV1],
) -> Result<Preflight, ObservationStoreError> {
    let mut checkpoint = load_checkpoint(transaction, channel, stream)?;
    if sequence <= checkpoint.accounted {
        let existing: Option<(String, String)> = cached_sql::query_row(transaction, "SELECT event_id, payload_digest FROM observation_events
             WHERE stream_ref=(SELECT stream_ref FROM observation_streams_v5 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4) AND sequence=?5",
            params![channel.as_str(), stream.producer_id.as_str(), stream.producer_epoch.as_str(),
                stream.stream_id.as_str(), i64_from_u64(sequence)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let detail = match existing {
            Some((expected_event_id, expected_digest))
                if expected_event_id == event_id && expected_digest == payload_digest =>
            {
                return Err(ObservationStoreError::Corrupt);
            }
            Some((expected_event_id, _)) => ObservationNackDetailV1::SequenceEventConflict {
                sequence,
                expected_event_id,
                rejected_event_id: event_id.into(),
            },
            None => ObservationNackDetailV1::MissingPrerequisite {
                prerequisite_sequence: checkpoint.accounted.saturating_add(1),
                prerequisite_event_id: None,
            },
        };
        return Ok(preflight_outcome(IngestOutcome::Nack(nack(
            channel_wire(channel),
            stream,
            sequence,
            checkpoint.accounted.saturating_add(1),
            false,
            detail,
        ))));
    }
    let existing_event: Option<(i64, String)> = cached_sql::query_row(transaction, "SELECT sequence, payload_digest FROM observation_events
         WHERE stream_ref=(SELECT stream_ref FROM observation_streams_v5 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4) AND event_id=?5",
        params![channel.as_str(), stream.producer_id.as_str(), stream.producer_epoch.as_str(),
            stream.stream_id.as_str(), event_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    if let Some((existing_sequence, existing_digest)) = existing_event {
        return Ok(preflight_outcome(IngestOutcome::Nack(nack(
            channel_wire(channel),
            stream,
            sequence,
            checkpoint.accounted.saturating_add(1),
            false,
            ObservationNackDetailV1::ImmutableProjectionConflict {
                projection_key: format!("event_id:{event_id}"),
                existing_digest,
                rejected_digest: format!("sequence={existing_sequence};digest={payload_digest}"),
            },
        ))));
    }
    if let Some(session_id) = session_id {
        let tombstoned: bool = cached_sql::query_row(transaction, "SELECT EXISTS(SELECT 1 FROM observation_session_barriers_v2 WHERE workspace_id=?1 AND session_id=?2 AND through_ms>=?3)",
            params![workspace_id.as_str(), session_id.as_str(),occurred_ms], |row| row.get(0),
        ).map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if tombstoned {
            return Ok(preflight_outcome(IngestOutcome::Nack(nack(
                channel_wire(channel),
                stream,
                sequence,
                checkpoint.accounted.saturating_add(1),
                false,
                ObservationNackDetailV1::ImmutableProjectionConflict {
                    projection_key: format!("tombstone:{}:{}", workspace_id, session_id),
                    existing_digest: "deleted".into(),
                    rejected_digest: payload_digest.into(),
                },
            ))));
        }
    }
    let mut losses = envelope_losses
        .iter()
        .map(|loss| (loss, envelope_loss_reason))
        .chain(channel_losses.iter().map(|loss| (loss, None)))
        .collect::<Vec<_>>();
    losses.sort_by_key(|(loss, _)| loss.range.first);
    let mut cursor = checkpoint.accounted.saturating_add(1);
    let mut had_gap = checkpoint.contiguous != checkpoint.accounted;
    for (loss, reason) in losses {
        if loss.scope.workspace_id != *workspace_id || loss.range.last >= sequence {
            return Ok(preflight_outcome(IngestOutcome::Nack(nack(
                channel_wire(channel),
                stream,
                sequence,
                cursor,
                false,
                ObservationNackDetailV1::InvalidEnvelope {
                    violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
                    field: Some("loss_watermark".into()),
                },
            ))));
        }
        if loss.range.last < cursor {
            continue;
        }
        if loss.range.first > cursor {
            break;
        }
        gap::record(transaction, channel, stream, loss, true, reason)?;
        gap::mark_session_partial(transaction, channel, loss)?;
        cursor = loss.range.last.saturating_add(1);
        checkpoint.accounted = loss.range.last;
        had_gap = true;
    }
    if cursor != sequence {
        let missing = SequenceRangeV1 {
            first: cursor,
            last: sequence - 1,
        };
        let loss = LossNoticeV1 {
            range: missing,
            scope: hiroute_domain::ObservationScopeV1 {
                workspace_id: workspace_id.clone(),
                session_id: session_id.cloned(),
            },
        };
        gap::record(transaction, channel, stream, &loss, false, None)?;
        gap::mark_session_partial(transaction, channel, &loss)?;
        increment_store_revision(transaction)?;
        return Ok(preflight_outcome(IngestOutcome::Nack(nack(
            channel_wire(channel),
            stream,
            sequence,
            cursor,
            true,
            ObservationNackDetailV1::MissingSequenceRanges {
                ranges: vec![ObservationSequenceRangeV1 {
                    first_sequence: missing.first,
                    last_sequence: missing.last,
                }],
            },
        ))));
    }
    Ok(Preflight::Ready {
        checkpoint,
        had_gap,
    })
}

pub(super) fn load_checkpoint(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
) -> Result<Checkpoint, ObservationStoreError> {
    cached_sql::query_row(transaction, "SELECT highest_contiguous_sequence, highest_accounted_sequence FROM observation_checkpoints
         WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4",
        params![channel.as_str(), stream.producer_id.as_str(), stream.producer_epoch.as_str(), stream.stream_id.as_str()],
        |row| { let contiguous: i64 = row.get(0)?; let accounted: i64 = row.get(1)?;
            Ok(Checkpoint { contiguous: contiguous.try_into().map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, contiguous))?,
                accounted: accounted.try_into().map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, accounted))? }) },
    ).optional().map(|value| value.unwrap_or(Checkpoint { contiguous: 0, accounted: 0 }))
        .map_err(|_| ObservationStoreError::Corrupt)
}

pub(super) fn advance_checkpoint(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    prior: Checkpoint,
    sequence: u64,
    had_gap: bool,
) -> Result<Checkpoint, ObservationStoreError> {
    let checkpoint = Checkpoint {
        contiguous: if !had_gap && prior.contiguous == prior.accounted {
            sequence
        } else {
            prior.contiguous
        },
        accounted: sequence,
    };
    write_checkpoint(transaction, channel, stream, checkpoint)?;
    Ok(checkpoint)
}

pub(super) fn write_checkpoint(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    checkpoint: Checkpoint,
) -> Result<(), ObservationStoreError> {
    cached_sql::execute(transaction, "INSERT INTO observation_checkpoints
         (channel, producer_id, producer_epoch, stream_id, highest_contiguous_sequence, highest_accounted_sequence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(channel, producer_id, producer_epoch, stream_id) DO UPDATE SET
           highest_contiguous_sequence=excluded.highest_contiguous_sequence,
           highest_accounted_sequence=excluded.highest_accounted_sequence",
        params![channel.as_str(), stream.producer_id.as_str(), stream.producer_epoch.as_str(), stream.stream_id.as_str(),
            i64_from_u64(checkpoint.contiguous)?, i64_from_u64(checkpoint.accounted)?],
    ).map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

pub(super) fn insert_event(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    payload_digest: &str,
) -> Result<(), ObservationStoreError> {
    let stream_ref = records::stream_ref(transaction, channel.as_str(), stream)?;
    cached_sql::execute(transaction, "INSERT INTO observation_events(stream_ref,sequence,event_id,payload_digest) VALUES(?1,?2,?3,?4)",
        params![stream_ref,i64_from_u64(sequence)?,event_id,payload_digest])
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    gap::resolve_sequence(transaction, channel, stream, sequence)?;

    Ok(())
}

pub(super) fn insert_gap_event(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    payload_digest: &str,
) -> Result<(), ObservationStoreError> {
    let stream_ref = records::stream_ref(transaction, channel.as_str(), stream)?;
    cached_sql::execute(transaction, "INSERT INTO observation_events(stream_ref,sequence,event_id,payload_digest) VALUES(?1,?2,?3,?4)",
        params![stream_ref,i64_from_u64(sequence)?,event_id,payload_digest])
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;

    Ok(())
}

pub(super) fn insert_content_event(
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    digest: &CanonicalDigest,
    projection: &ContentProjectionAck,
) -> Result<(), ObservationStoreError> {
    let mut metadata =
        serde_json::to_value(envelope).map_err(|_| ObservationStoreError::Corrupt)?;
    metadata
        .as_object_mut()
        .ok_or(ObservationStoreError::Corrupt)?
        .remove("canonical_bytes_base64");
    let stream_ref = records::stream_ref(
        transaction,
        "conversation_content",
        &envelope.producer.stream,
    )?;
    cached_sql::execute(
        transaction,
        "INSERT INTO conversation_content_events_v2
         (stream_ref, sequence, event_id, workspace_id, request_id,
          direction, fork_id, phase, metadata_json, canonical_chunk_object_ref,
          canonical_chunk_byte_offset, canonical_chunk_byte_count, envelope_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            stream_ref,
            i64_from_u64(envelope.sequence)?,
            envelope.event_id.as_str(),
            envelope.correlation.workspace_id.as_str(),
            envelope.correlation.request_id.as_str(),
            envelope.direction.as_str(),
            envelope.fork_id,
            envelope.phase.as_str(),
            record_projection::encode(
                metadata,
                &record_projection::ContentColumns::from_envelope(envelope).shared(),
            )?,
            projection.chunk_object_ref,
            projection.chunk_byte_offset.map(i64_from_u64).transpose()?,
            projection.chunk_byte_count.map(i64_from_u64).transpose()?,
            digest.as_str()
        ],
    )
    .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

pub(super) fn persist_feedback(
    transaction: &Transaction<'_>,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    envelope_digest: &str,
    outcome: &IngestOutcome,
    occurred_at_unix_nanos: u64,
) -> Result<(), ObservationStoreError> {
    let (channel, kind, json) = match outcome {
        IngestOutcome::Ack(value) => (&value.identity.channel, "ack", serde_json::to_value(value)),
        IngestOutcome::Nack(value) => {
            (&value.identity.channel, "nack", serde_json::to_value(value))
        }
    };
    let stream_ref = records::stream_ref(transaction, channel, stream)?;
    cached_sql::execute(
        transaction,
        "INSERT INTO observation_feedback_v2
         (stream_ref, rejected_or_acked_sequence,
          event_id, envelope_digest, feedback_kind, feedback_json, created_at_unix_nanos)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            stream_ref,
            i64_from_u64(sequence)?,
            event_id,
            envelope_digest,
            kind,
            record_projection::encode(
                json.map_err(|_| ObservationStoreError::Corrupt)?,
                &record_projection::feedback_fields(channel, stream),
            )?,
            i64_from_u64(occurred_at_unix_nanos)?
        ],
    )
    .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

pub(super) fn load_feedback(
    transaction: &Transaction<'_>,
    channel: &str,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    digest: &str,
) -> Result<Option<IngestOutcome>, ObservationStoreError> {
    let row: Option<(String, Vec<u8>)> = cached_sql::query_row(transaction, "SELECT feedback_kind, feedback_json FROM observation_feedback_v2
         WHERE stream_ref=(SELECT stream_ref FROM observation_streams_v5 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4)
           AND rejected_or_acked_sequence=?5 AND event_id=?6 AND envelope_digest=?7
         ORDER BY feedback_id DESC LIMIT 1",
            params![
                channel,
                stream.producer_id.as_str(),
                stream.producer_epoch.as_str(),
                stream.stream_id.as_str(),
                i64_from_u64(sequence)?,
                event_id,
                digest
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    match row {
        Some((kind, json)) if kind == "ack" => serde_json::from_value::<ObservationAckV2>(
            record_projection::decode(&json, &record_projection::feedback_fields(channel, stream))?,
        )
        .map(IngestOutcome::Ack)
        .map(Some)
        .map_err(|_| ObservationStoreError::Corrupt),
        Some((kind, json)) if kind == "nack" => serde_json::from_value::<ObservationNackV1>(
            record_projection::decode(&json, &record_projection::feedback_fields(channel, stream))?,
        )
        .map(IngestOutcome::Nack)
        .map(Some)
        .map_err(|_| ObservationStoreError::Corrupt),
        Some(_) => Err(ObservationStoreError::Corrupt),
        None => Ok(None),
    }
}

pub(super) fn ack(
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    checkpoint: Checkpoint,
    content: Option<ObservationContentAcknowledgementV1>,
) -> ObservationAckV2 {
    ObservationAckV2 {
        schema_version: OBSERVATION_ACK_SCHEMA_V2.into(),
        identity: identity(channel_wire(channel), stream),
        highest_contiguous_sequence: checkpoint.contiguous,
        highest_accounted_sequence: checkpoint.accounted,
        content_acknowledgement: content,
    }
}

pub(super) fn nack(
    channel: &str,
    stream: &ObservationStreamV1,
    rejected_sequence: u64,
    expected_sequence: u64,
    retryable: bool,
    detail: ObservationNackDetailV1,
) -> ObservationNackV1 {
    ObservationNackV1 {
        schema_version: OBSERVATION_NACK_SCHEMA_V1.into(),
        identity: identity(channel, stream),
        rejected_sequence,
        expected_sequence: expected_sequence.max(1),
        retryable,
        detail,
    }
}

fn identity(channel: &str, stream: &ObservationStreamV1) -> ObservationFeedbackIdentityV1 {
    ObservationFeedbackIdentityV1 {
        channel: channel.into(),
        producer_id: stream.producer_id.to_string(),
        producer_epoch: stream.producer_epoch.to_string(),
        stream_id: stream.stream_id.to_string(),
    }
}

pub(super) fn content_validation_nack(
    envelope: &ConversationContentEnvelopeV1,
    expected_sequence: u64,
    error: ObservationEnvelopeError,
) -> ObservationNackV1 {
    let detail = if error == ObservationEnvelopeError::UnsupportedSchema
        && envelope.schema_version == CONVERSATION_CONTENT_SCHEMA_V2
    {
        ObservationNackDetailV1::DigestMismatch {
            subject: hiroute_domain::ObservationDigestSubjectV1::EnvelopeSchema,
            subject_id: envelope.schema_version.clone(),
            expected_digest: hiroute_domain::CONVERSATION_CONTENT_PORT_DIGEST_V2.into(),
            rejected_digest: envelope.schema_digest.to_string(),
        }
    } else if error == ObservationEnvelopeError::UnsupportedSchema {
        ObservationNackDetailV1::UnsupportedSchema {
            rejected_schema_version: envelope.schema_version.clone(),
            supported_schema_versions: vec![CONVERSATION_CONTENT_SCHEMA_V2.into()],
        }
    } else {
        ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
            field: Some("envelope".into()),
        }
    };
    nack(
        "conversation_content",
        &envelope.producer.stream,
        envelope.sequence,
        expected_sequence,
        false,
        detail,
    )
}

pub(super) fn fact_validation_nack(
    envelope: &ExecutionFactEnvelopeV1,
    expected_sequence: u64,
    error: ExecutionFactError,
) -> ObservationNackV1 {
    let detail = match error {
        ExecutionFactError::UnsupportedSchema => ObservationNackDetailV1::UnsupportedSchema {
            rejected_schema_version: envelope.schema_version.clone(),
            supported_schema_versions: vec![hiroute_domain::EXECUTION_FACT_SCHEMA_V2.into()],
        },
        _ => ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
            field: Some("envelope".into()),
        },
    };
    nack(
        "execution_fact",
        &envelope.producer.stream,
        envelope.sequence,
        expected_sequence,
        false,
        detail,
    )
}

pub(super) fn heartbeat_validation_detail(
    error: ObservationEnvelopeError,
    rejected_schema_version: &str,
) -> ObservationNackDetailV1 {
    match error {
        ObservationEnvelopeError::UnsupportedSchema => ObservationNackDetailV1::UnsupportedSchema {
            rejected_schema_version: rejected_schema_version.into(),
            supported_schema_versions: vec![
                hiroute_domain::OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1.into(),
            ],
        },
        _ => ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
            field: Some("gap_heartbeat".into()),
        },
    }
}

pub(super) fn fact_projection_nack(
    envelope: &ExecutionFactEnvelopeV1,
    error: FactProjectionError,
) -> ObservationNackV1 {
    let detail = match error {
        FactProjectionError::MissingPrerequisite => ObservationNackDetailV1::MissingPrerequisite {
            prerequisite_sequence: envelope.sequence.saturating_sub(1).max(1),
            prerequisite_event_id: None,
        },
        FactProjectionError::ImmutableConflict => {
            ObservationNackDetailV1::ImmutableProjectionConflict {
                projection_key: envelope.correlation.request_id.to_string(),
                existing_digest: "existing".into(),
                rejected_digest: "rejected".into(),
            }
        }
        FactProjectionError::Invalid => ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
            field: Some("fact".into()),
        },
        FactProjectionError::Storage | FactProjectionError::Corrupt => unreachable!(),
    };
    nack(
        "execution_fact",
        &envelope.producer.stream,
        envelope.sequence,
        envelope.sequence,
        false,
        detail,
    )
}

pub(super) fn content_projection_nack(
    envelope: &ConversationContentEnvelopeV1,
    error: ContentProjectionError,
) -> ObservationNackV1 {
    let coordinates = || {
        (
            envelope.correlation.request_id.to_string(),
            envelope.direction,
            envelope.fork_id.clone(),
        )
    };
    let retryable = matches!(
        &error,
        ContentProjectionError::UnknownTranscriptRoot(_)
            | ContentProjectionError::MissingBlob(_)
            | ContentProjectionError::ChunkOrdinalConflict { .. }
    );
    let detail = match error {
        ContentProjectionError::UnknownTranscriptRoot(root) => {
            let (request_id, direction, fork_id) = coordinates();
            ObservationNackDetailV1::UnknownTranscriptRoot {
                request_id,
                direction,
                fork_id,
                transcript_root: root.to_string(),
            }
        }
        ContentProjectionError::MissingBlob(blobs) => {
            let (request_id, direction, fork_id) = coordinates();
            ObservationNackDetailV1::MissingBlob {
                request_id,
                direction,
                fork_id,
                blobs,
            }
        }
        ContentProjectionError::ContentStateConflict { expected, rejected } => {
            let (request_id, direction, fork_id) = coordinates();
            ObservationNackDetailV1::ContentStateConflict {
                request_id,
                direction,
                fork_id,
                expected_phase: expected,
                rejected_phase: rejected,
            }
        }
        ContentProjectionError::ChunkOrdinalConflict { expected, rejected } => {
            let (request_id, direction, fork_id) = coordinates();
            ObservationNackDetailV1::ChunkOrdinalConflict {
                request_id,
                direction,
                fork_id,
                expected_chunk_ordinal: expected,
                rejected_chunk_ordinal: rejected,
            }
        }
        ContentProjectionError::DigestMismatch {
            subject,
            subject_id,
            expected,
            rejected,
        } => ObservationNackDetailV1::DigestMismatch {
            subject,
            subject_id,
            expected_digest: expected,
            rejected_digest: rejected,
        },
        ContentProjectionError::ImmutableConflict {
            key,
            existing,
            rejected,
        } => ObservationNackDetailV1::ImmutableProjectionConflict {
            projection_key: key,
            existing_digest: existing,
            rejected_digest: rejected,
        },
        ContentProjectionError::Invalid(field) => ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
            field: Some(field.into()),
        },
        ContentProjectionError::ActivityStorage
        | ContentProjectionError::ContentStorage
        | ContentProjectionError::ContentCorrupt => unreachable!(),
    };
    nack(
        "conversation_content",
        &envelope.producer.stream,
        envelope.sequence,
        envelope.sequence,
        retryable,
        detail,
    )
}

pub(super) fn rollback_projection(
    transaction: &Transaction<'_>,
    name: &str,
) -> Result<(), ObservationStoreError> {
    transaction
        .execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"))
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

pub(super) fn release_projection(
    transaction: &Transaction<'_>,
    name: &str,
) -> Result<(), ObservationStoreError> {
    transaction
        .execute_batch(&format!("RELEASE {name}"))
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

const fn channel_wire(channel: ObservationChannel) -> &'static str {
    match channel {
        ObservationChannel::Fact => "execution_fact",
        ObservationChannel::Content => "conversation_content",
    }
}

pub(super) fn i64_from_u64(value: u64) -> Result<i64, ObservationStoreError> {
    value.try_into().map_err(|_| ObservationStoreError::Corrupt)
}
