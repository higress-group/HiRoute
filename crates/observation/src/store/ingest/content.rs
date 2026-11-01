//! Bounded content group commit. Original envelopes and per-record feedback stay unchanged.
use super::*;
use crate::content::durability::ContentDurability;
use rusqlite::Transaction;
use std::path::PathBuf;

impl LocalObservationStore {
    pub(super) fn commit_content_batch(
        &self,
        envelopes: &[&ConversationContentEnvelopeV1],
        channel_losses: &[LossNoticeV1],
    ) -> Result<Vec<IngestOutcome>, ObservationStoreError> {
        if envelopes.len() > crate::writer::MAX_CONTENT_BATCH_RECORDS {
            return Err(ObservationStoreError::Corrupt);
        }
        #[cfg(test)]
        let _total = super::super::ingest_profile::span("total");
        #[cfg(test)]
        let waiting = super::super::ingest_profile::span("connection_wait");
        let mut connection = self.connection.lock();
        #[cfg(test)]
        drop(waiting);
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let mut durability = ContentDurability::default();
        let mut cleanup = Vec::new();
        let mut outcomes = Vec::with_capacity(envelopes.len());
        for envelope in envelopes {
            let outcome = self.ingest_content_transaction(
                &transaction,
                envelope,
                channel_losses,
                &mut durability,
                &mut cleanup,
            )?;
            let rejected = matches!(outcome, IngestOutcome::Nack(_));
            outcomes.push(outcome);
            // A rejected record is the end of this committed prefix. Its gap
            // must reach the receiver before any later sequence is attempted.
            if rejected {
                break;
            }
        }
        durability
            .flush()
            .map_err(|_| ObservationStoreError::ContentUnavailable)?;
        #[cfg(test)]
        let committing = super::super::ingest_profile::span("transaction_commit");
        transaction
            .commit()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        #[cfg(test)]
        drop(committing);
        drop(connection);
        for directory in cleanup {
            if std::fs::remove_dir_all(&directory).is_ok() {
                self.note_removed_directory(&directory);
            }
        }
        Ok(outcomes)
    }

    fn ingest_content_transaction(
        &self,
        transaction: &Transaction<'_>,
        envelope: &ConversationContentEnvelopeV1,
        channel_losses: &[LossNoticeV1],
        durability: &mut ContentDurability,
        cleanup: &mut Vec<PathBuf>,
    ) -> Result<IngestOutcome, ObservationStoreError> {
        let payload_digest =
            CanonicalDigest::of(envelope).map_err(|_| ObservationStoreError::Corrupt)?;
        let stream = &envelope.producer.stream;
        if super::super::request_retention::retired(
            transaction,
            &envelope.correlation.workspace_id,
            &envelope.correlation.request_id,
        )? {
            return Ok(IngestOutcome::Nack(nack(
                "conversation_content",
                stream,
                envelope.sequence,
                load_checkpoint(transaction, ObservationChannel::Content, stream)?
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
            transaction,
            "conversation_content",
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
        )? && !matches!(&outcome, IngestOutcome::Nack(value) if value.retryable)
        {
            if let IngestOutcome::Ack(acknowledgement) = &outcome {
                match verify_content_replay(
                    self,
                    transaction,
                    envelope,
                    &payload_digest,
                    acknowledgement,
                ) {
                    Ok(()) => {}
                    Err(ContentProjectionError::ActivityStorage) => {
                        return Err(ObservationStoreError::ActivityUnavailable);
                    }
                    Err(ContentProjectionError::ContentStorage) => {
                        return Err(ObservationStoreError::ContentUnavailable);
                    }
                    Err(_) => return Err(ObservationStoreError::Corrupt),
                }
            }
            return Ok(outcome);
        }
        if let Err(error) = envelope.validate() {
            let expected = load_checkpoint(transaction, ObservationChannel::Content, stream)?
                .accounted
                .saturating_add(1);
            let outcome = IngestOutcome::Nack(content_validation_nack(envelope, expected, error));
            persist_feedback(
                transaction,
                stream,
                envelope.sequence,
                envelope.event_id.as_str(),
                payload_digest.as_str(),
                &outcome,
                envelope.occurred_at_unix_nanos,
            )?;
            return Ok(outcome);
        }
        let envelope_losses = envelope.reported_losses();
        let envelope_loss_reason = envelope
            .loss_watermark
            .as_ref()
            .map(|loss| loss.reason.as_str());
        let preflight = preflight(
            transaction,
            ObservationChannel::Content,
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
                transaction,
                stream,
                envelope.sequence,
                envelope.event_id.as_str(),
                payload_digest.as_str(),
                &outcome,
                envelope.occurred_at_unix_nanos,
            )?;
            return Ok(outcome);
        };
        transaction
            .execute_batch("SAVEPOINT content_projection")
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        super::super::request_retention::revive_for_new_request(
            transaction,
            &envelope.correlation.workspace_id,
            &envelope.correlation.conversation_id,
            &envelope.correlation.request_id,
        )?;
        #[cfg(test)]
        let projecting = super::super::ingest_profile::span("content_projection");
        let applied = apply_content(self, transaction, envelope, durability);
        #[cfg(test)]
        drop(projecting);
        let projection = match applied {
            Ok(projection) => projection,
            Err(ContentProjectionError::ActivityStorage) => {
                return Err(ObservationStoreError::ActivityUnavailable);
            }
            Err(ContentProjectionError::ContentStorage) => {
                return Err(ObservationStoreError::ContentUnavailable);
            }
            Err(ContentProjectionError::ContentCorrupt) => {
                return Err(ObservationStoreError::Corrupt);
            }
            Err(error) => {
                rollback_projection(transaction, "content_projection")?;
                let outcome = IngestOutcome::Nack(content_projection_nack(envelope, error));
                persist_feedback(
                    transaction,
                    stream,
                    envelope.sequence,
                    envelope.event_id.as_str(),
                    payload_digest.as_str(),
                    &outcome,
                    envelope.occurred_at_unix_nanos,
                )?;
                return Ok(outcome);
            }
        };
        release_projection(transaction, "content_projection")?;
        insert_content_event(transaction, envelope, &payload_digest, &projection)?;
        insert_event(
            transaction,
            ObservationChannel::Content,
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
        )?;
        let next = advance_checkpoint(
            transaction,
            ObservationChannel::Content,
            stream,
            checkpoint,
            envelope.sequence,
            had_gap,
        )?;
        let content_ack = ObservationContentAcknowledgementV1 {
            request_id: envelope.correlation.request_id.to_string(),
            direction: envelope.direction,
            fork_id: envelope.fork_id.clone(),
            next_chunk_ordinal: projection.next_chunk_ordinal,
            transcript_root: projection.transcript_root.clone(),
            delta_parent_transcript_root: projection.delta_parent_transcript_root.clone(),
            acknowledged_blobs: projection.acknowledged_blobs.clone(),
        };
        let outcome = IngestOutcome::Ack(ack(
            ObservationChannel::Content,
            stream,
            next,
            Some(content_ack),
        ));
        persist_feedback(
            transaction,
            stream,
            envelope.sequence,
            envelope.event_id.as_str(),
            payload_digest.as_str(),
            &outcome,
            envelope.occurred_at_unix_nanos,
        )?;
        increment_store_revision(transaction)?;
        cleanup.extend(
            projection
                .cleanup_content_ids
                .into_iter()
                .map(|content_id| {
                    self.staging_directory(&envelope.correlation.workspace_id, &content_id)
                }),
        );
        Ok(outcome)
    }
}
