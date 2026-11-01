//! Non-blocking producer queues and the single local writer consumer.

mod channel;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
mod port;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use hiroute_domain::ObservationChannel;

pub use channel::{ConversationContentChannel, FactChannel, OfferOutcome};
pub use hiroute_domain::{ObservationAckV2, ObservationNackDetailV1, ObservationNackV1};
pub use lifecycle::BoundedLifecycleReceiverV2;
pub use port::{
    IngestOutcome, MAX_CONTENT_BATCH_RECORDS, ObservationCommitPort, ObservationStoreError,
};

#[derive(Clone)]
pub struct LocalObservationWriter {
    store: Arc<dyn ObservationCommitPort>,
}

impl LocalObservationWriter {
    pub fn new(store: Arc<dyn ObservationCommitPort>) -> Self {
        Self { store }
    }

    pub fn consume_fact(&self, channel: &FactChannel) -> WriterCycleOutcome {
        let Some(item) = channel.pop() else {
            return WriterCycleOutcome::Idle;
        };
        let losses = item.channel_losses.clone();
        let outcome = self.isolate(|| self.store.ingest_fact(&item.envelope, &item.channel_losses));
        if !matches!(outcome, WriterCycleOutcome::Ack(_)) {
            channel.restore_losses(losses);
        }
        outcome
    }

    pub fn consume_content(&self, channel: &ConversationContentChannel) -> WriterCycleOutcome {
        let Some(item) = channel.pop() else {
            return WriterCycleOutcome::Idle;
        };
        let losses = item.channel_losses.clone();
        let outcome = self.isolate(|| {
            self.store
                .ingest_content(&item.envelope, &item.channel_losses)
        });
        if !matches!(outcome, WriterCycleOutcome::Ack(_)) {
            channel.restore_losses(losses);
        }
        outcome
    }

    pub fn flush_fact_losses(&self, channel: &FactChannel) -> WriterCycleOutcome {
        let losses = channel.take_losses();
        let outcome = self.flush_losses(ObservationChannel::Fact, channel.stream(), losses.clone());
        if matches!(
            outcome,
            WriterCycleOutcome::StoreFailed(_) | WriterCycleOutcome::StorePanicked
        ) {
            channel.restore_losses(losses);
        }
        outcome
    }

    pub fn flush_content_losses(&self, channel: &ConversationContentChannel) -> WriterCycleOutcome {
        let losses = channel.take_losses();
        let outcome = self.flush_losses(
            ObservationChannel::Content,
            channel.stream(),
            losses.clone(),
        );
        if matches!(
            outcome,
            WriterCycleOutcome::StoreFailed(_) | WriterCycleOutcome::StorePanicked
        ) {
            channel.restore_losses(losses);
        }
        outcome
    }

    fn flush_losses(
        &self,
        channel: ObservationChannel,
        stream: &hiroute_domain::ObservationStreamV1,
        losses: Vec<hiroute_domain::LossNoticeV1>,
    ) -> WriterCycleOutcome {
        if losses.is_empty() {
            return WriterCycleOutcome::Idle;
        }
        match catch_unwind(AssertUnwindSafe(|| {
            self.store.record_losses(channel, stream, &losses)
        })) {
            Ok(Ok(())) => WriterCycleOutcome::LossesRecorded(losses.len()),
            Ok(Err(error)) => WriterCycleOutcome::StoreFailed(error),
            Err(_) => WriterCycleOutcome::StorePanicked,
        }
    }

    fn isolate<F>(&self, operation: F) -> WriterCycleOutcome
    where
        F: FnOnce() -> Result<IngestOutcome, ObservationStoreError>,
    {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(Ok(IngestOutcome::Ack(ack))) => WriterCycleOutcome::Ack(ack),
            Ok(Ok(IngestOutcome::Nack(nack))) => WriterCycleOutcome::Nack(nack),
            Ok(Err(error)) => WriterCycleOutcome::StoreFailed(error),
            Err(_) => WriterCycleOutcome::StorePanicked,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriterCycleOutcome {
    Idle,
    Ack(ObservationAckV2),
    Nack(ObservationNackV1),
    LossesRecorded(usize),
    StoreFailed(ObservationStoreError),
    StorePanicked,
}
