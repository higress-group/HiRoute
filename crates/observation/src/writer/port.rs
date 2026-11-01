use hiroute_domain::{
    ConversationContentEnvelopeV1, ExecutionFactEnvelopeV1, LossNoticeV1, ObservationAckV2,
    ObservationChannel, ObservationGapHeartbeatV1, ObservationNackV1, ObservationStreamV1,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    Ack(ObservationAckV2),
    Nack(ObservationNackV1),
}

pub const MAX_CONTENT_BATCH_RECORDS: usize = 32;

pub trait ObservationCommitPort: Send + Sync {
    fn ingest_fact(
        &self,
        envelope: &ExecutionFactEnvelopeV1,
        channel_losses: &[LossNoticeV1],
    ) -> Result<IngestOutcome, ObservationStoreError>;

    fn ingest_content(
        &self,
        envelope: &ConversationContentEnvelopeV1,
        channel_losses: &[LossNoticeV1],
    ) -> Result<IngestOutcome, ObservationStoreError>;

    /// Commit an ordered prefix and stop at the first NACK. ACKs are returned
    /// only after the prefix's files and transaction are durable. A store error
    /// returns no ACK; the caller must retain the unprocessed records.
    fn ingest_content_batch(
        &self,
        envelopes: &[&ConversationContentEnvelopeV1],
    ) -> Result<Vec<IngestOutcome>, ObservationStoreError> {
        let mut outcomes = Vec::new();
        for envelope in envelopes {
            let outcome = match self.ingest_content(envelope, &[]) {
                Ok(outcome) => outcome,
                Err(error) if outcomes.is_empty() => return Err(error),
                Err(_) => break,
            };
            let rejected = matches!(outcome, IngestOutcome::Nack(_));
            outcomes.push(outcome);
            if rejected {
                break;
            }
        }
        Ok(outcomes)
    }

    fn ingest_gap_heartbeat(
        &self,
        heartbeat: &ObservationGapHeartbeatV1,
    ) -> Result<IngestOutcome, ObservationStoreError>;

    /// Internal queue-loss path used before a producer heartbeat can be materialized. It records
    /// only the exact ranges already owned by the queue and never invents content coordinates.
    fn record_losses(
        &self,
        channel: ObservationChannel,
        stream: &ObservationStreamV1,
        losses: &[LossNoticeV1],
    ) -> Result<(), ObservationStoreError>;
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ObservationStoreError {
    #[error("observation activity store is unavailable")]
    ActivityUnavailable,
    #[error("observation content store is unavailable")]
    ContentUnavailable,
    #[error("observation store data is corrupt or incompatible")]
    Corrupt,
    #[error("observation store I/O failed")]
    Io,
}
