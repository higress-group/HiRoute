use crate::server::core_runtime::{adapters::ProtocolAdapterError, model_ir::ModelIrError};
use hiroute_diagnostics::event::{ResponseFailureReason as Reason, ResponseFailureStage as Stage};
use std::sync::Arc;

pub(super) fn note(stage: Stage, reason: Reason) {
    if let Some(observation) = crate::server::core_runtime::observation::active_request() {
        observation.response_failure(stage, reason);
    }
}

pub(super) fn adapter(stage: Stage, error: ProtocolAdapterError) -> Arc<str> {
    note(stage, category(&error));
    Arc::from("provider response processing failed")
}

fn category(error: &ProtocolAdapterError) -> Reason {
    match error {
        ProtocolAdapterError::ModelIr(error) => match error {
            ModelIrError::BufferLimit(_) => Reason::ResourceLimit,
            ModelIrError::InvalidJson(_) => Reason::InvalidJson,
            ModelIrError::InvalidSse(_) => Reason::InvalidSse,
            ModelIrError::ExpectedObject | ModelIrError::InvalidField(_) => Reason::InvalidField,
            ModelIrError::InvalidResponseLifecycle(_) => Reason::InvalidLifecycle,
            ModelIrError::MissingTerminalEvent => Reason::MissingTerminal,
            ModelIrError::DuplicateTerminalEvent => Reason::DuplicateTerminal,
            _ => Reason::OtherProtocol,
        },
        ProtocolAdapterError::ClientUnrepresentable(_) => Reason::Unrepresentable,
        _ => Reason::OtherProtocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_categories_distinguish_local_limits_and_protocol_failures_without_payloads() {
        for (error, expected) in [
            (ModelIrError::BufferLimit(100), Reason::ResourceLimit),
            (
                ModelIrError::InvalidJson("secret provider body".into()),
                Reason::InvalidJson,
            ),
            (
                ModelIrError::InvalidSse("secret event".into()),
                Reason::InvalidSse,
            ),
            (ModelIrError::MissingTerminalEvent, Reason::MissingTerminal),
            (
                ModelIrError::DuplicateTerminalEvent,
                Reason::DuplicateTerminal,
            ),
        ] {
            let category = category(&error.into());
            assert_eq!(category, expected);
            assert!(!serde_json::to_string(&category).unwrap().contains("secret"));
        }
    }
}
