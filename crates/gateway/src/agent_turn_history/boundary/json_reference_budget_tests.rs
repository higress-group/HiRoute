//! A raw native JSON range is usable transport, not canonical comparison proof.
use super::*;
use crate::agent_turn_history::boundary::incoming_boundary;
use crate::content_ref::{ContentValueExt, JsonValueExt};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};
use hiroute_gateway_core::runtime::body::MemoryRole;
use std::io::Read;

fn old_arguments() -> String {
    format!(r#"  {{ "z" : "{}", "a" : 1 }}  "#, "x".repeat(32 * 1024))
}

fn retained_body() -> serde_json::Value {
    json!({"model":"route","input":[
        {"role":"user","content":"old task"},
        {"type":"function_call","call_id":"old-call","name":"read","arguments":old_arguments()},
        {"type":"function_call_output","call_id":"old-call","output":"old receipt"},
        {"role":"assistant","content":"old final"},
        {"role":"user","content":"actual new A"}
    ]})
}

fn parse_with_raw_old_argument(body: &serde_json::Value, replay: &ReplayStore) -> ModelRequestIRV1 {
    use crate::content_ref::{
        compact_ingress_document_with_markers, externalize_model_request, parse_ingress_document,
        scan_ingress_document,
    };
    let mut raw = replay.begin_raw().unwrap();
    raw.append(&serde_json::to_vec(body).unwrap()).unwrap();
    let raw = raw.seal().unwrap();
    let stats = scan_ingress_document(replay.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, replay, &raw, stats).unwrap();
    let old_ref = document["input"][1]["arguments"]
        .as_str()
        .unwrap()
        .content_ref()
        .expect("the real parser externalized the old native argument string");
    // Leave enough workspace for the future small tool argument, but less than
    // the old argument's required 2 * raw length plus structural reservation.
    let budget = replay.budget();
    let before = budget.snapshot().unwrap();
    let pressure = budget
        .reserve(
            MemoryRole::TransportInflight,
            16 * 1024 * 1024 - before.live - 16 * 1024,
        )
        .unwrap();
    let pressured = budget.snapshot().unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        replay,
        workspace.generated_markers(),
    )
    .expect("optional canonicalization refusal must preserve valid native ingress");
    assert_eq!(budget.snapshot().unwrap().rejected, pressured.rejected + 1);
    assert_eq!(
        document["input"][1]["arguments"]
            .as_str()
            .unwrap()
            .content_ref(),
        Some(old_ref.clone()),
        "the refused canonicalization retained the original registered range"
    );
    drop(pressure);
    let mut request = crate::server::core_runtime::adapters::decode_ingress_request_with_replay(
        IngressProtocol::Responses,
        &document,
        Some(replay),
    )
    .unwrap();
    externalize_model_request(&mut request, replay, 0).unwrap();
    assert!(
        !request.native_only,
        "storage pressure is not protocol uncertainty"
    );
    let ContentPart::ToolCall {
        arguments,
        raw_arguments: None,
        ..
    } = &request.messages[1].content[0]
    else {
        panic!("valid native JSON remains a JSON value reference")
    };
    assert_eq!(arguments.content_ref(), Some(old_ref.clone()));
    let mut reader = replay.reader(&old_ref).unwrap();
    let mut actual = String::new();
    reader.read_to_string(&mut actual).unwrap();
    reader.verify_terminal().unwrap();
    assert_eq!(actual, old_arguments(), "raw native bytes stay unchanged");
    request
}

fn assert_no_canonical_proof(request: &ModelRequestIRV1, replay: &ReplayStore) {
    assert!(
        incoming_boundary(request, 1, replay, &mut FingerprintBudget::default()).is_none(),
        "raw native JSON cannot prove accepted canonical output"
    );
    assert!(
        crate::context_hold::visible_history_with_replay(request, &[29; 32], Some(replay))
            .and_then(|history| history.measure(Some(request.messages.len())))
            .is_none(),
        "raw bytes cannot acquire strict provider affinity proof"
    );
}

#[test]
fn refused_argument_canonicalization_keeps_wire_without_canonical_boundary_or_affinity() {
    let (_directory, replay) = replay();
    let request = parse_with_raw_old_argument(&retained_body(), &replay);
    assert_no_canonical_proof(&request, &replay);
}

fn begin(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> AgentTurnBegin {
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            store
                .prepare_scoped_request(scope, request, replay)
                .unwrap(),
            TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            Instant::now(),
            || Some(()),
        )
        .unwrap()
        .unwrap()
        .0
}

#[test]
fn retained_raw_json_prefix_allows_a_complete_later_tool_stage_and_frozen_assessment() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "raw-json-recovery").unwrap();
    let mut body = retained_body();
    let request = parse_with_raw_old_argument(&body, &replay);
    assert_no_canonical_proof(&request, &replay);
    let (a, history) = new_turn(begin(&store, &scope, &request, &replay));
    assert!(
        history.history_partial,
        "the old client execution was never captured"
    );
    assert!(history.assessment_target.is_none());
    drop(history);
    store.commit_decision(&a, decision("regular")).unwrap();
    store.accept_output(
        &a,
        &[
            ModelStreamEventV1::new(
                0,
                ModelEvent::ToolCallStarted {
                    index: 0,
                    logical_id: "new-call".into(),
                    native_id: "new-call".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    name: "read".into(),
                    owner: Box::new(super::accepted_output::owner()),
                    item_id: None,
                },
            ),
            ModelStreamEventV1::new(
                1,
                ModelEvent::ToolCallFinished {
                    encrypted_function_args: None,
                    index: 0,
                    logical_id: "new-call".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    name: "read".into(),
                    arguments: json!({"a":1,"z":2}),
                    status: ResponseItemStatus::Completed,
                },
            ),
        ],
    );
    assert!(
        store
            .finish_request(
                &a,
                "a-tool".into(),
                AgentTurnStatus::Completed,
                vec![execution(
                    "actual-model",
                    "actual-profile",
                    "regular",
                    "a-tool"
                )],
                false,
                Instant::now()
            )
            .unwrap()
            .is_none()
    );
    body["input"].as_array_mut().unwrap().extend([
        json!({"type":"function_call","call_id":"new-call","name":"read","arguments":"{\"a\":1,\"z\":2}"}),
        json!({"type":"function_call_output","call_id":"new-call","output":"actual A receipt"}),
    ]);
    let continued = parse_with_raw_old_argument(&body, &replay);
    assert_no_canonical_proof(&continued, &replay);
    let AgentTurnBegin::Continuation {
        ticket: final_request,
        ..
    } = begin(&store, &scope, &continued, &replay)
    else {
        panic!("an old raw prefix must not poison the new accepted tool boundary")
    };
    assert_eq!(a.agent_turn_id, final_request.agent_turn_id);
    assert!(!final_request.allow_affinity);
    super::accepted_output::text_output(&store, &final_request, "actual A DONE");
    let completed = store
        .finish_request(
            &final_request,
            "a-final".into(),
            AgentTurnStatus::Completed,
            vec![execution(
                "actual-model",
                "actual-profile",
                "regular",
                "a-final",
            )],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    let membership = completed.history_membership.unwrap();
    assert_eq!(membership.request_ids, ["a-tool", "a-final"]);
    assert_eq!(
        membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    assert!(membership.association_unverified);
    assert!(!membership.capture_partial);
    body["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"actual A DONE"}),
        json!({"role":"user","content":"actual new B"}),
    ]);
    let next = parse_with_raw_old_argument(&body, &replay);
    let (b, history) = new_turn(begin(&store, &scope, &next, &replay));
    let target = history.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    assert_eq!(target.history_membership.as_ref(), Some(&membership));
    assert!(matches!(&target.attribution, ExecutionAttribution::Single {
        model_configuration_id, profile_digest, ..
    } if model_configuration_id == "actual-model" && profile_digest == "actual-profile"));
    let visible = serde_json::to_string(&history.visible_conversation).unwrap();
    assert!(visible.contains("actual new A") && visible.contains("actual A DONE"));
    assert!(!visible.contains("old final") && !visible.contains("old receipt"));
    store.abort(&b);
}

#[test]
fn released_canonicalization_capacity_recovers_after_raw_prefix_representation_changes() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "released-json-capacity")
        .unwrap();
    let mut body = retained_body();
    body["input"][4]["content"] = json!("actual pressure request");
    let pressured = parse_with_raw_old_argument(&body, &replay);
    assert_no_canonical_proof(&pressured, &replay);
    let raw_anchors =
        crate::context_hold::branch_anchors(&pressured, &[29; 32], Some(&replay), &[]).unwrap();
    assert!(raw_anchors.contextual);

    let finish_text =
        |ticket: &AgentTurnTicket, text: &str, id: &str, model: &str, profile: &str| {
            store.commit_decision(ticket, decision("regular")).unwrap();
            super::accepted_output::text_output(&store, ticket, text);
            store
                .finish_request(
                    ticket,
                    id.into(),
                    AgentTurnStatus::Completed,
                    vec![execution(model, profile, "regular", id)],
                    true,
                    Instant::now(),
                )
                .unwrap()
                .unwrap()
        };
    let (pressure_ticket, initial_history) = new_turn(begin(&store, &scope, &pressured, &replay));
    assert!(initial_history.assessment_target.is_none());
    drop(initial_history);
    finish_text(
        &pressure_ticket,
        "accepted pressure output",
        "pressure-request",
        "pressure-model",
        "pressure-profile",
    );

    // The pressure helper released its real reservation. No manual mutation of
    // Model IR, provenance or history is used for any following request.
    let parse = |body: &serde_json::Value| {
        super::reasoning_boundary::production_request_for(
            IngressProtocol::Responses,
            body,
            &replay,
            0,
        )
    };
    let rejected_before = replay.budget().snapshot().unwrap().rejected;
    let normal_same_input = parse(&body);
    assert_eq!(
        replay.budget().snapshot().unwrap().rejected,
        rejected_before
    );
    let ContentPart::ToolCall { arguments, .. } = &normal_same_input.messages[1].content[0] else {
        panic!("the old native JSON argument remains present")
    };
    assert!(replay.is_canonical_json(&arguments.content_ref().unwrap()));
    let canonical_anchors =
        crate::context_hold::branch_anchors(&normal_same_input, &[29; 32], Some(&replay), &[])
            .unwrap();
    assert!(!canonical_anchors.contextual);
    assert_ne!(
        raw_anchors.complete, canonical_anchors.complete,
        "raw and canonical evidence are separate domains even for equal JSON meaning"
    );
    assert!(
        crate::context_hold::visible_history_with_replay(
            &normal_same_input,
            &[29; 32],
            Some(&replay),
        )
        .and_then(|history| history.measure(None))
        .is_some()
    );

    body["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"accepted pressure output"}),
        json!({"role":"user","content":"actual transition request"}),
    ]);
    let transition = parse(&body);
    let transition_anchors = crate::context_hold::branch_anchors(
        &transition,
        &[29; 32],
        Some(&replay),
        &[pressured.messages.len()],
    )
    .unwrap();
    assert_eq!(transition_anchors.prefixes[0].1, canonical_anchors.complete);
    assert_ne!(transition_anchors.prefixes[0].1, raw_anchors.complete);
    let AgentTurnBegin::NewTurn {
        ticket: transition_ticket,
        history: transition_history,
        inherited_decision,
        ..
    } = begin(&store, &scope, &transition, &replay)
    else {
        panic!("a changed representation cannot establish an exact continuation")
    };
    assert!(
        inherited_decision.is_none(),
        "the transition must make its own execution decision"
    );
    let transition_target = transition_history.assessment_target.as_ref().unwrap();
    let transition_membership = transition_target.history_membership.as_ref().unwrap();
    assert_eq!(transition_membership.request_ids, ["pressure-request"]);
    assert_eq!(
        transition_membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    assert!(transition_membership.association_unverified);
    assert!(
        !transition_target.target_partial,
        "the actual completed pressure request was captured"
    );
    drop(transition_history);
    finish_text(
        &transition_ticket,
        "accepted transition output",
        "transition-request",
        "transition-model",
        "transition-profile",
    );

    body["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"accepted transition output"}),
        json!({"role":"user","content":"actual recovered A"}),
    ]);
    let a_request = parse(&body);
    let (a, a_history) = new_turn(begin(&store, &scope, &a_request, &replay));
    drop(a_history);
    let a_completed = finish_text(
        &a,
        "accepted recovered A output",
        "recovered-a",
        "recovered-model",
        "recovered-profile",
    );
    body["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"accepted recovered A output"}),
        json!({"role":"user","content":"actual recovered B"}),
    ]);
    let b_request = parse(&body);
    let (b, b_history) = new_turn(begin(&store, &scope, &b_request, &replay));
    let target = b_history.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    let frozen_membership = target.history_membership.as_ref().unwrap().clone();
    assert_eq!(frozen_membership.request_ids, ["recovered-a"]);
    assert!(!frozen_membership.capture_partial && !frozen_membership.clipped);
    assert!(matches!(&target.attribution, ExecutionAttribution::Single {
        model_configuration_id, profile_digest, ..
    } if model_configuration_id == "recovered-model" && profile_digest == "recovered-profile"));
    let assessed = serde_json::to_string(
        &b_history.visible_conversation[b_history.assessment_from.unwrap()..],
    )
    .unwrap();
    assert!(
        assessed.contains("actual recovered A") && assessed.contains("accepted recovered A output")
    );
    assert!(
        !assessed.contains("accepted pressure output")
            && !assessed.contains("accepted transition output")
    );
    let visible = serde_json::to_string(&b_history.visible_conversation).unwrap();
    assert!(
        !visible.contains("old final") && !visible.contains("old receipt"),
        "old client history is never imported as captured execution"
    );

    // Extending the same recovered model's stage cannot broaden B's frozen
    // target after the actual B request has finished.
    let b_completed = finish_text(
        &b,
        "accepted recovered B output",
        "recovered-b",
        "recovered-model",
        "recovered-profile",
    );
    assert_eq!(a_completed.segment_id, b_completed.segment_id);
    assert_eq!(
        b_history
            .assessment_target
            .as_ref()
            .unwrap()
            .history_membership
            .as_ref(),
        Some(&frozen_membership)
    );
    body["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"accepted recovered B output"}),
        json!({"role":"user","content":"next actual question"}),
    ]);
    let c_request = parse(&body);
    let (c, c_history) = new_turn(begin(&store, &scope, &c_request, &replay));
    let extended = c_history.assessment_target.as_ref().unwrap();
    assert!(!extended.target_partial);
    assert_eq!(
        extended.history_membership.as_ref().unwrap().request_ids,
        ["recovered-a", "recovered-b"]
    );
    assert_eq!(
        b_history
            .assessment_target
            .as_ref()
            .unwrap()
            .history_membership
            .as_ref(),
        Some(&frozen_membership)
    );
    store.abort(&c);
}
