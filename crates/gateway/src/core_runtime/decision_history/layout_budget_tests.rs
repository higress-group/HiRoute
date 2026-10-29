//! A refused optional layout cannot discard execution or poison future stages.
use super::*;
use crate::server::core_runtime::native_routing_view::NativeRoutingView;
use hiroute_gateway_core::runtime::body::MemoryRole;
use http::HeaderMap;

const SID: &str = "native-layout-budget-session";
const REPLAY_BUDGET: usize = 1024 * 1024;

fn metadata(turn: &str) -> Value {
    json!({
        "installation_id":"synthetic-installation", "session_id":SID, "thread_id":SID,
        "turn_id":turn, "window_id":format!("{SID}:0"), "request_kind":"turn",
        "thread_source":"user", "sandbox":"none"
    })
}

fn document(items: &[Value], turn: &str) -> Value {
    json!({
        "model":"route", "input":items, "stream":false,
        "future_setup":"opaque native setup",
        "client_metadata":{
            "session_id":SID, "thread_id":SID, "turn_id":turn,
            "x-codex-window-id":format!("{SID}:0"),
            "x-codex-turn-metadata":metadata(turn).to_string()
        }
    })
}

fn decode(document: &Value) -> model_ir::ModelRequestIRV1 {
    let request =
        super::super::super::adapters::decode_ingress_request(IngressProtocol::Responses, document)
            .unwrap();
    assert!(request.native_only, "unknown setup remains native-only");
    assert!(request.native_history_layout().is_some());
    request
}

fn begin(
    fixture: &Fixture,
    authorized: &crate::server::request_plan::AuthorizedRequestPlan,
    document: &Value,
    request: &model_ir::ModelRequestIRV1,
    turn: &str,
    request_id: &str,
) -> DecisionHistory {
    let mut headers = HeaderMap::new();
    headers.insert("user-agent", "codex_exec/0.147.0".parse().unwrap());
    headers.insert("session-id", SID.parse().unwrap());
    headers.insert("thread-id", SID.parse().unwrap());
    headers.insert(
        "x-codex-turn-metadata",
        metadata(turn).to_string().parse().unwrap(),
    );
    headers.insert("x-codex-window-id", format!("{SID}:0").parse().unwrap());
    let budget = BudgetTree::new(REPLAY_BUDGET, REPLAY_BUDGET)
        .unwrap()
        .stream(REPLAY_BUDGET)
        .unwrap();
    let view = NativeRoutingView::prepare(&headers, request, &fixture.replay, &budget);
    assert!(!view.current_input_unavailable());
    let (identity, prepared, _) = crate::context_hold::prepare_context(
        &fixture.runtime.context_holds,
        &[19; 32],
        authorized,
        crate::context_hold::ContextRequest {
            ingress: IngressProtocol::Responses,
            headers: &headers,
            document,
            request,
            semantic_request: Some(view.request(request)),
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
                request,
                replay: &fixture.replay,
                current_input: view.current_input(),
                current_input_position: view.current_input_position(),
                current_input_unavailable: view.current_input_unavailable(),
            },
        )
        .unwrap();
    assert!(
        history.guard.is_some(),
        "actual request still admits capture"
    );
    if let Some(ticket) = &history.hold_ticket {
        assert!(ticket.hint.is_none());
        assert!(ticket.cleaned_prefix_len.is_none());
    }
    history
}

#[test]
fn refused_native_layout_charge_keeps_capture_and_recovers_complete_tool_stage() {
    let fixture = Fixture::new();
    let authorized = crate::server::publication::authorized_native_consumer_fixture(None);
    let mut items = vec![json!({"role":"user", "content":"actual capacity-limited A"})];
    let a_document = document(&items, "a-turn");
    let mut a_request = decode(&a_document);
    let layout_bytes = a_request.native_history.as_ref().unwrap().capacity()
        * std::mem::size_of::<model_ir::NativeHistoryItem>();
    assert!(layout_bytes > 0);

    // Measure the real mandatory charges with an independent store. Keeping this
    // small request inline avoids confusing transient spill buffers with layout
    // admission; the later requests exercise fully externalized content.
    let probe_budget = BudgetTree::new(REPLAY_BUDGET, REPLAY_BUDGET)
        .unwrap()
        .stream(REPLAY_BUDGET)
        .unwrap();
    let probe_replay = fixture
        ._manager
        .begin_request(probe_budget.clone())
        .unwrap();
    let mut probe_request = decode(&a_document);
    let probe_before = probe_budget.snapshot().unwrap();
    crate::content_ref::externalize_model_request(&mut probe_request, &probe_replay, usize::MAX)
        .unwrap();
    assert!(probe_request.native_history_layout().is_some());
    assert_eq!(
        probe_request.native_history.as_ref().unwrap().capacity()
            * std::mem::size_of::<model_ir::NativeHistoryItem>(),
        layout_bytes
    );
    let mandatory_bytes = probe_budget.snapshot().unwrap().live - probe_before.live - layout_bytes;
    assert!(mandatory_bytes > 0);
    drop(probe_request);
    drop(probe_replay);

    let budget = fixture.replay.budget();
    let before = budget.snapshot().unwrap();
    let pressure = budget
        .reserve(
            MemoryRole::TransportInflight,
            REPLAY_BUDGET - before.live - mandatory_bytes - (layout_bytes - 1),
        )
        .unwrap();
    let pressured = budget.snapshot().unwrap();
    crate::content_ref::externalize_model_request(&mut a_request, &fixture.replay, usize::MAX)
        .expect("mandatory request storage succeeds before optional layout refusal");
    let refused = budget.snapshot().unwrap();
    assert_eq!(refused.rejected, pressured.rejected + 1);
    assert_eq!(refused.live, pressured.live + mandatory_bytes);
    assert!(
        a_request.native_history.is_none(),
        "the real charge dropped the layout"
    );
    assert!(a_request.native_only);
    assert_eq!(a_request.native_body.as_ref(), Some(&a_document));
    assert!(
        crate::context_hold::branch_anchors(&a_request, &[19; 32], Some(&fixture.replay), &[1],)
            .is_none()
    );

    // The failed association allocation does not remove the current execution.
    // Hold the pressure until its accepted output has entered the real store.
    let mut a = begin(
        &fixture,
        &authorized,
        &a_document,
        &a_request,
        "a-turn",
        "a-final",
    );
    fixture.finish_named(&mut a, &[text_event("accepted A output")], true, "a-final");
    drop(a);
    drop(pressure);

    items.extend([
        json!({"role":"assistant", "content":"accepted A output"}),
        json!({"role":"user", "content":"actual recovered B"}),
    ]);
    let b_document = document(&items, "b-turn");
    let mut b_request = decode(&b_document);
    crate::content_ref::externalize_model_request(&mut b_request, &fixture.replay, 0).unwrap();
    assert!(b_request.native_history_layout().is_some());
    let mut b = begin(
        &fixture,
        &authorized,
        &b_document,
        &b_request,
        "b-turn",
        "b-tool",
    );
    let a_target = b.history.assessment_target.as_ref().unwrap();
    assert!(
        !a_target.target_partial,
        "lost layout is not lost accepted output"
    );
    let a_membership = a_target.history_membership.as_ref().unwrap();
    assert_eq!(a_membership.request_ids, vec!["a-final".to_owned()]);
    assert_eq!(
        a_membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    let a_visible = serde_json::to_string(&b.history.visible_conversation).unwrap();
    assert!(a_visible.contains("actual capacity-limited A"));
    assert!(a_visible.contains("accepted A output"));
    let b_turn = b.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
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
        json!({"type":"function_call_output", "call_id":"tool-1", "output":"actual B tool result"}),
    ]);
    let continued_document = document(&items, "b-turn");
    let mut continued_request = decode(&continued_document);
    crate::content_ref::externalize_model_request(&mut continued_request, &fixture.replay, 0)
        .unwrap();
    assert!(continued_request.native_history_layout().is_some());
    let mut continued = begin(
        &fixture,
        &authorized,
        &continued_document,
        &continued_request,
        "b-turn",
        "b-final",
    );
    assert_eq!(
        continued.guard.as_ref().unwrap().ticket().agent_turn_id,
        b_turn
    );
    assert!(continued.correlated.is_some());
    fixture.finish_named(
        &mut continued,
        &[text_event("accepted B output")],
        true,
        "b-final",
    );
    drop(continued);

    items.extend([
        json!({"role":"assistant", "content":"accepted B output"}),
        json!({"role":"user", "content":"actual next C"}),
    ]);
    let c_document = document(&items, "c-turn");
    let mut c_request = decode(&c_document);
    crate::content_ref::externalize_model_request(&mut c_request, &fixture.replay, 0).unwrap();
    assert!(c_request.native_history_layout().is_some());
    let c = begin(
        &fixture,
        &authorized,
        &c_document,
        &c_request,
        "c-turn",
        "c-question",
    );
    let target = c.history.assessment_target.as_ref().unwrap();
    assert!(
        !target.target_partial,
        "a later complete stage recovers in the same session"
    );
    let membership = target.history_membership.as_ref().unwrap();
    assert_eq!(
        membership.request_ids,
        vec!["b-tool".to_owned(), "b-final".to_owned()]
    );
    assert_eq!(
        membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    assert!(membership.association_unverified);
    let visible = serde_json::to_string(&c.history.visible_conversation).unwrap();
    assert!(visible.contains("actual recovered B"));
    assert!(visible.contains("accepted B output"));
    assert!(
        c.history
            .visible_conversation
            .iter()
            .flat_map(|turn| &turn.steps)
            .flatten()
            .any(|part| matches!(part, VisibleContentPart::ToolActivity { tool, .. } if tool == "read"))
    );
}
