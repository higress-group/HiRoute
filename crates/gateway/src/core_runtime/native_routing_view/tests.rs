use super::*;
use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::core_runtime::adapters;
use crate::server::request_plan::IngressProtocol;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "hermes_tests.rs"]
mod hermes;

#[path = "codex_tests.rs"]
mod codex;

#[path = "dsh_tests.rs"]
mod dsh;

struct Fixture {
    root: std::path::PathBuf,
    _manager: ReplayManager,
    replay: ReplayStore,
    budget: StreamBudget,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hiroute-routing-view-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 4096,
            record_bytes: 4096,
            orphan_ttl: Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(32 * 1024 * 1024, 32 * 1024 * 1024)
            .unwrap()
            .stream(32 * 1024 * 1024)
            .unwrap();
        let replay = manager.begin_request(budget.clone()).unwrap();
        Self {
            root,
            _manager: manager,
            replay,
            budget,
        }
    }
    fn view(&self, request: &ModelRequestIRV1, metadata: Value) -> NativeRoutingView {
        let mut headers = HeaderMap::new();
        headers.insert("x-hiroute-agent", "dsh".parse().unwrap());
        headers.insert("x-deepseek-harness-session-id", "session".parse().unwrap());
        headers.insert(
            "x-hiroute-routing-context",
            metadata.to_string().parse().unwrap(),
        );
        NativeRoutingView::prepare(&headers, request, &self.replay, &self.budget)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn request(protocol: IngressProtocol, messages: Value) -> ModelRequestIRV1 {
    let body = match protocol {
        IngressProtocol::Responses => json!({"model":"route","input":messages,"stream":false}),
        IngressProtocol::Messages => {
            json!({"model":"route","messages":messages,"max_tokens":1024,"stream":false})
        }
        _ => unreachable!(),
    };
    adapters::decode_ingress_request(protocol, &body).unwrap()
}
fn digest(text: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&vec![text]).unwrap())
    )
}
fn metadata() -> Value {
    json!({"version":1,"agent":"dsh","session_id":"session","purpose":"main","turn_id":"1",
        "user_turns":[{"turn_id":"1","messages":[{"index":0,"sha256":digest("real task")}]}]})
}

#[test]
fn current_window_preserves_unlisted_users_and_binds_the_final_wire_scope() {
    let fixture = Fixture::new();
    let original = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user", "content":"older actual input"},
            {"role":"assistant", "content":"actual output"},
            {"role":"user", "content":"current native event"},
            {"role":"user", "content":"later ambiguous notification"}
        ]),
    );
    let hashes = [
        digest("older actual input"),
        digest("current native event"),
        digest("later ambiguous notification"),
    ];
    let mut envelope = json!({"version":2,"agent":"dsh","session_id":"session","purpose":"main","turn_id":"turn",
        "input":{"kind":"native_messages","event_id":"event","coverage":"window",
            "messages":[{"index":1,"sha256":hashes[1]}],"scope_digest":format!("{:x}", Sha256::digest(serde_json::to_vec(&hashes).unwrap()))}});
    let view = fixture.view(&original, envelope.clone());
    assert_eq!(view.projection(), NativeInputProjection::WindowApplied);
    assert!(std::ptr::eq(view.request(&original), &original));
    assert_eq!(
        view.current_input().unwrap(),
        &[ContentPart::Text {
            text: "current native event".into()
        }]
    );
    assert_eq!(original.messages[0].role, MessageRole::User);
    assert_eq!(original.messages[3].role, MessageRole::User);
    assert_eq!(
        view.current_input_position(),
        Some(CurrentInputPosition::Window { first: 2, last: 2 })
    );
    let selection = view.excerpt_selection(&original, true);
    assert_eq!(selection.message_ordinal, Some(2));
    assert_eq!(
        selection.evidence,
        hiroute_domain::RequestInputSelectionEvidenceV1::NativeMatched
    );
    assert!(selection.source_unverified);
    // Capture numbers instructions before message ordinals. The position must
    // refer to that unchanged original capture, not to a filtered User list.
    let instructed = adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","instructions":"native instruction","input":[
            {"role":"user","content":"older actual input"},
            {"role":"assistant","content":"actual output"},
            {"role":"user","content":"current native event"},
            {"role":"user","content":"later ambiguous notification"}]}),
    )
    .unwrap();
    assert_eq!(
        fixture
            .view(&instructed, envelope.clone())
            .excerpt_selection(&instructed, true)
            .message_ordinal,
        Some(3)
    );
    let mut crossed = envelope.clone();
    crossed["input"]["messages"] = json!([
        {"index":0,"sha256":hashes[0]}, {"index":1,"sha256":hashes[1]}
    ]);
    let rejected = fixture.view(&original, crossed);
    assert_eq!(rejected.projection(), NativeInputProjection::WindowRejected);
    assert!(
        rejected.current_input().is_none(),
        "a batch cannot bridge actual output"
    );
    assert!(rejected.current_input_position().is_none());
    assert_eq!(
        rejected.excerpt_selection(&original, true).evidence,
        hiroute_domain::RequestInputSelectionEvidenceV1::Unavailable
    );
    envelope["input"]["scope_digest"] = "0".repeat(64).into();
    let rejected = fixture.view(&original, envelope);
    assert_eq!(rejected.projection(), NativeInputProjection::WindowRejected);
    assert!(rejected.current_input().is_none());
    assert!(std::ptr::eq(rejected.request(&original), &original));
    assert!(rejected.origin_unverified());
}

#[test]
fn opaque_native_fields_retain_the_request_and_disclose_origin_uncertainty() {
    let fixture = Fixture::new();
    for peer in [false, true] {
        let mut original = request(
            IngressProtocol::Responses,
            json!([{"role":"user","content":"complete ordinary user"}]),
        );
        original.native_only = true;
        original.native_body = Some(if peer {
            json!({"input":[{"type":"agent_message","author":"parent","recipient":"child","content":[]}]})
        } else {
            json!({"input":[{"role":"user","content":"complete ordinary user","phase":"native phase"}]})
        });
        let view = NativeRoutingView::prepare(
            &HeaderMap::new(),
            &original,
            &fixture.replay,
            &fixture.budget,
        );
        assert!(view.origin_unverified());
        assert!(std::ptr::eq(view.request(&original), &original));
    }
}

#[test]
fn semantic_user_projection_preserves_execution_and_runtime_context() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let fixture = Fixture::new();
        let mut original = request(
            protocol,
            json!([
                {"role":"user","content":"real task"}, {"role":"user","content":"native policy"}
            ]),
        );
        original.native_body = Some(json!({"opaque":"x".repeat(MAX_COMPACT_IR_BYTES * 2)}));
        let saved = original.clone();
        let view = fixture.view(&original, metadata());
        assert!(view.origin_unverified());
        let projected = view.request(&original);
        assert_eq!(projected.messages[0].role, MessageRole::User);
        assert_eq!(projected.messages[1].role, MessageRole::User);
        assert_eq!(projected.messages[1].content, original.messages[1].content);
        assert_eq!(projected.native_body, original.native_body);
        assert_eq!(
            view.current_input().unwrap(),
            original.messages[0].content.as_slice()
        );
        assert_eq!(
            original, saved,
            "routing view changed the execution or observed request"
        );
    }
}

#[test]
fn native_user_batch_combines_all_inputs_and_keeps_tool_results_after_it() {
    let fixture = Fixture::new();
    let original = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":"real task"}, {"role":"user","content":"second input"},
            {"role":"user","content":"policy"},
            {"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"tool receipt"}
        ]),
    );
    let mut proof = metadata();
    proof["user_turns"][0]["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"index":1,"sha256":digest("second input")}));
    let view = fixture.view(&original, proof);
    assert!(view.origin_unverified());
    let projected = view.request(&original);
    let real = projected
        .messages
        .iter()
        .filter(|m| {
            m.role == MessageRole::User
                && m.content
                    .iter()
                    .any(|p| matches!(p, ContentPart::Text { .. }))
        })
        .collect::<Vec<_>>();
    assert_eq!(real.len(), 3);
    assert_eq!(view.current_input().unwrap().len(), 2);
    assert_eq!(
        view.current_input_position(),
        Some(CurrentInputPosition::Window { first: 0, last: 1 })
    );
    assert!(projected.messages.iter().any(|m| {
        m.content
            .iter()
            .any(|p| matches!(p, ContentPart::ToolResult { .. }))
    }));
}

#[test]
fn malformed_missing_or_unmatched_native_user_proof_origin_unverified() {
    let fixture = Fixture::new();
    let original = request(
        IngressProtocol::Responses,
        json!([{"role":"user","content":"real task"}]),
    );
    let good = metadata();
    let mut cases = Vec::new();
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("user_turns");
    cases.push(missing);
    let mut unknown = good.clone();
    unknown["user_turns"][0]["messages"][0]["unknown"] = json!(true);
    cases.push(unknown);
    let mut wrong = good.clone();
    wrong["user_turns"][0]["messages"][0]["sha256"] = json!(digest("different"));
    cases.push(wrong);
    let mut index = good.clone();
    index["user_turns"][0]["messages"][0]["index"] = json!(127);
    cases.push(index);
    let mut duplicate = good.clone();
    duplicate["user_turns"][0]["messages"]
        .as_array_mut()
        .unwrap()
        .push(good["user_turns"][0]["messages"][0].clone());
    cases.push(duplicate);
    let mut no_current_user = good.clone();
    no_current_user["turn_id"] = json!("2");
    cases.push(no_current_user);
    let mut auxiliary = good;
    auxiliary["purpose"] = json!("compaction");
    cases.push(auxiliary);
    for proof in cases {
        let view = fixture.view(&original, proof);
        assert!(view.origin_unverified());
        assert!(std::ptr::eq(view.request(&original), &original));
    }
}

#[test]
fn optional_view_capacity_and_budget_misses_preserve_original_execution() {
    let fixture = Fixture::new();
    let mut original = request(
        IngressProtocol::Responses,
        json!([{"role":"user","content":"real task"}]),
    );
    original.served_model_id = "x".repeat(MAX_COMPACT_IR_BYTES + 1);
    assert!(fixture.view(&original, metadata()).origin_unverified());
    original.served_model_id = "route".into();
    let tiny = BudgetTree::new(64, 64).unwrap().stream(64).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-hiroute-agent", "dsh".parse().unwrap());
    headers.insert(
        "x-hiroute-routing-context",
        metadata().to_string().parse().unwrap(),
    );
    assert!(
        NativeRoutingView::prepare(&headers, &original, &fixture.replay, &tiny).origin_unverified()
    );
}

#[test]
fn equivalent_native_user_batches_do_not_erase_strict_wire_message_boundaries() {
    let fixture = Fixture::new();
    let first = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":[{"type":"input_text","text":"A"}]},
            {"role":"user","content":[{"type":"input_text","text":"B"},{"type":"input_text","text":"C"}]}
        ]),
    );
    let second = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":[{"type":"input_text","text":"A"},{"type":"input_text","text":"B"}]},
            {"role":"user","content":[{"type":"input_text","text":"C"}]}
        ]),
    );
    let proof = |texts: [&[&str]; 2]| {
        json!({
            "version":1,"agent":"dsh","session_id":"session","purpose":"main","turn_id":"1",
            "user_turns":[{"turn_id":"1","messages":texts.iter().enumerate().map(|(index,parts)| json!({
                "index":index,"sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(parts).unwrap()))
            })).collect::<Vec<_>>()}]
        })
    };
    let first_view = fixture.view(&first, proof([&["A"], &["B", "C"]]));
    let second_view = fixture.view(&second, proof([&["A", "B"], &["C"]]));
    assert!(first_view.origin_unverified());
    assert!(second_view.origin_unverified());
    assert_eq!(first_view.current_input(), second_view.current_input());
    assert_ne!(first_view.request(&first), second_view.request(&second));
    let first_measured = crate::context_hold::visible_history(&first, &[37; 32])
        .unwrap()
        .with_semantic_request(Some(first_view.request(&first)))
        .unwrap()
        .measure(None)
        .unwrap();
    let second_measured = crate::context_hold::visible_history(&second, &[37; 32])
        .unwrap()
        .with_semantic_request(Some(second_view.request(&second)))
        .unwrap()
        .measure(None)
        .unwrap();
    assert_ne!(
        first_measured.complete_digest,
        second_measured.complete_digest
    );
    assert_ne!(
        first_measured.semantic_complete_digest,
        second_measured.semantic_complete_digest
    );
}
