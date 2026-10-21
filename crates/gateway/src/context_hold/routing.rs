//! Optional native routing evidence. It never grants execution authority.
mod codex;

use super::identity::{ContextIdentityFacts, Field, header_value, single_header};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NativeUserOrigin {
    #[default]
    Wire,
    NativeTurn,
    CurrentUser,
    ExistingTurn,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativeCheckpoint {
    pub(crate) turn: Option<[u8; 32]>,
    pub(crate) checkpoint: Option<[u8; 32]>,
    pub(crate) generation: Option<u64>,
    pub(crate) compacted: bool,
    pub(crate) requires_checkpoint: bool,
    pub(crate) user_origin: NativeUserOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RoutingFacts {
    pub(super) track: bool,
    pub(super) capture_gap: bool,
    pub(super) native: NativeCheckpoint,
    pub(super) root: Option<String>,
}

impl Default for RoutingFacts {
    fn default() -> Self {
        Self {
            track: true,
            capture_gap: false,
            native: NativeCheckpoint::default(),
            root: None,
        }
    }
}

impl RoutingFacts {
    fn detached() -> Self {
        Self {
            track: false,
            native: NativeCheckpoint {
                user_origin: NativeUserOrigin::ExistingTurn,
                ..Default::default()
            },
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
    #[serde(default)]
    user_turns: Option<Vec<NativeUserTurn>>,
    #[serde(default)]
    continuation_requires_existing_turn: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeUserTurn {
    pub(crate) turn_id: String,
    pub(crate) messages: Vec<NativeUserMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeUserMessage {
    /// Ordinal among wire user envelopes containing text/image, excluding tool-only results.
    pub(crate) index: usize,
    pub(crate) sha256: String,
}

fn valid_user_turns(context: &NativeContext) -> bool {
    let Some(turns) = &context.user_turns else {
        return false;
    };
    if turns.is_empty()
        || turns.len() > 16
        || turns.last().map(|t| &t.turn_id) != context.turn_id.as_ref()
    {
        return false;
    }
    let mut previous = None;
    let mut count = 0;
    let mut seen = std::collections::BTreeSet::new();
    for turn in turns {
        if !id(&turn.turn_id) || !seen.insert(&turn.turn_id) || turn.messages.is_empty() {
            return false;
        }
        for message in &turn.messages {
            count += 1;
            if count > 32
                || message.index >= 128
                || previous.is_some_and(|p| message.index <= p)
                || message.sha256.len() != 64
                || !message
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return false;
            }
            previous = Some(message.index);
        }
    }
    true
}

/// DSH's optional semantic user proof is separate from execution authority.
pub(crate) fn dsh_user_turns(headers: &HeaderMap) -> Option<Result<Vec<NativeUserTurn>, ()>> {
    let known = matches!(header_value(headers, "x-hiroute-agent"), Field::Value(v) if v == "dsh")
        || headers.contains_key("x-deepseek-harness-session-id")
        || matches!(header_value(headers, "user-agent"), Field::Value(v) if v.starts_with("deepseek-harness/") || v.starts_with("dsh/"));
    if !known {
        return None;
    }
    let parsed = match single_header(headers, "x-hiroute-routing-context", 4096) {
        Field::Value(value) => serde_json::from_str::<NativeContext>(&value).ok(),
        _ => None,
    };
    Some(
        parsed
            .filter(|context| {
                context.version == 1
                    && context.agent == "dsh"
                    && context.purpose == "main"
                    && id(&context.session_id)
                    && valid_user_turns(context)
            })
            .and_then(|context| context.user_turns)
            .ok_or(()),
    )
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
    let worker = worker.map(|id| match id {
        "codex_cli" => "codex",
        "claude_code" => "claude",
        "qoder_cli" => "qoder",
        "deepseek_harness" => "dsh",
        id => id,
    });
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
        _ if user_agent.starts_with("pi (") || user_agent.starts_with("pi/") => Some("pi"),
        _ if user_agent.starts_with("deepseek-harness/") || user_agent.starts_with("dsh/") => {
            Some("dsh")
        }
        _ if user_agent.starts_with("opencode/") => Some("opencode"),
        _ if user_agent.starts_with("claude-cli/")
            || user_agent.starts_with("claude-code/")
            || headers.contains_key("x-claude-code-request-class") =>
        {
            Some("claude")
        }
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
        return if hermes {
            RoutingFacts::detached()
        } else {
            RoutingFacts::default()
        };
    };
    if brand.as_deref().is_some_and(|brand| brand != agent) {
        return RoutingFacts::detached();
    }
    if agent == "qoder" {
        return RoutingFacts::detached();
    }
    if agent == "codex" {
        if worker.is_none()
            && !matches!(
                native_kind,
                Some("codex_session" | "codex_thread" | "codex_thread_session")
            )
        {
            return RoutingFacts::detached();
        }
        return codex::facts(headers, document);
    }
    if agent == "claude" {
        if worker.is_none()
            && !matches!(
                native_kind,
                Some("claude_metadata_session" | "claude_native_agent")
            )
        {
            return RoutingFacts::detached();
        }
        let purpose = header_value(headers, "x-claude-code-request-class");
        if !matches!(&purpose, Field::Value(value) if matches!(value.as_str(), "main" | "subagent"))
        {
            return RoutingFacts::detached();
        }
        if matches!(&purpose, Field::Value(value) if value == "subagent")
            && !matches!(
                header_value(headers, "x-claude-code-agent-id"),
                Field::Value(_)
            )
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
                generation: None,
                requires_checkpoint: false,
                // This flag is consumed once per model call. Later User-only
                // summaries have no stable original-user provenance, so they
                // cannot bootstrap optional history from a wire role alone.
                user_origin: NativeUserOrigin::ExistingTurn,
            },
            ..RoutingFacts::default()
        };
    }
    if worker == Some("opencode") {
        match header_value(headers, "x-hiroute-request-purpose") {
            Field::Value(value) if value == "main" => {
                // Pure-mode Workers have no per-call user-origin adapter. The
                // frozen route still executes, but a wire User may be synthetic.
                return RoutingFacts::detached();
            }
            Field::Value(_) | Field::Invalid => return RoutingFacts::detached(),
            Field::Absent => {}
        }
    }
    let context = match single_header(headers, "x-hiroute-routing-context", 4096) {
        Field::Value(value) => serde_json::from_str::<NativeContext>(&value).ok(),
        Field::Absent if agent == "hermes" && hermes => {
            // Older managed Hermes hooks only enter the selected conversation
            // call site. Its independent auxiliary clients lack this identity.
            let mut result = RoutingFacts::default();
            result.native.requires_checkpoint = true;
            result.native.user_origin = NativeUserOrigin::ExistingTurn;
            return hermes_root(headers, native_identity, result);
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
        || context.continuation_requires_existing_turn == Some(false) && context.turn_id.is_none()
        || (agent != "dsh" && context.user_turns.is_some())
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
    if agent == "dsh" && !valid_user_turns(&context) {
        // This proves only the affected main scope, not a semantic user. Record
        // a capture gap without admitting this request or guessing its task.
        return RoutingFacts {
            track: false,
            capture_gap: true,
            ..RoutingFacts::default()
        };
    }
    let result = RoutingFacts {
        native: NativeCheckpoint {
            turn: context.turn_id.as_deref().map(digest),
            checkpoint: context.checkpoint.as_deref().map(digest),
            generation: None,
            compacted: false,
            // Hermes enters the native conversation call site, rather than a
            // shared provider-header hook; its turn ID also covers in-place compression.
            requires_checkpoint: agent != "hermes",
            // Hermes whole-turn hooks also receive native async completion
            // notifications as User input. Old adapters' false flag cannot prove
            // a new original user; keep their transport compatible but conservative.
            user_origin: if agent == "dsh"
                || agent != "hermes" && context.continuation_requires_existing_turn == Some(false)
            {
                NativeUserOrigin::CurrentUser
            } else {
                NativeUserOrigin::ExistingTurn
            },
        },
        ..RoutingFacts::default()
    };
    if agent == "hermes" {
        hermes_root(headers, native_identity, result)
    } else {
        result
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
