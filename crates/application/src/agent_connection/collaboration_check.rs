//! Public failure projection after the existing protected Check admission.
use crate::control::CollaborationCheckError;
use hiroute_application_api::{
    AGENT_COLLABORATION_CHECK_FAILURE_SCHEMA_V1, AgentCollaborationCheckFailureReasonV1 as Reason,
    AgentCollaborationCheckFailureV1, ErrorCode, ErrorV1, MachineEnvelopeV2,
};
use serde_json::Value;

pub(crate) fn failed(
    error: CollaborationCheckError,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    match error {
        CollaborationCheckError::Control(error) => {
            crate::failed(crate::map_control_error(error), request_id)
        }
        CollaborationCheckError::Failed(reason) => {
            let code = match reason {
                Reason::LoginRequired
                | Reason::InstalledSkillMissing
                | Reason::InstalledSkillChanged
                | Reason::InstalledSkillInvalid
                | Reason::NativeContextUnavailable
                | Reason::NativeContextChanged
                | Reason::DependencyUnavailable => ErrorCode::ActionRequired,
                Reason::CheckTimedOut | Reason::VerificationFailed => {
                    ErrorCode::ObservationUnavailable
                }
            };
            let mut error = ErrorV1::new(code);
            error.details_schema = AGENT_COLLABORATION_CHECK_FAILURE_SCHEMA_V1.into();
            MachineEnvelopeV2::failed_with_data(
                error,
                serde_json::to_value(AgentCollaborationCheckFailureV1::new(reason))
                    .expect("closed collaboration failure is serializable"),
                Some(request_id),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlReadError;
    use serde_json::json;

    #[test]
    fn native_check_failures_expose_only_closed_actionable_details() {
        for (reason, wire, code) in [
            (
                Reason::LoginRequired,
                "login_required",
                ErrorCode::ActionRequired,
            ),
            (
                Reason::InstalledSkillMissing,
                "installed_skill_missing",
                ErrorCode::ActionRequired,
            ),
            (
                Reason::InstalledSkillChanged,
                "installed_skill_changed",
                ErrorCode::ActionRequired,
            ),
            (
                Reason::CheckTimedOut,
                "check_timed_out",
                ErrorCode::ObservationUnavailable,
            ),
            (
                Reason::VerificationFailed,
                "verification_failed",
                ErrorCode::ObservationUnavailable,
            ),
        ] {
            let result = failed(
                CollaborationCheckError::Failed(reason),
                "request/check".into(),
            );
            assert_eq!(result.error.as_ref().unwrap().code, code);
            assert_eq!(
                result.error.as_ref().unwrap().details_schema,
                "hiroute.agent-collaboration-check-failure/v1"
            );
            assert_eq!(
                result.data,
                Some(json!({
                    "schema":"hiroute.agent-collaboration-check-failure/v1", "reason":wire,
                }))
            );
        }
    }

    #[test]
    fn ordinary_control_failures_keep_the_previous_envelope() {
        for error in [
            ControlReadError::NotFound,
            ControlReadError::Unavailable,
            ControlReadError::Denied,
        ] {
            assert_eq!(
                failed(error.into(), "request/check".into()),
                crate::failed(crate::map_control_error(error), "request/check".into())
            );
        }
    }
}
