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

use super::{gap, increment_store_revision};

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
        let current = super::current_commit::load(transaction, channel.as_str(), stream)?;
        let detail = match current {
            Some(current) if current.sequence == sequence && current.event_id != event_id => {
                ObservationNackDetailV1::SequenceEventConflict {
                    sequence,
                    expected_event_id: current.event_id,
                    rejected_event_id: event_id.into(),
                }
            }
            Some(current) if current.sequence == sequence => {
                ObservationNackDetailV1::ImmutableProjectionConflict {
                    projection_key: format!("current_commit:{sequence}"),
                    existing_digest: current.digest,
                    rejected_digest: payload_digest.into(),
                }
            }
            _ => ObservationNackDetailV1::ImmutableProjectionConflict {
                projection_key: format!("obsolete_sequence:{sequence}"),
                existing_digest: format!("accounted:{}", checkpoint.accounted),
                rejected_digest: payload_digest.into(),
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
    // Business facts already retain immutable event identities. Content's
    // semantic identities/chunk ordinals are checked by its projection owner.
    if channel == ObservationChannel::Fact {
        let existing: Option<(i64, String)> = cached_sql::query_row(
            transaction,
            "SELECT sequence,envelope_digest FROM execution_fact_events
             WHERE producer_id=?1 AND producer_epoch=?2 AND stream_id=?3 AND event_id=?4",
            params![
                stream.producer_id.as_str(),
                stream.producer_epoch.as_str(),
                stream.stream_id.as_str(),
                event_id
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if let Some((previous, digest)) = existing {
            return Ok(preflight_outcome(IngestOutcome::Nack(nack(
                channel_wire(channel),
                stream,
                sequence,
                checkpoint.accounted.saturating_add(1),
                false,
                ObservationNackDetailV1::ImmutableProjectionConflict {
                    projection_key: format!("event_id:{event_id}"),
                    existing_digest: digest,
                    rejected_digest: format!("sequence={previous};digest={payload_digest}"),
                },
            ))));
        }
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

pub(super) fn persist_feedback(
    transaction: &Transaction<'_>,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    envelope_digest: &str,
    outcome: &IngestOutcome,
    _occurred_at_unix_nanos: u64,
) -> Result<(), ObservationStoreError> {
    if let IngestOutcome::Ack(value) = outcome {
        super::current_commit::save(
            transaction,
            &value.identity.channel,
            stream,
            sequence,
            event_id,
            envelope_digest,
        )?;
    }
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
    let Some(current) = super::current_commit::load(transaction, channel, stream)? else {
        return Ok(None);
    };
    if current.sequence != sequence || current.event_id != event_id || current.digest != digest {
        return Ok(None);
    }
    let kind = match super::current_commit::channel(channel)? {
        "fact" => ObservationChannel::Fact,
        _ => ObservationChannel::Content,
    };
    let checkpoint = load_checkpoint(transaction, kind, stream)?;
    if checkpoint.accounted < sequence {
        return Err(ObservationStoreError::Corrupt);
    }
    // This acknowledges the committed operation, not historical body availability
    // or the exact reply JSON produced by an earlier receiver invocation.
    Ok(Some(IngestOutcome::Ack(ack(
        kind, stream, checkpoint, None,
    ))))
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
