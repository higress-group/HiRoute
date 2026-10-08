use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::json;

use super::*;
use crate::server::core_runtime::profiles::fixed_reasoning;

fn tracker(budget: StreamBudget) -> (CanonicalResponseTracker, NativeResponseProjector) {
    let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    ));
    let projection = ToolIdProjection::new(IngressProtocol::Responses);
    let wire = NativeResponseProjector::new_for_observation(
        &profile,
        true,
        "physical".into(),
        None,
        projection.clone(),
    )
    .unwrap();
    let mut tracker = CanonicalResponseTracker::new_with_budget(
        profile,
        projection,
        None,
        true,
        "physical".into(),
        budget,
    );
    tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
    (tracker, wire)
}

fn feed(
    tracker: &mut CanonicalResponseTracker,
    wire: &mut NativeResponseProjector,
    event: serde_json::Value,
    accept: bool,
) -> Result<(), &'static str> {
    let bytes = format!("data: {event}\n\n").into_bytes();
    tracker.observe(CapturedNativeEvent::Body(bytes.clone()))?;
    let units = wire.feed(&bytes, false).unwrap();
    if accept {
        for unit in units {
            drop(tracker.correlate_accepted_output(&unit.bytes, false)?);
        }
    }
    Ok(())
}

fn created() -> serde_json::Value {
    json!({"type":"response.created", "response":{"id":"r", "model":"physical"}})
}

fn delta(text: &str) -> serde_json::Value {
    json!({"type":"response.output_text.delta", "item_id":"message",
        "output_index":0, "content_index":0, "delta":text})
}

#[test]
fn accepted_tiny_deltas_release_wire_and_keep_only_semantic_state() {
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let (mut capture, mut wire) = tracker(budget.clone());
    feed(&mut capture, &mut wire, created(), true).unwrap();
    for _ in 0..6_000 {
        feed(&mut capture, &mut wire, delta("abcd"), true).unwrap();
    }
    let snapshot = budget.snapshot().unwrap();
    assert!(
        snapshot.live > 24_000,
        "retained semantics must still be charged"
    );
    assert!(
        snapshot.peak < 512 * 1024,
        "wire traffic is not lifetime memory: {snapshot:?}"
    );
    drop(capture);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn unaccepted_output_remains_charged_and_is_released_on_cancellation() {
    let capacity = 256 * 1024;
    let budget = BudgetTree::new(capacity, capacity)
        .unwrap()
        .stream(capacity)
        .unwrap();
    let (mut capture, mut wire) = tracker(budget.clone());
    feed(&mut capture, &mut wire, created(), true).unwrap();
    let failed = (0..1_000).any(|_| feed(&mut capture, &mut wire, delta("abcd"), false).is_err());
    assert!(
        failed,
        "pending accepted-byte correlation must have a hard bound"
    );
    let snapshot = budget.snapshot().unwrap();
    assert!(snapshot.peak <= capacity);
    assert!(snapshot.rejected > 0);
    drop(capture);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn concurrent_decoders_share_one_limit_even_when_output_is_drained() {
    let capacity = 512 * 1024;
    let budget = BudgetTree::new(capacity, capacity)
        .unwrap()
        .stream(capacity)
        .unwrap();
    let mut captures = [tracker(budget.clone()), tracker(budget.clone())];
    for (capture, wire) in &mut captures {
        feed(capture, wire, created(), true).unwrap();
    }
    let text = "x".repeat(1024);
    let failed = (0..1_000).any(|ordinal| {
        let (capture, wire) = &mut captures[ordinal % 2];
        feed(capture, wire, delta(&text), true).is_err()
    });
    assert!(failed, "drained wire cannot erase retained decoder state");
    let snapshot = budget.snapshot().unwrap();
    assert!(snapshot.peak <= capacity);
    assert!(snapshot.rejected > 0);
    drop(captures);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}
