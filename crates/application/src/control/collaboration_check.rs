//! Only native collaboration checks carry action-specific failure details.
use super::ControlReadError;
use hiroute_application_api::AgentCollaborationCheckFailureReasonV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollaborationCheckError {
    Control(ControlReadError),
    Failed(AgentCollaborationCheckFailureReasonV1),
}

impl From<ControlReadError> for CollaborationCheckError {
    fn from(error: ControlReadError) -> Self {
        Self::Control(error)
    }
}
