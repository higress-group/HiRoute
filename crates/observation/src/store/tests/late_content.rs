//! Independent content may reach SQLite after the execution terminal.
use hiroute_domain::{ContentCompleteness, ContentMode, ObservationQueryPort, ReceiptId};

use super::support::*;

#[test]
fn delayed_content_after_request_finished_preserves_receipt_and_reopens_exactly() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("late-content");
    let store = open_store(root.path());
    install_value_request(
        &store,
        &fixture,
        fixture.request.as_str(),
        "accepted-attempt",
        finished("accepted-attempt"),
        100,
    );
    let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
    let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
    assert_eq!(receipt.ordered_facts.len(), 10);
    // Capture timestamps describe the request, not the later ingestion time.
    // A finished request is not a tombstone for its independent content lane.
    let text = format!("late capture: {}", "escape \\\"\n路由🦀".repeat(2048));
    install_content(&store, &fixture, text.as_bytes(), 101);
    let detail = store
        .get_session(
            &fixture.workspace,
            &fixture.session,
            ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(
        detail.summary.content_completeness,
        ContentCompleteness::Complete
    );
    assert_eq!(detail.turns[0].messages[0].bytes, text.as_bytes());
    assert_eq!(
        store.get_receipt(&fixture.workspace, &receipt_id).unwrap(),
        receipt
    );
    drop(store);

    let reopened = open_store(root.path());
    assert_eq!(
        reopened
            .get_session(
                &fixture.workspace,
                &fixture.session,
                ContentMode::MessagesAndTools
            )
            .unwrap(),
        detail
    );
    assert_eq!(
        reopened
            .get_receipt(&fixture.workspace, &receipt_id)
            .unwrap(),
        receipt
    );
    // An idempotent replay after reopen retains the existing ACK/duplicate path.
    install_content(&reopened, &fixture, text.as_bytes(), 101);
    assert_eq!(
        reopened
            .get_session(
                &fixture.workspace,
                &fixture.session,
                ContentMode::MessagesAndTools
            )
            .unwrap(),
        detail
    );
}
