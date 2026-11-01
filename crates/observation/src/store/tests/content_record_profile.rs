//! Opt-in paired measurement through the unchanged real commit port. No payload
//! is printed; each revision uses the same synthetic repeated-history workload.
use super::*;
use std::time::Instant;

fn cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}

#[test]
#[ignore = "paired storage/CPU measurement over a fresh private synthetic store"]
fn repeated_history_writer_size_and_cpu() {
    const REQUESTS: u32 = 12;
    const PARTS: u32 = 128;
    let fixture = Fixture::new();
    let before = cpu_ticks();
    let started = Instant::now();
    let mut sequence = 0;
    for request in 0..REQUESTS {
        let request_id = LogicalRequestId::parse(format!("request-{request}")).unwrap();
        sequence += 1;
        let mut begin = fixture.begin(sequence);
        begin.correlation.request_id = request_id.clone();
        ack(fixture.ingest(&begin));
        for part in 0..PARTS {
            sequence += 1;
            let body = format!(
                "unchanged-history-part-{part:03}: {}",
                "synthetic content ".repeat(4)
            );
            let mut append = fixture.append(
                sequence,
                part,
                &format!("message-{request}-{}", part / 4),
                &format!("content-{request}-{part}"),
                part / 4,
                part % 4,
                body.as_bytes(),
                body.as_bytes(),
            );
            append.correlation.request_id = request_id.clone();
            ack(fixture.ingest(&append));
        }
        sequence += 1;
        let mut finish = fixture.finish(sequence, &format!("{request:02x}"));
        finish.correlation.request_id = request_id;
        ack(fixture.ingest(&finish));
    }
    let write_wall = started.elapsed().as_micros();
    let write_cpu = before.zip(cpu_ticks()).map(|(a, b)| b - a);
    let db = fixture.store.connection.lock();
    let metadata: u64 = db
        .query_row(
            "SELECT SUM(length(metadata_json)) FROM conversation_content_events_v2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let feedback: u64 = db
        .query_row(
            "SELECT SUM(length(feedback_json)) FROM observation_feedback_v2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let instances: u64 = db
        .query_row("SELECT COUNT(*) FROM content_instances_v2", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(instances, u64::from(REQUESTS * PARTS));
    let blobs: u64 = db
        .query_row("SELECT COUNT(*) FROM content_blobs_v2", [], |r| r.get(0))
        .unwrap();
    assert_eq!(blobs, u64::from(PARTS));
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let database_bytes = std::fs::metadata(&fixture.store.activity_path)
        .unwrap()
        .len();
    println!(
        "{}",
        serde_json::json!({"measurement":"repeated_history_real_writer",
        "requests":REQUESTS,"parts_per_request":PARTS,"events":sequence,
        "metadata_frame_bytes":metadata,"feedback_frame_bytes":feedback,
        "database_bytes_after_checkpoint":database_bytes,
        "writer_wall_us":write_wall,"writer_cpu_ticks":write_cpu})
    );
}
