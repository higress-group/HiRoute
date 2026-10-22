//! Current optional adapter envelope. Parse the minimal purpose independently
//! from enrichment; no malformed proof can change authentication or model output.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    agent: String,
    purpose: String,
    #[serde(default)]
    session_id: Option<Value>,
    #[serde(default)]
    turn_id: Option<Value>,
    #[serde(default)]
    request_id: Option<Value>,
    #[serde(default)]
    checkpoint: Option<Value>,
    #[serde(default)]
    branch: Option<Value>,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    continuation: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NativeInput {
    ObservedBatch { event_id: Option<String> },
    NativeMessages(NativeMessageWindow),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeMessageWindow {
    pub(crate) event_id: String,
    pub(crate) coverage: String,
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

fn parsed(headers: &HeaderMap) -> Option<Envelope> {
    let Field::Value(value) = single_header(headers, "x-hiroute-routing-context", 4096) else {
        return None;
    };
    // A typed top-level deserializer rejects duplicate fields. Do not first
    // deserialize through Value, which would silently retain the last duplicate.
    serde_json::from_str::<Envelope>(&value)
        .ok()
        .filter(|e| e.version == 2)
}

fn optional_id(value: &Option<Value>) -> Option<[u8; 32]> {
    value
        .as_ref()?
        .as_str()
        .filter(|value| id(value))
        .map(digest)
}

fn known_auxiliary(purpose: &str) -> bool {
    matches!(
        purpose,
        "compaction" | "summary" | "title" | "session-title" | "prewarm"
    )
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
    if context.agent != agent || native_session_header(agent).is_none() {
        return Some(RoutingFacts::uncertain());
    }
    if known_auxiliary(&context.purpose) {
        return Some(RoutingFacts::detached());
    }
    // Unknown purpose is usable observed traffic, never a reason to disable routing.
    if !matches!(context.purpose.as_str(), "main" | "unknown") {
        return Some(RoutingFacts::uncertain());
    }
    let session = context
        .session_id
        .as_ref()
        .and_then(Value::as_str)
        .filter(|s| id(s));
    let Some(session) = session else {
        return Some(RoutingFacts::uncertain());
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
            kind == expected && parts.first().is_some_and(|s| s == session)
        })
    };
    if !consistent {
        return Some(RoutingFacts::uncertain());
    }
    let branch = context
        .branch
        .and_then(|v| serde_json::from_value::<Branch>(v).ok())
        .filter(|b| id(&b.leaf_id) && sha256(&b.path_digest));
    let continuation = context
        .continuation
        .and_then(|v| serde_json::from_value::<Continuation>(v).ok())
        .filter(|c| {
            matches!(c.kind.as_str(), "same_turn" | "compression")
                && [&c.from_turn_id, &c.from_checkpoint, &c.to_checkpoint]
                    .into_iter()
                    .all(|v| v.as_deref().is_none_or(id))
        });
    // Producer request IDs are advisory correlation only. Never use one as a
    // key for admission, deduplication or a claim about a completed execution.
    let _request_id = optional_id(&context.request_id);
    Some(RoutingFacts {
        native: NativeCheckpoint {
            turn: optional_id(&context.turn_id),
            checkpoint: optional_id(&context.checkpoint),
            compacted: continuation.is_some_and(|c| c.kind == "compression"),
            branch_leaf: branch.as_ref().map(|b| digest(&b.leaf_id)),
            branch_path: branch.as_ref().map(|b| digest(&b.path_digest)),
            user_origin: NativeUserOrigin::Observed,
            ..Default::default()
        },
        ..Default::default()
    })
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Shape validation only. The final wire envelope digest is checked by the
/// input projection owner before any position is admitted as current input.
pub(crate) fn native_input(headers: &HeaderMap) -> Option<Result<NativeInput, ()>> {
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
    let input: NativeInput = match context
        .input
        .and_then(|value| serde_json::from_value(value).ok())
    {
        Some(input) => input,
        None => return Some(Err(())),
    };
    let valid = match &input {
        NativeInput::ObservedBatch { event_id } => event_id.as_deref().is_none_or(id),
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
    Some(if valid { Ok(input) } else { Err(()) })
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
}
