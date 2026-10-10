//! Safe metadata from the authenticated managed bridge, not arbitrary providers.

use hiroute_diagnostics::event::CpaExecution;
use http::HeaderMap;

const HEADER: &str = "x-hiroute-cpa-execution";

pub(in crate::server::core_runtime::observation) fn from_headers(
    headers: &HeaderMap,
) -> CpaExecution {
    let mut values = headers.get_all(HEADER).iter();
    let Some(value) = values.next() else {
        return CpaExecution::Unknown;
    };
    if values.next().is_some() || value.as_bytes().len() > 32 {
        return CpaExecution::Unknown;
    }
    let Some(value) = value.to_str().ok() else {
        return CpaExecution::Unknown;
    };
    let mut fields = value.split(';');
    if fields.next() != Some("v1") {
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
    if fields.next().is_some() || counters[1] > counters[0] || counters[3] > counters[2] {
        return CpaExecution::Unknown;
    }
    CpaExecution::Known {
        inference_attempts: counters[0],
        unauthorized_responses: counters[1],
        auth_recovery_attempts: counters[2],
        auth_recovery_successes: counters[3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpa_metadata_records_wire_counts_without_inventing_usage() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER, "v1;2;1;1;1".parse().unwrap());
        assert_eq!(
            from_headers(&headers),
            CpaExecution::Known {
                inference_attempts: 2,
                unauthorized_responses: 1,
                auth_recovery_attempts: 1,
                auth_recovery_successes: 1,
            }
        );
        // Existing provider protocol repair can make additional sends. This
        // counter describes the wire, not a made-up maximum of two requests.
        headers.insert(HEADER, "v1;3;1;1;1".parse().unwrap());
        assert!(matches!(
            from_headers(&headers),
            CpaExecution::Known {
                inference_attempts: 3,
                ..
            }
        ));
    }

    #[test]
    fn cpa_missing_malformed_duplicate_or_overflow_metadata_is_unknown() {
        assert_eq!(from_headers(&HeaderMap::new()), CpaExecution::Unknown);
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
            assert_eq!(from_headers(&headers), CpaExecution::Unknown, "{value}");
        }
        let mut headers = HeaderMap::new();
        headers.append(HEADER, "v1;2;1;1;1".parse().unwrap());
        headers.append(HEADER, "v1;1;0;0;0".parse().unwrap());
        assert_eq!(from_headers(&headers), CpaExecution::Unknown);
    }
}
