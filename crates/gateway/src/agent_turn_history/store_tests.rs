use super::*;
use crate::agent_turn_history::ToolStatus;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, MODEL_REQUEST_IR_SCHEMA, MessageRole, RequestedReasoningControl,
    ToolChoice, ToolKindV1, ToolOutput, ToolResultStatusV1,
};
use crate::server::core_runtime::profiles::ComplexityV1;
use crate::server::request_plan::IngressProtocol;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[path = "accepted_output_tests.rs"]
mod accepted_output;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "hiroute-agent-turn-test-{}-{sequence}",
            std::process::id()
        ));
        Self(path)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn replay() -> (TestRoot, ReplayStore) {
    let directory = TestRoot::new();
    let manager = ReplayManager::open(ReplayConfig {
        root: directory.0.join("replay"),
        memory_threshold_bytes: 2 * 1024 * 1024,
        record_bytes: 4 * 1024,
        orphan_ttl: Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024)
        .unwrap()
        .stream(16 * 1024 * 1024)
        .unwrap();
    let replay = manager.begin_request(budget).unwrap();
    (directory, replay)
}

fn request(messages: Vec<CanonicalMessage>) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        served_model_id: "agent/test".into(),
        stream: false,
        instructions: Vec::new(),
        messages,
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::Auto,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
    }
}

fn text(role: MessageRole, value: impl Into<String>) -> CanonicalMessage {
    CanonicalMessage {
        role,
        content: vec![ContentPart::Text { text: value.into() }],
        name: None,
    }
}

fn tool_call(id: &str, name: &str) -> CanonicalMessage {
    CanonicalMessage {
        role: MessageRole::Assistant,
        content: vec![ContentPart::ToolCall {
            logical_id: id.into(),
            tool_kind: ToolKindV1::Function,
            namespace: Some("functions".into()),
            name: name.into(),
            arguments: json!({"must_not_be_projected": "secret detail"}),
        }],
        name: None,
    }
}

fn tool_result(id: &str, failed: bool) -> CanonicalMessage {
    CanonicalMessage {
        role: MessageRole::User,
        content: vec![ContentPart::ToolResult {
            logical_id: id.into(),
            tool_kind: ToolKindV1::Function,
            output: ToolOutput::Text("must not be projected".into()),
            status: if failed {
                ToolResultStatusV1::Failed
            } else {
                ToolResultStatusV1::Completed
            },
        }],
        name: None,
    }
}

fn plan(revision: u64) -> PlanSnapshot {
    PlanSnapshot {
        plan_id: "plan/test".into(),
        plan_revision: revision,
    }
}

fn decision(branch: &str) -> BranchDecisionV1 {
    let strategy = ComplexityV1::compile(Vec::<(String, String)>::new()).unwrap();
    let (mut decision, _) = ComplexityV1::decide(Some("test"), None, &strategy).unwrap();
    decision.branch_id = branch.into();
    decision
}

fn execution(model: &str, profile: &str, branch: &str, request_id: &str) -> AcceptedExecution {
    AcceptedExecution {
        model_configuration_id: model.into(),
        profile_digest: profile.into(),
        executed_branch_id: branch.into(),
        request_id: request_id.into(),
    }
}

fn new_turn(begin: AgentTurnBegin) -> (AgentTurnTicket, AgentTurnHistorySnapshot) {
    match begin {
        AgentTurnBegin::NewTurn {
            ticket, history, ..
        } => (ticket, history),
        AgentTurnBegin::Continuation { .. } => panic!("expected a new turn"),
    }
}

fn close_turn(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    branch: &str,
    model: &str,
    profile: &str,
    request_id: &str,
    now: Instant,
) -> CompletedAgentTurn {
    store.commit_decision(ticket, decision(branch)).unwrap();
    store
        .finish_request(
            ticket,
            request_id.into(),
            AgentTurnStatus::Completed,
            vec![execution(model, profile, branch, request_id)],
            true,
            now,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn same_execution_extends_segment_and_mixed_does_not() {
    let single = execution_attribution(
        &[AcceptedExecution {
            model_configuration_id: "model-a".into(),
            profile_digest: "profile-a".into(),
            executed_branch_id: "simple".into(),
            request_id: "request-a".into(),
        }],
        "simple",
    );
    assert!(matches!(single, ExecutionAttribution::Single { .. }));
    let mixed = execution_attribution(
        &[
            AcceptedExecution {
                model_configuration_id: "model-a".into(),
                profile_digest: "profile-a".into(),
                executed_branch_id: "simple".into(),
                request_id: "request-a".into(),
            },
            AcceptedExecution {
                model_configuration_id: "model-b".into(),
                profile_digest: "profile-b".into(),
                executed_branch_id: "complex".into(),
                request_id: "request-b".into(),
            },
        ],
        "simple",
    );
    assert_eq!(mixed, ExecutionAttribution::Mixed);
}

#[test]
fn opaque_ids_do_not_expose_counter() {
    let first = opaque_id("turn", &[7; 32], 1).unwrap();
    let second = opaque_id("turn", &[7; 32], 2).unwrap();
    assert_ne!(first, second);
    assert!(!first.ends_with("-1"));
}

#[test]
fn tool_continuation_freezes_branch_and_next_user_starts_one_new_turn() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "session").unwrap();
    let now = Instant::now();
    let first_messages = vec![text(MessageRole::User, "Fix the failing test")];
    let (first, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(first_messages.clone()),
                &replay,
                now,
            )
            .unwrap(),
    );
    assert!(history.visible_conversation.is_empty());
    assert!(!history.history_partial);
    drop(history);
    store
        .commit_decision(&first, decision("smart_saving_simple"))
        .unwrap();
    accepted_output::tool(&store, &first, "call-1", "run_tests");
    store
        .finish_request(
            &first,
            "request-1".into(),
            AgentTurnStatus::Completed,
            vec![execution(
                "model-a",
                "profile-a",
                "smart_saving_simple",
                "request-1",
            )],
            false,
            now,
        )
        .unwrap();

    let continuation_messages = vec![
        first_messages[0].clone(),
        tool_call("call-1", "run_tests"),
        tool_result("call-1", true),
    ];
    let continuation = store
        .begin(
            key.clone(),
            plan(1),
            &request(continuation_messages.clone()),
            &replay,
            now + Duration::from_secs(1),
        )
        .unwrap();
    let continuation_ticket = match continuation {
        AgentTurnBegin::Continuation { ticket, decision } => {
            assert_eq!(decision.branch_id, "smart_saving_simple");
            ticket
        }
        AgentTurnBegin::NewTurn { .. } => panic!("tool result must continue the turn"),
    };
    accepted_output::text_output(&store, &continuation_ticket, "I could not fix it.");
    store
        .finish_request(
            &continuation_ticket,
            "request-2".into(),
            AgentTurnStatus::Completed,
            vec![execution(
                "model-a",
                "profile-a",
                "smart_saving_simple",
                "request-2",
            )],
            true,
            now + Duration::from_secs(1),
        )
        .unwrap();

    let next_messages = vec![
        continuation_messages[0].clone(),
        continuation_messages[1].clone(),
        continuation_messages[2].clone(),
        text(MessageRole::Assistant, "I could not fix it."),
        text(MessageRole::User, "Try a different approach"),
    ];
    let (next, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(next_messages.clone()),
                &replay,
                now + Duration::from_secs(2),
            )
            .unwrap(),
    );
    assert_eq!(history.visible_conversation.len(), 1);
    assert_eq!(history.assessment_from, Some(0));
    let encoded = serde_json::to_string(&history.visible_conversation).unwrap();
    assert!(encoded.contains("functions.run_tests"));
    assert!(encoded.contains("failed"));
    assert!(encoded.contains("I could not fix it."));
    assert!(!encoded.contains("secret detail"));
    assert!(!encoded.contains("must not be projected"));
    drop(history);
    close_turn(
        &store,
        &next,
        "smart_saving_complex",
        "model-b",
        "profile-b",
        "request-3",
        now + Duration::from_secs(2),
    );

    let mut repeated_messages = next_messages;
    repeated_messages.push(text(MessageRole::Assistant, "Done"));
    repeated_messages.push(text(MessageRole::User, "Try a different approach"));
    let (repeated, history) = new_turn(
        store
            .begin(
                key,
                plan(1),
                &request(repeated_messages),
                &replay,
                now + Duration::from_secs(3),
            )
            .unwrap(),
    );
    assert_ne!(repeated.agent_turn_id, next.agent_turn_id);
    assert_eq!(history.visible_conversation.len(), 2);
}

#[test]
fn reused_tool_id_updates_only_the_new_occurrence_from_appended_results() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "reused-tool").unwrap();
    let now = Instant::now();
    let mut messages = vec![text(MessageRole::User, "Run tools")];
    let (ticket, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(messages.clone()),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    store.commit_decision(&ticket, decision("simple")).unwrap();
    accepted_output::tool(&store, &ticket, "same", "first");
    let finish = |ticket: &AgentTurnTicket| {
        store
            .finish_request(
                ticket,
                "request".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "simple", "request")],
                false,
                now,
            )
            .unwrap();
    };
    let resume = |messages: &[CanonicalMessage]| match store
        .begin(
            key.clone(),
            plan(1),
            &request(messages.to_vec()),
            &replay,
            now,
        )
        .unwrap()
    {
        AgentTurnBegin::Continuation { ticket, .. } => ticket,
        _ => panic!("expected continuation"),
    };
    let statuses = || {
        let inner = store.inner.lock();
        inner.entries[&key]
            .active
            .steps
            .iter()
            .flatten()
            .filter_map(|part| match part {
                VisibleContentPart::ToolActivity { status, .. } => Some(*status),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    finish(&ticket);
    messages.extend([tool_call("same", "first"), tool_result("same", true)]);
    let second = resume(&messages);
    accepted_output::tool(&store, &second, "same", "second");
    finish(&second);
    assert_eq!(statuses(), vec![ToolStatus::Failed, ToolStatus::Unknown]);

    // Retrying the same transcript must not apply its old failed result to second.
    let retry = resume(&messages);
    assert_eq!(statuses(), vec![ToolStatus::Failed, ToolStatus::Unknown]);
    finish(&retry);
    messages.extend([tool_call("same", "second"), tool_result("same", false)]);
    let third = resume(&messages);
    assert_eq!(statuses(), vec![ToolStatus::Failed, ToolStatus::Completed]);
    accepted_output::tool(&store, &third, "ambiguous", "third");
    finish(&third);
    let fourth = resume(&messages);
    accepted_output::tool(&store, &fourth, "ambiguous", "fourth");
    finish(&fourth);
    messages.extend([
        tool_call("ambiguous", "fourth"),
        tool_result("ambiguous", false),
    ]);
    let fifth = resume(&messages);
    assert_eq!(
        statuses(),
        vec![
            ToolStatus::Failed,
            ToolStatus::Completed,
            ToolStatus::Unknown,
            ToolStatus::Unknown
        ]
    );
    accepted_output::tool(&store, &fifth, "ambiguous", "fifth");
    finish(&fifth);
    messages.extend([
        tool_call("ambiguous", "fifth"),
        tool_result("ambiguous", true),
    ]);
    let sixth = resume(&messages);
    assert_eq!(
        statuses(),
        vec![
            ToolStatus::Failed,
            ToolStatus::Completed,
            ToolStatus::Unknown,
            ToolStatus::Unknown,
            ToolStatus::Unknown
        ]
    );
    finish(&sixth);
}

#[test]
fn rebuilt_context_does_not_attribute_shifted_old_result_to_reused_id() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "shifted-tool-result").unwrap();
    let now = Instant::now();
    let user = text(MessageRole::User, "Run tools");
    let (first, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![user.clone()]),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    store.commit_decision(&first, decision("simple")).unwrap();
    accepted_output::tool(&store, &first, "same", "first");
    let finish = |ticket: &AgentTurnTicket| {
        store
            .finish_request(
                ticket,
                "request".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "simple", "request")],
                false,
                now,
            )
            .unwrap();
    };
    finish(&first);
    let old_messages = vec![
        user.clone(),
        tool_call("same", "first"),
        tool_result("same", true),
    ];
    let AgentTurnBegin::Continuation { ticket: second, .. } = store
        .begin(
            key.clone(),
            plan(1),
            &request(old_messages.clone()),
            &replay,
            now,
        )
        .unwrap()
    else {
        panic!("expected continuation");
    };
    accepted_output::tool(&store, &second, "same", "second");
    finish(&second);

    let mut rebuilt = vec![text(MessageRole::Developer, "New instructions")];
    rebuilt.extend(old_messages);
    let (_, history) = new_turn(
        store
            .begin_with_context(
                key,
                plan(1),
                &request(rebuilt),
                &replay,
                ContextDecisionFacts {
                    history_continues: false,
                    has_hold_preference: false,
                },
                now,
            )
            .unwrap(),
    );
    let statuses = history.visible_conversation[0]
        .steps
        .iter()
        .flatten()
        .filter_map(|part| match part {
            VisibleContentPart::ToolActivity { status, .. } => Some(*status),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(statuses, vec![ToolStatus::Failed, ToolStatus::Unknown]);
}

#[test]
fn context_boundary_reclassifies_same_user_and_preserves_unknown_execution() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store
        .scope_key("workspace", "context-boundary-tool")
        .unwrap();
    let now = Instant::now();
    let first_request = request(vec![text(MessageRole::User, "Fix the failing test")]);
    let (first, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &first_request, &replay, now)
            .unwrap(),
    );
    drop(history);
    store
        .commit_decision(&first, decision("smart_saving_simple"))
        .unwrap();
    accepted_output::tool(&store, &first, "call-1", "run_tests");
    store
        .finish_request(
            &first,
            "request-1".into(),
            AgentTurnStatus::Completed,
            vec![execution(
                "model-a",
                "profile-a",
                "smart_saving_simple",
                "request-1",
            )],
            false,
            now,
        )
        .unwrap();
    assert_eq!(
        store.inner.lock().entries[&key].active.last_request_status,
        AgentTurnStatus::Unknown
    );

    let rebuilt_messages = vec![
        text(MessageRole::Developer, "Changed retained instructions"),
        text(MessageRole::User, "Fix the failing test"),
        tool_result("call-1", true),
    ];
    let boundary = store
        .begin_with_context(
            key.clone(),
            plan(1),
            &request(rebuilt_messages.clone()),
            &replay,
            ContextDecisionFacts {
                history_continues: false,
                has_hold_preference: false,
            },
            now + Duration::from_secs(1),
        )
        .unwrap();
    let AgentTurnBegin::NewTurn {
        ticket,
        history,
        completed,
    } = boundary
    else {
        panic!("missing ContextHold preference must start a decision round")
    };
    assert_eq!(completed.unwrap().status, AgentTurnStatus::Unknown);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    let encoded = serde_json::to_string(&history.visible_conversation).unwrap();
    assert!(encoded.contains("functions.run_tests"));
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::ToolActivity {
            tool: "functions.run_tests".into(),
            status: ToolStatus::Unknown,
        }]]
    );
    assert_eq!(history.assessment_from, Some(0));
    assert!(!history.history_partial);
    drop(history);
    let cancelled_turn_id = ticket.agent_turn_id.clone();
    store.abort(&ticket);

    let retry = store
        .begin_with_context(
            key.clone(),
            plan(1),
            &request(rebuilt_messages),
            &replay,
            ContextDecisionFacts {
                history_continues: true,
                has_hold_preference: false,
            },
            now + Duration::from_secs(2),
        )
        .unwrap();
    let AgentTurnBegin::NewTurn {
        ticket: retry_ticket,
        history: retry_history,
        completed: retry_completed,
    } = retry
    else {
        panic!("a cancelled empty decision round must be replaceable")
    };
    assert!(
        retry_completed.is_none(),
        "the old round must not close twice"
    );
    assert_eq!(retry_history.visible_conversation.len(), 1);
    assert!(!retry_history.history_partial);
    assert_ne!(retry_ticket.agent_turn_id, cancelled_turn_id);
    assert_eq!(store.inner.lock().entries[&key].active.ordinal, 2);
    drop(retry_history);
    store.abort(&retry_ticket);
}

#[test]
fn missing_hold_preference_is_a_decision_boundary_even_when_history_continues() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "missing-hold").unwrap();
    let now = Instant::now();
    let input = request(vec![text(MessageRole::User, "Continue this task")]);
    let (first, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &input, &replay, now)
            .unwrap(),
    );
    drop(history);
    store
        .commit_decision(&first, decision("smart_saving_simple"))
        .unwrap();
    accepted_output::text_output(&store, &first, "work in progress");
    store
        .finish_request(
            &first,
            "request-1".into(),
            AgentTurnStatus::Completed,
            vec![execution(
                "model-a",
                "profile-a",
                "smart_saving_simple",
                "request-1",
            )],
            false,
            now,
        )
        .unwrap();

    let boundary = store
        .begin_with_context(
            key,
            plan(1),
            &input,
            &replay,
            ContextDecisionFacts {
                history_continues: true,
                has_hold_preference: false,
            },
            now + Duration::from_secs(1),
        )
        .unwrap();
    let AgentTurnBegin::NewTurn {
        ticket,
        history,
        completed,
    } = boundary
    else {
        panic!("hint=None must not inherit the active decision")
    };
    assert_eq!(completed.unwrap().status, AgentTurnStatus::Unknown);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    drop(history);
    store.abort(&ticket);
}

#[test]
fn segment_extends_only_for_the_same_executed_identity() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "segments").unwrap();
    let now = Instant::now();
    let mut messages = vec![text(MessageRole::User, "one")];
    let (first, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(7),
                &request(messages.clone()),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    let first = close_turn(
        &store,
        &first,
        "smart_saving_simple",
        "model-a",
        "profile-a",
        "request-1",
        now,
    );

    messages.extend([
        text(MessageRole::Assistant, "answer one"),
        text(MessageRole::User, "two"),
    ]);
    let (second, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(7),
                &request(messages.clone()),
                &replay,
                now + Duration::from_secs(1),
            )
            .unwrap(),
    );
    assert_eq!(
        history.assessment_target.as_ref().unwrap().segment_id,
        first.segment_id
    );
    drop(history);
    let second = close_turn(
        &store,
        &second,
        "smart_saving_simple",
        "model-a",
        "profile-a",
        "request-2",
        now + Duration::from_secs(1),
    );
    assert_eq!(second.segment_id, first.segment_id);

    messages.extend([
        text(MessageRole::Assistant, "answer two"),
        text(MessageRole::User, "three"),
    ]);
    let (third, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(7),
                &request(messages.clone()),
                &replay,
                now + Duration::from_secs(2),
            )
            .unwrap(),
    );
    assert_eq!(history.assessment_from, Some(0));
    assert_eq!(
        history.assessment_target.as_ref().unwrap().through_ordinal,
        2
    );
    drop(history);
    let third = close_turn(
        &store,
        &third,
        "smart_saving_simple",
        "model-b",
        "profile-a",
        "request-3",
        now + Duration::from_secs(2),
    );
    assert_ne!(third.segment_id, first.segment_id);

    messages.extend([
        text(MessageRole::Assistant, "answer three"),
        text(MessageRole::User, "four"),
    ]);
    let (fourth, history) = new_turn(
        store
            .begin(
                key,
                plan(8),
                &request(messages),
                &replay,
                now + Duration::from_secs(3),
            )
            .unwrap(),
    );
    assert_eq!(history.assessment_from, Some(2));
    drop(history);
    let fourth = close_turn(
        &store,
        &fourth,
        "smart_saving_simple",
        "model-b",
        "profile-a",
        "request-4",
        now + Duration::from_secs(3),
    );
    assert_ne!(fourth.segment_id, third.segment_id);
}

#[test]
fn pending_request_conflicts_and_stale_ticket_cannot_commit() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(64 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "conflict").unwrap();
    let now = Instant::now();
    let request = request(vec![text(MessageRole::User, "hello")]);
    let (first, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &request, &replay, now)
            .unwrap(),
    );
    drop(history);
    assert!(matches!(
        store.begin(key.clone(), plan(1), &request, &replay, now),
        Err(AgentTurnHistoryError::SessionTurnConflict)
    ));
    store.abort(&first);
    let (replacement, history) =
        new_turn(store.begin(key, plan(1), &request, &replay, now).unwrap());
    drop(history);
    assert_eq!(
        store.commit_decision(&first, decision("smart_saving_simple")),
        Err(AgentTurnHistoryError::TurnContextUnavailable)
    );
    store.abort(&replacement);
}

#[test]
fn ttl_discards_idle_session_and_marks_unobserved_prefix_partial() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(128 * 1024, Duration::from_secs(1));
    let key = store.scope_key("workspace", "ttl").unwrap();
    let now = Instant::now();
    let first_request = request(vec![text(MessageRole::User, "first")]);
    let (first, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &first_request, &replay, now)
            .unwrap(),
    );
    drop(history);
    close_turn(
        &store,
        &first,
        "smart_saving_simple",
        "model-a",
        "profile-a",
        "request-1",
        now,
    );
    let second_request = request(vec![
        text(MessageRole::User, "first"),
        text(MessageRole::Assistant, "answer"),
        text(MessageRole::User, "second"),
    ]);
    let (second, history) = new_turn(
        store
            .begin(
                key,
                plan(1),
                &second_request,
                &replay,
                now + Duration::from_secs(2),
            )
            .unwrap(),
    );
    assert_ne!(first.agent_turn_id, second.agent_turn_id);
    assert!(history.visible_conversation.is_empty());
    assert!(history.history_partial);
}

#[test]
fn oversized_current_input_is_not_truncated_by_history_budget() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(8 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "large").unwrap();
    let content = format!("begin-{}-end", "界".repeat(12_000));
    let request = request(vec![text(MessageRole::User, content.clone())]);
    let (ticket, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &request, &replay, Instant::now())
            .unwrap(),
    );
    assert!(history.visible_conversation.is_empty());
    let inner = store.inner.lock();
    let entry = inner.entries.get(&key).unwrap();
    assert!(entry.active.user.is_empty());
    assert!(entry.active.capture_partial);
    drop(inner);
    let projected = project_user(&request.messages[0], &replay).unwrap();
    assert_eq!(projected, vec![VisibleContentPart::Text { text: content }]);
    store.abort(&ticket);
}

#[test]
fn pinned_snapshot_prevents_lru_eviction_until_the_request_releases_it() {
    fn pinned_entry_bytes(
        store: &AgentTurnHistoryStore,
        replay: &ReplayStore,
        key: &AgentTurnHistoryKey,
        now: Instant,
    ) -> AgentTurnHistorySnapshot {
        let first_request = request(vec![text(MessageRole::User, "x".repeat(4_000))]);
        let (first, history) = new_turn(
            store
                .begin(key.clone(), plan(1), &first_request, replay, now)
                .unwrap(),
        );
        drop(history);
        close_turn(
            store,
            &first,
            "smart_saving_simple",
            "model-a",
            "profile-a",
            "request-a",
            now,
        );
        let second_request = request(vec![
            text(MessageRole::User, "x".repeat(4_000)),
            text(MessageRole::Assistant, "done"),
            text(MessageRole::User, "next"),
        ]);
        let (ticket, history) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &second_request,
                    replay,
                    now + Duration::from_millis(1),
                )
                .unwrap(),
        );
        store.abort(&ticket);
        history
    }

    let (_probe_directory, probe_replay) = replay();
    let probe = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let probe_key = probe.scope_key("workspace", "pinned").unwrap();
    let probe_history = pinned_entry_bytes(&probe, &probe_replay, &probe_key, Instant::now());
    let pinned_bytes = probe.inner.lock().entries[&probe_key].accounted_bytes;
    drop(probe_history);

    let other_probe = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let other_key = other_probe.scope_key("workspace", "other").unwrap();
    let (other_ticket, other_history) = new_turn(
        other_probe
            .begin(
                other_key.clone(),
                plan(1),
                &request(vec![text(MessageRole::User, "y".repeat(4_000))]),
                &probe_replay,
                Instant::now(),
            )
            .unwrap(),
    );
    drop(other_history);
    let other_bytes = other_probe.inner.lock().entries[&other_key].accounted_bytes;
    other_probe.abort(&other_ticket);

    let budget = store_fixed_bytes() + pinned_bytes.max(other_bytes) + 128;
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(budget, Duration::from_secs(60));
    let pinned_key = store.scope_key("workspace", "pinned").unwrap();
    let other_key = store.scope_key("workspace", "other").unwrap();
    let history = pinned_entry_bytes(&store, &replay, &pinned_key, Instant::now());
    let other_request = request(vec![text(MessageRole::User, "y".repeat(4_000))]);
    assert!(matches!(
        store.begin(
            other_key.clone(),
            plan(1),
            &other_request,
            &replay,
            Instant::now() + Duration::from_secs(1),
        ),
        Err(AgentTurnHistoryError::Resource)
    ));
    assert!(store.inner.lock().entries.contains_key(&pinned_key));

    drop(history);
    let (other_ticket, other_history) = new_turn(
        store
            .begin(
                other_key.clone(),
                plan(1),
                &other_request,
                &replay,
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap(),
    );
    drop(other_history);
    let inner = store.inner.lock();
    assert!(!inner.entries.contains_key(&pinned_key));
    assert!(inner.entries.contains_key(&other_key));
    assert!(inner.accounted_bytes <= budget);
    drop(inner);
    store.abort(&other_ticket);
}
