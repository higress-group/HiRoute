//! Codex's typed peer trigger is separate from the original execution document.
use super::*;
use serde_json::Value;

const MAX_NATIVE_ITEMS: usize = 65_536;
const MAX_PEER_MESSAGES: usize = 32;
const MAX_PEER_PARTS: usize = 128;

pub(super) fn prepare(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
) -> Option<NativeRoutingView> {
    if !request.native_only
        || request.ingress_protocol != crate::server::request_plan::IngressProtocol::Responses
    {
        return None;
    }
    let items = request.native_body.as_ref()?.get("input")?.as_array()?;
    // The wire document already exists. Bound the optional scan before looking
    // for a peer; exceeding it must not obstruct forwarding or observed routing.
    if items.len() <= MAX_NATIVE_ITEMS && !items.iter().any(is_peer) {
        return None;
    }
    let latest = (items.len() <= MAX_NATIVE_ITEMS)
        .then(|| {
            items.iter().rposition(|item| {
                is_peer(item)
                    || (item.get("role").and_then(Value::as_str) == Some("user")
                        && item
                            .get("type")
                            .is_none_or(|kind| kind.as_str() == Some("message")))
            })
        })
        .flatten();
    let peer = latest.filter(|last| is_peer(&items[*last]));
    let index = peer.and_then(|last| canonical_index(request, last));
    let projected = peer.zip(index).and_then(|(last, index)| {
        peer_input(items, last, replay, budget).map(|parts| (parts, index))
    });
    Some(match projected {
        Some(((parts, reservation), index)) => NativeRoutingView {
            current_input: Some(parts),
            projection: NativeInputProjection::PeerApplied,
            current_input_position: Some(CurrentInputPosition::Event(index)),
            current_input_unavailable: false,
            origin_unverified: true,
            _reservation: Some(reservation),
        },
        None => NativeRoutingView {
            current_input: None,
            projection: if peer.is_some() || latest.is_none() {
                NativeInputProjection::PeerUnavailable
            } else {
                NativeInputProjection::Wire
            },
            current_input_position: index.map(CurrentInputPosition::Event),
            // An opaque current peer must never select an ancestor User as its
            // task. A later User event still uses the normal complete fallback.
            current_input_unavailable: peer.is_some() || latest.is_none(),
            origin_unverified: true,
            _reservation: None,
        },
    })
}

fn is_peer(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("agent_message")
}

fn canonical_index(request: &ModelRequestIRV1, last: usize) -> Option<usize> {
    let index = request
        .native_history_layout()?
        .iter()
        .position(|item| item.native_index == Some(last))?;
    let placeholder = request.messages.get(index)?;
    if placeholder.role != MessageRole::Assistant
        || !placeholder.content.is_empty()
        || placeholder.name.is_some()
    {
        return None;
    }
    Some(index)
}

fn peer_input(
    items: &[Value],
    last: usize,
    replay: &ReplayStore,
    budget: &StreamBudget,
) -> Option<(Vec<ContentPart>, Reservation)> {
    let first = items[..=last]
        .iter()
        .rposition(|item| !is_peer(item))
        .map_or(0, |index| index + 1);
    if last - first + 1 > MAX_PEER_MESSAGES {
        return None;
    }
    let mut text = Vec::new();
    let mut bytes = 0usize;
    for item in &items[first..=last] {
        let object = item.as_object()?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "type"
                    | "id"
                    | "author"
                    | "recipient"
                    | "content"
                    | "internal_chat_message_metadata_passthrough"
            )
        }) {
            return None;
        }
        for field in ["author", "recipient"] {
            let value = item.get(field)?.as_str()?;
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return None;
            }
        }
        for part in item.get("content")?.as_array()? {
            let part = part.as_object()?;
            // Mixed plaintext/ciphertext is opaque in the native producer too.
            // A readable wrapper cannot stand in for the encrypted task.
            if part.get("type")?.as_str()? != "input_text"
                || part
                    .keys()
                    .any(|key| !matches!(key.as_str(), "type" | "text"))
            {
                return None;
            }
            let value = part.get("text")?.as_str()?;
            let length = value.content_ref().map_or(Some(value.len()), |reference| {
                usize::try_from(reference.byte_len()).ok()
            })?;
            bytes = bytes
                .checked_add(length)
                .filter(|bytes| *bytes <= MAX_USER_TEXT_BYTES)?;
            if text.len() >= MAX_PEER_PARTS {
                return None;
            }
            text.push(value);
        }
    }
    let reservation = budget
        .reserve(
            MemoryRole::ModelIrBacking,
            bytes.checked_mul(4)?.checked_add(32 * 1024)?,
        )
        .ok()?;
    let mut parts = Vec::with_capacity(text.len());
    let mut usable = false;
    for text in text {
        let value = if let Some(reference) = text.content_ref() {
            let mut reader = replay.reader(&reference).ok()?;
            let mut value = String::new();
            reader
                .by_ref()
                .take(MAX_USER_TEXT_BYTES as u64 + 1)
                .read_to_string(&mut value)
                .ok()?;
            if value.len() > MAX_USER_TEXT_BYTES {
                return None;
            }
            reader.verify_terminal().ok()?;
            value
        } else {
            text.to_owned()
        };
        usable |= !value.trim().is_empty();
        parts.push(ContentPart::Text { text: value });
    }
    usable.then_some((parts, reservation))
}
