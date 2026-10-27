//! A saved native session retains opaque peers when later real questions arrive.
use super::*;
use crate::server::core_runtime::native_routing_view::NativeRoutingView;
use http::HeaderMap;

const SID: &str = "retained-native-session";

fn retained_begin(
    fixture: &Fixture,
    authorized: &crate::server::request_plan::AuthorizedRequestPlan,
    items: &[Value],
    turn: &str,
    request_id: &str,
) -> DecisionHistory {
    let metadata = json!({
        "installation_id":"synthetic-installation", "session_id":SID, "thread_id":SID,
        "turn_id":turn, "window_id":format!("{SID}:0"), "request_kind":"turn",
        "thread_source":"user", "sandbox":"none"
    });
    let document = json!({
        "model":"route", "input":items, "stream":false,
        "client_metadata":{
            "session_id":SID, "thread_id":SID, "turn_id":turn,
            "x-codex-window-id":format!("{SID}:0"),
            "x-codex-turn-metadata":metadata.to_string()
        }
    });
    let mut request = super::super::super::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &document,
    )
    .unwrap();
    crate::content_ref::externalize_model_request(&mut request, &fixture.replay, 0).unwrap();
    assert!(
        request.native_only,
        "opaque native history remains on the wire"
    );
    let original = serde_json::to_value(&request).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("user-agent", "codex_exec/0.147.0".parse().unwrap());
    headers.insert("session-id", SID.parse().unwrap());
    headers.insert("thread-id", SID.parse().unwrap());
    headers.insert(
        "x-codex-turn-metadata",
        metadata.to_string().parse().unwrap(),
    );
    headers.insert("x-codex-window-id", format!("{SID}:0").parse().unwrap());
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let view = NativeRoutingView::prepare(&headers, &request, &fixture.replay, &budget);
    let (identity, prepared, _) = crate::context_hold::prepare_context(
        &fixture.runtime.context_holds,
        &[19; 32],
        authorized,
        crate::context_hold::ContextRequest {
            ingress: IngressProtocol::Responses,
            headers: &headers,
            document: &document,
            request: &request,
            semantic_request: Some(view.request(&request)),
            user_origin_unverified: view.origin_unverified(),
            replay: &fixture.replay,
        },
        Instant::now(),
    );
    let key = fixture.runtime.context_holds.digest_key().unwrap();
    let capture = CaptureScope::authorized(
        authorized,
        IngressProtocol::Responses,
        &identity,
        &key,
        prepared.as_ref(),
    );
    let history = fixture
        .runtime
        .begin_authorized_decision_history(
            &super::super::super::observation::accepted_request_named_for_runtime_test(request_id),
            PlanSnapshot {
                plan_id: "plan".into(),
                plan_revision: 1,
            },
            capture,
            prepared,
            HistoryInput {
                request: &request,
                replay: &fixture.replay,
                current_input: view.current_input(),
                current_input_position: view.current_input_position(),
                current_input_unavailable: view.current_input_unavailable(),
            },
        )
        .unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), original);
    assert!(
        history.guard.is_some(),
        "optional native evidence cannot stop capture"
    );
    // A ticket can record a checkpoint without authorizing provider affinity.
    // Only strict history can grant that preference or a reusable cleanup prefix.
    assert!(
        visible_history_with_replay(&request, &key, Some(&fixture.replay)).is_none(),
        "opaque history must not supply strict provider-prefix evidence"
    );
    if let Some(ticket) = &history.hold_ticket {
        assert!(
            ticket.hint.is_none(),
            "opaque history cannot prefer a provider"
        );
        assert!(
            ticket.cleaned_prefix_len.is_none(),
            "opaque history cannot authorize a reusable cleanup prefix"
        );
    }
    history
}

fn opaque_prefix(
    fixture: &Fixture,
    authorized: &crate::server::request_plan::AuthorizedRequestPlan,
    capture_prior: bool,
) -> Vec<Value> {
    opaque_prefix_item(fixture, authorized, peer_item(), capture_prior)
}

fn peer_item() -> Value {
    json!({"type":"agent_message", "author":"child", "recipient":"parent", "content":[
        {"type":"input_text", "text":"native peer envelope"},
        {"type":"encrypted_content", "encrypted_content":"synthetic opaque peer"}
    ]})
}

fn opaque_prefix_item(
    fixture: &Fixture,
    authorized: &crate::server::request_plan::AuthorizedRequestPlan,
    old_item: Value,
    capture_prior: bool,
) -> Vec<Value> {
    let mut items = vec![
        json!({"role":"user", "content":"uncaptured ancestor task"}),
        old_item,
    ];
    if capture_prior {
        let mut opaque =
            retained_begin(fixture, authorized, &items, "opaque-turn", "opaque-request");
        fixture.finish_named(
            &mut opaque,
            &[text_event("accepted opaque-stage output")],
            true,
            "opaque-request",
        );
    }
    items.push(json!({"role":"assistant", "content":"accepted opaque-stage output"}));
    items
}

fn assert_actual_b_target(
    history: &DecisionHistory,
    expected_requests: &[&str],
    capture_prior: bool,
) {
    let target = history
        .history
        .assessment_target
        .as_ref()
        .expect("actual B execution remains evaluable");
    assert!(
        !target.target_partial,
        "retained opaque history must not poison a newly captured complete B stage"
    );
    let membership = target.history_membership.as_ref().unwrap();
    for id in expected_requests {
        assert!(
            membership.request_ids.iter().any(|actual| actual == id),
            "actual B request {id} was lost"
        );
    }
    assert_eq!(membership.request_ids.len(), expected_requests.len());
    assert!(
        membership.source_unverified,
        "recovery does not upgrade input-source trust"
    );
    assert!(membership.association_unverified);
    assert_eq!(
        membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    let visible = serde_json::to_string(&history.history.visible_conversation).unwrap();
    assert!(visible.contains("real new question B"));
    assert!(visible.contains("accepted B final output"));
    if !capture_prior {
        assert!(!visible.contains("uncaptured ancestor task"));
        assert!(!visible.contains("accepted opaque-stage output"));
    }
    assert!(!visible.contains("synthetic opaque peer"));
}

#[test]
fn retained_opaque_peer_history_recovers_new_text_pair_in_same_native_session() {
    for capture_prior in [false, true] {
        new_text_pair(capture_prior);
    }
}

fn new_text_pair(capture_prior: bool) {
    let fixture = Fixture::new();
    let authorized = crate::server::publication::authorized_native_consumer_fixture(None);
    let mut items = opaque_prefix(&fixture, &authorized, capture_prior);
    items.push(json!({"role":"user", "content":"real new question B"}));
    let mut b = retained_begin(&fixture, &authorized, &items, "b-turn", "b-final");
    fixture.finish_named(
        &mut b,
        &[text_event("accepted B final output")],
        true,
        "b-final",
    );
    drop(b);
    items.extend([
        json!({"role":"assistant", "content":"accepted B final output"}),
        json!({"role":"user", "content":"real new question C"}),
    ]);
    let c = retained_begin(&fixture, &authorized, &items, "c-turn", "c-question");
    let expected: &[&str] = if capture_prior {
        &["opaque-request", "b-final"]
    } else {
        &["b-final"]
    };
    assert_actual_b_target(&c, expected, capture_prior);
}

#[test]
fn retained_opaque_peer_history_recovers_complete_new_tool_stage_in_same_native_session() {
    complete_new_tool_stage(peer_item());
}

#[test]
fn retained_unknown_message_history_recovers_complete_new_tool_stage() {
    complete_new_tool_stage(
        json!({"role":"assistant","content":"old output", "vendor_extension":{"state":"opaque"}}),
    );
}

#[test]
fn retained_unknown_reasoning_history_recovers_complete_new_tool_stage() {
    complete_new_tool_stage(
        json!({"type":"reasoning","summary":[],"vendor_extension":{"state":"opaque"}}),
    );
}

fn complete_new_tool_stage(old_item: Value) {
    for capture_prior in [false, true] {
        complete_new_tool_stage_with_capture(old_item.clone(), capture_prior);
    }
}

fn complete_new_tool_stage_with_capture(old_item: Value, capture_prior: bool) {
    let complete_prior = old_item["type"] == "agent_message";
    let fixture = Fixture::new();
    let authorized = crate::server::publication::authorized_native_consumer_fixture(None);
    let mut items = opaque_prefix_item(&fixture, &authorized, old_item, capture_prior);
    items.push(json!({"role":"user", "content":"real new question B"}));
    let mut b = retained_begin(&fixture, &authorized, &items, "b-turn", "b-tool");
    fixture.finish_named(
        &mut b,
        &[
            tool_event(),
            ModelStreamEventV1::new(
                1,
                ModelEvent::ToolCallFinished {
                    encrypted_function_args: None,
                    index: 0,
                    logical_id: "tool-1".into(),
                    status: model_ir::ResponseItemStatus::Completed,
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    name: "read".into(),
                    arguments: json!({}),
                },
            ),
        ],
        false,
        "b-tool",
    );
    drop(b);
    items.extend([
        json!({"type":"function_call", "call_id":"tool-1", "name":"read", "arguments":"{}"}),
        json!({"type":"function_call_output", "call_id":"tool-1", "output":"actual tool result"}),
    ]);
    let mut continued = retained_begin(&fixture, &authorized, &items, "b-turn", "b-final");
    fixture.finish_named(
        &mut continued,
        &[text_event("accepted B final output")],
        true,
        "b-final",
    );
    drop(continued);
    items.extend([
        json!({"role":"assistant", "content":"accepted B final output"}),
        json!({"role":"user", "content":"real new question C"}),
    ]);
    let c = retained_begin(&fixture, &authorized, &items, "c-turn", "c-question");
    let expected: &[&str] = if capture_prior && complete_prior {
        &["opaque-request", "b-tool", "b-final"]
    } else {
        &["b-tool", "b-final"]
    };
    assert_actual_b_target(&c, expected, capture_prior);
    if capture_prior && !complete_prior {
        let visible = serde_json::to_string(&c.history.visible_conversation).unwrap();
        assert!(
            visible.contains("uncaptured ancestor task"),
            "actual earlier input remains background"
        );
        assert!(
            visible.contains("accepted opaque-stage output"),
            "actual earlier output is retained"
        );
    }
}
