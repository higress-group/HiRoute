use hiroute_diagnostics::event::{UpstreamWire, UpstreamWirePhase, WireContentType};
use http::HeaderMap;

fn shape(headers: &HeaderMap, phase: UpstreamWirePhase, status: Option<u16>) -> UpstreamWire {
    let authorization = headers.get(http::header::AUTHORIZATION);
    let content_type = match headers.get(http::header::CONTENT_TYPE) {
        None => WireContentType::Missing,
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|v| v.split(';').next())
            .map(str::trim)
        {
            Some(v) if v.eq_ignore_ascii_case("application/json") => WireContentType::Json,
            Some(v) if v.eq_ignore_ascii_case("text/event-stream") => WireContentType::EventStream,
            Some(v) if v.eq_ignore_ascii_case("text/html") => WireContentType::Html,
            _ => WireContentType::Other,
        },
    };
    UpstreamWire {
        request_token: None,
        phase,
        bearer_present: authorization.is_some_and(|v| v.as_bytes().starts_with(b"Bearer ")),
        api_key_present: headers.contains_key("x-api-key"),
        authorization_bytes: authorization.map_or(0, |v| v.as_bytes().len() as u64),
        body_bytes: headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok()),
        http_status: status,
        content_type,
    }
}

pub(super) fn request(headers: &HeaderMap) -> UpstreamWire {
    shape(headers, UpstreamWirePhase::Request, None)
}

pub(super) fn response(headers: &HeaderMap, status: u16) -> UpstreamWire {
    shape(headers, UpstreamWirePhase::Response, Some(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_diagnostic_never_serializes_untrusted_header_values() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret-marker".parse().unwrap());
        headers.insert("x-api-key", "other-secret".parse().unwrap());
        headers.insert("content-type", "secret-marker".parse().unwrap());
        headers.insert("www-authenticate", "private-error-body".parse().unwrap());
        let event = request(&headers);
        assert!(event.bearer_present && event.api_key_present);
        assert_eq!(event.content_type, WireContentType::Other);
        let encoded = serde_json::to_string(&event).unwrap();
        for secret in ["secret-marker", "other-secret", "private-error-body"] {
            assert!(!encoded.contains(secret));
        }
        headers.insert(
            "content-type",
            "application/json; charset=utf-8".parse().unwrap(),
        );
        assert_eq!(response(&headers, 403).content_type, WireContentType::Json);
        assert_eq!(response(&headers, 403).http_status, Some(403));
    }
}
