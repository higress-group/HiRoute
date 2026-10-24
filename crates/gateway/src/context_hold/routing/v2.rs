//! Current optional adapter envelope. Parse the minimal purpose independently
//! from enrichment; no malformed proof can change authentication or model output.
use super::*;
use serde_json::value::RawValue;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    agent: String,
    purpose: String,
    #[serde(default)]
    session_id: Option<Box<RawValue>>,
    #[serde(default)]
    turn_id: Option<Box<RawValue>>,
    #[serde(default)]
    request_id: Option<Box<RawValue>>,
    #[serde(default)]
    checkpoint: Option<Box<RawValue>>,
    #[serde(default)]
    branch: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_input")]
    input: Option<Box<RawValue>>,
    #[serde(default)]
    continuation: Option<Box<RawValue>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NativeInput {
    ObservedBatch { event_id: Option<String> },
    NativeMessages(NativeMessageWindow),
    ExistingTurn { event_id: String },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeMessageWindow {
    pub(crate) event_id: String,
    pub(crate) coverage: String,
    #[serde(deserialize_with = "message_objects")]
    pub(crate) messages: Vec<NativeUserMessage>,
    pub(crate) scope_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Branch {
    leaf_id: String,
    path_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Continuation {
    kind: String,
    from_turn_id: Option<String>,
    from_checkpoint: Option<String>,
    to_checkpoint: Option<String>,
}

fn object_json<T: serde::de::DeserializeOwned>(value: &str) -> Option<T> {
    // Derived structs also accept positional arrays. The wire contract requires
    // objects; decode their original bytes so duplicate fields still fail.
    if !value.trim_start().starts_with('{') {
        return None;
    }
    serde_json::from_str(value).ok()
}

fn present_input<'de, D>(deserializer: D) -> Result<Option<Box<RawValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Absent enrichment is correlation only. An explicit null is still an
    // invalid declaration, not evidence that the producer omitted the field.
    Box::<RawValue>::deserialize(deserializer).map(Some)
}

fn message_objects<'de, D>(deserializer: D) -> Result<Vec<NativeUserMessage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct MessageObject(NativeUserMessage);
    impl<'de> Deserialize<'de> for MessageObject {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct ObjectVisitor;
            impl<'de> serde::de::Visitor<'de> for ObjectVisitor {
                type Value = NativeUserMessage;

                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("a native message object")
                }

                fn visit_map<M: serde::de::MapAccess<'de>>(
                    self,
                    map: M,
                ) -> Result<Self::Value, M::Error> {
                    NativeUserMessage::deserialize(serde::de::value::MapAccessDeserializer::new(
                        map,
                    ))
                }
            }
            deserializer.deserialize_map(ObjectVisitor).map(Self)
        }
    }
    Vec::<MessageObject>::deserialize(deserializer)
        .map(|values| values.into_iter().map(|value| value.0).collect())
}

fn parsed(headers: &HeaderMap) -> Option<Envelope> {
    let Field::Value(value) = single_header(headers, "x-hiroute-routing-context", 4096) else {
        return None;
    };
    // A typed top-level deserializer rejects duplicate fields. Do not first
    // deserialize through Value, which would silently retain the last duplicate.
    object_json::<Envelope>(&value).filter(|e| e.version == 2)
}

fn optional_id(value: &Option<Box<RawValue>>) -> Option<[u8; 32]> {
    value
        .as_ref()
        .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
        .filter(|value| id(value))
        .map(|value| digest(&value))
}

fn known_auxiliary(purpose: &str) -> bool {
    matches!(
        purpose,
        "compaction" | "summary" | "title" | "session-title" | "prewarm"
    )
}

/// Decode just the minimum purpose. Optional fields may be malformed, but a
/// duplicate version/family/purpose still fails typed JSON deserialization.
pub(super) fn auxiliary_purpose(headers: &HeaderMap, agent: &str) -> bool {
    #[derive(Deserialize)]
    struct Purpose {
        version: u8,
        agent: String,
        purpose: String,
    }
    let Field::Value(value) = single_header(headers, "x-hiroute-routing-context", 4096) else {
        return false;
    };
    object_json::<Purpose>(&value).is_some_and(|p| {
        matches!(p.version, 1 | 2)
            && p.agent == agent
            && native_session_header(agent).is_some()
            && known_auxiliary(&p.purpose)
    })
}

fn native_session_header(agent: &str) -> Option<&'static str> {
    match agent {
        "pi" => Some("x-session-affinity"),
        "dsh" => Some("x-deepseek-harness-session-id"),
        "opencode" => Some("x-opencode-session-id"),
        "hermes" => Some("x-hermes-session-id"),
        _ => None,
    }
}

pub(super) fn routing_facts(
    headers: &HeaderMap,
    agent: &str,
    native: Option<(&str, &[String])>,
    worker: bool,
) -> Option<RoutingFacts> {
    let context = parsed(headers)?;
    use hiroute_diagnostics::event::{
        NativeAdapterDisposition as D, NativeAdapterEvidence, NativeRoutingAgent as A,
        NativeRoutingPurpose as P,
    };
    let evidence = NativeAdapterEvidence {
        version: 2,
        agent: match context.agent.as_str() {
            "pi" => A::Pi,
            "dsh" => A::Dsh,
            "opencode" => A::OpenCode,
            "hermes" => A::Hermes,
            _ => A::Other,
        },
        purpose: match context.purpose.as_str() {
            "main" => P::Main,
            "unknown" => P::Unknown,
            "title" => P::Title,
            "summary" => P::Summary,
            "compaction" => P::Compaction,
            _ => P::Other,
        },
        disposition: D::Applied,
    };
    // Keep the verdict from the actual consumer, not a second diagnostic parser.
    let observed = |mut facts: RoutingFacts, disposition| {
        facts.adapter = Some(NativeAdapterEvidence {
            disposition,
            ..evidence
        });
        Some(facts)
    };
    if context.agent != agent || native_session_header(agent).is_none() {
        return observed(RoutingFacts::uncertain(), D::AgentMismatch);
    }
    if known_auxiliary(&context.purpose) {
        return observed(RoutingFacts::detached(), D::Auxiliary);
    }
    // Unknown purpose is usable observed traffic, never a reason to disable routing.
    if !matches!(context.purpose.as_str(), "main" | "unknown") {
        return observed(RoutingFacts::uncertain(), D::UnsupportedPurpose);
    }
    let session = context
        .session_id
        .as_ref()
        .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
        .filter(|s| id(s));
    let Some(session) = session else {
        return observed(RoutingFacts::uncertain(), D::InvalidSession);
    };
    let consistent = if worker {
        matches!(header_value(headers, native_session_header(agent)?), Field::Value(value) if value == session)
    } else {
        let expected = match agent {
            "pi" => "pi_session",
            "dsh" => "deepseek_harness_session",
            "opencode" => "opencode_session",
            "hermes" => "hermes_session",
            _ => unreachable!(),
        };
        native.is_some_and(|(kind, parts)| {
            kind == expected && parts.first().is_some_and(|s| s == &session)
        })
    };
    if !consistent {
        return observed(RoutingFacts::uncertain(), D::SessionMismatch);
    }
    // A positive native synthetic-input marker limits only this request. It
    // grants no ancestry: a named predecessor/checkpoint cannot recover input
    // or a decision without an independently captured execution boundary.
    let user_origin = if matches!(
        native_input(headers),
        Some(Ok(Some(NativeInput::ExistingTurn { .. })))
    ) {
        NativeUserOrigin::ExistingTurn
    } else {
        NativeUserOrigin::Observed
    };
    let branch = context
        .branch
        .and_then(|v| object_json::<Branch>(v.get()))
        .filter(|b| id(&b.leaf_id) && sha256(&b.path_digest));
    let continuation = context
        .continuation
        .and_then(|v| object_json::<Continuation>(v.get()))
        .filter(|c| {
            matches!(c.kind.as_str(), "same_turn" | "compression")
                && [&c.from_turn_id, &c.from_checkpoint, &c.to_checkpoint]
                    .into_iter()
                    .all(|v| v.as_deref().is_none_or(id))
        });
    // Producer request IDs are advisory correlation only. Never use one as a
    // key for admission, deduplication or a claim about a completed execution.
    let _request_id = optional_id(&context.request_id);
    observed(
        RoutingFacts {
            native: NativeCheckpoint {
                turn: optional_id(&context.turn_id),
                checkpoint: optional_id(&context.checkpoint),
                compacted: continuation.is_some_and(|c| c.kind == "compression"),
                branch_leaf: branch.as_ref().map(|b| digest(&b.leaf_id)),
                branch_path: branch.as_ref().map(|b| digest(&b.path_digest)),
                user_origin,
                ..Default::default()
            },
            ..Default::default()
        },
        D::Applied,
    )
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Shape validation only. The final wire envelope digest is checked by the
/// input projection owner before any position is admitted as current input.
/// A valid envelope may omit input enrichment; `Ok(None)` preserves that
/// distinction from both an absent envelope and an invalid input declaration.
pub(crate) fn native_input(headers: &HeaderMap) -> Option<Result<Option<NativeInput>, ()>> {
    let context = parsed(headers)?;
    let brand = match header_value(headers, "x-hiroute-agent") {
        Field::Value(value) => value,
        _ => return Some(Err(())),
    };
    if brand != context.agent
        || context.purpose != "main"
        || native_session_header(&brand).is_none()
    {
        return Some(Err(()));
    }
    let session = context
        .session_id
        .as_ref()
        .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
        .filter(|s| id(s));
    if !matches!((session, header_value(headers, native_session_header(&brand)?)),
        (Some(session), Field::Value(native)) if session == native)
    {
        return Some(Err(()));
    }
    let turn = optional_id(&context.turn_id);
    let Some(input) = context.input else {
        return Some(Ok(None));
    };
    let input: NativeInput = match object_json(input.get()) {
        Some(input) => input,
        None => return Some(Err(())),
    };
    let valid = match &input {
        NativeInput::ObservedBatch { event_id } => event_id.as_deref().is_none_or(id),
        NativeInput::ExistingTurn { event_id } => id(event_id) && turn == Some(digest(event_id)),
        NativeInput::NativeMessages(window) => {
            id(&window.event_id)
                && window.coverage == "window"
                && sha256(&window.scope_digest)
                && !window.messages.is_empty()
                && window.messages.len() <= 32
                && window
                    .messages
                    .iter()
                    .all(|message| message.index < 128 && sha256(&message.sha256))
                && window
                    .messages
                    .windows(2)
                    .all(|pair| pair[0].index < pair[1].index)
        }
    };
    Some(if valid { Ok(Some(input)) } else { Err(()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(value: Value) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "x-hiroute-routing-context",
            value.to_string().parse().unwrap(),
        );
        h.insert("x-hiroute-agent", "pi".parse().unwrap());
        h
    }

    #[test]
    fn auxiliary_purpose_survives_invalid_optional_enrichment() {
        let h = headers(json!({"version":2,"agent":"pi","purpose":"summary",
            "input":{"kind":"native_messages","messages":false},"session_id":false,"turn_id":[]}));
        assert!(!routing_facts(&h, "pi", None, false).unwrap().track);
        assert!(routing_facts(&h, "dsh", None, false).unwrap().track);
    }

    #[test]
    fn duplicate_fields_and_future_versions_do_not_gain_purpose_authority() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-hiroute-routing-context",
            r#"{"version":2,"agent":"pi","purpose":"main","purpose":"summary"}"#
                .parse()
                .unwrap(),
        );
        assert!(parsed(&h).is_none());
        let h = headers(json!({"version":3,"agent":"pi","purpose":"summary"}));
        assert!(parsed(&h).is_none());
    }

    #[test]
    fn positional_arrays_cannot_supply_envelope_or_minimal_auxiliary_purpose() {
        for value in [
            json!([2, "pi", "summary"]),
            json!([1, "pi", "summary"]),
            json!([
                2, "pi", "main", "session", "turn", null, null, null, null, null
            ]),
            Value::Null,
            json!("summary"),
        ] {
            let h = headers(value);
            assert!(parsed(&h).is_none());
            assert!(!auxiliary_purpose(&h, "pi"));
            assert!(native_input(&h).is_none());
        }
    }

    #[test]
    fn positional_nested_evidence_is_discarded_without_losing_valid_turn() {
        let h = headers(json!({"version":2,"agent":"pi","purpose":"main",
            "session_id":"session","turn_id":"turn",
            "branch":["leaf","a".repeat(64)],
            "continuation":["compression",null,null,null]}));
        let facts =
            routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false).unwrap();
        assert!(facts.track);
        assert_eq!(facts.native.turn, Some(digest("turn")));
        assert!(facts.native.branch_leaf.is_none());
        assert!(!facts.native.compacted);
    }

    #[test]
    fn window_positions_require_objects_and_preserve_duplicate_field_rejection() {
        let mut valid = headers(json!({"version":2,"agent":"pi","purpose":"main",
            "session_id":"session","turn_id":"turn",
            "input":{"kind":"native_messages","event_id":"event","coverage":"window",
                     "messages":[{"index":0,"sha256":"a".repeat(64)}],
                     "scope_digest":"b".repeat(64)}}));
        valid.insert("x-session-affinity", "session".parse().unwrap());
        assert!(matches!(
            native_input(&valid),
            Some(Ok(Some(NativeInput::NativeMessages(_))))
        ));
        for message in [
            format!(r#"[0,"{}"]"#, "a".repeat(64)),
            format!(r#"{{"index":0,"index":0,"sha256":"{}"}}"#, "a".repeat(64)),
        ] {
            let value = format!(
                r#"{{"version":2,"agent":"pi","purpose":"main","session_id":"session","turn_id":"turn","input":{{"kind":"native_messages","event_id":"event","coverage":"window","messages":[{message}],"scope_digest":"{}"}}}}"#,
                "b".repeat(64)
            );
            let mut h = HeaderMap::new();
            h.insert("x-hiroute-agent", "pi".parse().unwrap());
            h.insert("x-session-affinity", "session".parse().unwrap());
            h.insert("x-hiroute-routing-context", value.parse().unwrap());
            assert!(matches!(native_input(&h), Some(Err(()))));
            assert_eq!(
                routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false)
                    .unwrap()
                    .native
                    .turn,
                Some(digest("turn"))
            );
        }
    }

    #[test]
    fn invalid_window_keeps_the_valid_session_and_turn() {
        let h = headers(
            json!({"version":2,"agent":"pi","purpose":"main","session_id":"session", "turn_id":"turn",
            "input":{"kind":"native_messages","event_id":"event","coverage":"window","messages":[],"scope_digest":"bad"}}),
        );
        let facts =
            routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false).unwrap();
        assert!(facts.track);
        assert_eq!(facts.native.turn, Some(digest("turn")));
        assert!(matches!(native_input(&h), Some(Err(()))));
    }

    #[test]
    fn purpose_is_independent_of_v1_and_v2_enrichment_and_never_mined_from_invalid_json() {
        for version in [1, 2] {
            let h = headers(json!({"version":version,"agent":"pi","purpose":"summary",
                "session_id":[],"user_turns":false,"input":{"messages":42}}));
            assert!(auxiliary_purpose(&h, "pi"));
            assert!(!auxiliary_purpose(&h, "dsh"));
        }
        for value in [
            r#"{"version":2,"agent":"pi","purpose":"summary","purpose":"main"}"#,
            r#"{"version":2,"agent":"pi","purpose":"summary",}"#,
            r#"{"version":3,"agent":"pi","purpose":"summary"}"#,
        ] {
            let mut h = HeaderMap::new();
            h.insert("x-hiroute-routing-context", value.parse().unwrap());
            assert!(!auxiliary_purpose(&h, "pi"));
        }
    }

    #[test]
    fn duplicate_nested_input_fields_reject_projection_without_losing_purpose_or_turn() {
        let mut h = HeaderMap::new();
        h.insert("x-hiroute-agent", "pi".parse().unwrap());
        h.insert("x-session-affinity", "session".parse().unwrap());
        h.insert("x-hiroute-routing-context", r#"{"version":2,"agent":"pi","purpose":"main","session_id":"session","turn_id":"turn","input":{"kind":"observed_batch","event_id":"one","event_id":"two"}}"#.parse().unwrap());
        assert!(matches!(native_input(&h), Some(Err(()))));
        assert_eq!(
            routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false)
                .unwrap()
                .native
                .turn,
            Some(digest("turn"))
        );
    }

    #[test]
    fn absent_input_is_distinct_from_explicit_null_and_duplicate_declarations() {
        for (agent, native_header) in [
            ("pi", "x-session-affinity"),
            ("dsh", "x-deepseek-harness-session-id"),
            ("opencode", "x-opencode-session-id"),
            ("hermes", "x-hermes-session-id"),
        ] {
            let base = json!({"version":2,"agent":agent,"purpose":"main",
                "session_id":"session","turn_id":"turn"});
            let mut h = headers(base.clone());
            h.insert("x-hiroute-agent", agent.parse().unwrap());
            h.insert(native_header, "session".parse().unwrap());
            assert!(matches!(native_input(&h), Some(Ok(None))));
            let mut null = base.clone();
            null["input"] = Value::Null;
            h.insert(
                "x-hiroute-routing-context",
                null.to_string().parse().unwrap(),
            );
            assert!(matches!(native_input(&h), Some(Err(()))));
            let mut duplicate = base.to_string();
            duplicate.pop();
            duplicate.push_str(r#", "input":null,"input":{"kind":"observed_batch"}}"#);
            h.insert("x-hiroute-routing-context", duplicate.parse().unwrap());
            assert!(parsed(&h).is_none());
        }
    }

    #[test]
    fn existing_turn_is_a_bound_main_input_guard_not_a_history_claim() {
        for (agent, native_header, native_kind) in [
            ("pi", "x-session-affinity", "pi_session"),
            (
                "dsh",
                "x-deepseek-harness-session-id",
                "deepseek_harness_session",
            ),
            ("opencode", "x-opencode-session-id", "opencode_session"),
            ("hermes", "x-hermes-session-id", "hermes_session"),
        ] {
            let mut h = headers(json!({"version":2,"agent":agent,"purpose":"main",
                "session_id":"session","turn_id":"current",
                "input":{"kind":"existing_turn","event_id":"current"},
                "branch":false,"continuation":{"kind":"compression","from_turn_id":"unproved"}}));
            h.insert("x-hiroute-agent", agent.parse().unwrap());
            h.insert(native_header, "session".parse().unwrap());
            for worker in [false, true] {
                let facts =
                    routing_facts(&h, agent, Some((native_kind, &["session".into()])), worker)
                        .unwrap();
                assert!(facts.track);
                assert_eq!(facts.native.user_origin, NativeUserOrigin::ExistingTurn);
                assert_eq!(facts.native.turn, Some(digest("current")));
                assert!(facts.native.branch_leaf.is_none());
            }
            h.insert(native_header, "other".parse().unwrap());
            assert!(matches!(native_input(&h), Some(Err(()))));
            assert_ne!(
                routing_facts(&h, agent, Some((native_kind, &["session".into()])), false)
                    .unwrap()
                    .native
                    .user_origin,
                NativeUserOrigin::ExistingTurn
            );
        }
    }

    #[test]
    fn invalid_existing_turn_and_compression_cursor_alone_keep_observed_routing() {
        for input in [
            json!({"kind":"observed_batch","event_id":"current"}),
            json!({"kind":"existing_turn","event_id":"old"}),
            json!({"kind":"existing_turn","event_id":""}),
            json!({"kind":"existing_turn","event_id":"x".repeat(129)}),
            json!({"kind":"existing_turn","event_id":true}),
            json!({"kind":"existing_turn"}),
            json!({"kind":"existing_turn","event_id":"current","unexpected":true}),
            json!(["existing_turn", "current"]),
            Value::Null,
        ] {
            let mut h = headers(json!({"version":2,"agent":"pi","purpose":"main",
                "session_id":"session","turn_id":"current","checkpoint":"summary",
                "continuation":{"kind":"compression","from_turn_id":"old"},"input":input}));
            h.insert("x-session-affinity", "session".parse().unwrap());
            let facts =
                routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false).unwrap();
            assert!(facts.track && facts.native.compacted);
            assert_eq!(facts.native.turn, Some(digest("current")));
            assert_eq!(
                facts.native.user_origin,
                NativeUserOrigin::Observed,
                "{input}"
            );
        }
    }

    #[test]
    fn existing_turn_does_not_override_auxiliary_or_unknown_purpose() {
        for purpose in ["summary", "unknown"] {
            let mut h = headers(json!({"version":2,"agent":"pi","purpose":purpose,
                "session_id":"session","turn_id":"current",
                "input":{"kind":"existing_turn","event_id":"current"}}));
            h.insert("x-session-affinity", "session".parse().unwrap());
            let facts =
                routing_facts(&h, "pi", Some(("pi_session", &["session".into()])), false).unwrap();
            assert_eq!(facts.track, purpose == "unknown");
            assert!(matches!(native_input(&h), Some(Err(()))));
            if purpose == "unknown" {
                assert_eq!(facts.native.user_origin, NativeUserOrigin::Observed);
            }
        }
    }
}
