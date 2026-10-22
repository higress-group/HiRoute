//! Claude native identity stays separate from prompt/request-class routing facts.
use super::{
    Field, IngressProtocol, NativeIdentity, header_value, merge_dimension, session_identity,
    valid_identity,
};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

pub(super) fn identity(
    ingress: IngressProtocol,
    headers: &HeaderMap,
    document: &Value,
) -> Field<NativeIdentity> {
    let metadata = if ingress == IngressProtocol::Messages {
        metadata_identity(document)
    } else {
        Field::Absent
    };
    // Preserve the released metadata namespace when native headers are enabled.
    // Neither malformed nor contradictory identity claims can be repaired by text.
    let session = session_identity(
        "claude_metadata_session",
        merge_dimension(header_value(headers, "x-claude-code-session-id"), metadata),
    );
    match (session, header_value(headers, "x-claude-code-agent-id")) {
        (Field::Value((_, mut parts)), Field::Value(agent)) => {
            parts.push(agent);
            Field::Value(("claude_native_agent", parts))
        }
        (Field::Absent | Field::Invalid, Field::Value(_)) | (_, Field::Invalid) => Field::Invalid,
        (value, Field::Absent) => value,
    }
}

fn metadata_identity(document: &Value) -> Field<String> {
    let Some(metadata) = document.get("metadata") else {
        return Field::Absent;
    };
    let Some(user_id) = metadata.get("user_id") else {
        return Field::Absent;
    };
    let Some(user_id) = user_id.as_str() else {
        return Field::Invalid;
    };
    // Derived structs also accept positional arrays; only the native JSON object
    // is an identity channel. The typed decode below rejects duplicate fields.
    if user_id.len() > 2_048 || !user_id.trim_start().starts_with('{') {
        return Field::Invalid;
    }
    #[derive(Deserialize)]
    struct Session {
        session_id: String,
    }
    let Ok(value) = serde_json::from_str::<Session>(user_id) else {
        return Field::Invalid;
    };
    valid_identity(&value.session_id).map_or(Field::Invalid, Field::Value)
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
