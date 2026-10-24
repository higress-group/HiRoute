use super::*;

#[path = "concurrent_encoding_tests.rs"]
mod encoding;

fn scoped(
    store: &AgentTurnHistoryStore,
    key: &AgentTurnHistoryKey,
    input: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
    store
        .begin_with_admission(
            key.clone(),
            plan(1),
            store.prepare_scoped_request(key, input, replay)?,
            TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            Instant::now(),
            || Some(()),
        )
        .map(|result| result.unwrap().0)
}

#[test]
fn fingerprint_overflow_preserves_current_capture_and_later_same_scope_assessment() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "fingerprint-limit").unwrap();
    let input = request(vec![
        text(MessageRole::User, "prior uncaptured task"),
        text(
            MessageRole::Assistant,
            "x".repeat(crate::context_hold::fingerprint::MAX_BYTES + 1),
        ),
        text(MessageRole::User, "actual A"),
    ]);
    let (a, a_snapshot) = new_turn(scoped(&store, &key, &input, &replay).unwrap());
    assert!(a_snapshot.visible_conversation.is_empty());
    assert!(a_snapshot.history_partial);
    accepted_output::text_output(&store, &a, "actual accepted A");
    close_turn(
        &store,
        &a,
        "regular",
        "model-a",
        "profile-a",
        "request-a",
        Instant::now(),
    );
    let next = request(vec![text(MessageRole::User, "actual B")]);
    let (_b, b_snapshot) = new_turn(scoped(&store, &key, &next, &replay).unwrap());
    assert_eq!(
        b_snapshot
            .assessment_target
            .as_ref()
            .unwrap()
            .through_turn_id,
        a.agent_turn_id
    );
    let serialized = serde_json::to_string(&b_snapshot.visible_conversation).unwrap();
    assert!(serialized.contains("actual A"));
    assert!(serialized.contains("actual accepted A"));
    assert!(serialized.len() < 4096);
}

#[test]
fn overlapping_same_scope_requests_capture_distinct_models_and_immutable_targets() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "parallel").unwrap();
    let a = request(vec![text(MessageRole::User, "A")]);
    let (a_ticket, _) = new_turn(scoped(&store, &key, &a, &replay).unwrap());
    close_turn(
        &store,
        &a_ticket,
        "regular",
        "model-a",
        "profile-a",
        "request-a",
        Instant::now(),
    );
    let slow = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "answer A"),
        text(MessageRole::User, "slow C"),
    ]);
    let (slow_ticket, slow_snapshot) = new_turn(scoped(&store, &key, &slow, &replay).unwrap());
    let fast = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "answer A"),
        text(MessageRole::User, "fast B"),
    ]);
    let (fast_ticket, fast_snapshot) = new_turn(scoped(&store, &key, &fast, &replay).unwrap());
    assert_ne!(fast_ticket.key, slow_ticket.key);
    assert_ne!(fast_ticket.request_token, slow_ticket.request_token);
    let target = fast_snapshot.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, a_ticket.agent_turn_id);
    close_turn(
        &store,
        &slow_ticket,
        "regular",
        "model-c",
        "profile-c",
        "request-c",
        Instant::now(),
    );
    assert_eq!(
        fast_snapshot
            .assessment_target
            .as_ref()
            .unwrap()
            .through_turn_id,
        a_ticket.agent_turn_id
    );
    let fast_completed = close_turn(
        &store,
        &fast_ticket,
        "primary",
        "model-b",
        "profile-b",
        "request-b",
        Instant::now(),
    );
    assert!(
        matches!(fast_completed.attribution, ExecutionAttribution::Single { model_configuration_id, .. } if model_configuration_id == "model-b")
    );
    assert_eq!(slow_snapshot.visible_conversation.len(), 1);
    assert_eq!(fast_snapshot.visible_conversation.len(), 1);
}

#[test]
fn live_scope_limit_is_optional_and_recovers_without_a_new_session() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "capacity").unwrap();
    let input = request(vec![text(
        MessageRole::User,
        "identical input is not deduplication",
    )]);
    let mut tickets = Vec::new();
    for _ in 0..32 {
        let (ticket, snapshot) = new_turn(scoped(&store, &key, &input, &replay).unwrap());
        drop(snapshot);
        tickets.push(ticket);
    }
    assert!(matches!(
        scoped(&store, &key, &input, &replay),
        Err(AgentTurnHistoryError::Resource)
    ));
    store.abort(&tickets.pop().unwrap());
    let (recovered, _) = new_turn(scoped(&store, &key, &input, &replay).unwrap());
    assert!(
        tickets
            .iter()
            .all(|ticket| ticket.request_token != recovered.request_token)
    );
    store.abort(&recovered);
    for ticket in tickets {
        store.abort(&ticket);
    }
}

#[test]
fn captured_response_selects_the_branch_instead_of_shared_prompt_or_finish_order() {
    use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1};
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "shared-prefix").unwrap();
    let input = request(vec![text(MessageRole::User, "same input")]);
    let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    let (b, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    for (ticket, answer, id) in [
        (&b, "B accepted output", "b"),
        (&a, "A accepted output", "a"),
    ] {
        store.accept_output(
            ticket,
            &[ModelStreamEventV1::new(
                0,
                ModelEvent::TextDelta {
                    index: 0,
                    text: answer.into(),
                },
            )],
        );
        close_turn(&store, ticket, "regular", id, "profile", id, Instant::now());
    }
    let follow = request(vec![
        text(MessageRole::User, "same input"),
        text(MessageRole::Assistant, "B accepted output"),
        text(MessageRole::User, "new input"),
    ]);
    let (next, snapshot) = new_turn(scoped(&store, &scope, &follow, &replay).unwrap());
    assert_eq!(
        next.key, b.key,
        "late A must not replace the response-bound predecessor"
    );
    let target = snapshot.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, b.agent_turn_id);
    assert!(
        matches!(&target.attribution, ExecutionAttribution::Single { model_configuration_id, .. } if model_configuration_id == "b")
    );
    store.abort(&next);
    let forged = request(vec![
        text(MessageRole::User, "same input"),
        text(MessageRole::Assistant, "uncaptured output"),
        text(MessageRole::User, "another input"),
    ]);
    let (separate, uncertain) = new_turn(scoped(&store, &scope, &forged, &replay).unwrap());
    assert_ne!(separate.key, a.key);
    assert_ne!(separate.key, b.key);
    assert!(
        uncertain
            .assessment_target
            .as_ref()
            .unwrap()
            .history_membership
            .as_ref()
            .unwrap()
            .association_unverified
    );
    store.abort(&separate);
}

#[test]
fn equal_user_text_does_not_replay_a_completed_routing_decision() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "repeat").unwrap();
    let input = request(vec![text(MessageRole::User, "repeat this input")]);
    let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    close_turn(
        &store,
        &a,
        "regular",
        "model",
        "profile",
        "a",
        Instant::now(),
    );
    let (b, snapshot) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    assert_ne!(a.key, b.key);
    assert_ne!(a.agent_turn_id, b.agent_turn_id);
    assert!(snapshot.previous_decision.is_none());
    assert!(
        snapshot.assessment_target.is_some(),
        "observed execution remains eligible for contextual evaluation"
    );
    store.abort(&b);
}

#[test]
fn a_shorter_context_records_its_boundary_without_inventing_compaction_or_inheritance() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "shortened-context").unwrap();
    let first = request(vec![
        text(MessageRole::User, "uncaptured past task"),
        text(MessageRole::Assistant, "uncaptured past output"),
        text(MessageRole::User, "actual A"),
    ]);
    let (a, _) = new_turn(scoped(&store, &scope, &first, &replay).unwrap());
    close_turn(
        &store,
        &a,
        "regular",
        "model",
        "profile",
        "a",
        Instant::now(),
    );
    let shortened = request(vec![text(MessageRole::User, "actual B")]);
    let (b, history) = new_turn(scoped(&store, &scope, &shortened, &replay).unwrap());
    assert_ne!(a.key, b.key);
    assert_eq!(
        history.assessment_target.as_ref().unwrap().through_turn_id,
        a.agent_turn_id
    );
    let completed = close_turn(
        &store,
        &b,
        "primary",
        "model-b",
        "profile-b",
        "b",
        Instant::now(),
    );
    assert_eq!(
        completed.opening_cause,
        hiroute_domain::AgentTurnOpeningCauseV1::HistoryShortened
    );
}

#[test]
fn exact_ingress_retry_retains_cancelled_branch_gap_without_reusing_its_decision() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "cancel-retry").unwrap();
    let input = request(vec![text(MessageRole::User, "A")]);
    let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    close_turn(&store, &a, "regular", "a", "profile", "a", Instant::now());
    let changed = request(vec![
        text(MessageRole::Developer, "Changed instructions"),
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "uncaptured output"),
    ]);
    let (cancelled, history) = new_turn(scoped(&store, &scope, &changed, &replay).unwrap());
    drop(history);
    store.abort(&cancelled);
    let (retry, history) = new_turn(scoped(&store, &scope, &changed, &replay).unwrap());
    assert_ne!(retry.key, cancelled.key);
    assert!(history.history_partial);
    assert!(history.previous_decision.is_none());
    assert!(
        history.assessment_target.is_none(),
        "a retry cannot heal a known gap"
    );
    store.abort(&retry);
}

#[test]
fn long_tool_phase_counts_terminal_requests_and_recovers_after_retirement() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "long-tool-phase").unwrap();
    let input = request(vec![text(MessageRole::User, "Continue the task")]);
    let mut phases = 0;
    for n in 0..160 {
        let begin = store
            .begin(scope.clone(), plan(1), &input, &replay, Instant::now())
            .unwrap();
        let ticket = match begin {
            AgentTurnBegin::NewTurn {
                ticket, history, ..
            } => {
                drop(history);
                phases += 1;
                store.commit_decision(&ticket, decision("regular")).unwrap();
                ticket
            }
            AgentTurnBegin::Continuation { ticket, .. } => ticket,
        };
        let id = format!("tool-request-{n}");
        store.bind_request(&ticket, &id);
        store
            .finish_request(
                &ticket,
                id.clone(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", &id)],
                false,
                Instant::now(),
            )
            .unwrap();
        assert!(store.inner.lock().retained_records(&scope) <= 128);
    }
    assert!(phases >= 2, "an unbounded active stage must be retired");
    let a = request(vec![text(MessageRole::User, "new A")]);
    let (a, history) = new_turn(scoped(&store, &scope, &a, &replay).unwrap());
    drop(history);
    close_turn(
        &store,
        &a,
        "regular",
        "model",
        "profile",
        "new-a",
        Instant::now(),
    );
    let b = request(vec![text(MessageRole::User, "new B")]);
    let (b, history) = new_turn(scoped(&store, &scope, &b, &replay).unwrap());
    assert_eq!(
        history.assessment_target.unwrap().through_turn_id,
        a.agent_turn_id
    );
    store.abort(&b);
}

#[test]
fn cached_decision_respects_budget_before_mutation() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(128 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "decision-budget").unwrap();
    let (ticket, history) = new_turn(
        scoped(
            &store,
            &scope,
            &request(vec![text(MessageRole::User, "A")]),
            &replay,
        )
        .unwrap(),
    );
    drop(history);
    let before = store.inner.lock().accounted_bytes;
    let mut oversized = decision("regular");
    oversized.strategy_id = "x".repeat(256 * 1024);
    assert!(matches!(
        store.commit_decision(&ticket, oversized),
        Err(AgentTurnHistoryError::Resource)
    ));
    assert_eq!(store.inner.lock().accounted_bytes, before);
    store.commit_decision(&ticket, decision("regular")).unwrap();
    let inner = store.inner.lock();
    assert!(inner.accounted_bytes > before);
    assert!(inner.accounted_bytes <= store.max_memory_bytes);
    drop(inner);
    store.abort(&ticket);
}

#[test]
fn colliding_gap_stripes_disclose_loss_and_allow_new_complete_pairs() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store.scope_key("workspace", "stripe-owner").unwrap();
    let collision = (0..100_000)
        .map(|n| store.scope_key("workspace", &format!("other-{n}")).unwrap())
        .find(|other| other.0[0] == scope.0[0] && *other != scope)
        .unwrap();
    let (old, _) = new_turn(
        scoped(
            &store,
            &collision,
            &request(vec![text(MessageRole::User, "old")]),
            &replay,
        )
        .unwrap(),
    );
    close_turn(
        &store,
        &old,
        "regular",
        "old",
        "profile",
        "old",
        Instant::now(),
    );
    store.with_lock_for_runtime_test(|| store.note_capture_gap(&scope));
    let (a, history) = new_turn(
        scoped(
            &store,
            &collision,
            &request(vec![text(MessageRole::User, "new A")]),
            &replay,
        )
        .unwrap(),
    );
    assert!(history.history_partial);
    assert!(history.assessment_target.is_none());
    drop(history);
    close_turn(
        &store,
        &a,
        "regular",
        "model",
        "profile",
        "new-a",
        Instant::now(),
    );
    let (b, history) = new_turn(
        scoped(
            &store,
            &collision,
            &request(vec![text(MessageRole::User, "new B")]),
            &replay,
        )
        .unwrap(),
    );
    let target = history.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, a.agent_turn_id);
    assert!(
        !target.target_partial,
        "old stripe uncertainty must not permanently disable evaluation"
    );
    store.abort(&b);
}

#[test]
fn captured_tool_boundary_inherits_matching_output_independently_of_model_setup() {
    use crate::server::core_runtime::model_ir::{
        CanonicalInstruction, InstructionRole, ModelEvent, ModelStreamEventV1, ResponseItemStatus,
    };
    for changed in [
        "none",
        "replay-arguments",
        "empty-placeholder",
        "replay-empty-placeholder",
        "accepted-empty-placeholder",
        "whitespace",
        "arguments",
        "instructions",
    ] {
        let inherits = !matches!(changed, "whitespace" | "arguments");
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
        let scope = store.scope_key("workspace", changed).unwrap();
        let input = request(vec![text(MessageRole::User, "A")]);
        let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
        store.commit_decision(&a, decision("regular")).unwrap();
        accepted_output::tool(&store, &a, "call", "lookup");
        store.accept_output(
            &a,
            &[ModelStreamEventV1::new(
                1,
                ModelEvent::ToolCallFinished {
                    encrypted_function_args: None,
                    index: 1,
                    logical_id: "call".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: Some("functions".into()),
                    name: "lookup".into(),
                    arguments: json!({"id": 7}),
                    status: ResponseItemStatus::Completed,
                },
            )],
        );
        if changed == "accepted-empty-placeholder" {
            store.accept_output(
                &a,
                &[ModelStreamEventV1::new(
                    2,
                    ModelEvent::TextFinished {
                        index: 0,
                        text: String::new(),
                        annotations: Vec::new(),
                        status: ResponseItemStatus::Completed,
                    },
                )],
            );
        }
        store
            .finish_request(
                &a,
                "a".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", "a")],
                false,
                Instant::now(),
            )
            .unwrap();
        let mut continued = request(vec![
            text(MessageRole::User, "A"),
            CanonicalMessage {
                role: MessageRole::Assistant,
                name: None,
                content: vec![ContentPart::ToolCall {
                    encrypted_function_args: None,
                    logical_id: "call".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: Some("functions".into()),
                    name: "lookup".into(),
                    arguments: json!({"id": if changed == "arguments" { 8 } else { 7 }}),
                    raw_arguments: None,
                }],
            },
            tool_result("call", false),
        ]);
        if changed == "replay-arguments" {
            let mut pool = replay.begin_content_pool().unwrap();
            let reference = pool.append_canonical_json(&json!({"id": 7})).unwrap();
            pool.seal().unwrap();
            if let ContentPart::ToolCall { arguments, .. } = &mut continued.messages[1].content[0] {
                *arguments = reference.json_marker();
            }
        }
        if matches!(
            changed,
            "empty-placeholder" | "replay-empty-placeholder" | "whitespace"
        ) {
            let text = if changed == "replay-empty-placeholder" {
                let mut pool = replay.begin_content_pool().unwrap();
                let reference = pool.append(b"").unwrap();
                pool.seal().unwrap();
                reference.wire_marker()
            } else if changed == "whitespace" {
                " ".into()
            } else {
                String::new()
            };
            continued.messages[1]
                .content
                .insert(0, ContentPart::Text { text });
        }
        if changed == "instructions" {
            continued.instructions.push(CanonicalInstruction {
                role: InstructionRole::System,
                content: vec![ContentPart::Text {
                    text: "new task constraints".into(),
                }],
            });
        }
        let begin = scoped(&store, &scope, &continued, &replay).unwrap();
        match begin {
            AgentTurnBegin::Continuation { ticket, decision } => {
                assert!(inherits, "changed content must not inherit: {changed}");
                assert_eq!(ticket.key, a.key);
                assert_eq!(decision.branch_id, "regular");
                store.abort(&ticket);
            }
            AgentTurnBegin::NewTurn {
                ticket, history, ..
            } => {
                assert!(
                    !inherits,
                    "a captured exact tool receipt should inherit: {changed}"
                );
                assert_ne!(ticket.key, a.key);
                assert!(history.previous_decision.is_none());
                assert!(
                    history.assessment_target.is_some(),
                    "changed setup still permits disclosed contextual evaluation"
                );
                store.abort(&ticket);
            }
        }
    }
}
