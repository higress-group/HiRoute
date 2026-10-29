//! A closed client-recoverable cause, never a copy of a provider error payload.
use bytes::Bytes;
use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::sse::SemanticProvenance;
use http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};

use crate::server::core_runtime::model_ir::ModelError;
use crate::server::request_plan::IngressProtocol;

pub(super) const BODY: &[u8] = br#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"Input exceeds the context window."}}"#;

pub(super) fn matches(protocol: IngressProtocol, error: &ModelError) -> bool {
    if error.status != Some(400) {
        return false;
    }
    match error.code.as_deref() {
        Some("context_length_exceeded") => true,
        Some("invalid_request_error") if protocol == IngressProtocol::Messages => {
            error.message.as_deref().is_some_and(messages_prompt_limit)
        }
        _ => false,
    }
}

fn messages_prompt_limit(message: &str) -> bool {
    let Some((input, maximum)) = message
        .strip_prefix("prompt is too long: ")
        .and_then(|text| text.strip_suffix(" maximum"))
        .and_then(|text| text.split_once(" tokens > "))
    else {
        return false;
    };
    let count = |text: &str| {
        (!text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse::<u64>().ok())
            .flatten()
    };
    count(input)
        .zip(count(maximum))
        .is_some_and(|(input, maximum)| maximum > 0 && input > maximum)
}

pub(super) fn local_reply() -> LocalReply {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    LocalReply {
        status: StatusCode::BAD_REQUEST,
        headers,
        body: Bytes::from_static(BODY),
        provenance: SemanticProvenance::NonSemantic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_closed_error_forms_and_status_admit_context_recovery() {
        let mut error = ModelError {
            status: Some(400),
            code: Some("context_length_exceeded".into()),
            message: Some("private arbitrary body".into()),
            retryable: None,
        };
        for protocol in [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ] {
            assert!(matches(protocol, &error));
            for status in [None, Some(200), Some(413), Some(429), Some(503)] {
                error.status = status;
                assert!(!matches(protocol, &error));
            }
            error.status = Some(400);
        }
        error.code = Some("invalid_request_error".into());
        for message in [
            "prompt is too long",
            "prompt is too long: 1 tokens > 2 maximum",
            "prompt is too long: 2 tokens > 0 maximum",
            "prompt is too long: 3 tokens > 2 maximum private tail",
            "context_length_exceeded",
            "rate limit: prompt is too long: 3 tokens > 2 maximum",
        ] {
            error.message = Some(message.into());
            assert!(!matches(IngressProtocol::Messages, &error));
        }
        error.message = Some("prompt is too long: 123456 tokens > 100000 maximum".into());
        assert!(matches(IngressProtocol::Messages, &error));
        assert!(!matches(IngressProtocol::Responses, &error));
        assert!(!matches(IngressProtocol::ChatCompletions, &error));
        error.code = Some("unknown_error".into());
        assert!(!matches(IngressProtocol::Messages, &error));
    }
}
