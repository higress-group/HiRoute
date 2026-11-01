//! Bounded group delivery preserves original per-record ACK/NACK and gap ordering.
use super::*;

const RECORDS: usize = 32;
const BYTES: usize = 256 * 1024;

pub(super) fn drain_ready(
    receiver: &Receiver<ProducerMessage>,
    shared: &ProducerShared,
    expected: u64,
    records: &mut BTreeMap<u64, ObservationRecord>,
    flushes: &mut Vec<(u64, SyncSender<()>)>,
) {
    for message in receiver.try_iter().take(RECORDS - 1) {
        match message {
            ProducerMessage::Record(record) => {
                if record.stamp.sequence < expected {
                    release_bytes(shared, record.accounted_bytes);
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                } else if let Some(replaced) = records.insert(record.stamp.sequence, record) {
                    release_bytes(shared, replaced.accounted_bytes);
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            ProducerMessage::GapHeartbeat => {}
            #[cfg(test)]
            ProducerMessage::Flush {
                through_sequence,
                acknowledge,
            } => flushes.push((through_sequence, acknowledge)),
        }
    }
    #[cfg(not(test))]
    let _ = flushes;
}

pub(super) fn deliver_ready(
    sink: &dyn ObservationRecordSink,
    shared: &ProducerShared,
    pending: &mut BTreeMap<u64, ObservationRecord>,
    expected: &mut u64,
    contiguous_ceiling: Option<u64>,
) {
    let mut batch = Vec::with_capacity(RECORDS);
    let mut bytes = 0usize;
    for offset in 0..RECORDS {
        let sequence = expected.saturating_add(offset as u64);
        if lock(&shared.pending_loss)
            .iter()
            .any(|loss| loss.first_sequence <= sequence && sequence <= loss.last_sequence)
        {
            break;
        }
        let Some(record) = pending.get(&sequence) else {
            break;
        };
        if !batch.is_empty() && bytes.saturating_add(record.accounted_bytes) > BYTES {
            break;
        }
        bytes = bytes.saturating_add(record.accounted_bytes);
        batch.push(pending.remove(&sequence).unwrap());
    }
    if batch.is_empty() {
        return;
    }
    let started: Vec<_> = batch
        .iter()
        .map(|record| record.timing.as_ref().map(|timing| timing.begin_delivery()))
        .collect();
    let result = catch_unwind(AssertUnwindSafe(|| sink.deliver_batch(&batch)));
    let work_us = started.iter().flatten().next().map_or(0, |at| {
        at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    });
    let panicked = result.is_err();
    let results = result.unwrap_or_default();
    let processed = results.len().clamp(1, batch.len()) as u64;
    let mut results = if results.len() > batch.len() {
        Vec::new()
    } else {
        results
    }
    .into_iter();
    let mut stopped = false;
    for (index, (record, started)) in batch.into_iter().zip(started).enumerate() {
        if stopped {
            pending.insert(record.stamp.sequence, record);
            continue;
        }
        let result = results.next();
        let delivered = matches!(&result, Some(Ok(ack)) if acknowledgement_covers_record(&record, ack, contiguous_ceiling));
        release_bytes(shared, record.accounted_bytes);
        if let (Some(timing), Some(_)) = (&record.timing, started) {
            timing.end_delivery_work(
                work_us / processed + u64::from((index as u64) < work_us % processed),
                delivered,
            );
        }
        if delivered {
            shared.delivered.fetch_add(1, Ordering::Relaxed);
            *expected = record.stamp.sequence.saturating_add(1);
        } else {
            let reason = match result {
                Some(Err(nack)) if nack_matches_record(&record, &nack) => {
                    shared.sink_nacks.fetch_add(1, Ordering::Relaxed);
                    ObservationLossReason::SinkNack
                }
                _ if panicked => {
                    shared.sink_panics.fetch_add(1, Ordering::Relaxed);
                    ObservationLossReason::SinkPanicked
                }
                _ => ObservationLossReason::SinkFailed,
            };
            shared.sink_failures.fetch_add(1, Ordering::Relaxed);
            restore_worker_loss(shared, record.stamp.sequence, reason);
            stopped = true;
        }
        // A valid short prefix did not attempt later records. Retain them,
        // without manufacturing a loss or consuming their reserved credit.
        if results.len() == 0 {
            stopped = true;
        }
    }
}
