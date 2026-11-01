use super::*;

#[test]
#[ignore = "controlled real-store persistence pressure; explicitly selected without model calls"]
fn thousands_of_small_items_remain_durable_and_replay_exactly() {
    pressure(1);
}

#[test]
#[ignore = "controlled real-store persistence pressure; explicitly selected without model calls"]
fn bounded_batches_keep_the_same_small_item_bytes_and_feedback() {
    pressure(crate::writer::MAX_CONTENT_BATCH_RECORDS);
}

fn pressure(batch_size: usize) {
    let fixture = Fixture::new();
    crate::store::ingest_profile::begin();
    ack(fixture.ingest(&fixture.begin(1)));
    const ITEMS: u32 = 4096;
    let mut envelopes = Vec::new();
    for index in 0..ITEMS {
        let text = format!("captured field {index}");
        envelopes.push(fixture.append(
            u64::from(index) + 2,
            index,
            "message-pressure",
            &format!("content-{index}"),
            0,
            index,
            text.as_bytes(),
            text.as_bytes(),
        ));
    }
    for batch in envelopes.chunks(batch_size) {
        let references: Vec<_> = batch.iter().collect();
        let outcomes = fixture.store.ingest_content_batch(&references).unwrap();
        assert_eq!(outcomes.len(), batch.len());
        for (envelope, outcome) in batch.iter().zip(outcomes) {
            let acknowledgement = ack(outcome);
            assert_eq!(
                acknowledgement.highest_contiguous_sequence,
                envelope.sequence
            );
            assert_eq!(
                acknowledgement
                    .content_acknowledgement
                    .as_ref()
                    .unwrap()
                    .next_chunk_ordinal,
                envelope.chunk_ordinal.unwrap() + 1
            );
        }
    }
    let finish = fixture.finish(u64::from(ITEMS) + 2, "33");
    let acknowledgement = ack(fixture.ingest(&finish));
    let profile = crate::store::ingest_profile::finish();
    println!(
        "real content persistence batch={batch_size} stages (count, inclusive_us): {profile:?}"
    );
    assert_eq!(
        acknowledgement.highest_contiguous_sequence,
        u64::from(ITEMS) + 2
    );
    let content = acknowledgement.content_acknowledgement.as_ref().unwrap();
    assert_eq!(content.next_chunk_ordinal, ITEMS);
    assert_eq!(content.acknowledged_blobs.len(), ITEMS as usize);
    drop(fixture.store);
    let reopened = LocalObservationStore::open(fixture._root.path(), fixture.authority).unwrap();
    // Replay checks every completed blob's exact bytes and digest, not only a row count.
    assert_eq!(
        acknowledgement,
        ack(reopened.ingest_content(&finish, &[]).unwrap())
    );
}
