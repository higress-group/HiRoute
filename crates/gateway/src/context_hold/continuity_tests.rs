use super::*;
use crate::context_hold::{NativeCheckpoint, NativeUserOrigin, visible_history};
use crate::server::core_runtime::adapters::decode_ingress_request;
use serde_json::{Value, json};

fn prepare(
    store: &ContextHoldStore,
    input: Value,
    turn: u8,
    generation: u64,
) -> PreparedHold<'static> {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":input}),
    )
    .unwrap();
    let evidence = visible_history(&request, &[17; 32]).unwrap();
    let mut prepared = store
        .prepare(
            ContextHoldKey::for_runtime_test(1),
            &evidence,
            Instant::now(),
        )
        .unwrap();
    prepared.set_native(NativeCheckpoint {
        turn: Some([turn; 32]),
        generation: Some(generation),
        compacted: generation > 0,
        user_origin: NativeUserOrigin::NativeTurn,
        ..Default::default()
    });
    prepared
}

#[test]
fn cold_compacted_or_preexisting_model_history_is_observed_without_claiming_user_proof() {
    for (input, generation) in [
        (json!("summary only"), 1),
        (
            json!([{"role":"assistant","content":"previous result"},{"role":"user","content":"unproved user"}]),
            0,
        ),
        (
            json!([{"role":"user","content":"task"},{"type":"function_call_output","call_id":"call","output":"receipt"}]),
            0,
        ),
    ] {
        let store = ContextHoldStore::default();
        let prepared = prepare(&store, input, 1, generation);
        assert!(!prepared.history_is_proven());
        assert!(!prepared.native_user_unavailable());
        assert!(prepared.user_origin_unverified());
    }
}

#[test]
fn a_changed_native_id_needs_an_appended_user_and_cannot_restore_a_replayed_window() {
    let store = ContextHoldStore::default();
    let user = json!({"role":"user","content":"original"});
    let first = prepare(&store, json!([user]), 1, 0);
    assert!(first.history_is_proven());
    assert!(
        first.user_origin_unverified(),
        "a clean native window is not human-source proof"
    );
    store.commit(first, Instant::now()).unwrap();
    let replay = prepare(&store, json!([user]), 2, 0);
    assert!(!replay.history_is_proven());
    assert!(!replay.native_user_unavailable());
    assert!(replay.user_origin_unverified());
    let appended = json!([user,{"role":"assistant","content":"accepted"},{"role":"user","content":"new task"}]);
    let appended = prepare(&store, appended, 2, 0);
    assert!(appended.history_is_proven());
    assert!(appended.user_origin_unverified());
    let compressed = prepare(
        &store,
        json!([user,{"role":"user","content":"summary"}]),
        1,
        1,
    );
    assert!(compressed.history_is_proven());
    store.commit(compressed, Instant::now()).unwrap();
    let rollback = prepare(&store, json!([user]), 1, 0);
    assert!(!rollback.history_is_proven());
    assert!(rollback.native_user_unavailable());
    assert!(rollback.user_origin_unverified());
}
