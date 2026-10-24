mod claude;
mod diagnostic;

use hmac::{Hmac, Mac};
use http::HeaderMap;
use serde_json::Value;
use sha2::Sha256;

use crate::content_ref::ContentValueExt;
use crate::server::core_runtime::model_ir::{ContentPart, MessageRole, ModelRequestIRV1};
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextIdentityFacts {
    identity: Identity,
    routing: super::routing::RoutingFacts,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Identity {
    Reliable {
        kind: &'static str,
        parts: Vec<String>,
        verified_worker: bool,
    },
    Inferred {
        hold_digest: String,
        observation_digest: String,
    },
    RequestScoped,
}

impl ContextIdentityFacts {
    pub(crate) fn native_auxiliary(&self) -> bool {
        !self.routing.track
    }

    fn apply_routing(&mut self, mut routing: super::routing::RoutingFacts) {
        if routing.identity_conflict {
            self.identity = Identity::RequestScoped;
            routing.root = None;
            routing.native = super::routing::NativeCheckpoint {
                user_origin: if routing.track {
                    super::routing::NativeUserOrigin::Observed
                } else {
                    super::routing::NativeUserOrigin::ExistingTurn
                },
                ..Default::default()
            };
        }
        self.routing = routing;
    }

    pub(crate) fn requires_captured_input(&self) -> bool {
        self.routing.native.user_origin == super::routing::NativeUserOrigin::ExistingTurn
    }

    pub(crate) fn mark_routing_origin_unverified(&mut self) {
        if self.routing.track && !self.requires_captured_input() {
            self.routing.native.user_origin = super::routing::NativeUserOrigin::Observed;
        }
    }

    pub(crate) fn observed_routing_lane(&self) -> bool {
        self.routing.native.user_origin == super::routing::NativeUserOrigin::Observed
    }

    pub(crate) fn input_origin_unverified(&self, hold: Option<&super::PreparedHold>) -> bool {
        // Optional preparation can miss under contention or bounded resources.
        // Missing continuity evidence must not imply successful source verification.
        self.observed_routing_lane() || hold.is_none_or(super::PreparedHold::user_origin_unverified)
    }

    pub(crate) fn native_checkpoint(&self) -> super::routing::NativeCheckpoint {
        self.routing.native
    }

    pub(crate) fn hold_identity(&self) -> Option<(&'static str, &[String])> {
        if !self.routing.track {
            return None;
        }
        self.routing_identity()
    }

    fn routing_identity(&self) -> Option<(&'static str, &[String])> {
        if let Some(root) = &self.routing.root {
            return Some(("hermes_session", std::slice::from_ref(root)));
        }
        match &self.identity {
            Identity::Reliable { kind, parts, .. } => Some((*kind, parts)),
            Identity::Inferred { hold_digest, .. } => {
                Some(("inferred_first_user", std::slice::from_ref(hold_digest)))
            }
            Identity::RequestScoped => None,
        }
    }

    pub(crate) fn observation_identity(&self) -> Option<(&'static str, &[String])> {
        match &self.identity {
            Identity::Reliable { kind, parts, .. } => Some((*kind, parts)),
            Identity::Inferred {
                observation_digest, ..
            } => Some((
                "inferred_first_user",
                std::slice::from_ref(observation_digest),
            )),
            Identity::RequestScoped => None,
        }
    }

    pub(crate) fn session_scope(&self) -> &'static str {
        match self.identity {
            Identity::RequestScoped => "request_scoped",
            Identity::Reliable { .. } | Identity::Inferred { .. } => "conversation",
        }
    }

    pub(crate) fn correlation_provenance(&self) -> &'static str {
        match self.identity {
            Identity::Reliable {
                verified_worker: true,
                ..
            } => "protocol_state",
            Identity::Reliable {
                verified_worker: false,
                ..
            } => "agent_supplied",
            Identity::Inferred { .. } => "gateway_generated",
            Identity::RequestScoped => "unproven",
        }
    }

    pub(crate) fn request_scoped() -> Self {
        Self {
            identity: Identity::RequestScoped,
            routing: Default::default(),
        }
    }
}

pub(crate) fn identity_facts(
    authorized: &AuthorizedRequestPlan,
    ingress: IngressProtocol,
    headers: &HeaderMap,
    document: &Value,
    request: &ModelRequestIRV1,
    hold_key: &[u8; 32],
    observation_key: &[u8; 32],
) -> ContextIdentityFacts {
    let mut result = base_identity_facts(
        authorized,
        ingress,
        headers,
        document,
        request,
        hold_key,
        observation_key,
    );
    let routing = super::routing::facts(
        &result,
        headers,
        document,
        authorized
            .verified_run_observation()
            .map(|run| run.harness_id.as_str()),
        authorized.is_hermes_consumer(),
    );
    result.apply_routing(routing);
    result
}

fn base_identity_facts(
    authorized: &AuthorizedRequestPlan,
    ingress: IngressProtocol,
    headers: &HeaderMap,
    document: &Value,
    request: &ModelRequestIRV1,
    hold_key: &[u8; 32],
    observation_key: &[u8; 32],
) -> ContextIdentityFacts {
    if let Some(run) = authorized.verified_run_observation() {
        let child = match run.harness_id.as_str() {
            "claude" | "claude_code" => header_value(headers, "x-claude-code-agent-id"),
            "codex" | "codex_cli" if headers.contains_key("x-codex-parent-thread-id") => {
                header_value(headers, "thread-id")
            }
            "dsh" | "deepseek_harness"
                if headers.contains_key("x-deepseek-harness-parent-session-id") =>
            {
                header_value(headers, "x-deepseek-harness-session-id")
            }
            "opencode" if headers.contains_key("x-opencode-parent-session-id") => {
                header_value(headers, "x-opencode-session-id")
            }
            _ => Field::Absent,
        };
        return match child {
            Field::Value(child) => reliable(
                "verified_worker_native_child",
                vec![run.task_id.clone(), run.harness_id.clone(), child],
                true,
            ),
            Field::Invalid => ContextIdentityFacts::request_scoped(),
            Field::Absent => verified_worker_identity(run),
        };
    }

    match native_identity(ingress, headers, document, request) {
        Field::Value((kind, _)) if authorized.is_hermes_consumer() && kind != "hermes_session" => {
            return ContextIdentityFacts::request_scoped();
        }
        Field::Value((kind, parts)) => return reliable(kind, parts, false),
        Field::Invalid => return ContextIdentityFacts::request_scoped(),
        Field::Absent => {}
    }

    if authorized.is_hermes_consumer() {
        return ContextIdentityFacts::request_scoped();
    }
    let Some(first_user) = first_user_digest(request, hold_key) else {
        return ContextIdentityFacts::request_scoped();
    };
    let Some(observation_digest) = first_user_digest(request, observation_key) else {
        return ContextIdentityFacts::request_scoped();
    };
    ContextIdentityFacts {
        routing: Default::default(),
        identity: Identity::Inferred {
            hold_digest: hex(&domain_digest(
                hold_key,
                b"context-hold-inferred-first-user/v1",
                &first_user,
            )),
            observation_digest: hex(&domain_digest(
                observation_key,
                b"observation-inferred-first-user/v1",
                &observation_digest,
            )),
        },
    }
}

fn verified_worker_identity(
    run: &crate::server::request_plan::VerifiedRunObservationContext,
) -> ContextIdentityFacts {
    // Every request remains authorized by its exact run token. A Continue resumes the
    // same task's conversation and decision history; run IDs are separate receipt links.
    valid_identity(&run.task_id).map_or_else(ContextIdentityFacts::request_scoped, |task_id| {
        reliable("verified_worker_task", vec![task_id], true)
    })
}

fn reliable(kind: &'static str, parts: Vec<String>, verified_worker: bool) -> ContextIdentityFacts {
    ContextIdentityFacts {
        routing: Default::default(),
        identity: Identity::Reliable {
            kind,
            parts,
            verified_worker,
        },
    }
}

pub(super) enum Field<T> {
    Absent,
    Value(T),
    Invalid,
}

type NativeIdentity = (&'static str, Vec<String>);

fn native_identity(
    ingress: IngressProtocol,
    headers: &HeaderMap,
    document: &Value,
    request: &ModelRequestIRV1,
) -> Field<NativeIdentity> {
    let claude = claude::identity(ingress, headers, document);
    let dsh = session_identity(
        "deepseek_harness_session",
        header_value(headers, "x-deepseek-harness-session-id"),
    );
    let vendor = merge_vendor_identity(claude, dsh);
    // OpenCode emits the actual session ID even for custom providers. Its
    // cache-affinity headers may instead identify a shared parent/fork source.
    let vendor = merge_vendor_identity(
        vendor,
        session_identity(
            "opencode_session",
            header_value(headers, "x-opencode-session-id"),
        ),
    );
    let vendor = merge_vendor_identity(vendor, pi_ai_identity(ingress, headers));
    let generic = codex_identity(ingress, headers, request);
    let existing = match (generic, vendor) {
        (Field::Invalid, _) | (_, Field::Invalid) => Field::Invalid,
        (Field::Absent, value) | (value, Field::Absent) => value,
        (Field::Value((kind, parts)), Field::Value((_, session)))
            if matches!(kind, "codex_session" | "codex_thread_session")
                && parts.last() == session.first() =>
        {
            // Pi's Codex transport also sends session-id. Preserve all already
            // accepted generic dimensions and their historical HMAC namespace.
            Field::Value((kind, parts))
        }
        (Field::Value(_), Field::Value(_)) => Field::Invalid,
    };
    // Hermes retains its own namespace; every competing family degrades to request scope.
    merge_vendor_identity(
        existing,
        session_identity(
            "hermes_session",
            header_value(headers, "x-hermes-session-id"),
        ),
    )
}

fn session_identity(kind: &'static str, session: Field<String>) -> Field<NativeIdentity> {
    match session {
        Field::Absent => Field::Absent,
        Field::Invalid => Field::Invalid,
        Field::Value(session) => Field::Value((kind, vec![session])),
    }
}

fn merge_vendor_identity(
    left: Field<NativeIdentity>,
    right: Field<NativeIdentity>,
) -> Field<NativeIdentity> {
    match (left, right) {
        (Field::Invalid, _) | (_, Field::Invalid) => Field::Invalid,
        (Field::Absent, value) | (value, Field::Absent) => value,
        (Field::Value(left), Field::Value(right)) if left == right => Field::Value(left),
        (Field::Value(_), Field::Value(_)) => Field::Invalid,
    }
}

fn pi_ai_identity(ingress: IngressProtocol, headers: &HeaderMap) -> Field<NativeIdentity> {
    // pi-ai copies options.sessionId into these transport headers. Their names
    // alone do not imply session semantics for arbitrary API clients. Branding
    // is still only an Agent claim; authorization comes from the existing grant.
    let kind = match single_header(headers, "user-agent", 1_024) {
        Field::Value(agent) if agent.starts_with("pi (") && agent.ends_with(')') => "pi_session",
        Field::Value(agent)
            if agent
                .strip_prefix("deepseek-harness/")
                .is_some_and(|tail| !tail.is_empty() && !tail.starts_with(char::is_whitespace)) =>
        {
            "deepseek_harness_session"
        }
        Field::Invalid
            if [
                "x-client-request-id",
                "session_id",
                "x-session-id",
                "x-session-affinity",
            ]
            .iter()
            .any(|name| headers.contains_key(*name)) =>
        {
            return Field::Invalid;
        }
        Field::Absent | Field::Invalid | Field::Value(_) => return Field::Absent,
    };
    let session = match ingress {
        IngressProtocol::Responses => merge_dimension(
            header_value(headers, "x-client-request-id"),
            header_value(headers, "session_id"),
        ),
        IngressProtocol::Messages => header_value(headers, "x-session-affinity"),
        IngressProtocol::ChatCompletions => return Field::Absent,
    };
    session_identity(
        kind,
        merge_dimension(session, header_value(headers, "x-session-id")),
    )
}

fn codex_identity(
    ingress: IngressProtocol,
    headers: &HeaderMap,
    request: &ModelRequestIRV1,
) -> Field<(&'static str, Vec<String>)> {
    let header_thread = header_value(headers, "thread-id");
    let header_session = header_value(headers, "session-id");
    let metadata = (ingress == IngressProtocol::Responses)
        .then_some(request.responses_options.as_ref())
        .flatten()
        .and_then(|options| options.client_metadata.as_ref());
    let body_thread = metadata_field(metadata, "thread_id");
    let body_session = metadata_field(metadata, "session_id");
    let thread = merge_dimension(header_thread, body_thread);
    let session = merge_dimension(header_session, body_session);
    if matches!(thread, Field::Invalid) || matches!(session, Field::Invalid) {
        return Field::Invalid;
    }
    match (thread, session) {
        (Field::Value(thread), Field::Value(session)) => {
            Field::Value(("codex_thread_session", vec![thread, session]))
        }
        (Field::Value(thread), Field::Absent) => Field::Value(("codex_thread", vec![thread])),
        (Field::Absent, Field::Value(session)) => Field::Value(("codex_session", vec![session])),
        (Field::Absent, Field::Absent) => Field::Absent,
        (Field::Invalid, _) | (_, Field::Invalid) => Field::Invalid,
    }
}

pub(super) fn header_value(headers: &HeaderMap, name: &'static str) -> Field<String> {
    single_header(headers, name, 128)
}

pub(super) fn single_header(
    headers: &HeaderMap,
    name: &'static str,
    max_bytes: usize,
) -> Field<String> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Field::Absent;
    };
    if values.next().is_some() {
        return Field::Invalid;
    }
    value
        .to_str()
        .ok()
        .filter(|value| {
            !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .map_or(Field::Invalid, Field::Value)
}

fn metadata_field(
    metadata: Option<&std::collections::BTreeMap<String, String>>,
    name: &str,
) -> Field<String> {
    let Some(value) = metadata.and_then(|metadata| metadata.get(name)) else {
        return Field::Absent;
    };
    valid_identity(value).map_or(Field::Invalid, Field::Value)
}

fn merge_dimension(header: Field<String>, body: Field<String>) -> Field<String> {
    match (header, body) {
        (Field::Invalid, _) | (_, Field::Invalid) => Field::Invalid,
        (Field::Absent, Field::Absent) => Field::Absent,
        (Field::Value(value), Field::Absent) | (Field::Absent, Field::Value(value)) => {
            Field::Value(value)
        }
        (Field::Value(header), Field::Value(body)) if header == body => Field::Value(header),
        (Field::Value(_), Field::Value(_)) => Field::Invalid,
    }
}

fn valid_identity(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

fn first_user_digest(request: &ModelRequestIRV1, key: &[u8; 32]) -> Option<[u8; 32]> {
    let message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)?;
    if message.content.is_empty() {
        return None;
    }
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(b"complete-first-user/v1");
    let mut non_whitespace = false;
    let mut first_text = true;
    for part in &message.content {
        let ContentPart::Text { text } = part else {
            return None;
        };
        if text.content_ref().is_some() {
            return None;
        }
        if first_text && is_public_wrapper(text) {
            return None;
        }
        first_text = false;
        non_whitespace |= text.chars().any(|character| !character.is_whitespace());
        mac.update(&(text.len() as u64).to_be_bytes());
        mac.update(text.as_bytes());
    }
    non_whitespace.then(|| mac.finalize().into_bytes().into())
}

fn is_public_wrapper(value: &str) -> bool {
    let markers = [
        "# AGENTS.md instructions",
        "<environment_context>",
        "<system-reminder>",
        "<recommended_plugins>",
        "<send_user_message_question_reply>",
    ];
    value.lines().any(|line| {
        let line = line.trim_start();
        markers.iter().any(|marker| line.starts_with(marker))
    })
}

fn domain_digest(key: &[u8; 32], domain: &[u8], digest: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts every key length");
    mac.update(&(domain.len() as u64).to_be_bytes());
    mac.update(domain);
    mac.update(&(digest.len() as u64).to_be_bytes());
    mac.update(digest);
    mac.finalize().into_bytes().into()
}

fn hex(value: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(64);
    for byte in value {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
