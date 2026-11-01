use hiroute_domain::{
    CanonicalDigest, ConversationContentEnvelopeV1, ExecutionFactEnvelopeV1, LossNoticeV1,
    ObservationChannel, ObservationContentAcknowledgementV1, ObservationEnvelopeViolationV1,
    ObservationGapHeartbeatV1, ObservationNackDetailV1, ObservationStreamV1,
    SequencedObservationEnvelope,
};
use rusqlite::{OptionalExtension, params};

use crate::content::lifecycle::{ContentProjectionError, apply_content};
use crate::content::storage::verify_content_replay;
use crate::receipt::{FactProjectionError, apply_fact};
use crate::writer::{IngestOutcome, ObservationCommitPort, ObservationStoreError};

use super::protocol::{
    Preflight, ack, advance_checkpoint, content_projection_nack, content_validation_nack,
    fact_projection_nack, fact_validation_nack, heartbeat_validation_detail, i64_from_u64,
    insert_content_event, insert_event, insert_gap_event, load_checkpoint, load_feedback, nack,
    persist_feedback, preflight, release_projection, rollback_projection, write_checkpoint,
};
use super::{LocalObservationStore, gap, increment_store_revision};

mod content;

impl ObservationCommitPort for LocalObservationStore {
    fn ingest_fact(
        &self,
        envelope: &ExecutionFactEnvelopeV1,
        channel_losses: &[LossNoticeV1],
    ) -> Result<IngestOutcome, ObservationStoreError> {
        let payload_digest =
            CanonicalDigest::of(envelope).map_err(|_| ObservationStoreError::Corrupt)?;
        let stream = &envelope.producer.stream;
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if super::request_retention::retired(
            &transaction,
            &envelope.correlation.workspace_id,
            &envelope.correlation.request_id,
        )? {
            return Ok(IngestOutcome::Nack(nack(
                "execution_fact",
                stream,
                envelope.sequence,
                load_checkpoint(&transaction, ObservationChannel::Fact, stream)?
                    .accounted
                    .saturating_add(1),
                false,
                ObservationNackDetailV1::ImmutableProjectionConflict {
                    projection_key: format!("expired-request:{}", envelope.correlation.request_id),
                    existing_digest: "retention_expired".into(),
                    rejected_digest: payload_digest.to_string(),
                },
            )));
        }
        if let Some(outcome) = load_feedback(
            &transaction,
            "execution_fact",
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
        )? && !matches!(&outcome, IngestOutcome::Nack(value) if value.retryable)
        {
            let content_erased: bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND session_id=?2 AND content_completeness IN('deleted','expired'))",params![envelope.correlation.workspace_id.as_str(),envelope.correlation.conversation_id.as_str()],|row|row.get(0)).map_err(|_|ObservationStoreError::ActivityUnavailable)?;
            if matches!(outcome, IngestOutcome::Ack(_)) && !content_erased {
                let facts = super::fact_log::load_request(
                    &transaction,
                    &envelope.correlation.workspace_id,
                    &envelope.correlation.request_id,
                )?;
                if !facts.iter().any(|stored| stored == envelope) {
                    return Err(ObservationStoreError::Corrupt);
                }
            }
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        if let Err(error) = envelope.validate() {
            let expected = load_checkpoint(&transaction, ObservationChannel::Fact, stream)?
                .accounted
                .saturating_add(1);
            let outcome = IngestOutcome::Nack(fact_validation_nack(envelope, expected, error));
            persist_feedback(
                &transaction,
                stream,
                envelope.sequence,
                envelope.event_id.as_str(),
                payload_digest.as_str(),
                &outcome,
                envelope.occurred_at_unix_nanos,
            )?;
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        let envelope_losses = envelope.reported_losses();
        let envelope_loss_reason = envelope
            .loss_watermark
            .as_ref()
            .map(|loss| loss.reason.as_str());
        let preflight = preflight(
            &transaction,
            ObservationChannel::Fact,
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
            &envelope.correlation.workspace_id,
            Some(&envelope.correlation.conversation_id),
            i64::try_from(envelope.occurred_at_unix_nanos / 1_000_000)
                .map_err(|_| ObservationStoreError::Corrupt)?,
            &envelope_losses,
            envelope_loss_reason,
            channel_losses,
        )?;
        let Preflight::Ready {
            checkpoint,
            had_gap,
        } = preflight
        else {
            let Preflight::Outcome(outcome) = preflight else {
                unreachable!()
            };
            let outcome = *outcome;
            persist_feedback(
                &transaction,
                stream,
                envelope.sequence,
                envelope.event_id.as_str(),
                payload_digest.as_str(),
                &outcome,
                envelope.occurred_at_unix_nanos,
            )?;
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        };
        transaction
            .execute_batch("SAVEPOINT fact_projection")
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        super::request_retention::revive_for_new_request(
            &transaction,
            &envelope.correlation.workspace_id,
            &envelope.correlation.conversation_id,
            &envelope.correlation.request_id,
        )?;
        super::fact_log::insert(&transaction, envelope, &payload_digest)?;
        if let Err(error) = apply_fact(&transaction, envelope) {
            match error {
                FactProjectionError::Storage => {
                    return Err(ObservationStoreError::ActivityUnavailable);
                }
                FactProjectionError::Corrupt => return Err(ObservationStoreError::Corrupt),
                _ => {
                    rollback_projection(&transaction, "fact_projection")?;
                    let outcome = IngestOutcome::Nack(fact_projection_nack(envelope, error));
                    persist_feedback(
                        &transaction,
                        stream,
                        envelope.sequence,
                        envelope.event_id.as_str(),
                        payload_digest.as_str(),
                        &outcome,
                        envelope.occurred_at_unix_nanos,
                    )?;
                    transaction
                        .commit()
                        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
                    return Ok(outcome);
                }
            }
        }
        // New authenticated ingestion alone establishes Agent attribution. Receipt
        // rebuild/backfill must never assign identity to old observations.
        super::agent_attribution::project(&transaction, envelope)
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        release_projection(&transaction, "fact_projection")?;
        insert_event(
            &transaction,
            ObservationChannel::Fact,
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
        )?;
        let next = advance_checkpoint(
            &transaction,
            ObservationChannel::Fact,
            stream,
            checkpoint,
            envelope.sequence,
            had_gap,
        )?;
        let outcome = IngestOutcome::Ack(ack(ObservationChannel::Fact, stream, next, None));
        persist_feedback(
            &transaction,
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
            &outcome,
            envelope.occurred_at_unix_nanos,
        )?;
        increment_store_revision(&transaction)?;
        transaction
            .commit()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        Ok(outcome)
    }

    fn ingest_content(
        &self,
        envelope: &ConversationContentEnvelopeV1,
        channel_losses: &[LossNoticeV1],
    ) -> Result<IngestOutcome, ObservationStoreError> {
        self.commit_content_batch(&[envelope], channel_losses)?
            .into_iter()
            .next()
            .ok_or(ObservationStoreError::Corrupt)
    }

    fn ingest_content_batch(
        &self,
        envelopes: &[&ConversationContentEnvelopeV1],
    ) -> Result<Vec<IngestOutcome>, ObservationStoreError> {
        self.commit_content_batch(envelopes, &[])
    }

    fn ingest_gap_heartbeat(
        &self,
        heartbeat: &ObservationGapHeartbeatV1,
    ) -> Result<IngestOutcome, ObservationStoreError> {
        let channel = match heartbeat.channel.as_str() {
            "conversation_content" => ObservationChannel::Content,
            "execution_fact" => ObservationChannel::Fact,
            _ => {
                return Ok(IngestOutcome::Nack(nack(
                    &heartbeat.channel,
                    &heartbeat.producer.stream,
                    heartbeat.sequence,
                    heartbeat.sequence,
                    false,
                    ObservationNackDetailV1::InvalidEnvelope {
                        violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
                        field: Some("channel".into()),
                    },
                )));
            }
        };
        let digest = CanonicalDigest::of(heartbeat).map_err(|_| ObservationStoreError::Corrupt)?;
        let stream = &heartbeat.producer.stream;
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if let Some(outcome) = load_feedback(
            &transaction,
            &heartbeat.channel,
            stream,
            heartbeat.sequence,
            heartbeat.event_id.as_str(),
            digest.as_str(),
        )? {
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        if let Err(error) = heartbeat.validate() {
            let detail = heartbeat_validation_detail(error, &heartbeat.schema_version);
            let outcome = IngestOutcome::Nack(nack(
                &heartbeat.channel,
                stream,
                heartbeat.sequence,
                load_checkpoint(&transaction, channel, stream)?
                    .accounted
                    .saturating_add(1),
                false,
                detail,
            ));
            persist_feedback(
                &transaction,
                stream,
                heartbeat.sequence,
                heartbeat.event_id.as_str(),
                digest.as_str(),
                &outcome,
                0,
            )?;
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        let mut checkpoint = load_checkpoint(&transaction, channel, stream)?;
        let first = heartbeat.loss_watermark.first_sequence;
        let last = heartbeat.loss_watermark.last_sequence;
        let expected = checkpoint.accounted.saturating_add(1);
        if heartbeat.sequence <= checkpoint.accounted {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT event_id FROM observation_events
                     WHERE stream_ref=(SELECT stream_ref FROM observation_streams_v5 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4)
                       AND sequence=?5",
                    params![
                        channel.as_str(),
                        stream.producer_id.as_str(),
                        stream.producer_epoch.as_str(),
                        stream.stream_id.as_str(),
                        i64_from_u64(heartbeat.sequence)?,
                    ],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            let detail = existing.map_or_else(
                || ObservationNackDetailV1::MissingPrerequisite {
                    prerequisite_sequence: expected,
                    prerequisite_event_id: None,
                },
                |expected_event_id| ObservationNackDetailV1::SequenceEventConflict {
                    sequence: heartbeat.sequence,
                    expected_event_id,
                    rejected_event_id: heartbeat.event_id.to_string(),
                },
            );
            let outcome = IngestOutcome::Nack(nack(
                &heartbeat.channel,
                stream,
                heartbeat.sequence,
                expected,
                false,
                detail,
            ));
            persist_feedback(
                &transaction,
                stream,
                heartbeat.sequence,
                heartbeat.event_id.as_str(),
                digest.as_str(),
                &outcome,
                0,
            )?;
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        if first != expected {
            let (retryable, detail) = if first > expected {
                (
                    true,
                    ObservationNackDetailV1::MissingSequenceRanges {
                        ranges: vec![hiroute_domain::ObservationSequenceRangeV1 {
                            first_sequence: expected,
                            last_sequence: first - 1,
                        }],
                    },
                )
            } else {
                (
                    false,
                    ObservationNackDetailV1::InvalidEnvelope {
                        violation: ObservationEnvelopeViolationV1::InvalidFieldValue,
                        field: Some("loss_watermark".into()),
                    },
                )
            };
            let outcome = IngestOutcome::Nack(nack(
                &heartbeat.channel,
                stream,
                heartbeat.sequence,
                expected,
                retryable,
                detail,
            ));
            persist_feedback(
                &transaction,
                stream,
                heartbeat.sequence,
                heartbeat.event_id.as_str(),
                digest.as_str(),
                &outcome,
                0,
            )?;
            transaction
                .commit()
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
            return Ok(outcome);
        }
        transaction
            .execute(
                "INSERT INTO observation_gaps
             (channel, producer_id, producer_epoch, stream_id, first_sequence, last_sequence,
              known_loss, workspace_id, session_id, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, '', NULL, ?7)",
                params![
                    channel.as_str(),
                    stream.producer_id.as_str(),
                    stream.producer_epoch.as_str(),
                    stream.stream_id.as_str(),
                    i64_from_u64(first)?,
                    i64_from_u64(last)?,
                    heartbeat.loss_watermark.reason
                ],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        transaction
            .execute(
                "INSERT INTO observation_gap_heartbeats_v1
             (channel, producer_id, producer_epoch, stream_id, sequence, event_id, heartbeat_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    heartbeat.channel,
                    stream.producer_id.as_str(),
                    stream.producer_epoch.as_str(),
                    stream.stream_id.as_str(),
                    i64_from_u64(heartbeat.sequence)?,
                    heartbeat.event_id.as_str(),
                    serde_json::to_string(heartbeat).map_err(|_| ObservationStoreError::Corrupt)?
                ],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        checkpoint.accounted = heartbeat.sequence;
        write_checkpoint(&transaction, channel, stream, checkpoint)?;
        insert_gap_event(
            &transaction,
            channel,
            stream,
            heartbeat.sequence,
            heartbeat.event_id.as_str(),
            digest.as_str(),
        )?;
        let outcome = IngestOutcome::Ack(ack(channel, stream, checkpoint, None));
        persist_feedback(
            &transaction,
            stream,
            heartbeat.sequence,
            heartbeat.event_id.as_str(),
            digest.as_str(),
            &outcome,
            0,
        )?;
        increment_store_revision(&transaction)?;
        transaction
            .commit()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        Ok(outcome)
    }

    fn record_losses(
        &self,
        channel: ObservationChannel,
        stream: &ObservationStreamV1,
        losses: &[LossNoticeV1],
    ) -> Result<(), ObservationStoreError> {
        if losses.is_empty() {
            return Ok(());
        }
        let mut losses = losses.to_vec();
        losses.sort_by_key(|loss| loss.range.first);
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let mut checkpoint = load_checkpoint(&transaction, channel, stream)?;
        for loss in &losses {
            gap::record(&transaction, channel, stream, loss, true, None)?;
            gap::mark_session_partial(&transaction, channel, loss)?;
            if loss.range.first <= checkpoint.accounted.saturating_add(1)
                && loss.range.last > checkpoint.accounted
            {
                checkpoint.accounted = loss.range.last;
            }
        }
        write_checkpoint(&transaction, channel, stream, checkpoint)?;
        increment_store_revision(&transaction)?;
        transaction
            .commit()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)
    }
}
