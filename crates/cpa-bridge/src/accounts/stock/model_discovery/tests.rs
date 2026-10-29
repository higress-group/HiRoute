use super::*;
use hiroute_diagnostics::event::{CpaDiscoveryPhase, CpaDiscoveryReason, DiagnosticEvent};
use hiroute_diagnostics::level::DiagnosticLevel;
use serde_json::json;

fn payload() -> Value {
    json!({"models":[], "hiroute_model_discovery": {
        "schema":"hiroute.cpa-model-discovery/v1", "process_id":123,
        "evicted_accounts":0, "first_failure":null, "registration_pending":false,
        "latest": {"attempt_id":1,"phase":"request","outcome":"failed",
            "reason":"proxy_rejected","elapsed_ms":47,"phase_elapsed_ms":46,
            "proxy_status":503,"timings":[{"phase":"request","outcome":"failed",
                "reason":"proxy_rejected","elapsed_ms":46}]}
    }})
}

#[test]
fn recovery_requires_current_quiescent_transient_failure() {
    for reason in [
        "timeout",
        "dns_failure",
        "connect_failure",
        "body_read_failure",
    ] {
        let mut value = payload();
        value["hiroute_model_discovery"]["latest"]["reason"] = json!(reason);
        assert_eq!(retryable_failure(&value), Some(123));
        value["hiroute_model_discovery"]["registration_pending"] = json!(true);
        assert_eq!(retryable_failure(&value), None);
        value["hiroute_model_discovery"]["registration_pending"] = json!(false);
        value["hiroute_model_discovery"]["first_failure"] =
            value["hiroute_model_discovery"]["latest"].clone();
        value["hiroute_model_discovery"]["latest"]["reason"] = json!("account_changed");
        assert_eq!(
            retryable_failure(&value),
            None,
            "retained failure cannot trigger recovery"
        );
    }
    for status in [200, 400, 401, 403, 408, 429, 500, 502, 503, 504] {
        let mut value = payload();
        value["hiroute_model_discovery"]["latest"]["reason"] = json!("http_rejected");
        value["hiroute_model_discovery"]["latest"]["http_status"] = json!(status);
        assert_eq!(
            retryable_failure(&value).is_some(),
            matches!(status, 408 | 500 | 502 | 503 | 504)
        );
    }
    for reason in [
        "missing_access_token",
        "client_version_unavailable",
        "cancelled",
        "tls_failure",
        "invalid_catalog",
        "empty_catalog",
        "account_disabled",
        "account_missing",
    ] {
        let mut value = payload();
        value["hiroute_model_discovery"]["latest"]["reason"] = json!(reason);
        assert_eq!(retryable_failure(&value), None);
    }
    assert_eq!(retryable_failure(&json!({"models":[]})), None);
}

#[test]
fn retains_first_failure_after_recovery_and_deduplicates_polling() {
    let mut observations = Observations::default();
    let mut value = payload();
    value["hiroute_model_discovery"]["first_failure"] =
        value["hiroute_model_discovery"]["latest"].clone();
    let first = observations.read(&value, Some(0));
    assert_eq!(first.len(), 2);
    assert_eq!(
        DiagnosticEvent::CpaModelDiscovery(first[0].clone()).level(),
        DiagnosticLevel::Warn
    );
    for elapsed in 100..1000 {
        value["hiroute_model_discovery"]["latest"]["elapsed_ms"] = json!(elapsed);
        assert!(observations.read(&value, Some(0)).is_empty());
    }
    value["hiroute_model_discovery"]["latest"] = json!({
        "attempt_id":2,"phase":"registration","outcome":"completed",
        "elapsed_ms":1060,"phase_elapsed_ms":1,"http_status":200,"model_count":9,
        "timings":[{"phase":"registration","outcome":"completed","elapsed_ms":1,"model_count":9}]
    });
    let recovered = observations.read(&value, Some(0));
    assert_eq!(recovered.len(), 1);
    assert!(matches!(
        &recovered[0].observation,
        CpaDiscoveryObservation::Observed {
            retained_first_failure: false,
            attempt: CpaDiscoveryAttempt {
                phase: CpaDiscoveryPhase::Registration,
                model_count: Some(9),
                ..
            },
            ..
        }
    ));
    // A later control span still sees the retained first failure with its original duration.
    let later = Observations::default().read(&value, Some(0));
    assert!(matches!(
        &later[0].observation,
        CpaDiscoveryObservation::Observed {
            retained_first_failure: true,
            attempt: CpaDiscoveryAttempt {
                reason: Some(CpaDiscoveryReason::ProxyRejected),
                proxy_status: Some(503),
                http_status: None,
                elapsed_ms: 47,
                ..
            },
            ..
        }
    ));
    let event = DiagnosticEvent::CpaModelDiscovery(later[0].clone());
    let encoded = serde_json::to_string(&event).unwrap();
    assert_eq!(
        serde_json::from_str::<DiagnosticEvent>(&encoded).unwrap(),
        event
    );
}

#[test]
fn rejects_untrusted_metadata_without_forwarding_strings_or_breaking_business_response() {
    for modification in [
        json!({"reason":"private.example/token"}),
        json!({"url":"https://private.example/token"}),
        json!({"phase":"private.example/token"}),
        json!({"http_status":999}),
        json!({"attempt_id":0}),
        json!({"timings": vec![json!({"phase":"request","outcome":"completed","elapsed_ms":0});7]}),
    ] {
        let mut value = payload();
        for (key, item) in modification.as_object().unwrap() {
            value["hiroute_model_discovery"]["latest"][key] = item.clone();
        }
        let events = Observations::default().read(&value, Some(0));
        assert!(matches!(
            events[0].observation,
            CpaDiscoveryObservation::Unavailable {
                reason: CpaDiscoveryUnavailable::InvalidMetadata
            }
        ));
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("private.example")
        );
        assert_eq!(value["models"], json!([]));
    }
    let absent = Observations::default().read(&json!({"models":[]}), None);
    assert!(matches!(
        absent[0].observation,
        CpaDiscoveryObservation::Unavailable {
            reason: CpaDiscoveryUnavailable::MetadataAbsent
        }
    ));
}

#[test]
fn observation_limit_is_explicit_and_bounded() {
    let mut observations = Observations::default();
    let mut value = payload();
    let mut emitted = Vec::new();
    for id in 1..=200 {
        value["hiroute_model_discovery"]["latest"]["attempt_id"] = json!(id);
        emitted.extend(observations.read(&value, Some(0)));
    }
    assert_eq!(observations.seen.len(), MAX_OBSERVATIONS);
    assert_eq!(emitted.len(), MAX_OBSERVATIONS + 1);
    assert!(matches!(
        emitted.last().unwrap().observation,
        CpaDiscoveryObservation::Unavailable {
            reason: CpaDiscoveryUnavailable::ObservationLimit
        }
    ));
}

#[test]
fn registration_guard_reports_the_exact_unmet_version_checks() {
    let mut value = payload();
    value["hiroute_model_discovery"]["latest"] = json!({
        "attempt_id": 2, "phase":"registration", "outcome":"discarded",
        "reason":"account_changed", "elapsed_ms":860, "phase_elapsed_ms":0,
        "http_status":200, "model_count":9, "generation_matches":true,
        "registration_epoch_matches":false, "timings":[]
    });
    let events = Observations::default().read(&value, Some(0));
    assert!(matches!(
        &events[0].observation,
        CpaDiscoveryObservation::Observed {
            attempt: CpaDiscoveryAttempt {
                generation_matches: Some(true),
                registration_epoch_matches: Some(false),
                http_status: Some(200),
                ..
            },
            ..
        }
    ));
}

#[test]
fn only_the_latest_valid_terminal_attempt_can_shorten_an_empty_wait() {
    let mut value = payload();
    assert!(latest_is_terminal(&value));
    value["hiroute_model_discovery"]["first_failure"] =
        value["hiroute_model_discovery"]["latest"].clone();
    value["hiroute_model_discovery"]["latest"]["outcome"] = json!("entered");
    value["hiroute_model_discovery"]["latest"]["reason"] = Value::Null;
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]["latest"]["phase"] = json!("catalog");
    value["hiroute_model_discovery"]["latest"]["outcome"] = json!("completed");
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]["latest"]["phase"] = json!("registration");
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]["latest"]["model_count"] = json!(0);
    assert!(latest_is_terminal(&value));
    value["hiroute_model_discovery"]["registration_pending"] = json!(true);
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]
        .as_object_mut()
        .unwrap()
        .remove("registration_pending");
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]["registration_pending"] = json!(false);
    value["hiroute_model_discovery"]["latest"]["model_count"] = json!(1);
    assert!(!latest_is_terminal(&value));
    value["hiroute_model_discovery"]["latest"]["model_count"] = json!(0);
    value["hiroute_model_discovery"]["latest"]["unexpected"] = json!(true);
    assert!(!latest_is_terminal(&value));
    assert!(!latest_is_terminal(&json!({"models":[]})));
}
