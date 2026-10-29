//! Codex native task and persistent compaction-window evidence.
use super::*;

/// Core's canonical body and its bounded compatibility header omit only the
/// Code Mode map. Decode relevant fields directly so duplicate keys cannot be
/// silently overwritten by a Value intermediary. Optional malformed enrichment
/// must not hide a positively identified auxiliary request.
#[derive(Default, Deserialize, PartialEq)]
#[serde(default)]
struct Metadata {
    request_kind: Value,
    installation_id: Value,
    session_id: Value,
    thread_id: Value,
    turn_id: Value,
    window_id: Value,
    parent_thread_id: Value,
    parent_turn_id: Value,
    forked_from_thread_id: Value,
    subagent_kind: Value,
    thread_source: Value,
    compaction: Value,
}

pub(super) fn facts(headers: &HeaderMap, document: &Value) -> RoutingFacts {
    let header = match single_header(headers, "x-codex-turn-metadata", 8192) {
        Field::Absent => None,
        Field::Value(value) => decode(&value),
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
        .and_then(decode);
    if body.is_some() && body_value.is_none() {
        return RoutingFacts::uncertain();
    }
    let Some(metadata) = header.as_ref().or(body_value.as_ref()) else {
        return RoutingFacts::uncertain();
    };
    let identity_conflict = header.iter().chain(body_value.iter()).any(|metadata| {
        [
            (&metadata.session_id, "session-id", "session_id"),
            (&metadata.thread_id, "thread-id", "thread_id"),
        ]
        .into_iter()
        .any(|(canonical, name, flat)| {
            conflicting_identity(canonical, headers, document, name, flat)
        })
    }) || header
        .as_ref()
        .zip(body_value.as_ref())
        .is_some_and(|(h, b)| {
            [(&h.session_id, &b.session_id), (&h.thread_id, &b.thread_id)]
                .into_iter()
                .any(|(h, b)| valid_id(h).zip(valid_id(b)).is_some_and(|(h, b)| h != b))
        });
    if identity_conflict {
        return RoutingFacts {
            identity_conflict: true,
            ..RoutingFacts::uncertain()
        };
    }
    if header
        .as_ref()
        .zip(body_value.as_ref())
        .is_some_and(|(h, b)| h != b)
    {
        return RoutingFacts::uncertain();
    }
    for (canonical, header_name, body_name) in [
        (&metadata.session_id, "session-id", "session_id"),
        (&metadata.thread_id, "thread-id", "thread_id"),
        (
            &metadata.window_id,
            "x-codex-window-id",
            "x-codex-window-id",
        ),
        (
            &metadata.installation_id,
            "x-codex-installation-id",
            "x-codex-installation-id",
        ),
        (
            &metadata.parent_thread_id,
            "x-codex-parent-thread-id",
            "x-codex-parent-thread-id",
        ),
    ] {
        if !coherent_dimension(canonical, headers, document, header_name, body_name) {
            return RoutingFacts::uncertain();
        }
    }
    for (canonical, name) in [
        (&metadata.turn_id, "turn_id"),
        (&metadata.parent_turn_id, "parent_turn_id"),
        (&metadata.request_kind, "request_kind"),
    ] {
        if document
            .get("client_metadata")
            .and_then(|m| m.get(name))
            .is_some_and(|value| !canonical.is_null() && value != canonical)
        {
            return RoutingFacts::uncertain();
        }
    }
    // rust-v0.147.0 emits exactly these kinds. An unfamiliar future kind is
    // observed traffic, not permission to discard an ordinary execution.
    match metadata.request_kind.as_str() {
        Some("memory" | "prewarm" | "compaction") => return RoutingFacts::detached(),
        Some("turn") => {}
        _ => return RoutingFacts::uncertain(),
    }
    let session = metadata.session_id.as_str().filter(|value| id(value));
    let thread = metadata.thread_id.as_str().filter(|value| id(value));
    let turn = metadata.turn_id.as_str().filter(|value| id(value));
    if session.is_none() || thread.is_none() || turn.is_none() {
        return RoutingFacts::uncertain();
    }
    let window = metadata.window_id.as_str();
    // rust-v0.147.0 emits thread_id:window_number. Every native compaction
    // increments that persisted number; it survives resume. It is negative
    // evidence only: zero still needs a clean bootstrap or captured task.
    let generation = thread.zip(window).and_then(|(thread, window)| {
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

fn decode(value: &str) -> Option<Metadata> {
    // Serde's derived struct visitor also accepts positional JSON arrays. Native
    // metadata is an object; ["memory"] must not become a trusted purpose.
    value
        .trim_start()
        .starts_with('{')
        .then(|| serde_json::from_str::<Metadata>(value).ok())
        .flatten()
}

fn coherent_dimension(
    canonical: &Value,
    headers: &HeaderMap,
    document: &Value,
    header_name: &'static str,
    body_name: &str,
) -> bool {
    let flat = document
        .get("client_metadata")
        .and_then(|value| value.get(body_name));
    if flat.is_some_and(|value| !canonical.is_null() && value != canonical) {
        return false;
    }
    match single_header(headers, header_name, 160) {
        Field::Invalid => false,
        Field::Absent => true,
        Field::Value(value) => {
            (canonical.is_null() || canonical.as_str() == Some(value.as_str()))
                && flat.is_none_or(|flat| flat.as_str() == Some(value.as_str()))
        }
    }
}

fn valid_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| id(value))
}

fn conflicting_identity(
    canonical: &Value,
    headers: &HeaderMap,
    document: &Value,
    header_name: &'static str,
    body_name: &str,
) -> bool {
    let header = match header_value(headers, header_name) {
        Field::Value(value) => Some(value),
        _ => None,
    };
    let flat = document
        .get("client_metadata")
        .and_then(|metadata| metadata.get(body_name))
        .and_then(valid_id);
    let mut claims = [valid_id(canonical), header.as_deref(), flat]
        .into_iter()
        .flatten();
    claims
        .next()
        .is_some_and(|first| claims.any(|other| first != other))
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
