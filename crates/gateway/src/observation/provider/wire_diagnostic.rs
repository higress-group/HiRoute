use hiroute_diagnostics::event::{
    UpstreamWire, UpstreamWirePhase, WireContentType, WireReasoningEffort, WireRequestReasoning,
};
use http::HeaderMap;
use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

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
        request_reasoning: None,
    }
}

pub(in crate::server::core_runtime::observation) fn request(
    headers: &HeaderMap,
    serialized_template: &[u8],
) -> UpstreamWire {
    let mut event = shape(headers, UpstreamWirePhase::Request, None);
    if serialized_template
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(&b'{')
    {
        return event;
    }
    // Read only controls from the exact encoder template. Serde skips other
    // fields without allocating their strings or expanding Replay references.
    event.request_reasoning = serde_json::from_slice::<Controls>(serialized_template)
        .ok()
        .map(|body| WireRequestReasoning {
            responses_effort: body.reasoning.and_then(|value| value.effort.0),
            chat_effort: body.reasoning_effort.0,
            messages_effort: body.output_config.and_then(|value| value.effort.0),
        });
    event
}

pub(super) fn response(headers: &HeaderMap, status: u16) -> UpstreamWire {
    shape(headers, UpstreamWirePhase::Response, Some(status))
}

#[derive(Deserialize)]
struct Controls {
    reasoning: Option<EffortObject>,
    #[serde(default)]
    reasoning_effort: Effort,
    output_config: Option<EffortObject>,
}

#[derive(Deserialize)]
struct EffortObject {
    #[serde(default)]
    effort: Effort,
}

#[derive(Default)]
struct Effort(Option<WireReasoningEffort>);

impl<'de> Deserialize<'de> for Effort {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ClosedEffort;
        impl<'de> Visitor<'de> for ClosedEffort {
            type Value = WireReasoningEffort;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a reasoning control")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(match value {
                    "none" => WireReasoningEffort::None,
                    "minimal" => WireReasoningEffort::Minimal,
                    "low" => WireReasoningEffort::Low,
                    "medium" => WireReasoningEffort::Medium,
                    "high" => WireReasoningEffort::High,
                    "xhigh" => WireReasoningEffort::Xhigh,
                    "max" => WireReasoningEffort::Max,
                    "ultra" => WireReasoningEffort::Ultra,
                    _ => WireReasoningEffort::Other,
                })
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(WireReasoningEffort::Other)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(WireReasoningEffort::Other)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(WireReasoningEffort::Other)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(WireReasoningEffort::Other)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(WireReasoningEffort::Other)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
                while values.next_element::<IgnoredAny>()?.is_some() {}
                Ok(WireReasoningEffort::Other)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
                while values.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(WireReasoningEffort::Other)
            }
        }
        deserializer
            .deserialize_any(ClosedEffort)
            .map(|value| Self(Some(value)))
    }
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
        let event = request(&headers, br#"{}"#);
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
        assert!(response(&headers, 403).request_reasoning.is_none());
    }

    #[test]
    fn wire_reasoning_controls_are_closed_and_distinguish_missing_from_unavailable() {
        let headers = HeaderMap::new();
        let encoded = serde_json::to_vec(&serde_json::json!({
            "model":"secret-model", "input":"secret-prompt".repeat(20_000),
            "reasoning":{"effort":"medium", "summary":"secret-summary"},
            "reasoning_effort":"secret-control", "output_config":{"effort":null}
        }))
        .unwrap();
        let event = request(&headers, &encoded);
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["request_reasoning"]["responses_effort"], "medium");
        assert_eq!(value["request_reasoning"]["chat_effort"], "other");
        assert_eq!(value["request_reasoning"]["messages_effort"], "other");
        assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
        let empty = request(&headers, br#"{}"#).request_reasoning.unwrap();
        assert_eq!(empty.responses_effort, None);
        assert_eq!(empty.chat_effort, None);
        assert_eq!(empty.messages_effort, None);
        for invalid in [b"invalid".as_slice(), b"[]", br#"{"reasoning":true}"#] {
            assert!(request(&headers, invalid).request_reasoning.is_none());
        }
        for value in [
            serde_json::json!(true),
            serde_json::json!(42),
            serde_json::json!(-1),
            serde_json::json!(2.5),
            serde_json::json!(["secret"]),
            serde_json::json!({"key":"secret"}),
        ] {
            let body = serde_json::to_vec(&serde_json::json!({"reasoning_effort":value})).unwrap();
            assert_eq!(
                request(&headers, &body)
                    .request_reasoning
                    .unwrap()
                    .chat_effort,
                Some(WireReasoningEffort::Other)
            );
        }
    }

    #[test]
    fn request_diagnostic_reads_effort_from_the_actual_encoder_template() {
        use crate::server::core_runtime::{adapters, profiles::CandidateProtocolProfile};
        use crate::server::request_plan::IngressProtocol;
        for effort in [
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ] {
            let profile = CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                IngressProtocol::Responses,
                "native-model",
                serde_json::from_value(serde_json::json!({
                    "profile_id":"selected", "control_kind":"discrete",
                    "render":{"kind":"exact_fields","protocol":"responses","fields":[
                        {"path":["reasoning","effort"],"value":{"kind":"string","value":effort}}
                    ]}, "accounting":"within_output_cap", "additional_reservation_tokens":0
                }))
                .unwrap(),
            );
            let input = adapters::decode_ingress_request(
                IngressProtocol::Responses,
                &serde_json::json!({"model":"alias","input":"private input",
                    "reasoning":{"effort":"low"},"stream":false}),
            )
            .unwrap();
            let template = adapters::project_candidate_request_template(&input, &profile).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&template.bytes).unwrap();
            let event = request(&HeaderMap::new(), &template.bytes);
            let projected = serde_json::to_value(event).unwrap();
            assert_eq!(body["reasoning"]["effort"], effort);
            assert_eq!(
                projected["request_reasoning"]["responses_effort"],
                body["reasoning"]["effort"]
            );
        }
    }
}
