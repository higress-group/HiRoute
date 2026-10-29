use super::{ActionResult, MutationOutcome, mutation_outcome_result, mutation_result};
use hiroute_application_api::{CanonicalDigest, ClientOperationViewV1};

#[test]
fn action_results_and_failures_classify_without_business_text() {
    use crate::failure::DesktopFailure;
    use hiroute_client_core::{ClientFailure, FailureCode};
    use hiroute_diagnostics::error::EventErrorCode;
    assert_eq!(mutation_result("succeeded"), ActionResult::Applied);
    assert_eq!(
        mutation_result("needs_attention"),
        ActionResult::Failed {
            code: EventErrorCode::Recovery
        }
    );
    assert_eq!(
        mutation_result("rolled_back"),
        ActionResult::Failed {
            code: EventErrorCode::ExternalError
        }
    );
    let succeeded = MutationOutcome {
        state: "submitted".into(),
        operation: Some(ClientOperationViewV1 {
            model_budget_conflict: None,
            operation_id: "op_diagnostics".into(),
            state: "succeeded".into(),
            sequence: 1,
            cancellable: false,
            accepted_digest: CanonicalDigest::of_bytes(b"diagnostics"),
            safe_error_code: None,
        }),
    };
    assert_eq!(mutation_outcome_result(&succeeded), ActionResult::Applied);
    let unknown = MutationOutcome {
        state: "response_unknown".into(),
        operation: None,
    };
    assert_eq!(
        mutation_outcome_result(&unknown),
        ActionResult::Failed {
            code: EventErrorCode::ExternalError
        }
    );
    let native = |code: &str| DesktopFailure::Native { code: code.into() };
    assert_eq!(
        native("window_denied").event_code(),
        EventErrorCode::WindowDenied
    );
    assert_eq!(
        native("RESIDENT_UNAVAILABLE").event_code(),
        EventErrorCode::ResidentUnavailable
    );
    for (source, expected) in [
        (
            "CLIENT_CONTEXT_INVALID",
            EventErrorCode::ClientContextInvalid,
        ),
        ("READ_SNAPSHOT_CHANGED", EventErrorCode::ReadSnapshotChanged),
        ("CONFIRMATION_STALE", EventErrorCode::ConfirmationStale),
        ("RESPONSE_DATA_MISSING", EventErrorCode::ResponseDataMissing),
        (
            "PRIVATE_UNRECOGNIZED_FAILURE",
            EventErrorCode::ExternalError,
        ),
    ] {
        assert_eq!(native(source).event_code(), expected);
    }
    let transport = DesktopFailure::Transport {
        failure: ClientFailure::before_send(FailureCode::TransportUnavailable),
    };
    assert_eq!(transport.event_code(), EventErrorCode::TransportUnavailable);
}
