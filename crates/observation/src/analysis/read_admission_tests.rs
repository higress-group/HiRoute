use crate::{DigestAuthority, LocalObservationStore, query_v2::search_tests};
use hiroute_domain::*;
use std::{sync::mpsc, time::Duration};

fn range() -> AnalysisTimeRangeV1 {
    AnalysisTimeRangeV1 {
        requested_from_ms: 0,
        requested_to_ms: 1000,
        effective_from_ms: 0,
        effective_to_ms: 1000,
        from_utc_offset_seconds: 0,
        to_utc_offset_seconds: 0,
        clipped_for_retention: false,
    }
}

fn indexed(root: &std::path::Path) -> LocalObservationStore {
    let store = LocalObservationStore::open(root, DigestAuthority::new([2; 32])).unwrap();
    search_tests::source(&store, b"original needle question");
    // One bounded worker cycle may leave fixture input pending. Complete its
    // indexing before starting the query deadlines exercised below.
    search_tests::index(&store);
    store
}

fn query() -> ObservationAnalysisQueryV1 {
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    q.query = Some("needle".into());
    q
}

#[test]
fn fresh_keyword_scope_shares_writer_admission_with_capture_inside_the_existing_deadline() {
    let root = tempfile::tempdir().unwrap();
    let store = indexed(root.path());
    std::thread::scope(|threads| {
        let (writer_ready, ready) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let writer_store = &store;
        let writer = threads.spawn(move || {
            let mut db = writer_store.connection.lock();
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            writer_ready.send(()).unwrap();
            released.recv().unwrap();
            tx.commit().unwrap();
        });
        ready.recv().unwrap();
        let (started, start) = mpsc::channel();
        let reading_store = &store;
        let read = threads.spawn(move || {
            let reader = search_tests::reader(true)
                .with_query_deadline(std::time::Instant::now() + Duration::from_millis(500));
            started.send(()).unwrap();
            reading_store.observed_analysis(&reader, &query(), Some(&range()), None, 500)
        });
        start.recv().unwrap();
        // A real capture/maintenance transaction can exceed SQLite's 100 ms
        // busy slice without consuming the existing 500 ms query allowance.
        std::thread::sleep(Duration::from_millis(200));
        release.send(()).unwrap();
        writer.join().unwrap();
        let result = read.join().unwrap().unwrap();
        assert_eq!(result.conversations.len(), 1);
        assert!(!result.gaps.iter().any(|gap| gap == "index_unavailable"));
    });
}

#[test]
fn writer_admission_respects_the_callers_remaining_deadline_without_creating_a_scope() {
    let root = tempfile::tempdir().unwrap();
    let store = indexed(root.path());
    std::thread::scope(|threads| {
        let mut db = store.connection.lock();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let (finished, result) = mpsc::channel();
        let reading_store = &store;
        threads.spawn(move || {
            let reader = search_tests::reader(true)
                .with_query_deadline(std::time::Instant::now() + Duration::from_millis(40));
            finished
                .send(reading_store.observed_analysis(&reader, &query(), Some(&range()), None, 500))
                .unwrap();
        });
        let read = result.recv_timeout(Duration::from_millis(500));
        tx.commit().unwrap();
        drop(db);
        assert!(matches!(
            read.unwrap(),
            Err(crate::query_v2::ObservationV2Error::Busy
                | crate::query_v2::ObservationV2Error::Timeout)
        ));
        assert_eq!(
            store
                .connection
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM observation_analysis_scopes_v1",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    });
}

#[test]
fn an_existing_keyword_scope_stays_read_only_while_capture_owns_the_writer() {
    let root = tempfile::tempdir().unwrap();
    let store = indexed(root.path());
    let mut q = query();
    q.analysis_scope = store
        .observed_analysis(&search_tests::reader(true), &q, Some(&range()), None, 500)
        .unwrap()
        .analysis_scope;
    std::thread::scope(|threads| {
        let mut db = store.connection.lock();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let (finished, result) = mpsc::channel();
        let reading_store = &store;
        let selected = &q;
        threads.spawn(move || {
            let reader = search_tests::reader(true)
                .with_query_deadline(std::time::Instant::now() + Duration::from_millis(200));
            finished
                .send(reading_store.observed_analysis(&reader, selected, None, None, 500))
                .unwrap();
        });
        let read = result.recv_timeout(Duration::from_millis(500));
        tx.commit().unwrap();
        drop(db);
        assert_eq!(read.unwrap().unwrap().conversations.len(), 1);
    });
}
