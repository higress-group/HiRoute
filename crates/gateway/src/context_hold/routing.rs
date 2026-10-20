//! Optional native routing evidence. It never grants execution authority.
use super::identity::{ContextIdentityFacts, Field, header_value, single_header};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativeCheckpoint {
    pub(crate) turn: Option<[u8; 32]>,
    pub(crate) checkpoint: Option<[u8; 32]>,
    pub(crate) compacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RoutingFacts {
    pub(super) track: bool,
    pub(super) native: NativeCheckpoint,
    pub(super) root: Option<String>,
}

impl Default for RoutingFacts {
    fn default() -> Self {
        Self {
            track: true,
            native: NativeCheckpoint::default(),
            root: None,
        }
    }
}

impl RoutingFacts {
    fn detached() -> Self {
        Self {
            track: false,
            ..Self::default()
        }
    }
}

/// Versioned metadata emitted by the bundled native adapters. Invalid metadata is
/// an optimization miss, not an invalid model request. IDs remain grant-scoped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeContext {
    version: u8,
    agent: String,
    session_id: String,
    purpose: String,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    checkpoint: Option<String>,
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(super) fn facts(
    identity: &ContextIdentityFacts,
    headers: &HeaderMap,
    document: &Value,
    worker: Option<&str>,
    hermes: bool,
) -> RoutingFacts {
    let native_identity = identity.observation_identity();
    let native_kind = native_identity.map(|(kind, _)| kind);
    let brand = match single_header(headers, "x-hiroute-agent", 32) {
        Field::Value(brand) => Some(brand),
        Field::Absent => None,
        Field::Invalid => return RoutingFacts::detached(),
    };
    let user_agent = match single_header(headers, "user-agent", 1024) {
        Field::Value(value) => value.to_ascii_lowercase(),
        _ => String::new(),
    };
    let agent = worker.or(brand.as_deref()).or(match native_kind {
        Some("claude_metadata_session" | "claude_native_agent") => Some("claude"),
        Some("pi_session") => Some("pi"),
        Some("deepseek_harness_session") => Some("dsh"),
        Some("opencode_session") => Some("opencode"),
        Some("hermes_session") if hermes => Some("hermes"),
        _ if user_agent.contains("qoder") => Some("qoder"),
        _ if headers.contains_key("x-codex-turn-metadata")
            || document
                .get("client_metadata")
                .and_then(|v| v.get("x-codex-turn-metadata"))
                .is_some()
            || user_agent.starts_with("codex") =>
        {
            Some("codex")
        }
        _ => None,
    });
    let Some(agent) = agent else {
        return RoutingFacts::default();
    };
    if brand.as_deref().is_some_and(|brand| brand != agent) {
        return RoutingFacts::detached();
    }
    if agent == "qoder" {
        return RoutingFacts::detached();
    }
    if agent == "codex" {
        return codex(headers, document);
    }
    if agent == "claude" {
        let purpose = header_value(headers, "x-claude-code-request-class");
        if !matches!(&purpose, Field::Value(value) if matches!(value.as_str(), "main" | "subagent"))
        {
            return RoutingFacts::detached();
        }
        let turn = match header_value(headers, "x-claude-code-prompt-id") {
            Field::Value(value) => Some(digest(&value)),
            Field::Absent => None,
            Field::Invalid => return RoutingFacts::detached(),
        };
        let compacted = match header_value(headers, "x-claude-code-context-compacted") {
            Field::Absent => false,
            Field::Value(value) if matches!(value.as_str(), "manual" | "auto") => true,
            _ => return RoutingFacts::detached(),
        };
        return RoutingFacts {
            native: NativeCheckpoint {
                turn,
                compacted,
                checkpoint: None,
            },
            ..RoutingFacts::default()
        };
    }
    if worker == Some("opencode") {
        match header_value(headers, "x-hiroute-request-purpose") {
            Field::Value(value) if value == "main" => return RoutingFacts::default(),
            Field::Value(_) | Field::Invalid => return RoutingFacts::detached(),
            Field::Absent => {}
        }
    }
    let context = match single_header(headers, "x-hiroute-routing-context", 4096) {
        Field::Value(value) => serde_json::from_str::<NativeContext>(&value).ok(),
        Field::Absent if agent == "hermes" && hermes => {
            // Older managed Hermes hooks only enter the selected conversation
            // call site. Its independent auxiliary clients lack this identity.
            return hermes_root(headers, native_identity, RoutingFacts::default());
        }
        Field::Absent | Field::Invalid => None,
    };
    let Some(context) = context else {
        return RoutingFacts::detached();
    };
    if context.version != 1
        || context.agent != agent
        || !id(&context.session_id)
        || context.purpose != "main"
        || context.turn_id.as_deref().is_some_and(|v| !id(v))
        || context.checkpoint.as_deref().is_some_and(|v| !id(v))
    {
        return RoutingFacts::detached();
    }
    // A plugin cannot silently rename the native conversation or another family.
    let expected = match agent {
        "pi" => "pi_session",
        "dsh" => "deepseek_harness_session",
        "opencode" => "opencode_session",
        "hermes" if hermes => "hermes_session",
        _ => return RoutingFacts::detached(),
    };
    let session_header = match agent {
        "pi" => "x-session-affinity",
        "dsh" => "x-deepseek-harness-session-id",
        "opencode" => "x-opencode-session-id",
        "hermes" => "x-hermes-session-id",
        _ => return RoutingFacts::detached(),
    };
    if worker.is_some() {
        if !matches!(header_value(headers, session_header), Field::Value(session) if session == context.session_id)
        {
            return RoutingFacts::detached();
        }
    } else if !native_identity
        .is_some_and(|(kind, parts)| kind == expected && parts.first() == Some(&context.session_id))
    {
        return RoutingFacts::detached();
    }
    let result = RoutingFacts {
        native: NativeCheckpoint {
            turn: context.turn_id.as_deref().map(digest),
            checkpoint: context.checkpoint.as_deref().map(digest),
            compacted: false,
        },
        ..RoutingFacts::default()
    };
    if agent == "hermes" {
        hermes_root(headers, native_identity, result)
    } else {
        result
    }
}

fn codex(headers: &HeaderMap, document: &Value) -> RoutingFacts {
    let header = match single_header(headers, "x-codex-turn-metadata", 8192) {
        Field::Absent => None,
        Field::Value(value) => serde_json::from_str::<Value>(&value).ok(),
        Field::Invalid => return RoutingFacts::detached(),
    };
    if headers.contains_key("x-codex-turn-metadata") && header.is_none() {
        return RoutingFacts::detached();
    }
    let body = document
        .get("client_metadata")
        .and_then(|v| v.get("x-codex-turn-metadata"));
    let body_value = body
        .and_then(Value::as_str)
        .filter(|v| v.len() <= 8192)
        .and_then(|v| serde_json::from_str::<Value>(v).ok());
    if body.is_some() && body_value.is_none() {
        return RoutingFacts::detached();
    }
    let Some(metadata) = header.as_ref().or(body_value.as_ref()) else {
        return RoutingFacts::detached();
    };
    for field in ["session_id", "thread_id", "turn_id", "request_kind"] {
        if header
            .as_ref()
            .zip(body_value.as_ref())
            .is_some_and(|(h, b)| h.get(field) != b.get(field))
        {
            return RoutingFacts::detached();
        }
    }
    for (field, name) in [("session_id", "session-id"), ("thread_id", "thread-id")] {
        let Some(value) = metadata
            .get(field)
            .and_then(Value::as_str)
            .filter(|v| id(v))
        else {
            return RoutingFacts::detached();
        };
        match header_value(headers, name) {
            Field::Value(other) if other != value => return RoutingFacts::detached(),
            Field::Invalid => return RoutingFacts::detached(),
            _ => {}
        }
        if document
            .get("client_metadata")
            .and_then(|m| m.get(field))
            .is_some_and(|v| v.as_str() != Some(value))
        {
            return RoutingFacts::detached();
        }
    }
    let turn = metadata
        .get("turn_id")
        .and_then(Value::as_str)
        .filter(|v| id(v));
    if metadata.get("request_kind").and_then(Value::as_str) != Some("turn") || turn.is_none() {
        return RoutingFacts::detached();
    }
    RoutingFacts {
        native: NativeCheckpoint {
            turn: turn.map(digest),
            ..NativeCheckpoint::default()
        },
        ..RoutingFacts::default()
    }
}

fn hermes_root(
    headers: &HeaderMap,
    identity: Option<(&str, &[String])>,
    mut result: RoutingFacts,
) -> RoutingFacts {
    let Some(("hermes_session", parts)) = identity else {
        return RoutingFacts::detached();
    };
    let Some(current) = parts.first() else {
        return RoutingFacts::detached();
    };
    if let Some(proof) =
        crate::server::core_runtime::observation::native_continuation::read(headers, current)
        && proof.complete
        && proof.invalid_edges.is_none()
        && proof
            .current_state
            .as_ref()
            .is_some_and(|s| s.value == hiroute_domain::NativeSessionStateV1::Open)
    {
        result.root = proof.ids.first().cloned();
        result.native.checkpoint = Some(digest(current));
    }
    result
}
