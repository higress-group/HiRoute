use super::*;
use hiroute_domain::{ObservationCaptureMetadataV1, ObservationCatalogQueryV2};

fn capture(event: &mut ConversationContentEnvelopeV1, context: bool, anchor: &str) {
    event.capture = Some(ObservationCaptureMetadataV1 {
        context_only: context,
        boundary_verified: false,
        input_anchor_id: Some(MessageInstanceId::parse(anchor).unwrap()),
    });
}

fn next(event: &mut ConversationContentEnvelopeV1) {
    event.correlation.request_id = LogicalRequestId::parse("request-2").unwrap();
    event.correlation.turn_id = TurnId::parse("turn-2").unwrap();
    event.fork_id = "fork-request-2".into();
    event.occurred_at_unix_nanos = 10_000_000_000 + event.sequence;
}

#[test]
fn first_context_seed_survives_reopen_and_only_new_activity_is_retained_afterwards() {
    let mut fixture = Fixture::new();
    let mut events = vec![
        fixture.begin(1),
        fixture.append(2, 0, "context-1", "context-part-1", 0, 0, b"same", b"same"),
        fixture.append(3, 1, "context-2", "context-part-2", 1, 0, b"same", b"same"),
        fixture.append(4, 2, "current-user", "current-part", 2, 0, b"same", b"same"),
        fixture.finish(5, "11"),
    ];
    for (index, event) in events.iter_mut().enumerate() {
        capture(event, matches!(index, 1 | 2), "current-user");
        ack(fixture.ingest(event));
    }
    // Equal text is one shared body, but three distinct message occurrences.
    {
        let db = fixture.store.connection.lock();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM content_message_instances_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM content_blobs_v2", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
    fixture.store = Arc::new(
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap(),
    );
    let mut events = vec![
        fixture.begin(6),
        fixture.append(
            7,
            0,
            "do-not-reseed",
            "discarded-context",
            0,
            0,
            b"unverified old history",
            b"unverified old history",
        ),
        fixture.append(8, 1, "new-user", "new-part", 3, 0, b"same", b"same"),
        fixture.finish(9, "22"),
    ];
    for (index, event) in events.iter_mut().enumerate() {
        next(event);
        capture(event, index == 1, "new-user");
        ack(fixture.ingest(event));
    }
    let db = fixture.store.connection.lock();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM content_message_instances_v2",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM content_instances_v2 WHERE content_id='discarded-context'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM conversation_content_streams_v2 WHERE state='finish'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_capture_scopes_v6",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn input_anchor_is_opt_in_one_hop_and_cannot_revive_expired_source_content() {
    let fixture = Fixture::new();
    let mut events = vec![
        fixture.begin(1),
        fixture.append(
            2,
            0,
            "source-user",
            "source-part",
            0,
            0,
            b"original user",
            b"original user",
        ),
        fixture.finish(3, "11"),
    ];
    for event in &mut events {
        capture(event, false, "source-user");
        ack(fixture.ingest(event));
    }
    let mut events = vec![
        fixture.begin(4),
        fixture.append(
            5,
            0,
            "tool-result",
            "tool-part",
            1,
            0,
            b"tool result",
            b"tool result",
        ),
        fixture.finish(6, "22"),
    ];
    for event in &mut events {
        next(event);
        capture(event, false, "source-user");
        ack(fixture.ingest(event));
    }
    let reader = hiroute_domain::ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        hiroute_domain::SEVEN_DAYS_MILLIS * 2,
        true,
        true,
    )
    .unwrap();
    let mut query = ObservationCatalogQueryV2 {
        request_id: LogicalRequestId::parse("request-2").unwrap(),
        limit: 20,
        cursor: None,
        include_input_selection: false,
        include_semantic_metadata: false,
    };
    let legacy = fixture
        .store
        .observed_catalog(&reader, &query, 20_000)
        .unwrap();
    let json = serde_json::to_value(legacy).unwrap();
    assert!(json.get("input_anchor").is_none());
    assert!(json["contents"][0].get("capture").is_none());
    query.include_semantic_metadata = true;
    let page = fixture
        .store
        .observed_catalog(&reader, &query, 20_000)
        .unwrap();
    assert_eq!(
        page.input_anchor.unwrap().source_request_id.as_str(),
        "request-1"
    );
    assert!(
        page.contents[0]
            .capture
            .as_ref()
            .is_some_and(|c| !c.context_only)
    );
    assert_eq!(
        fixture
            .store
            .expire_request_details(hiroute_domain::SEVEN_DAYS_MILLIS + 5_000, 200)
            .unwrap(),
        1
    );
    assert!(
        fixture
            .store
            .observed_catalog(&reader, &query, hiroute_domain::SEVEN_DAYS_MILLIS + 5_000)
            .unwrap()
            .input_anchor
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .connection
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM content_instances_v2 WHERE request_id='request-2'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn a_partial_first_seed_is_not_reimported_by_a_new_request() {
    let fixture = Fixture::new();
    let mut begin = fixture.begin(1);
    capture(&mut begin, false, "missing-current");
    ack(fixture.ingest(&begin));
    let mut part = fixture.append(
        2,
        0,
        "partial-context",
        "partial-part",
        0,
        0,
        b"first prefix",
        b"first prefix",
    );
    capture(&mut part, true, "missing-current");
    ack(fixture.ingest(&part));
    let mut abort = fixture.finish(3, "11");
    abort.phase = ConversationContentPhaseV2::Abort;
    abort.abort_reason = Some("cancelled".into());
    abort.completeness_delta = Some(ContentCompletenessDeltaV2::Partial);
    capture(&mut abort, false, "missing-current");
    ack(fixture.ingest(&abort));
    let mut begin = fixture.begin(4);
    next(&mut begin);
    capture(&mut begin, false, "new-current");
    ack(fixture.ingest(&begin));
    let mut context = fixture.append(
        5,
        0,
        "must-not-import",
        "must-not-import-part",
        0,
        0,
        b"a new old prefix",
        b"a new old prefix",
    );
    next(&mut context);
    capture(&mut context, true, "new-current");
    ack(fixture.ingest(&context));
    assert_eq!(
        fixture
            .store
            .connection
            .lock()
            .query_row("SELECT COUNT(*) FROM content_instances_v2", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
    // Retrying a deliberately omitted append acknowledges its commit without
    // inventing a content object or rejecting the original model request.
    ack(fixture.ingest(&context));
}
