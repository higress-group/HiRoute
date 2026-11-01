use super::*;
use crate::model_ir::ResponseItemStatus;

fn capture() -> SemanticResponse {
    let request = crate::observation::tests::request_with_sinks_capacity(
        crate::observation::otel::OtelContentPolicy::Disabled,
        hiroute_diagnostics::runtime::DiagnosticsPort::default(),
        true,
        crate::observation::GatewayObservationSinks::discard(),
        4 * 1024 * 1024,
    );
    let reservation = request
        .inner
        .channels
        .content
        .reserve_work(RESERVATION_BYTES)
        .unwrap();
    SemanticResponse::new("attempt-1", [9; 32], reservation)
}

fn delta(sequence: u64, index: u32, value: &str) -> ModelStreamEventV1 {
    ModelStreamEventV1::new(
        sequence,
        ModelEvent::TextDelta {
            index,
            text: value.into(),
        },
    )
}

#[test]
fn thousands_of_frames_become_bounded_exact_unicode_segments_and_one_terminal_metadata() {
    let mut capture = capture();
    let mut output = Vec::new();
    let mut expected = String::new();
    for sequence in 1..=10_001 {
        expected.push_str("中文\0🙂");
        output.extend(capture.accept(delta(sequence, 0, "中文\0🙂")));
    }
    output.extend(capture.accept(ModelStreamEventV1::new(
        10_002,
        ModelEvent::TextFinished {
            index: 0,
            text: expected.clone(),
            annotations: Vec::new(),
            status: ResponseItemStatus::Completed,
            final_phase: None,
        },
    )));
    output.extend(capture.finish());
    let actual = output
        .iter()
        .filter_map(|event| match &event.event {
            ModelEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(actual, expected);
    assert!(output.len() < 5, "no permanent per-delta projection");
    assert!(output.iter().all(|event| {
        text(&event.event).is_none_or(|(text, _, _, _)| text.len() <= CONTENT_CHUNK_BYTES)
    }));
    assert!(
        matches!(&output.last().unwrap().event,ModelEvent::TextFinished{text,..} if text.is_empty())
    );
    assert!(!capture.partial);
}

#[test]
fn interleaved_blocks_and_kinds_preserve_order_and_cancelled_prefix() {
    let mut capture = capture();
    let mut output = Vec::new();
    for event in [
        delta(1, 0, "A"),
        delta(2, 0, "B"),
        ModelStreamEventV1::new(
            3,
            ModelEvent::ReasoningDelta {
                index: 1,
                text: "think".into(),
            },
        ),
        delta(4, 0, "C"),
        ModelStreamEventV1::new(
            5,
            ModelEvent::RefusalDelta {
                index: 2,
                text: "refuse".into(),
            },
        ),
    ] {
        output.extend(capture.accept(event));
    }
    output.extend(capture.finish());
    assert_eq!(output.iter().filter_map(|event|text(&event.event).map(|(text,_,kind,_)|(kind,text.to_owned()))).collect::<Vec<_>>(),vec![(0,"AB".into()),(1,"think".into()),(0,"C".into()),(2,"refuse".into())]);
}

#[test]
fn conflicting_and_empty_final_snapshots_cannot_erase_accepted_output() {
    for final_text in ["", "different"] {
        let mut capture = capture();
        assert!(capture.accept(delta(1, 0, "accepted")).is_empty());
        let output = capture.accept(ModelStreamEventV1::new(
            2,
            ModelEvent::TextFinished {
                index: 0,
                text: final_text.into(),
                annotations: Vec::new(),
                status: ResponseItemStatus::Incomplete,
                final_phase: None,
            },
        ));
        assert_eq!(text(&output[0].event).unwrap().0, "accepted");
        assert_eq!(text(&output[1].event).unwrap().0, final_text);
        assert!(capture.partial);
    }
}

#[test]
fn tool_arguments_with_reused_block_index_do_not_merge_different_logical_calls() {
    let mut capture = capture();
    let mut output = Vec::new();
    for (sequence, id, value) in [(1, "first", "{"), (2, "first", "}"), (3, "second", "[]")] {
        output.extend(capture.accept(ModelStreamEventV1::new(
            sequence,
            ModelEvent::ToolArgumentsDelta {
                index: 0,
                logical_id: id.into(),
                delta: value.into(),
            },
        )));
    }
    output.extend(capture.finish());
    assert_eq!(output.len(), 2);
    assert!(
        matches!(&output[0].event,ModelEvent::ToolArgumentsDelta{logical_id,delta,..} if logical_id=="first"&&delta=="{}")
    );
    assert!(
        matches!(&output[1].event,ModelEvent::ToolArgumentsDelta{logical_id,delta,..} if logical_id=="second"&&delta=="[]")
    );
}
