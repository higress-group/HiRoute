use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use hiroute_gateway_core::runtime::body::{
    BodyDirection, BodyEmitter, BodyError, BodyLengthKnowledge, BodyPlan, BodyPlanExecutor,
    BudgetTree, ChargedBodyQueue, ChargedBytes, FramingLedger, HttpFraming, MemoryRole,
    RawRequestBodyStore, RequestLeaseBook,
};
use http::HeaderMap;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use static_assertions::assert_not_impl_any;

assert_not_impl_any!(hiroute_gateway_core::runtime::body::RequestBodyLease: Clone);
assert_not_impl_any!(hiroute_gateway_core::runtime::body::IrReleaseReady: Clone);
assert_not_impl_any!(BodyPlanExecutor: Clone);

#[test]
fn directional_plan_executor_preflights_and_enforces_cumulative_eos() {
    let mut executor = BodyPlanExecutor::new(
        BodyDirection::AttemptRequest,
        BodyPlan::StreamingReplay {
            max_chunk_bytes: 4,
            max_replay_bytes: 6,
        },
        32,
    )
    .unwrap();
    assert_eq!(
        executor.preflight_content_length(7).unwrap_err(),
        BodyError::BodyLimitExceeded
    );
    executor.preflight_content_length(6).unwrap();
    executor.admit_chunk(4).unwrap();
    executor.admit_chunk(2).unwrap();
    assert_eq!(executor.observed_bytes(), 6);
    assert_eq!(executor.finish().unwrap(), 6);
    assert_eq!(
        executor.admit_chunk(0).unwrap_err(),
        BodyError::BodyAfterEos
    );
    assert_eq!(executor.finish().unwrap_err(), BodyError::DuplicateBodyEos);
}

#[test]
fn hierarchy_reserves_atomically_and_tracks_role_peaks() {
    let tree = BudgetTree::new(100, 80).unwrap();
    let stream = tree.stream(60).unwrap();
    let first = stream.reserve(MemoryRole::RawRequest, 40).unwrap();
    assert_eq!(
        stream.reserve(MemoryRole::AttemptWire, 30).unwrap_err(),
        BodyError::BudgetExceeded
    );
    let snapshot = stream.snapshot().unwrap();
    assert_eq!(snapshot.live, 40);
    assert_eq!(snapshot.peak, 40);
    assert_eq!(snapshot.rejected, 1);
    assert_eq!(snapshot.role_live[MemoryRole::RawRequest as usize], 40);
    drop(first);
    assert_eq!(tree.snapshot().process_live, 0);
    drop(stream);
    assert_eq!(tree.snapshot().active_streams, 0);
}

#[test]
fn body_backing_is_reserved_before_copy_and_queue_release_drops_capacity() {
    let tiny_tree = BudgetTree::new(8, 8).unwrap();
    let tiny = tiny_tree.stream(1).unwrap();
    let plan = BodyPlan::PassThrough { max_chunk_bytes: 8 };
    assert_eq!(
        hiroute_gateway_core::runtime::body::ChargedBodyQueue::new(
            &tiny,
            MemoryRole::RawRequest,
            &plan,
            8,
            2,
        )
        .unwrap_err(),
        BodyError::BudgetExceeded
    );
    assert_eq!(
        ChargedBytes::copy_from_opaque(&tiny, MemoryRole::RawRequest, b"ab").unwrap_err(),
        BodyError::BudgetExceeded
    );
    let rejected = tiny.snapshot().unwrap();
    assert_eq!(rejected.live, 0);
    assert_eq!(rejected.peak, 0);
    assert_eq!(rejected.rejected, 2);

    let tree = BudgetTree::new(4096, 4096).unwrap();
    let stream = tree.stream(4096).unwrap();
    let mut queue = hiroute_gateway_core::runtime::body::ChargedBodyQueue::new(
        &stream,
        MemoryRole::RawRequest,
        &plan,
        8,
        2,
    )
    .unwrap();
    queue
        .push_back(ChargedBytes::copy_from_opaque(&stream, MemoryRole::RawRequest, b"ab").unwrap())
        .unwrap();
    assert!(queue.allocated_chunk_capacity() >= 2);
    assert!(stream.snapshot().unwrap().live > 0);
    queue.clear_and_release();
    assert_eq!(queue.allocated_chunk_capacity(), 0);
    assert_eq!(stream.snapshot().unwrap().live, 0);
}

#[test]
fn max_capacity_empty_queue_units_keep_metadata_live_and_peak_exact() {
    const CAPACITY: usize = 257;
    let metadata_bytes = CAPACITY * std::mem::size_of::<ChargedBytes>();
    let tree = BudgetTree::new(metadata_bytes + 1024, metadata_bytes + 1024).unwrap();
    let stream = tree.stream(metadata_bytes + 1024).unwrap();
    let plan = BodyPlan::PassThrough { max_chunk_bytes: 1 };
    let mut queue =
        ChargedBodyQueue::new(&stream, MemoryRole::AttemptWire, &plan, 1, CAPACITY).unwrap();

    assert_eq!(queue.allocated_chunk_capacity(), CAPACITY);
    for _ in 0..CAPACITY {
        queue
            .push_back(
                ChargedBytes::copy_from_opaque(&stream, MemoryRole::AttemptWire, &[]).unwrap(),
            )
            .unwrap();
    }
    assert_eq!(queue.len(), CAPACITY);
    assert_eq!(queue.high_water_chunks(), CAPACITY);
    assert_eq!(queue.visible_bytes(), 0);
    let full = stream.snapshot().unwrap();
    assert_eq!(full.live, metadata_bytes);
    assert_eq!(
        full.role_live[MemoryRole::AttemptWire as usize],
        metadata_bytes
    );
    assert_eq!(
        full.role_peak[MemoryRole::AttemptWire as usize],
        metadata_bytes
    );

    assert_eq!(
        queue
            .push_back(
                ChargedBytes::copy_from_opaque(&stream, MemoryRole::AttemptWire, &[]).unwrap(),
            )
            .unwrap_err(),
        BodyError::BodyLimitExceeded
    );
    assert_eq!(stream.snapshot().unwrap().live, metadata_bytes);
    queue.clear_and_release();
    assert_eq!(queue.allocated_chunk_capacity(), 0);
    let released = stream.snapshot().unwrap();
    assert_eq!(released.live, 0);
    assert_eq!(released.role_live[MemoryRole::AttemptWire as usize], 0);
    assert_eq!(
        released.role_peak[MemoryRole::AttemptWire as usize],
        metadata_bytes
    );
}

#[test]
fn opaque_small_slice_is_copied_and_charged_by_exact_backing() {
    let tree = BudgetTree::new(1 << 20, 1 << 20).unwrap();
    let stream = tree.stream(1 << 20).unwrap();
    let large = Bytes::from(vec![7_u8; 128 * 1024]);
    let tiny = large.slice(0..8);
    let charged =
        ChargedBytes::copy_from_opaque(&stream, MemoryRole::TransportInflight, &tiny).unwrap();
    drop(large);
    drop(tiny);
    assert_eq!(charged.retained_capacity(), 8);
    assert_eq!(stream.snapshot().unwrap().live, 8);
    drop(charged);
    assert_eq!(stream.snapshot().unwrap().live, 0);
}

#[test]
fn eight_mib_raw_to_ir_is_a_consuming_role_transfer() {
    const SIZE: usize = 8 * 1024 * 1024;
    let tree = BudgetTree::new(SIZE + 1024, SIZE + 1024).unwrap();
    let stream = tree.stream(SIZE + 1024).unwrap();
    let mut raw = RawRequestBodyStore::new(BodyPlan::BufferedTransform {
        max_body_bytes: SIZE,
    })
    .unwrap();
    for _ in 0..8 {
        raw.push(
            ChargedBytes::from_exact_vec(&stream, MemoryRole::RawRequest, vec![1_u8; 1024 * 1024])
                .unwrap(),
        )
        .unwrap();
    }
    assert_eq!(stream.snapshot().unwrap().live, SIZE);
    let model = raw.into_model_backing().unwrap();
    let snapshot = stream.snapshot().unwrap();
    assert_eq!(
        snapshot.live, SIZE,
        "ownership transfer must not duplicate the body"
    );
    assert_eq!(snapshot.role_live[MemoryRole::RawRequest as usize], 0);
    assert_eq!(
        snapshot.role_live[MemoryRole::ModelIrBacking as usize],
        SIZE
    );
    assert_eq!(model.visible_bytes(), SIZE);
    drop(model);
    assert_eq!(stream.snapshot().unwrap().live, 0);
}

#[test]
fn runtime_body_cap_rejects_before_retaining_extra_chunk() {
    let tree = BudgetTree::new(64, 64).unwrap();
    let stream = tree.stream(64).unwrap();
    let mut raw = RawRequestBodyStore::new(BodyPlan::StreamingReplay {
        max_chunk_bytes: 16,
        max_replay_bytes: 8,
    })
    .unwrap();
    raw.push(ChargedBytes::from_exact_vec(&stream, MemoryRole::RawRequest, vec![0; 8]).unwrap())
        .unwrap();
    let extra = ChargedBytes::from_exact_vec(&stream, MemoryRole::RawRequest, vec![0; 1]).unwrap();
    assert_eq!(raw.push(extra).unwrap_err(), BodyError::BodyLimitExceeded);
    assert_eq!(stream.snapshot().unwrap().live, 8);
}

#[test]
fn lease_zero_and_terminal_ir_release_proofs_are_distinct_and_one_shot() {
    let book = RequestLeaseBook::new();
    let lease = book.acquire().unwrap();
    assert_eq!(
        book.lease_zero_for_continue().unwrap_err(),
        BodyError::OutstandingRequestLease(1)
    );
    drop(lease);
    let _continue_proof = book.lease_zero_for_continue().unwrap();
    let proof = book.mark_terminal_and_release_ready().unwrap();

    struct FakeModelIr {
        drops: Arc<AtomicUsize>,
    }
    impl Drop for FakeModelIr {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }
    fn consume_release_proof(
        _proof: hiroute_gateway_core::runtime::body::IrReleaseReady,
        ir: FakeModelIr,
    ) {
        drop(ir);
    }

    let drops = Arc::new(AtomicUsize::new(0));
    consume_release_proof(
        proof,
        FakeModelIr {
            drops: Arc::clone(&drops),
        },
    );
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert_eq!(
        book.mark_terminal_and_release_ready().unwrap_err(),
        BodyError::ReleaseProofAlreadyIssued
    );
}

#[test]
fn framing_ledger_reconciles_h1_h2_and_rejects_conflicts() {
    let mut h1_headers = HeaderMap::new();
    h1_headers.insert(CONTENT_LENGTH, "4".parse().unwrap());
    let mut ledger = FramingLedger::default();
    ledger.streaming_transform().unwrap();
    ledger
        .finalize(&mut h1_headers, HttpFraming::Http1, None, None)
        .unwrap();
    assert!(!h1_headers.contains_key(CONTENT_LENGTH));
    assert_eq!(h1_headers[TRANSFER_ENCODING], "chunked");
    assert_eq!(
        ledger.streaming_transform().unwrap_err(),
        BodyError::FramingAlreadyCommitted
    );

    let tree = BudgetTree::new(32, 32).unwrap();
    let stream = tree.stream(32).unwrap();
    let mut emitter = BodyEmitter::new();
    emitter
        .emit_transformed(
            ChargedBytes::from_exact_vec(&stream, MemoryRole::OutputQueue, b"longer".to_vec())
                .unwrap(),
        )
        .unwrap();
    assert_eq!(emitter.ledger().knowledge(), BodyLengthKnowledge::Unknown);
    emitter.complete_buffered().unwrap();
    assert_eq!(emitter.ledger().knowledge(), BodyLengthKnowledge::Exact(6));

    let mut conflicting = HeaderMap::new();
    conflicting.insert(CONTENT_LENGTH, "1".parse().unwrap());
    conflicting.insert(TRANSFER_ENCODING, "chunked".parse().unwrap());
    assert_eq!(
        FramingLedger::default()
            .finalize(&mut conflicting, HttpFraming::Http2, None, None)
            .unwrap_err(),
        BodyError::ContentLengthTransferEncodingConflict
    );
}

#[test]
fn framing_header_mutation_is_not_trusted_as_body_length_fact() {
    use hiroute_gateway_core::core::filter::FramingLedgerPort;

    let mut mutated = HeaderMap::new();
    mutated.insert(CONTENT_LENGTH, "1".parse().unwrap());
    let mut ledger = FramingLedger::default();
    ledger.header_mutated(&CONTENT_LENGTH);
    assert!(ledger.framing_headers_mutated());
    assert_eq!(ledger.knowledge(), BodyLengthKnowledge::Unknown);
    ledger
        .finalize(&mut mutated, HttpFraming::Http1, None, None)
        .unwrap();
    assert!(!mutated.contains_key(CONTENT_LENGTH));
    assert_eq!(mutated[TRANSFER_ENCODING], "chunked");

    let mut exact = HeaderMap::new();
    exact.insert(CONTENT_LENGTH, "1".parse().unwrap());
    exact.insert(TRANSFER_ENCODING, "chunked".parse().unwrap());
    let mut ledger = FramingLedger::default();
    ledger.header_mutated(&CONTENT_LENGTH);
    ledger.header_mutated(&TRANSFER_ENCODING);
    ledger.buffered_eos(4).unwrap();
    ledger
        .finalize(&mut exact, HttpFraming::Http1, None, None)
        .unwrap();
    assert_eq!(exact[CONTENT_LENGTH], "4");
    assert!(!exact.contains_key(TRANSFER_ENCODING));
}

#[test]
fn spooled_prefix_preserves_original_frame_total_and_framing_bounds() {
    let make = || {
        BodyPlanExecutor::new(
            BodyDirection::LogicalRequest,
            BodyPlan::StreamingReplay {
                max_chunk_bytes: 4,
                max_replay_bytes: 12,
            },
            12,
        )
        .unwrap()
    };
    let mut owner = make();
    owner.preflight_content_length(12).unwrap();
    owner.admit_prefix(9, 4).unwrap();
    owner.admit_chunk(3).unwrap();
    assert_eq!(owner.finish().unwrap(), 12);
    assert_eq!(owner.admit_prefix(0, 0), Err(BodyError::BodyAfterEos));
    let mut owner = make();
    assert_eq!(owner.admit_prefix(9, 5), Err(BodyError::BodyLimitExceeded));
    assert_eq!(owner.observed_bytes(), 0);
    owner.admit_prefix(9, 4).unwrap();
    assert_eq!(owner.admit_chunk(4), Err(BodyError::BodyLimitExceeded));
    let mut owner = make();
    owner.preflight_content_length(10).unwrap();
    owner.admit_prefix(9, 4).unwrap();
    assert_eq!(owner.finish(), Err(BodyError::ContentLengthMismatch));
}
