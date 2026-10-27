use super::*;
use crate::store::tests::support::{Fixture, fact_channel, offer_fact, open_store, writer};
use crate::{WriterCycleOutcome, query_v2::search_tests::reader};
use hiroute_domain::*;

#[test]
fn catalog_selection_is_opt_in_scope_bound_retained_and_authenticated() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("input-selection");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    let selected = RequestInputSelectionV1 {
        version: 1,
        evidence: RequestInputSelectionEvidenceV1::NativeMatched,
        message_ordinal: Some(1),
        source_unverified: true,
    };
    for (sequence, fact) in [
        (
            1,
            crate::store::tests::support::route_decision("plan/codex-daily"),
        ),
        (2, ExecutionFactV1::RequestInputSelection(selected.clone())),
    ] {
        assert!(matches!(
            offer_fact(&writer, &channel, fixture.fact(sequence, fact, 1000)),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut query = ObservationCatalogQueryV2 {
        request_id: fixture.request.clone(),
        limit: 10,
        cursor: None,
        include_input_selection: false,
    };
    let legacy = store.observed_catalog(&reader(true), &query, 5000).unwrap();
    assert!(legacy.input_selection.is_none());
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("input_selection")
            .is_none()
    );
    query.include_input_selection = true;
    assert_eq!(
        store
            .observed_catalog(&reader(true), &query, 5000)
            .unwrap()
            .input_selection,
        Some(selected)
    );
    assert_eq!(
        store.observed_catalog(&reader(false), &query, 5000),
        Err(crate::query_v2::ObservationV2Error::Unauthorized)
    );
    query.request_id = LogicalRequestId::parse("missing-request").unwrap();
    assert!(
        store
            .observed_catalog(&reader(true), &query, 5000)
            .unwrap()
            .input_selection
            .is_none()
    );
    query.request_id = fixture.request.clone();
    // Corruption cannot turn optional evidence into a main read failure or a guessed ordinal.
    store.connection.lock().execute("UPDATE observation_sensitive_payloads_v2 SET body_json=replace(body_json,'native_matched','wire_observed') WHERE request_id=?1 AND body_json LIKE '%request_input_selection%'", [fixture.request.as_str()]).unwrap();
    assert!(
        store
            .observed_catalog(&reader(true), &query, 5000)
            .unwrap()
            .input_selection
            .is_none()
    );
}

#[test]
fn excess_request_local_metadata_does_not_fall_back_to_unbounded_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("input-selection-bound");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            fixture.fact(
                1,
                ExecutionFactV1::RequestInputSelection(RequestInputSelectionV1 {
                    version: 1,
                    evidence: RequestInputSelectionEvidenceV1::WireObserved,
                    message_ordinal: Some(0),
                    source_unverified: true
                }),
                1000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let db = store.connection.lock();
    for n in 0..129 {
        db.execute(
            "INSERT INTO observation_safe_facts_v2 VALUES(?1,?2,?3,'{}')",
            params![
                fixture.workspace.as_str(),
                fixture.request.as_str(),
                format!("padding-{n}")
            ],
        )
        .unwrap();
    }
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        read(
            &tx,
            fixture.workspace.as_str(),
            fixture.request.as_str(),
            5000
        )
        .is_none()
    );
}

#[test]
fn reopened_input_position_honors_content_deletion_and_retention() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("input-selection-reopen");
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&fixture, 256 * 1024);
        assert!(matches!(
            offer_fact(
                &writer(&store),
                &channel,
                fixture.fact(
                    1,
                    ExecutionFactV1::RequestInputSelection(RequestInputSelectionV1 {
                        version: 1,
                        evidence: RequestInputSelectionEvidenceV1::WireObserved,
                        message_ordinal: Some(0),
                        source_unverified: true
                    }),
                    1000
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let store = open_store(directory.path());
    let query = ObservationCatalogQueryV2 {
        request_id: fixture.request.clone(),
        limit: 10,
        cursor: None,
        include_input_selection: true,
    };
    assert_eq!(
        store
            .observed_catalog(&reader(true), &query, 5000)
            .unwrap()
            .input_selection
            .unwrap()
            .message_ordinal,
        Some(0)
    );
    // The same authenticated metadata cannot outlive its retained request.
    let db = store.connection.lock();
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        read(
            &tx,
            fixture.workspace.as_str(),
            fixture.request.as_str(),
            crate::managed_text::RETENTION_MS + 2000
        )
        .is_none()
    );
    tx.rollback().unwrap();
    drop(db);
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: fixture.workspace.clone(),
        session_id: fixture.session.clone(),
        data_class: DeletionDataClass::ContentOnly,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            5000,
        )
        .unwrap();
    assert!(
        store
            .observed_catalog(&reader(true), &query, 5000)
            .unwrap()
            .input_selection
            .is_none()
    );
}
