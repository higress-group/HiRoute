use super::*;

#[test]
#[ignore = "controlled real-store persistence pressure; explicitly selected without model calls"]
fn thousands_of_small_items_remain_durable_and_replay_exactly() {
    let fixture = Fixture::new();
    crate::store::ingest_profile::begin();
    ack(fixture.ingest(&fixture.begin(1)));
    const ITEMS: u32 = 4096;
    for index in 0..ITEMS {
        let text = format!("captured field {index}");
        ack(fixture.ingest(&fixture.append(
            u64::from(index) + 2,
            index,
            "message-pressure",
            &format!("content-{index}"),
            0,
            index,
            text.as_bytes(),
            text.as_bytes(),
        )));
    }
    let finish = fixture.finish(u64::from(ITEMS) + 2, "33");
    let acknowledgement = ack(fixture.ingest(&finish));
    let profile = crate::store::ingest_profile::finish();
    println!("real content persistence stages (count, inclusive_us): {profile:?}");
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
