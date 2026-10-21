//! Codex native task and persistent compaction-window evidence.
use super::*;

pub(super) fn facts(headers: &HeaderMap, document: &Value) -> RoutingFacts {
    let header = match single_header(headers, "x-codex-turn-metadata", 8192) {
        Field::Absent => None,
        Field::Value(value) => serde_json::from_str::<Value>(&value).ok(),
        Field::Invalid => return RoutingFacts::uncertain(),
    };
    if headers.contains_key("x-codex-turn-metadata") && header.is_none() {
        return RoutingFacts::uncertain();
    }
    let body = document
        .get("client_metadata")
        .and_then(|v| v.get("x-codex-turn-metadata"));
    let body_value = body
        .and_then(Value::as_str)
        .filter(|v| v.len() <= 8192)
        .and_then(|v| serde_json::from_str::<Value>(v).ok());
    if body.is_some() && body_value.is_none() {
        return RoutingFacts::uncertain();
    }
    let Some(metadata) = header.as_ref().or(body_value.as_ref()) else {
        return RoutingFacts::uncertain();
    };
    for field in [
        "session_id",
        "thread_id",
        "turn_id",
        "request_kind",
        "window_id",
    ] {
        if header
            .as_ref()
            .zip(body_value.as_ref())
            .is_some_and(|(h, b)| h.get(field) != b.get(field))
        {
            return RoutingFacts::uncertain();
        }
    }
    for (field, name) in [("session_id", "session-id"), ("thread_id", "thread-id")] {
        let Some(value) = metadata
            .get(field)
            .and_then(Value::as_str)
            .filter(|v| id(v))
        else {
            return RoutingFacts::uncertain();
        };
        match header_value(headers, name) {
            Field::Value(other) if other != value => return RoutingFacts::uncertain(),
            Field::Invalid => return RoutingFacts::uncertain(),
            _ => {}
        }
        if document
            .get("client_metadata")
            .and_then(|m| m.get(field))
            .is_some_and(|v| v.as_str() != Some(value))
        {
            return RoutingFacts::uncertain();
        }
    }
    let turn = metadata
        .get("turn_id")
        .and_then(Value::as_str)
        .filter(|v| id(v));
    if metadata
        .get("request_kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "turn")
    {
        return RoutingFacts::detached();
    }
    if turn.is_none() {
        return RoutingFacts::uncertain();
    }
    let window = metadata.get("window_id").and_then(Value::as_str);
    match single_header(headers, "x-codex-window-id", 160) {
        Field::Value(value) if Some(value.as_str()) != window => return RoutingFacts::uncertain(),
        Field::Invalid => return RoutingFacts::uncertain(),
        _ => {}
    }
    if document
        .get("client_metadata")
        .and_then(|m| m.get("x-codex-window-id"))
        .is_some_and(|value| value.as_str() != window)
    {
        return RoutingFacts::uncertain();
    }
    // rust-v0.147.0 emits thread_id:window_number. Every native compaction
    // increments that persisted number; it survives resume. It is negative
    // evidence only: zero still needs a clean bootstrap or captured task.
    let generation = metadata
        .get("thread_id")
        .and_then(Value::as_str)
        .zip(window)
        .and_then(|(thread, window)| {
            let suffix = window.strip_prefix(thread)?.strip_prefix(':')?;
            if suffix.is_empty()
                || suffix.len() > 20
                || !suffix.bytes().all(|b| b.is_ascii_digit())
                || suffix.len() > 1 && suffix.starts_with('0')
            {
                return None;
            }
            suffix.parse::<u64>().ok()
        });
    let known_producer = matches!(
        single_header(headers, "user-agent", 1024),
        Field::Value(value) if matches!(value.split_whitespace().next(),
            Some("codex_exec/0.147.0" | "codex_cli_rs/0.147.0"))
    );
    RoutingFacts {
        native: NativeCheckpoint {
            turn: turn.map(digest),
            compacted: generation.is_some_and(|number| number > 0),
            generation,
            user_origin: if generation.is_some() && known_producer {
                NativeUserOrigin::NativeTurn
            } else {
                NativeUserOrigin::Observed
            },
            ..NativeCheckpoint::default()
        },
        ..RoutingFacts::default()
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
