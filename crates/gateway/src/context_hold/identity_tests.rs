use super::*;
use crate::context_hold::NativeUserOrigin;
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl,
    ResponsesRequestOptionsV1, ToolChoice,
};

#[test]
fn conflicting_native_identity_loses_association_without_disabling_later_requests() {
    let mut identity = reliable(
        "codex_thread_session",
        vec!["child".into(), "session".into()],
        false,
    );
    identity.apply_routing(super::super::routing::RoutingFacts {
        identity_conflict: true,
        root: Some("untrusted-root".into()),
        native: super::super::routing::NativeCheckpoint {
            turn: Some([1; 32]),
            branch_leaf: Some([2; 32]),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(identity.session_scope(), "request_scoped");
    assert!(identity.hold_identity().is_none());
    assert!(identity.observation_identity().is_none());
    assert!(identity.observed_routing_lane());
    assert!(!identity.requires_captured_input());
    assert!(identity.routing.track);
    assert!(identity.native_checkpoint().turn.is_none());
    assert!(identity.native_checkpoint().branch_leaf.is_none());

    // Missing optional proof must not invalidate the next independent request.
    let mut recovered = reliable(
        "codex_thread_session",
        vec!["child".into(), "session".into()],
        false,
    );
    recovered.apply_routing(super::super::routing::RoutingFacts {
        native: super::super::routing::NativeCheckpoint {
            user_origin: NativeUserOrigin::Observed,
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(recovered.session_scope(), "conversation");
    assert!(recovered.hold_identity().is_some());
    assert!(recovered.observed_routing_lane());
}

#[test]
fn worker_continue_keeps_conversation_identity_without_joining_other_tasks() {
    let context = |task: &str, run: &str, native: Option<&str>| {
        crate::server::request_plan::VerifiedRunObservationContext {
            task_id: task.into(),
            run_id: run.into(),
            plan_id: "plan/one".into(),
            plan_revision: 1,
            publication_ref: "publication/one".into(),
            harness_id: "pi".into(),
            native_session_id: native.map(str::to_owned),
            parent_context_ref: None,
            continued_from_run_id: None,
        }
    };
    let initial = verified_worker_identity(&context("task/one", "run/initial", None));
    let continued =
        verified_worker_identity(&context("task/one", "run/continue", Some("native-one")));
    assert_eq!(initial.hold_identity(), continued.hold_identity());
    assert_eq!(
        initial.observation_identity(),
        continued.observation_identity()
    );
    assert_eq!(continued.correlation_provenance(), "protocol_state");
    assert_eq!(continued.session_scope(), "conversation");
    let other = verified_worker_identity(&context("task/two", "run/other", Some("native-one")));
    assert_ne!(initial.hold_identity(), other.hold_identity());
    assert_ne!(initial.observation_identity(), other.observation_identity());
    let invalid = verified_worker_identity(&context("", "run/valid", None));
    assert_eq!(invalid.session_scope(), "request_scoped");
}

fn managed_plan(consumer: Option<hiroute_domain::AgentModelConsumerV1>) -> AuthorizedRequestPlan {
    crate::server::publication::authorized_native_consumer_fixture(consumer)
}

#[test]
fn managed_hermes_requires_its_real_identity_and_never_infers_equal_content() {
    let authorized = managed_plan(Some(hiroute_domain::AgentModelConsumerV1::Hermes));
    assert!(authorized.is_hermes_consumer());
    let real_user = request("Equal real user content.");
    let mut headers = HeaderMap::new();
    for claims in [
        vec![],
        vec![("x-hermes-session-id", "")],
        vec![
            ("x-hermes-session-id", "one"),
            ("x-hermes-session-id", "one"),
        ],
        vec![("x-hermes-session-id", "one"), ("session-id", "one")],
        vec![("session-id", "other-client")],
    ] {
        headers.clear();
        for (name, value) in claims {
            headers.append(name, value.parse().unwrap());
        }
        let facts = identity_facts(
            &authorized,
            IngressProtocol::Responses,
            &headers,
            &serde_json::json!({}),
            &real_user,
            &[1; 32],
            &[2; 32],
        );
        assert_eq!(facts.session_scope(), "request_scoped");
    }
    headers.clear();
    headers.insert(
        "x-hermes-session-id",
        "one-real-native-session".parse().unwrap(),
    );
    let facts = identity_facts(
        &authorized,
        IngressProtocol::Responses,
        &headers,
        &serde_json::json!({}),
        &real_user,
        &[1; 32],
        &[2; 32],
    );
    assert_eq!(facts.hold_identity().unwrap().0, "hermes_session");
    assert_eq!(facts.session_scope(), "conversation");
    headers.insert("x-hermes-session-id", "a".repeat(129).parse().unwrap());
    let facts = identity_facts(
        &authorized,
        IngressProtocol::Responses,
        &headers,
        &serde_json::json!({}),
        &real_user,
        &[1; 32],
        &[2; 32],
    );
    assert_eq!(facts.session_scope(), "request_scoped");
}

#[test]
fn legacy_hermes_fresh_flag_cannot_prove_human_or_async_user_origin() {
    let authorized = managed_plan(Some(hiroute_domain::AgentModelConsumerV1::Hermes));
    for content in [
        "Actual new human question",
        "[ASYNC DELEGATION BATCH COMPLETE] child results",
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("x-hiroute-agent", "hermes".parse().unwrap());
        headers.insert("x-hermes-session-id", "parent".parse().unwrap());
        headers.insert(
            "x-hiroute-routing-context",
            serde_json::json!({"version":1,"agent":"hermes","session_id":"parent",
                "purpose":"main","turn_id":"new-turn",
                "continuation_requires_existing_turn":false})
            .to_string()
            .parse()
            .unwrap(),
        );
        let facts = identity_facts(
            &authorized,
            IngressProtocol::Responses,
            &headers,
            &serde_json::json!({}),
            &request(content),
            &[1; 32],
            &[2; 32],
        );
        assert_eq!(facts.session_scope(), "conversation");
        assert_eq!(
            facts.native_checkpoint().user_origin,
            NativeUserOrigin::Observed
        );
    }
}

#[test]
fn claimed_hermes_user_agent_does_not_establish_managed_consumer_authority() {
    let authorized = managed_plan(None);
    assert!(!authorized.is_hermes_consumer());
    let mut headers = HeaderMap::new();
    headers.insert("user-agent", "Hermes/0.19.1".parse().unwrap());
    let facts = identity_facts(
        &authorized,
        IngressProtocol::Responses,
        &headers,
        &serde_json::json!({}),
        &request("A complete real user prompt."),
        &[1; 32],
        &[2; 32],
    );
    assert_eq!(facts.hold_identity().unwrap().0, "inferred_first_user");
}

fn request(text: &str) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        native_body: None,
        native_only: false,
        native_history: None,
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        served_model_id: "agent/test".into(),
        stream: false,
        instructions: Vec::new(),
        messages: vec![CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text { text: text.into() }],
            name: None,
        }],
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
    }
}

#[test]
fn codex_requires_consistent_dimensions_and_keeps_thread_with_session() {
    let mut headers = HeaderMap::new();
    headers.insert("thread-id", "thread".parse().unwrap());
    headers.insert("session-id", "session".parse().unwrap());
    let mut request = request("hello");
    request.responses_options = Some(ResponsesRequestOptionsV1 {
        store: None,
        include: None,
        prompt_cache_key: None,
        client_metadata: Some(
            [
                ("thread_id".into(), "thread".into()),
                ("session_id".into(), "session".into()),
            ]
            .into_iter()
            .collect(),
        ),
        reasoning_summary: None,
        reasoning_context: None,
    });
    match codex_identity(IngressProtocol::Responses, &headers, &request) {
        Field::Value((kind, parts)) => {
            assert_eq!(kind, "codex_thread_session");
            assert_eq!(parts, ["thread", "session"]);
        }
        Field::Absent | Field::Invalid => panic!("consistent identity was not accepted"),
    }
    request
        .responses_options
        .as_mut()
        .unwrap()
        .client_metadata
        .as_mut()
        .unwrap()
        .insert("session_id".into(), "other".into());
    assert!(matches!(
        codex_identity(IngressProtocol::Responses, &headers, &request),
        Field::Invalid
    ));
    headers.append("thread-id", "duplicate".parse().unwrap());
    assert!(matches!(
        codex_identity(IngressProtocol::Responses, &headers, &request),
        Field::Invalid
    ));
}

#[test]
fn inferred_anchor_uses_complete_user_and_rejects_public_wrappers() {
    let key = [3; 32];
    let common = "x".repeat(4_096);
    assert_ne!(
        first_user_digest(&request(&(common.clone() + "a")), &key),
        first_user_digest(&request(&(common + "b")), &key)
    );
    for public in [
        "# AGENTS.md instructions for /tmp",
        "<environment_context>cwd</environment_context>",
        "<system-reminder>rules</system-reminder>",
        "<recommended_plugins>plugins</recommended_plugins>\n# AGENTS.md instructions for /tmp",
        "<send_user_message_question_reply>prior answer</send_user_message_question_reply>",
        "   ",
    ] {
        assert!(first_user_digest(&request(public), &key).is_none());
    }
}

fn native(
    headers: &[(&'static str, &str)],
    ingress: IngressProtocol,
    document: Value,
) -> Field<NativeIdentity> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    native_identity(
        ingress,
        &map,
        &document,
        &request("# AGENTS.md instructions"),
    )
}

fn assert_native(field: Field<NativeIdentity>, kind: &str, expected: &[&str]) {
    match field {
        Field::Value((actual, parts)) => {
            assert_eq!(actual, kind);
            assert_eq!(parts, expected);
        }
        Field::Absent | Field::Invalid => panic!("native identity was not accepted"),
    }
}

#[test]
fn native_header_validation_rejects_duplicates_empty_oversized_and_nontext_values() {
    for name in [
        "x-claude-code-session-id",
        "x-deepseek-harness-session-id",
        "x-opencode-session-id",
        "session-id",
    ] {
        for value in ["".to_owned(), "x".repeat(129)] {
            assert!(matches!(
                native(&[(name, &value)], IngressProtocol::Messages, Value::Null),
                Field::Invalid
            ));
        }
        assert!(matches!(
            native(
                &[(name, "same"), (name, "same")],
                IngressProtocol::Messages,
                Value::Null
            ),
            Field::Invalid
        ));
        let mut headers = HeaderMap::new();
        headers.insert(name, http::HeaderValue::from_bytes(&[0xff]).unwrap());
        assert!(matches!(
            native_identity(
                IngressProtocol::Messages,
                &headers,
                &Value::Null,
                &request("same")
            ),
            Field::Invalid
        ));
        assert!(valid_identity(&"x".repeat(128)).is_some());
        assert!(valid_identity("native\tbad").is_none());
        assert!(valid_identity(&"界".repeat(43)).is_none());
    }
}

#[test]
fn opencode_uses_actual_session_without_merging_parent_or_cache_affinity() {
    for ingress in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        assert_native(
            native(&[("X-OpenCode-Session-Id", "native")], ingress, Value::Null),
            "opencode_session",
            &["native"],
        );
        let hints = [
            ("user-agent", "opencode/1.19.0"),
            ("x-opencode-session", "shared-cache"),
            ("x-session-affinity", "shared-cache"),
            ("x-session-id", "shared-cache"),
            ("x-opencode-parent-session-id", "parent"),
            ("x-parent-session-id", "parent"),
            ("x-opencode-project", "project"),
            ("x-opencode-request", "request"),
        ];
        assert!(matches!(
            native(&hints, ingress, Value::Null),
            Field::Absent
        ));
        let mut headers = hints.to_vec();
        headers.push(("x-opencode-session-id", "child"));
        assert_native(
            native(&headers, ingress, Value::Null),
            "opencode_session",
            &["child"],
        );
    }
    // Pi's OpenCode provider also sends the routing header; it is not another
    // vendor's identity claim and must preserve Pi's accepted session namespace.
    assert_native(
        native(
            &[
                ("user-agent", "pi (linux; x64)"),
                ("x-client-request-id", "pi-session"),
                ("x-opencode-session", "pi-session"),
            ],
            IngressProtocol::Responses,
            Value::Null,
        ),
        "pi_session",
        &["pi-session"],
    );
}

#[test]
fn pi_ai_session_headers_are_scoped_to_client_and_protocol() {
    for (agent, kind) in [
        ("pi (darwin 25; arm64)", "pi_session"),
        (
            "deepseek-harness/0.2.0-rc.2 (+https://github.com/deepseek-ai/deepseek-harness)",
            "deepseek_harness_session",
        ),
    ] {
        for name in ["x-client-request-id", "session_id", "x-session-id"] {
            assert_native(
                native(
                    &[("user-agent", agent), (name, "native")],
                    IngressProtocol::Responses,
                    Value::Null,
                ),
                kind,
                &["native"],
            );
        }
        for name in ["x-session-affinity", "x-session-id"] {
            assert_native(
                native(
                    &[("user-agent", agent), (name, "native")],
                    IngressProtocol::Messages,
                    Value::Null,
                ),
                kind,
                &["native"],
            );
        }
        assert!(matches!(
            native(
                &[
                    ("user-agent", agent),
                    ("x-client-request-id", "per-request")
                ],
                IngressProtocol::Messages,
                Value::Null
            ),
            Field::Absent
        ));
        assert!(matches!(
            native(
                &[("user-agent", agent), ("x-session-affinity", "affinity")],
                IngressProtocol::Responses,
                Value::Null
            ),
            Field::Absent
        ));
        assert!(matches!(
            native(
                &[
                    ("user-agent", agent),
                    ("x-client-request-id", "a"),
                    ("session_id", "b")
                ],
                IngressProtocol::Responses,
                Value::Null
            ),
            Field::Invalid
        ));
        assert!(matches!(
            native(
                &[
                    ("user-agent", agent),
                    ("x-session-id", "a"),
                    ("x-session-id", "a")
                ],
                IngressProtocol::Responses,
                Value::Null
            ),
            Field::Invalid
        ));
    }
    for agent in ["other-client/1", "pi-other/1", "deepseek-harness/"] {
        assert!(matches!(
            native(
                &[
                    ("user-agent", agent),
                    ("x-client-request-id", "a"),
                    ("x-session-id", "a"),
                    ("session_id", "a")
                ],
                IngressProtocol::Responses,
                Value::Null
            ),
            Field::Absent
        ));
    }
    assert!(matches!(
        native(
            &[("x-client-request-id", "a"), ("x-session-affinity", "a")],
            IngressProtocol::Responses,
            Value::Null
        ),
        Field::Absent
    ));
}

#[test]
fn native_families_cannot_override_conflicts_or_discard_codex_dimensions() {
    assert_native(
        native(
            &[
                ("x-opencode-session-id", "native"),
                ("session-id", "native"),
                ("thread-id", "thread"),
            ],
            IngressProtocol::Responses,
            Value::Null,
        ),
        "codex_thread_session",
        &["thread", "native"],
    );
    assert_native(
        native(
            &[
                ("user-agent", "pi (linux; x64)"),
                ("session-id", "native"),
                ("thread-id", "thread"),
                ("x-client-request-id", "native"),
            ],
            IngressProtocol::Responses,
            Value::Null,
        ),
        "codex_thread_session",
        &["thread", "native"],
    );
    assert_native(
        native(
            &[
                ("x-deepseek-harness-session-id", "native"),
                ("user-agent", "deepseek-harness/0.2"),
                ("x-client-request-id", "native"),
            ],
            IngressProtocol::Responses,
            Value::Null,
        ),
        "deepseek_harness_session",
        &["native"],
    );
    for headers in [
        vec![("session-id", "a"), ("x-claude-code-session-id", "b")],
        vec![("session-id", "a"), ("x-opencode-session-id", "b")],
        vec![
            ("x-claude-code-session-id", "a"),
            ("x-opencode-session-id", "a"),
        ],
        vec![
            ("x-deepseek-harness-session-id", "a"),
            ("x-opencode-session-id", "a"),
        ],
        vec![
            ("x-opencode-session-id", "a"),
            ("user-agent", "pi (browser)"),
            ("x-client-request-id", "a"),
        ],
        vec![
            ("x-claude-code-session-id", "a"),
            ("x-deepseek-harness-session-id", "a"),
        ],
        vec![
            ("x-deepseek-harness-session-id", "a"),
            ("user-agent", "pi (browser)"),
            ("x-client-request-id", "a"),
        ],
    ] {
        assert!(matches!(
            native(&headers, IngressProtocol::Responses, Value::Null),
            Field::Invalid
        ));
    }
}
