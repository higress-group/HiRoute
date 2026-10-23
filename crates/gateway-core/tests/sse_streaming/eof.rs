use super::*;

#[test]
fn strict_eof_preserves_comment_only_tail_across_fragmentation() {
    for tail in [
        b": keepalive".as_slice(),
        b": keepalive\n",
        b": keepalive\r",
        b": keepalive\r\n",
        b": first\n: second",
        b"\n\r\n: final\r",
        b"\r",
    ] {
        let input = [b"data: previous\n\n".as_slice(), tail].concat();
        for fragment in [1, 7, input.len()] {
            for eof_with_last_chunk in [false, true] {
                let budget = budget();
                let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
                let mut visitor = ParsingVisitor::default();
                let mut sink = CollectSink::default();
                let mut consumed = 0;
                for chunk in input.chunks(fragment) {
                    consumed += chunk.len();
                    let result = framer.feed(
                        ChargedBytes::copy_from_opaque(
                            &budget,
                            MemoryRole::TransportInflight,
                            chunk,
                        )
                        .unwrap(),
                        eof_with_last_chunk && consumed == input.len(),
                        &mut visitor,
                        &mut sink,
                    );
                    assert_eq!(
                        result.unwrap(),
                        SseFeedOutcome::Complete,
                        "tail {tail:?}, fragment {fragment}"
                    );
                }
                if !eof_with_last_chunk {
                    framer
                        .feed(
                            ChargedBytes::copy_from_opaque(
                                &budget,
                                MemoryRole::TransportInflight,
                                b"",
                            )
                            .unwrap(),
                            true,
                            &mut visitor,
                            &mut sink,
                        )
                        .unwrap();
                }
                assert_eq!(sink.output.concat(), input);
                assert_eq!(visitor.data[0], b"previous");
                assert!(visitor.data[1..].iter().all(Vec::is_empty));
                assert_eq!(framer.pending_bytes(), 0);
                assert!(!framer.needs_drain());
                assert!(framer.complexity().scanned_bytes <= input.len());
                drop(framer);
                assert_eq!(budget.snapshot().unwrap().live, 0);
            }
        }
    }
}

#[test]
fn strict_eof_comment_tail_need_drain_preserves_one_charged_owner() {
    let prefix = b"data: previous\n\n";
    for tail in [
        b": keepalive".as_slice(),
        b": keepalive\n",
        b": keepalive\r\n",
    ] {
        let input = [prefix.as_slice(), tail].concat();
        let budget = budget();
        let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
        let mut visitor = OneSlotProductionVisitor::new(&budget);
        let mut sink = CollectSink::default();
        assert!(matches!(
            framer
                .feed(
                    ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, &input,)
                        .unwrap(),
                    true,
                    &mut visitor,
                    &mut sink,
                )
                .unwrap(),
            SseFeedOutcome::NeedDrain { .. }
        ));
        assert!(framer.needs_drain());
        assert_eq!(framer.pending_bytes(), tail.len());
        let retained = budget.snapshot().unwrap();
        assert!(retained.role_live[MemoryRole::SseFrame as usize] >= tail.len());
        let complexity = framer.complexity();
        for _ in 0..3 {
            assert_eq!(
                framer.resume(&mut visitor, &mut sink).unwrap(),
                SseFeedOutcome::NeedDrain { consumed_bytes: 0 }
            );
            assert_eq!(framer.pending_bytes(), tail.len());
            assert_eq!(framer.complexity(), complexity);
            let retry = budget.snapshot().unwrap();
            assert_eq!(retry.live, retained.live);
            assert_eq!(retry.peak, retained.peak);
            assert_eq!(retry.rejected, retained.rejected);
        }
        visitor.classify_one();
        assert_eq!(
            framer.resume(&mut visitor, &mut sink).unwrap(),
            SseFeedOutcome::Complete
        );
        assert!(!framer.needs_drain());
        assert_eq!(framer.pending_bytes(), 0);
        visitor.classify_one();
        drop(framer);
        assert_eq!(visitor.decoded(), [prefix.to_vec(), tail.to_vec()]);
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}

#[test]
fn strict_eof_still_rejects_unfinished_fields_and_json() {
    let prefix = b"data: previous\n\n";
    for tail in [
        b"data: {\"unfinished\":".as_slice(),
        b"data:\n",
        b"event: future\n",
        b"id: last\r\n",
        b"retry: 1000\r",
        b"vendor-control: unknown",
        b": comment\ndata: {\"unfinished\":",
        b": comment\nevent: future\n",
        b": comment\nx",
        b" : not a comment",
    ] {
        let input = [prefix.as_slice(), tail].concat();
        for fragment in [1, input.len()] {
            let budget = budget();
            let mut framer = SseFramer::new(limits(EofPolicy::Strict), budget.clone()).unwrap();
            let mut visitor = ParsingVisitor::default();
            let mut sink = CollectSink::default();
            for chunk in input.chunks(fragment) {
                framer
                    .feed(
                        ChargedBytes::copy_from_opaque(
                            &budget,
                            MemoryRole::TransportInflight,
                            chunk,
                        )
                        .unwrap(),
                        false,
                        &mut visitor,
                        &mut sink,
                    )
                    .unwrap();
            }
            assert_eq!(
                framer
                    .feed(
                        ChargedBytes::copy_from_opaque(
                            &budget,
                            MemoryRole::TransportInflight,
                            b"",
                        )
                        .unwrap(),
                        true,
                        &mut visitor,
                        &mut sink,
                    )
                    .unwrap_err(),
                SseError::IncompleteEventAtEof,
                "tail {tail:?}, fragment {fragment}"
            );
            assert_eq!(sink.output.concat(), prefix);
            assert_eq!(visitor.data, [b"previous".to_vec()]);
            assert_eq!(framer.pending_bytes(), 0);
            drop(framer);
            assert_eq!(budget.snapshot().unwrap().live, 0);
        }
    }
}

#[test]
fn strict_eof_comment_tail_keeps_event_and_budget_limits() {
    for (limit, budget_bytes, input, expected) in [
        (8, 1024, b": too long".as_slice(), SseError::PendingLimit),
        (4096, 8, b": eof".as_slice(), SseError::BudgetExceeded),
    ] {
        let budget = BudgetTree::new(budget_bytes, budget_bytes)
            .unwrap()
            .stream(budget_bytes)
            .unwrap();
        let mut small = limits(EofPolicy::Strict);
        small.max_event_bytes = limit;
        small.max_pending_bytes = limit;
        let mut framer = SseFramer::new(small, budget.clone()).unwrap();
        assert_eq!(
            framer
                .feed(
                    ChargedBytes::copy_from_opaque(&budget, MemoryRole::TransportInflight, input)
                        .unwrap(),
                    true,
                    &mut ParsingVisitor::default(),
                    &mut CollectSink::default(),
                )
                .unwrap_err(),
            expected
        );
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}
