//! Content binding for observed intent, never proof of human origin or authority.
use super::{Field, NativeContext, id, single_header};
use http::HeaderMap;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservedUserInput {
    version: u8,
    kind: String,
    pub(crate) reverse_user_index: usize,
    pub(crate) text_sha256: String,
}

pub(crate) fn hermes_observed_input(headers: &HeaderMap) -> Option<ObservedUserInput> {
    if !matches!(single_header(headers, "x-hiroute-agent", 32), Field::Value(v) if v == "hermes") {
        return None;
    }
    let session = match single_header(headers, "x-hermes-session-id", 128) {
        Field::Value(v) if id(&v) => v,
        _ => return None,
    };
    let context = match single_header(headers, "x-hiroute-routing-context", 4096) {
        Field::Value(v) => serde_json::from_str::<NativeContext>(&v).ok()?,
        _ => return None,
    };
    if context.version != 1
        || context.agent != "hermes"
        || context.purpose != "main"
        || context.session_id != session
        || !context.turn_id.as_deref().is_some_and(id)
    {
        return None;
    }
    let hint = match single_header(headers, "x-hermes-observed-input", 512) {
        Field::Value(v) => serde_json::from_str::<ObservedUserInput>(&v).ok()?,
        _ => return None,
    };
    (hint.version == 1
        && hint.kind == "observed_turn_input"
        && hint.reverse_user_index < 128
        && hint.text_sha256.len() == 64
        && hint
            .text_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(hint)
}
