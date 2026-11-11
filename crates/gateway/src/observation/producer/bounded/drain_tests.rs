use super::super::super::feedback::{accounted_acknowledgement, receiver_unavailable_nack};
use super::*;
use std::sync::mpsc::{Sender, channel};

struct HeldAck {
    entered: Sender<(u64, bool)>,
    release: Mutex<Receiver<()>>,
    reject_record: bool,
}

impl ObservationRecordSink for HeldAck {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        if self.reject_record && !record.is_gap_heartbeat() {
            return Err(receiver_unavailable_nack(record, true));
        }
        self.entered
            .send((record.stamp().sequence, record.is_gap_heartbeat()))
            .unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(accounted_acknowledgement(record))
    }
}

fn identity() -> ObservationProducerIdentity {
    ObservationProducerIdentity {
        channel: Arc::from("execution_fact"),
        component: Arc::from("shutdown-test"),
        revision: Arc::from("1"),
        producer_id: Arc::from("producer-shutdown"),
        producer_epoch: Arc::from("epoch-shutdown"),
        stream_id: Arc::from("stream-shutdown"),
    }
}

#[test]
fn drain_waits_for_sink_ack_even_after_the_event_queue_is_empty() {
    let (entered, observing) = channel();
    let (release, released) = channel();
    let producer = ByteBoundedObservationProducer::new(
        identity(),
        4096,
        1,
        Arc::new(HeldAck {
            entered,
            release: Mutex::new(released),
            reject_record: false,
        }),
    )
    .unwrap();
    producer.try_publish(|_| b"request-finished".to_vec());
    assert_eq!(
        observing.recv_timeout(Duration::from_secs(10)).unwrap(),
        (1, false)
    );
    // The worker owns the only record, but the sink has not acknowledged it.
    assert_eq!(
        producer.drain(Duration::ZERO),
        Err(ObservationError::Unavailable)
    );
    release.send(()).unwrap();
    producer.drain(Duration::from_secs(10)).unwrap();
    assert_eq!(producer.stats().delivered, 1);
    assert_eq!(producer.drain(Duration::ZERO), Ok(()));
}

#[test]
fn shutdown_barrier_needs_no_slot_in_a_full_event_queue() {
    let (entered, observing) = channel();
    let (release, released) = channel();
    let producer = ByteBoundedObservationProducer::new(
        identity(),
        4096,
        1,
        Arc::new(HeldAck {
            entered,
            release: Mutex::new(released),
            reject_record: false,
        }),
    )
    .unwrap();
    producer.try_publish(|_| b"credential-denied".to_vec());
    assert_eq!(
        observing.recv_timeout(Duration::from_secs(10)).unwrap(),
        (1, false)
    );
    producer.try_publish(|_| b"request-finished".to_vec());
    assert_eq!(producer.stats().enqueued, 2);
    let (finished, completion) = channel();
    let draining = producer.clone();
    let thread = std::thread::spawn(move || {
        finished
            .send(draining.drain(Duration::from_secs(10)))
            .unwrap()
    });
    assert!(completion.recv_timeout(Duration::from_millis(50)).is_err());
    release.send(()).unwrap();
    assert_eq!(
        observing.recv_timeout(Duration::from_secs(10)).unwrap(),
        (2, false)
    );
    assert!(completion.recv_timeout(Duration::from_millis(50)).is_err());
    release.send(()).unwrap();
    assert_eq!(
        completion.recv_timeout(Duration::from_secs(10)).unwrap(),
        Ok(())
    );
    thread.join().unwrap();
    assert_eq!(producer.stats().delivered, 2);
    assert_eq!(producer.stats().dropped, 0);
}

#[test]
fn nack_does_not_advance_shutdown_until_the_loss_is_acknowledged() {
    let (entered, observing) = channel();
    let (release, released) = channel();
    let producer = ByteBoundedObservationProducer::new(
        identity(),
        4096,
        1,
        Arc::new(HeldAck {
            entered,
            release: Mutex::new(released),
            reject_record: true,
        }),
    )
    .unwrap();
    producer.try_publish(|_| b"request-finished".to_vec());
    assert_eq!(
        observing.recv_timeout(Duration::from_secs(10)).unwrap(),
        (1, true)
    );
    assert_eq!(
        producer.drain(Duration::ZERO),
        Err(ObservationError::Unavailable)
    );
    release.send(()).unwrap();
    producer.drain(Duration::from_secs(10)).unwrap();
    assert_eq!(producer.stats().delivered, 0);
    assert_eq!(producer.stats().sink_nacks, 1);
    assert_eq!(producer.stats().gap_heartbeats_delivered, 1);
}
