use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use hiroute_gateway_core::core::execution_plan::{
    AtomicityGroupId, CompiledAcceptedResponsePlan, ConfigBindingPolicy, ConfigBundle,
    ConfigCellDescriptor, ConfigCellHandle, ConfigCellId, ConfigGeneration, ImmutableConfig,
};
use hiroute_gateway_core::runtime::body::{
    BodyPlan, BudgetTree, ChargedBytes, MemoryRole, StreamBudget,
};
use hiroute_gateway_core::runtime::sse::{
    AcceptedEvent, BoundedEventEmitter, BoundedOutputSink, BudgetedResponseMailbox,
    EncodedOutputUnit, EofPolicy, OwnedSseEvent, PrecommitResponseState, SemanticProvenance,
    SemanticReplacementCapability, SseData, SseError, SseEventView, SseFeedOutcome, SseFramer,
    SseLimits, SseVisitor,
};
use proptest::prelude::*;
use static_assertions::assert_not_impl_any;

#[path = "sse_streaming/eof.rs"]
mod eof;

assert_not_impl_any!(OwnedSseEvent: Clone);
assert_not_impl_any!(SseEventView<'static>: Clone, Copy, Send, Sync);

fn limits(eof_policy: EofPolicy) -> SseLimits {
    SseLimits {
        max_event_bytes: 4096,
        max_pending_bytes: 4096,
        max_output_event_bytes: 8192,
        expansion_ratio_numerator: 2,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 64,
        retained_capacity_threshold: 256,
        eof_policy,
    }
}

fn budget() -> StreamBudget {
    BudgetTree::new(1 << 20, 1 << 20)
        .unwrap()
        .stream(1 << 20)
        .unwrap()
}

#[derive(Default)]
struct CollectSink {
    output: Vec<Vec<u8>>,
}

impl BoundedOutputSink for CollectSink {
    fn emit_borrowed(&mut self, bytes: &[u8]) -> Result<(), SseError> {
        self.output.push(bytes.to_vec());
        Ok(())
    }

    fn emit_owned(&mut self, bytes: ChargedBytes) -> Result<(), SseError> {
        self.output.push(bytes.bytes().to_vec());
        Ok(())
    }
}

struct ParsingVisitor {
    budget: StreamBudget,
    data: Vec<Vec<u8>>,
    event_types: Vec<Option<Vec<u8>>>,
    ids: Vec<Option<Vec<u8>>>,
    retries: Vec<Option<u64>>,
}

struct OversizedBuilderVisitor;

impl SseVisitor for OversizedBuilderVisitor {
    fn on_event(
        &mut self,
        _event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        let _ = emitter.output_builder(12)?;
        unreachable!("the stream budget must reject before output allocation")
    }
}

impl Default for ParsingVisitor {
    fn default() -> Self {
        Self {
            budget: budget(),
            data: Vec::new(),
            event_types: Vec::new(),
            ids: Vec::new(),
            retries: Vec::new(),
        }
    }
}

impl SseVisitor for ParsingVisitor {
    fn on_event(
        &mut self,
        event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        self.data.push(event.data(&self.budget)?.as_ref().to_vec());
        self.event_types
            .push(event.event_type().map(<[u8]>::to_vec));
        self.ids.push(event.id().map(<[u8]>::to_vec));
        self.retries.push(event.retry());
        emitter.pass_raw()
    }
}

fn run_fragments(
    input: &[u8],
    cuts: &[usize],
) -> (
    ParsingVisitor,
    CollectSink,
    hiroute_gateway_core::runtime::sse::SseComplexity,
) {
    let budget = budget();
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    let mut visitor = ParsingVisitor::default();
    let mut sink = CollectSink::default();
    let mut start = 0;
    for &end in cuts {
        let charged = ChargedBytes::copy_from_opaque(
            &budget,
            MemoryRole::TransportInflight,
            &input[start..end],
        )
        .unwrap();
        framer
            .feed(charged, end == input.len(), &mut visitor, &mut sink)
            .unwrap();
        start = end;
    }
    (visitor, sink, framer.complexity())
}

#[test]
fn transform_builder_reserves_before_allocating_output() {
    let tree = BudgetTree::new(16, 16).unwrap();
    let budget = tree.stream(16).unwrap();
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    let input =
        ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"data:x\n\n")
            .unwrap();
    let mut visitor = OversizedBuilderVisitor;
    let mut sink = CollectSink::default();

    assert_eq!(
        framer
            .feed(input, true, &mut visitor, &mut sink)
            .unwrap_err(),
        SseError::BudgetExceeded,
    );
    let snapshot = budget.snapshot().unwrap();
    assert_eq!(snapshot.peak, 8, "rejected output never became live");
    assert_eq!(snapshot.live, 0);
    assert_eq!(snapshot.rejected, 1);
}

struct OneSlotProductionVisitor {
    budget: StreamBudget,
    handoff: PrecommitResponseState<ChargedBytes>,
    mailbox: BudgetedResponseMailbox<u64>,
}

impl OneSlotProductionVisitor {
    fn new(budget: &StreamBudget) -> Self {
        Self {
            budget: budget.clone(),
            handoff: PrecommitResponseState::new(),
            mailbox: BudgetedResponseMailbox::new(1).unwrap(),
        }
    }

    fn classify_one(&mut self) {
        let sequence = self.mailbox.pop().expect("one-slot marker");
        let (raw, _) = self.handoff.take_raw_for_classification(sequence).unwrap();
        self.handoff.mark_decoded(sequence, raw.into_raw()).unwrap();
    }

    fn decoded(self) -> Vec<Vec<u8>> {
        self.handoff
            .publish_accept()
            .unwrap()
            .map(|event| match event {
                AcceptedEvent::Decoded { decoded, .. } => decoded.bytes().to_vec(),
                AcceptedEvent::NeedsDecode { .. } => panic!("all markers were classified"),
            })
            .collect()
    }
}

impl SseVisitor for OneSlotProductionVisitor {
    fn on_event(
        &mut self,
        event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        if self.mailbox.is_full() {
            return Err(SseError::NeedDrain);
        }
        let sequence = self.handoff.push_raw(event.promote(&self.budget)?)?;
        self.mailbox
            .try_push(sequence)
            .map_err(|_| SseError::NeedDrain)?;
        emitter.drop_event()
    }
}

fn run_one_slot_production_fragments(input: &[u8], cuts: &[usize]) -> Vec<Vec<u8>> {
    let budget = budget();
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    let mut visitor = OneSlotProductionVisitor::new(&budget);
    let mut sink = CollectSink::default();
    let mut start = 0;
    for &end in cuts {
        while !visitor.mailbox.is_empty() {
            visitor.classify_one();
        }
        let mut outcome = framer
            .feed(
                ChargedBytes::copy_from_opaque(
                    &budget,
                    MemoryRole::TransportInflight,
                    &input[start..end],
                )
                .unwrap(),
                end == input.len(),
                &mut visitor,
                &mut sink,
            )
            .unwrap();
        while matches!(outcome, SseFeedOutcome::NeedDrain { .. }) {
            visitor.classify_one();
            outcome = framer.resume(&mut visitor, &mut sink).unwrap();
        }
        start = end;
    }
    while !visitor.mailbox.is_empty() {
        visitor.classify_one();
    }
    drop(framer);
    let decoded = visitor.decoded();
    assert_eq!(budget.snapshot().unwrap().live, 0);
    decoded
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn randomized_multi_fragment_stream_matches_whole_buffer(
        widths in prop::collection::vec(1_usize..64, 1..128)
    ) {
        let input = b"event: first\ndata: alpha\ndata: beta\n\nid: 7\rdata: gamma\r\r:data-comment\r\ndata: delta\r\n\r\n";
        let mut cuts = Vec::new();
        let mut cursor = 0;
        for width in widths {
            if cursor == input.len() {
                break;
            }
            cursor = cursor.saturating_add(width).min(input.len());
            cuts.push(cursor);
        }
        if cuts.last().copied() != Some(input.len()) {
            cuts.push(input.len());
        }
        let (expected_visitor, expected_sink, _) = run_fragments(input, &[input.len()]);
        let (actual_visitor, actual_sink, complexity) = run_fragments(input, &cuts);
        prop_assert_eq!(actual_visitor.data, expected_visitor.data);
        prop_assert_eq!(actual_visitor.event_types, expected_visitor.event_types);
        prop_assert_eq!(actual_visitor.ids, expected_visitor.ids);
        prop_assert_eq!(actual_sink.output, expected_sink.output);
        prop_assert!(complexity.scanned_bytes <= input.len());
        prop_assert!(complexity.copied_bytes <= input.len() * 3);
    }

    #[test]
    fn production_one_slot_handoff_is_fragmentation_independent(
        widths in prop::collection::vec(1_usize..32, 1..64)
    ) {
        let input = b": heartbeat\n\ndata: alpha\n\ndata: beta\n\ndata: gamma\n\n";
        let mut cuts = Vec::new();
        let mut cursor = 0;
        for width in widths {
            if cursor == input.len() {
                break;
            }
            cursor = cursor.saturating_add(width).min(input.len());
            cuts.push(cursor);
        }
        if cuts.last().copied() != Some(input.len()) {
            cuts.push(input.len());
        }
        let expected = run_one_slot_production_fragments(input, &[input.len()]);
        let actual = run_one_slot_production_fragments(input, &cuts);
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn arbitrary_byte_fuzz_releases_all_charged_state(
        input in prop::collection::vec(any::<u8>(), 0..8192),
        widths in prop::collection::vec(1_usize..128, 0..128),
    ) {
        struct Pass;
        impl SseVisitor for Pass {
            fn on_event(
                &mut self,
                _event: SseEventView<'_>,
                emitter: &mut BoundedEventEmitter<'_, '_>,
            ) -> Result<(), SseError> {
                emitter.pass_raw()
            }
        }

        let tree = BudgetTree::new(1 << 20, 1 << 20).unwrap();
        let budget = tree.stream(1 << 20).unwrap();
        let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
        let mut visitor = Pass;
        let mut sink = CollectSink::default();
        let mut cursor = 0;
        for width in widths {
            if cursor == input.len() {
                break;
            }
            let end = cursor.saturating_add(width).min(input.len());
            let result = framer.feed(
                ChargedBytes::copy_from_opaque(
                    &budget,
                    MemoryRole::TransportInflight,
                    &input[cursor..end],
                ).unwrap(),
                end == input.len(),
                &mut visitor,
                &mut sink,
            );
            cursor = end;
            if result.is_err() {
                break;
            }
        }
        if cursor < input.len() {
            let _ = framer.feed(
                ChargedBytes::copy_from_opaque(
                    &budget,
                    MemoryRole::TransportInflight,
                    &input[cursor..],
                ).unwrap(),
                true,
                &mut visitor,
                &mut sink,
            );
        } else if input.is_empty() {
            let _ = framer.feed(
                ChargedBytes::copy_from_opaque(
                    &budget,
                    MemoryRole::TransportInflight,
                    &[],
                ).unwrap(),
                true,
                &mut visitor,
                &mut sink,
            );
        }
        drop(framer);
        prop_assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}

#[test]
fn every_single_split_matches_whole_buffer_with_mixed_newlines() {
    let input = b"\xef\xbb\xbfevent: message\rdata: first\r\ndata:second\nid: good\rretry: 42\r\n\r\n:data-comment\ndata: tail\n\n";
    let (whole_visitor, whole_sink, whole_stats) = run_fragments(input, &[input.len()]);
    assert_eq!(whole_visitor.data[0], b"first\nsecond");
    assert_eq!(
        whole_visitor.event_types[0].as_deref(),
        Some(&b"message"[..])
    );
    assert_eq!(whole_visitor.ids[0].as_deref(), Some(&b"good"[..]));
    assert_eq!(whole_visitor.retries[0], Some(42));
    assert_eq!(
        whole_stats.copied_bytes, 0,
        "same-chunk events are borrowed"
    );

    for split in 1..input.len() {
        let (visitor, sink, stats) = run_fragments(input, &[split, input.len()]);
        assert_eq!(visitor.data, whole_visitor.data, "split at {split}");
        assert_eq!(sink.output, whole_sink.output, "split at {split}");
        assert!(stats.scanned_bytes <= input.len());
        assert!(stats.copied_bytes <= input.len() * 3);
    }
}

#[test]
fn byte_at_a_time_is_linear_and_retry_overflow_id_nul_are_ignored() {
    let input = b"data: x\nid: bad\0id\nretry: 18446744073709551616\nunknown: value\n\n";
    let cuts: Vec<usize> = (1..=input.len()).collect();
    let (visitor, sink, stats) = run_fragments(input, &cuts);
    assert_eq!(visitor.data, [b"x".to_vec()]);
    assert_eq!(visitor.ids, [None]);
    assert_eq!(visitor.retries, [None]);
    assert_eq!(sink.output.concat(), input);
    assert!(stats.scanned_bytes <= input.len());
    assert!(stats.copied_bytes <= input.len() * 3);
    assert!(stats.allocations <= input.len().ilog2() as usize + 2);
}

#[test]
fn incomplete_eof_policy_and_limits_fail_closed_deterministically() {
    let budget = budget();
    let incomplete =
        ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"data: incomplete")
            .unwrap();
    let mut strict = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    assert_eq!(
        strict
            .feed(
                incomplete,
                true,
                &mut ParsingVisitor::default(),
                &mut CollectSink::default()
            )
            .unwrap_err(),
        SseError::IncompleteEventAtEof
    );

    let mut final_event =
        SseFramer::new(limits(EofPolicy::ProviderFinalEvent), budget.clone()).unwrap();
    let mut visitor = ParsingVisitor::default();
    final_event
        .feed(
            ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"data: final")
                .unwrap(),
            true,
            &mut visitor,
            &mut CollectSink::default(),
        )
        .unwrap();
    assert_eq!(visitor.data, [b"final".to_vec()]);

    let mut tiny_limits = limits(EofPolicy::Strict);
    tiny_limits.max_event_bytes = 8;
    tiny_limits.max_pending_bytes = 8;
    let mut tiny = SseFramer::new(tiny_limits, budget.clone()).unwrap();
    assert!(matches!(
        tiny.feed(
            ChargedBytes::copy_from_opaque(
                &budget,
                MemoryRole::TransportInflight,
                b"data: way too large\n\n"
            )
            .unwrap(),
            false,
            &mut ParsingVisitor::default(),
            &mut CollectSink::default()
        ),
        Err(SseError::EventLimit | SseError::PendingLimit)
    ));
}

#[test]
fn fragmented_tail_limits_each_completed_event_not_the_transport_chunk() {
    let budget = budget();
    let mut small = limits(EofPolicy::Strict);
    small.max_event_bytes = 16;
    small.max_pending_bytes = 16;
    let mut framer = SseFramer::new(small, budget.clone()).unwrap();
    let mut visitor = ParsingVisitor::default();
    let mut sink = CollectSink::default();

    framer
        .feed(
            ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"data:a")
                .unwrap(),
            false,
            &mut visitor,
            &mut sink,
        )
        .unwrap();
    framer
        .feed(
            ChargedBytes::copy_from_opaque(
                &budget,
                MemoryRole::TransportInflight,
                b"\n\ndata:b\n\ndata:c\n\n",
            )
            .unwrap(),
            true,
            &mut visitor,
            &mut sink,
        )
        .unwrap();

    assert_eq!(visitor.data, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    assert_eq!(sink.output.concat(), b"data:a\n\ndata:b\n\ndata:c\n\n");
    assert_eq!(framer.pending_bytes(), 0);
}

#[test]
fn need_drain_retains_only_charged_unconsumed_tail_and_resumes_exactly_once() {
    #[derive(Default)]
    struct OneSlotVisitor {
        full: bool,
        events: Vec<Vec<u8>>,
    }

    impl SseVisitor for OneSlotVisitor {
        fn on_event(
            &mut self,
            event: SseEventView<'_>,
            emitter: &mut BoundedEventEmitter<'_, '_>,
        ) -> Result<(), SseError> {
            if self.full {
                return Err(SseError::NeedDrain);
            }
            self.events.push(event.raw().to_vec());
            self.full = true;
            emitter.drop_event()
        }
    }

    let input = b"data:a\n\ndata:b\n\ndata:c\n\n";
    let first_len = b"data:a\n\n".len();
    let budget = budget();
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    let mut visitor = OneSlotVisitor::default();
    let mut sink = CollectSink::default();
    assert_eq!(
        framer
            .feed(
                ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, input,)
                    .unwrap(),
                true,
                &mut visitor,
                &mut sink,
            )
            .unwrap(),
        SseFeedOutcome::NeedDrain {
            consumed_bytes: first_len,
        }
    );
    assert!(framer.needs_drain());
    assert!(budget.snapshot().unwrap().role_live[MemoryRole::SseFrame as usize] > 0);

    visitor.full = false;
    assert!(matches!(
        framer.resume(&mut visitor, &mut sink).unwrap(),
        SseFeedOutcome::NeedDrain { .. }
    ));
    visitor.full = false;
    assert_eq!(
        framer.resume(&mut visitor, &mut sink).unwrap(),
        SseFeedOutcome::Complete
    );
    assert_eq!(
        visitor.events,
        [
            b"data:a\n\n".to_vec(),
            b"data:b\n\n".to_vec(),
            b"data:c\n\n".to_vec()
        ]
    );
    assert!(!framer.needs_drain());
    drop(framer);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn pending_limit_must_cover_one_complete_event_at_compile_time() {
    let mut invalid = limits(EofPolicy::Strict);
    invalid.max_event_bytes = 128;
    invalid.max_pending_bytes = 32;
    assert!(matches!(
        SseFramer::new(invalid, budget()),
        Err(SseError::InvalidLimits)
    ));
}

#[test]
fn pending_append_error_is_irrecoverably_terminal() {
    let budget = budget();
    let mut small = limits(EofPolicy::Strict);
    small.max_event_bytes = 8;
    small.max_pending_bytes = 8;
    let mut framer = SseFramer::new(small, budget.clone()).unwrap();
    let mut visitor = ParsingVisitor::default();
    let mut sink = CollectSink::default();

    framer
        .feed(
            ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"data:123")
                .unwrap(),
            false,
            &mut visitor,
            &mut sink,
        )
        .unwrap();
    assert_eq!(
        framer
            .feed(
                ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"4",)
                    .unwrap(),
                false,
                &mut visitor,
                &mut sink,
            )
            .unwrap_err(),
        SseError::EventLimit
    );
    assert_eq!(framer.pending_bytes(), 0, "failed state releases backing");

    framer.reset();
    assert_eq!(
        framer
            .feed(
                ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, b"\n\n",)
                    .unwrap(),
                true,
                &mut visitor,
                &mut sink,
            )
            .unwrap_err(),
        SseError::FramerFailed
    );
}

struct PromoteVisitor {
    budget: StreamBudget,
    promoted: Vec<OwnedSseEvent>,
    cell: ConfigCellHandle,
    generations: Vec<ConfigGeneration>,
}

impl SseVisitor for PromoteVisitor {
    fn on_event(
        &mut self,
        event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        // EventLive guard is callback-scoped and !Send; copy only the small
        // generation value before any asynchronous work.
        let generation = self.cell.acquire_event().unwrap().value().generation;
        self.generations.push(generation);
        self.promoted.push(event.promote(&self.budget)?);
        emitter.drop_event()
    }
}

#[test]
fn promotion_is_explicitly_charged_and_event_live_generation_is_short_acquired() {
    let budget = budget();
    let descriptor = ConfigCellDescriptor {
        id: ConfigCellId(1),
        compatibility_hash: [9; 32],
        atomicity_group: AtomicityGroupId(1),
        binding_policy: ConfigBindingPolicy::EventLive,
    };
    let config = ImmutableConfig {
        generation: ConfigGeneration(1),
        compatibility_hash: [9; 32],
        bytes: Arc::from(&b"large-config"[..]),
    };
    let cell = ConfigCellHandle::new(
        descriptor,
        Arc::new(ConfigBundle::new(
            AtomicityGroupId(1),
            HashMap::from([(ConfigCellId(1), config)]),
        )),
    )
    .unwrap();
    let mut visitor = PromoteVisitor {
        budget: budget.clone(),
        promoted: Vec::new(),
        cell,
        generations: Vec::new(),
    };
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    framer
        .feed(
            ChargedBytes::copy_from_opaque(
                &budget,
                MemoryRole::TransportInflight,
                b"data: keep\n\n",
            )
            .unwrap(),
            true,
            &mut visitor,
            &mut CollectSink::default(),
        )
        .unwrap();
    assert_eq!(visitor.generations, [ConfigGeneration(1)]);
    assert_eq!(
        visitor.promoted[0].raw(),
        &Bytes::from_static(b"data: keep\n\n")
    );
}

#[test]
fn precommit_handoff_preserves_sequence_and_decode_exactly_once() {
    let budget = budget();
    let mut state = PrecommitResponseState::new();
    let raw0 = make_owned_event(&budget, b"data: zero\n\n");
    let raw1 = make_owned_event(&budget, b"data: one\n\n");
    let s0 = state.push_raw(raw0).unwrap();
    let s1 = state.push_raw(raw1).unwrap();
    let (classified0, provenance0) = state.take_raw_for_classification(s0).unwrap();
    assert_eq!(classified0.raw(), &Bytes::from_static(b"data: zero\n\n"));
    assert_eq!(provenance0, SemanticProvenance::ProducesSemantic);
    drop(classified0);
    state.mark_decoded(s0, "decoded-zero").unwrap();
    assert_eq!(
        state.mark_decoded(s0, "again").unwrap_err(),
        SseError::SequenceAlreadyDecoded(0)
    );
    let handoff = state.publish_accept().unwrap().collect::<Vec<_>>();
    match &handoff[0] {
        AcceptedEvent::Decoded {
            sequence,
            decoded,
            source_bytes,
            provenance,
        } => {
            assert_eq!(*sequence, 0);
            assert_eq!(*decoded, "decoded-zero");
            assert_eq!(*source_bytes, b"data: zero\n\n".len());
            assert_eq!(*provenance, SemanticProvenance::ProducesSemantic);
        }
        _ => panic!("expected decoded event"),
    }
    assert!(matches!(
        &handoff[1],
        AcceptedEvent::NeedsDecode { sequence: 1, .. }
    ));
    assert_eq!(s1, 1);

    let mut dropped: PrecommitResponseState<()> = PrecommitResponseState::new();
    dropped
        .push_raw(make_owned_event(&budget, b"data: drop\n\n"))
        .unwrap();
    dropped.publish_non_accept();
}

#[test]
fn accept_rejects_classifier_owned_raw_without_allocating_a_second_copy() {
    let budget = budget();
    let mut state = PrecommitResponseState::<()>::new_budgeted(2, &budget).unwrap();
    let sequence = state
        .push_raw(make_owned_event(&budget, b"data: linear\n\n"))
        .unwrap();
    let peak_before_transfer = budget.snapshot().unwrap().peak;
    let (raw, _) = state.take_raw_for_classification(sequence).unwrap();
    assert_eq!(
        budget.snapshot().unwrap().peak,
        peak_before_transfer,
        "linear transfer must not allocate a replay copy"
    );
    assert!(matches!(
        state.publish_accept(),
        Err(SseError::SequenceStillInFlight(candidate)) if candidate == sequence
    ));
    drop(raw);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn production_handoff_capacity_and_metadata_are_budgeted_until_linear_publication() {
    let budget = budget();
    let mut state = PrecommitResponseState::<()>::new_budgeted(1, &budget).unwrap();
    state
        .push_raw_with_provenance(
            make_owned_event(&budget, b": heartbeat\n\n"),
            SemanticProvenance::NonSemantic,
        )
        .unwrap();
    assert_eq!(
        state
            .push_raw(make_owned_event(&budget, b"data: overflow\n\n"))
            .unwrap_err(),
        SseError::HandoffCapacityExceeded
    );
    assert!(budget.snapshot().unwrap().live > 0);
    let mut accepted = state.publish_accept().unwrap();
    assert!(matches!(
        accepted.next(),
        Some(AcceptedEvent::NeedsDecode {
            sequence: 0,
            provenance: SemanticProvenance::NonSemantic,
            ..
        })
    ));
    drop(accepted);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

fn make_owned_event(budget: &StreamBudget, raw: &[u8]) -> OwnedSseEvent {
    struct Capture {
        budget: StreamBudget,
        event: Option<OwnedSseEvent>,
    }
    impl SseVisitor for Capture {
        fn on_event(
            &mut self,
            event: SseEventView<'_>,
            emitter: &mut BoundedEventEmitter<'_, '_>,
        ) -> Result<(), SseError> {
            self.event = Some(event.promote(&self.budget)?);
            emitter.drop_event()
        }
    }
    let mut capture = Capture {
        budget: budget.clone(),
        event: None,
    };
    SseFramer::new(limits(EofPolicy::Strict), budget.clone())
        .unwrap()
        .feed(
            ChargedBytes::copy_from_opaque(budget, MemoryRole::TransportInflight, raw).unwrap(),
            true,
            &mut capture,
            &mut CollectSink::default(),
        )
        .unwrap();
    capture.event.unwrap()
}

#[test]
fn mailbox_backpressures_without_drop_or_growth() {
    let mut mailbox = BudgetedResponseMailbox::new(2).unwrap();
    mailbox.try_push(1).unwrap();
    mailbox.try_push(2).unwrap();
    assert_eq!(mailbox.try_push(3), Err(3));
    assert!(mailbox.is_full());
    assert_eq!(mailbox.pop(), Some(1));
    mailbox.try_push(3).unwrap();
    assert_eq!(mailbox.pop(), Some(2));
    assert_eq!(mailbox.pop(), Some(3));
}

#[test]
fn semantic_provenance_algebra_is_conservative_and_authorized() {
    let budget = budget();
    let semantic = unit(&budget, b"abcd", SemanticProvenance::ProducesSemantic);
    let (left, right) = semantic.split(&budget, 2).unwrap();
    assert_eq!(left.provenance, SemanticProvenance::ProducesSemantic);
    assert_eq!(right.provenance, SemanticProvenance::ProducesSemantic);
    let merged = EncodedOutputUnit::merge(
        left,
        unit(&budget, b"heartbeat", SemanticProvenance::NonSemantic),
        &budget,
    )
    .unwrap();
    assert_eq!(merged.provenance, SemanticProvenance::ProducesSemantic);

    let replacement =
        ChargedBytes::copy_from_opaque(&budget, MemoryRole::OutputQueue, b"replace").unwrap();
    let inherited = merged.replace_inheriting(replacement);
    assert_eq!(inherited.provenance, SemanticProvenance::ProducesSemantic);
    let denied_plan = CompiledAcceptedResponsePlan {
        filters: Arc::new([]),
        body_plan: BodyPlan::PassThrough {
            max_chunk_bytes: 64 * 1024,
        },
        semantic_replacement_authorized: false,
        config_cell_ids: Arc::new([]),
    };
    let denied = SemanticReplacementCapability::from_compiled_plan(&denied_plan);
    assert_eq!(
        inherited
            .replace_authorized(
                ChargedBytes::copy_from_opaque(
                    &budget,
                    MemoryRole::OutputQueue,
                    b"claim-nonsemantic"
                )
                .unwrap(),
                SemanticProvenance::NonSemantic,
                &denied
            )
            .unwrap_err(),
        SseError::SemanticReplacementNotAuthorized
    );

    let authorized_plan = CompiledAcceptedResponsePlan {
        semantic_replacement_authorized: true,
        ..denied_plan
    };
    let authorized = SemanticReplacementCapability::from_compiled_plan(&authorized_plan);
    let replaced = unit(&budget, b"semantic", SemanticProvenance::ProducesSemantic)
        .replace_authorized(
            ChargedBytes::copy_from_opaque(&budget, MemoryRole::OutputQueue, b"heartbeat").unwrap(),
            SemanticProvenance::NonSemantic,
            &authorized,
        )
        .unwrap();
    assert_eq!(replaced.provenance, SemanticProvenance::NonSemantic);
}

fn unit(budget: &StreamBudget, bytes: &[u8], provenance: SemanticProvenance) -> EncodedOutputUnit {
    EncodedOutputUnit {
        bytes: ChargedBytes::copy_from_opaque(budget, MemoryRole::OutputQueue, bytes).unwrap(),
        provenance,
    }
}

#[test]
fn single_line_data_is_borrowed_multiline_allocates() {
    struct BorrowCheck {
        budget: StreamBudget,
        single_borrowed: bool,
        multi_owned: bool,
    }
    impl SseVisitor for BorrowCheck {
        fn on_event(
            &mut self,
            event: SseEventView<'_>,
            emitter: &mut BoundedEventEmitter<'_, '_>,
        ) -> Result<(), SseError> {
            match event.data(&self.budget)? {
                SseData::Borrowed(_) => self.single_borrowed = true,
                SseData::Owned(_) => self.multi_owned = true,
            }
            emitter.pass_raw()
        }
    }
    let budget = budget();
    let mut visitor = BorrowCheck {
        budget: budget.clone(),
        single_borrowed: false,
        multi_owned: false,
    };
    let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
    framer
        .feed(
            ChargedBytes::copy_from_opaque(
                &budget,
                MemoryRole::TransportInflight,
                b"data: one\n\ndata: two\ndata: three\n\n",
            )
            .unwrap(),
            true,
            &mut visitor,
            &mut CollectSink::default(),
        )
        .unwrap();
    assert!(visitor.single_borrowed);
    assert!(visitor.multi_owned);
}
