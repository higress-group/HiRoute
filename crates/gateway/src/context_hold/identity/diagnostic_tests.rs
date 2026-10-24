use super::*;
use hiroute_diagnostics::event::{NativeAdapterDisposition as D, NativeRoutingHeader as H};
use serde_json::json;

fn consume(headers: &HeaderMap) -> ContextIdentityFacts {
    let mut identity =
        super::super::reliable("opencode_session", vec!["private-session".into()], false);
    let facts = super::super::super::routing::facts(&identity, headers, &json!({}), None, false);
    identity.apply_routing(facts);
    identity
}

#[test]
fn evidence_binds_the_consumed_v2_verdict_without_revealing_source_values() {
    let mut headers = HeaderMap::new();
    headers.insert("x-hiroute-agent", "opencode".parse().unwrap());
    headers.insert(
        "authorization",
        "Bearer private-credential".parse().unwrap(),
    );
    let context = json!({"version":2,"agent":"opencode","purpose":"main",
        "session_id":"private-session","turn_id":"private-turn","checkpoint":"private-checkpoint",
        "input":{"kind":"observed_batch","event_id":"private-turn"}});
    headers.insert(
        "x-hiroute-routing-context",
        context.to_string().parse().unwrap(),
    );
    let identity = consume(&headers);
    let evidence = identity.routing_evidence(&headers);
    assert_eq!(evidence.header, H::Present);
    assert_eq!(evidence.adapter.unwrap().disposition, D::Applied);
    assert!(evidence.track_history && evidence.turn_applied && evidence.checkpoint_applied);
    assert_eq!(evidence.origin, NativeRoutingOrigin::Observed);
    assert!(
        !serde_json::to_string(&evidence)
            .unwrap()
            .contains("private-")
    );

    let mut mismatched = context;
    mismatched["session_id"] = json!("other-session");
    headers.insert(
        "x-hiroute-routing-context",
        mismatched.to_string().parse().unwrap(),
    );
    let evidence = consume(&headers).routing_evidence(&headers);
    assert_eq!(evidence.adapter.unwrap().disposition, D::SessionMismatch);
    assert!(evidence.track_history);
    assert!(!evidence.turn_applied && !evidence.checkpoint_applied);
}

#[test]
fn missing_duplicate_and_malformed_headers_do_not_claim_adapter_success_or_disable_capture() {
    let mut headers = HeaderMap::new();
    headers.insert("x-hiroute-agent", "opencode".parse().unwrap());
    let evidence = consume(&headers).routing_evidence(&headers);
    assert_eq!(evidence.header, H::Missing);
    assert!(evidence.adapter.is_none() && evidence.track_history);
    headers.insert("x-hiroute-routing-context", "{broken".parse().unwrap());
    let evidence = consume(&headers).routing_evidence(&headers);
    assert_eq!(evidence.header, H::Present);
    assert!(evidence.adapter.is_none() && evidence.track_history);
    headers.append("x-hiroute-routing-context", "{}".parse().unwrap());
    let evidence = consume(&headers).routing_evidence(&headers);
    assert_eq!(evidence.header, H::Invalid);
    assert!(evidence.adapter.is_none() && evidence.track_history);
}
