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
    pub(crate) fn hold_identity(&self) -> Option<(&'static str, &[String])> {
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
    if let Some(run) = authorized.verified_run_observation() {
        return valid_identity(&run.run_id)
            .map_or_else(ContextIdentityFacts::request_scoped, |run_id| {
                reliable("verified_worker", vec![run_id], true)
            });
    }

    match codex_identity(ingress, headers, request) {
        Field::Value((kind, parts)) => return reliable(kind, parts, false),
        Field::Invalid => return ContextIdentityFacts::request_scoped(),
        Field::Absent => {}
    }
    if ingress == IngressProtocol::Messages {
        match claude_identity(document) {
            Field::Value(session_id) => {
                return reliable("claude_metadata_session", vec![session_id], false);
            }
            Field::Invalid => return ContextIdentityFacts::request_scoped(),
            Field::Absent => {}
        }
    }

    let Some(first_user) = first_user_digest(request, hold_key) else {
        return ContextIdentityFacts::request_scoped();
    };
    let Some(observation_digest) = first_user_digest(request, observation_key) else {
        return ContextIdentityFacts::request_scoped();
    };
    ContextIdentityFacts {
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

fn reliable(kind: &'static str, parts: Vec<String>, verified_worker: bool) -> ContextIdentityFacts {
    ContextIdentityFacts {
        identity: Identity::Reliable {
            kind,
            parts,
            verified_worker,
        },
    }
}

enum Field<T> {
    Absent,
    Value(T),
    Invalid,
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

fn header_value(headers: &HeaderMap, name: &'static str) -> Field<String> {
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
        .and_then(valid_identity)
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

fn claude_identity(document: &Value) -> Field<String> {
    let Some(metadata) = document.get("metadata") else {
        return Field::Absent;
    };
    let Some(user_id) = metadata.get("user_id") else {
        return Field::Absent;
    };
    let Some(user_id) = user_id.as_str() else {
        return Field::Invalid;
    };
    if user_id.len() > 2_048 {
        return Field::Invalid;
    }
    let Ok(value) = serde_json::from_str::<Value>(user_id) else {
        return Field::Invalid;
    };
    value
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(valid_identity)
        .map_or(Field::Invalid, Field::Value)
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
mod tests {
    use super::*;
    use crate::server::core_runtime::model_ir::{
        CanonicalMessage, MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl,
        ResponsesRequestOptionsV1, ToolChoice,
    };

    fn request(text: &str) -> ModelRequestIRV1 {
        ModelRequestIRV1 {
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

    #[test]
    fn claude_metadata_extracts_only_bounded_json_session() {
        assert!(matches!(
            claude_identity(&serde_json::json!({
                "metadata":{"user_id":"{\"session_id\":\"claude-session\",\"device_id\":\"ignored\"}"}
            })),
            Field::Value(value) if value == "claude-session"
        ));
        assert!(matches!(
            claude_identity(&serde_json::json!({"metadata":{"user_id":"not-json"}})),
            Field::Invalid
        ));
    }
}
