use super::ContentRef;

/// A trusted ingress image keeps its media-type prefix inline and only its
/// Base64 payload in Replay. This is one substitution, never recursive marker
/// expansion: literal client markers have already been isolated by ingress.
pub(crate) fn data_uri_payload_ref(value: &str) -> Option<ContentRef> {
    let (media_type, payload) = value.strip_prefix("data:")?.split_once(";base64,")?;
    if media_type.is_empty() {
        return None;
    }
    ContentRef::from_wire_marker(payload)
}
