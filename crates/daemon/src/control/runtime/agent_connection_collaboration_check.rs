//! Project native implementation failures into the closed collaboration-check contract.
use hiroute_application::control::CollaborationCheckError;
use hiroute_application_api::AgentCollaborationCheckFailureReasonV1 as Reason;
use hiroute_integrations::QoderNativeError;

pub(super) fn qoder_failure(error: QoderNativeError) -> CollaborationCheckError {
    let reason = match error.stage {
        "native login required" => Reason::LoginRequired,
        "installed user Skill missing" => Reason::InstalledSkillMissing,
        "installed user Skill changed" => Reason::InstalledSkillChanged,
        "installed user Skill target"
        | "installed user Skill read"
        | "installed user Skill encoding" => Reason::InstalledSkillInvalid,
        "selected native context" => Reason::NativeContextUnavailable,
        "native context changed" => Reason::NativeContextChanged,
        "selected executable"
        | "trusted CLI"
        | "native executable identity"
        | "trusted CLI identity"
        | "trusted CLI path"
        | "trusted CLI baseline"
        | "trusted CLI contract" => Reason::DependencyUnavailable,
        "native check deadline" => Reason::CheckTimedOut,
        _ => Reason::VerificationFailed,
    };
    CollaborationCheckError::Failed(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_failures_keep_actionable_causes_without_exposing_stage_text() {
        for (stage, expected) in [
            ("native login required", Reason::LoginRequired),
            (
                "installed user Skill missing",
                Reason::InstalledSkillMissing,
            ),
            (
                "installed user Skill changed",
                Reason::InstalledSkillChanged,
            ),
            ("installed user Skill target", Reason::InstalledSkillInvalid),
            ("native context changed", Reason::NativeContextChanged),
            ("native check deadline", Reason::CheckTimedOut),
            (
                "unknown native failure /private/user token=never-export",
                Reason::VerificationFailed,
            ),
        ] {
            assert_eq!(
                qoder_failure(QoderNativeError { stage }),
                CollaborationCheckError::Failed(expected)
            );
        }
    }
}
