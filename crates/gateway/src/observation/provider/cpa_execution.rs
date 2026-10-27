//! Safe metadata from the authenticated managed bridge, not arbitrary providers.

use hiroute_diagnostics::event::{
    CpaExecution, CpaInferenceAttempt, CpaInferenceError, CpaInferenceObservation,
    CpaInferencePhase,
};
use http::HeaderMap;

const HEADER: &str = "x-hiroute-cpa-execution";

pub(in crate::server::core_runtime::observation) fn from_headers(
    headers: &HeaderMap,
    response_status: u16,
) -> CpaExecution {
    let mut values = headers.get_all(HEADER).iter();
    let Some(value) = values.next() else {
        return CpaExecution::Unknown;
    };
    if values.next().is_some() || value.as_bytes().len() > 192 {
        return CpaExecution::Unknown;
    }
    let Some(value) = value.to_str().ok() else {
        return CpaExecution::Unknown;
    };
    let mut fields = value.split(';');
    let version = fields.next();
    if !matches!(version, Some("v1" | "v2")) {
        return CpaExecution::Unknown;
    }
    let mut counters = [0_u8; 4];
    for counter in &mut counters {
        let Some(raw) = fields.next() else {
            return CpaExecution::Unknown;
        };
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return CpaExecution::Unknown;
        }
        let Some(value) = raw.parse::<u8>().ok().filter(|value| *value <= 32) else {
            return CpaExecution::Unknown;
        };
        *counter = value;
    }
    let inference = if version == Some("v2") {
        let Some(detail) = inference_detail(&mut fields, counters[0], response_status) else {
            return CpaExecution::Unknown;
        };
        Some(detail)
    } else {
        None
    };
    if fields.next().is_some() || counters[1] > counters[0] || counters[3] > counters[2] {
        return CpaExecution::Unknown;
    }
    CpaExecution::Known {
        inference_attempts: counters[0],
        unauthorized_responses: counters[1],
        auth_recovery_attempts: counters[2],
        auth_recovery_successes: counters[3],
        inference,
    }
}

fn inference_detail<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    count: u8,
    actual_status: u16,
) -> Option<CpaInferenceObservation> {
    let downstream_status = decimal(fields.next()?)?;
    if !(100..=599).contains(&downstream_status) || downstream_status != actual_status {
        return None;
    }
    let first = inference_attempt(fields.next()?)?;
    let last = inference_attempt(fields.next()?)?;
    if count == 0 {
        if first.is_some() || last.is_some() {
            return None;
        }
    } else {
        if first.as_ref()?.ordinal != 1 || last.as_ref()?.ordinal != count {
            return None;
        }
        if count == 1 && first != last {
            return None;
        }
    }
    Some(CpaInferenceObservation {
        downstream_status,
        first,
        last,
    })
}

fn decimal(value: &str) -> Option<u16> {
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn inference_attempt(value: &str) -> Option<Option<CpaInferenceAttempt>> {
    if value == "-" {
        return Some(None);
    }
    let mut fields = value.split(',');
    let ordinal = decimal(fields.next()?)?;
    if !(1..=32).contains(&ordinal) {
        return None;
    }
    let status = decimal(fields.next()?)?;
    let upstream_status = if status == 0 {
        None
    } else if (100..=599).contains(&status) {
        Some(status)
    } else {
        return None;
    };
    let phase = match fields.next()? {
        "started" => CpaInferencePhase::Started,
        "headers" => CpaInferencePhase::Headers,
        "transport" => CpaInferencePhase::Transport,
        "body" => CpaInferencePhase::Body,
        _ => return None,
    };
    let error = match fields.next()? {
        "none" => CpaInferenceError::None,
        "cancelled" => CpaInferenceError::Cancelled,
        "timeout" => CpaInferenceError::Timeout,
        "eof" => CpaInferenceError::Eof,
        "unexpected_eof" => CpaInferenceError::UnexpectedEof,
        "dns" => CpaInferenceError::Dns,
        "tls" => CpaInferenceError::Tls,
        "network" => CpaInferenceError::Network,
        "other" => CpaInferenceError::Other,
        _ => return None,
    };
    let valid = match phase {
        CpaInferencePhase::Started => upstream_status.is_none() && error == CpaInferenceError::None,
        CpaInferencePhase::Headers => upstream_status.is_some() && error == CpaInferenceError::None,
        CpaInferencePhase::Transport => error != CpaInferenceError::None,
        CpaInferencePhase::Body => {
            upstream_status.is_some()
                && !matches!(error, CpaInferenceError::None | CpaInferenceError::Eof)
        }
    };
    if !valid || fields.next().is_some() {
        return None;
    }
    Some(Some(CpaInferenceAttempt {
        ordinal: ordinal as u8,
        upstream_status,
        phase,
        error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpa_extended_trace_keeps_bridge_and_transport_failures_separate() {
        for (detail, expected_status, phase, error) in [
            (
                "1,500,headers,none",
                Some(500),
                CpaInferencePhase::Headers,
                CpaInferenceError::None,
            ),
            (
                "1,200,body,unexpected_eof",
                Some(200),
                CpaInferencePhase::Body,
                CpaInferenceError::UnexpectedEof,
            ),
            (
                "1,0,transport,network",
                None,
                CpaInferencePhase::Transport,
                CpaInferenceError::Network,
            ),
            (
                "1,200,headers,none",
                Some(200),
                CpaInferencePhase::Headers,
                CpaInferenceError::None,
            ),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                HEADER,
                format!("v2;1;0;0;0;500;{detail};{detail}").parse().unwrap(),
            );
            let CpaExecution::Known {
                inference: Some(observed),
                ..
            } = from_headers(&headers, 500)
            else {
                panic!("valid trace")
            };
            assert_eq!(observed.downstream_status, 500);
            assert_eq!(observed.first, observed.last);
            let actual = observed.last.unwrap();
            assert_eq!(actual.upstream_status, expected_status);
            assert_eq!(actual.phase, phase);
            assert_eq!(actual.error, error);
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            HEADER,
            "v2;3;1;1;1;200;1,401,headers,none;3,200,headers,none"
                .parse()
                .unwrap(),
        );
        let CpaExecution::Known {
            inference: Some(observed),
            ..
        } = from_headers(&headers, 200)
        else {
            panic!("valid first/last trace")
        };
        assert_eq!(observed.first.unwrap().ordinal, 1);
        assert_eq!(observed.last.unwrap().ordinal, 3);
        headers.insert(HEADER, "v2;0;0;0;0;503;-;-".parse().unwrap());
        assert!(matches!(
            from_headers(&headers, 503),
            CpaExecution::Known {
                inference: Some(CpaInferenceObservation {
                    first: None,
                    last: None,
                    ..
                }),
                ..
            }
        ));
    }

    #[test]
    fn cpa_extended_trace_rejects_mixed_or_unbounded_evidence() {
        for value in [
            "v2;1;0;0;0;200;1,500,headers,none;1,500,headers,none",
            "v2;1;0;0;0;500;1,500,headers,none;1,200,headers,none",
            "v2;2;0;0;0;500;2,500,headers,none;2,500,headers,none",
            "v2;3;0;0;0;500;1,500,headers,none;2,500,headers,none",
            "v2;0;0;0;0;500;1,0,started,none;1,0,started,none",
            "v2;1;0;0;0;500;1,200,started,none;1,200,started,none",
            "v2;1;0;0;0;500;1,0,body,timeout;1,0,body,timeout",
            "v2;1;0;0;0;500;1,200,body,eof;1,200,body,eof",
            "v2;1;0;0;0;500;1,0,transport,private-error;1,0,transport,private-error",
            "v2;unknown",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(HEADER, value.parse().unwrap());
            assert_eq!(
                from_headers(&headers, 500),
                CpaExecution::Unknown,
                "{value}"
            );
        }
        let mut headers = HeaderMap::new();
        headers.insert(HEADER, "a".repeat(193).parse().unwrap());
        assert_eq!(from_headers(&headers, 500), CpaExecution::Unknown);
    }

    #[test]
    fn historical_execution_records_keep_counts_without_inventing_detail() {
        let old = serde_json::json!({"status":"known","inference_attempts":1,"unauthorized_responses":0,"auth_recovery_attempts":0,"auth_recovery_successes":0});
        let record: CpaExecution = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            record,
            CpaExecution::Known {
                inference: None,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(record).unwrap(), old);
    }

    #[test]
    fn cpa_metadata_records_wire_counts_without_inventing_usage() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER, "v1;2;1;1;1".parse().unwrap());
        assert_eq!(
            from_headers(&headers, 200),
            CpaExecution::Known {
                inference_attempts: 2,
                unauthorized_responses: 1,
                auth_recovery_attempts: 1,
                auth_recovery_successes: 1,
                inference: None,
            }
        );
        // Existing provider protocol repair can make additional sends. This
        // counter describes the wire, not a made-up maximum of two requests.
        headers.insert(HEADER, "v1;3;1;1;1".parse().unwrap());
        assert!(matches!(
            from_headers(&headers, 200),
            CpaExecution::Known {
                inference_attempts: 3,
                ..
            }
        ));
    }

    #[test]
    fn cpa_missing_malformed_duplicate_or_overflow_metadata_is_unknown() {
        assert_eq!(from_headers(&HeaderMap::new(), 200), CpaExecution::Unknown);
        for value in [
            "v1;unknown",
            "v2;2;1;1;1",
            "v1;33;0;0;0",
            "v1;1;2;0;0",
            "v1;1;1;0;1",
            "v1;+1;0;0;0",
            "v1;1;0;0",
            "v1;1;0;0;0;0",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(HEADER, value.parse().unwrap());
            assert_eq!(
                from_headers(&headers, 200),
                CpaExecution::Unknown,
                "{value}"
            );
        }
        let mut headers = HeaderMap::new();
        headers.append(HEADER, "v1;2;1;1;1".parse().unwrap());
        headers.append(HEADER, "v1;1;0;0;0".parse().unwrap());
        assert_eq!(from_headers(&headers, 200), CpaExecution::Unknown);
    }
}
