use super::*;
use hiroute_application_api::DecisionRemovalPreviewRequestV1;

impl ApplicationService {
    pub(super) fn preview_decision_removal(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Ok(input) = serde_json::from_value::<DecisionRemovalPreviewRequestV1>(request.payload)
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if input.id.is_empty() || input.id.len() > 256 || input.expected_revision == 0 {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(port) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.routing.as_deref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        match port.preview_decision_removal(&WorkspaceId::default(), input) {
            Ok(result) => succeeded(result, request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }
}
