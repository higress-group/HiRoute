use hiroute_application_api::{
    ComputeSubscriptionLoginRequestV1, ErrorCode, LocalControlRequestV2, MachineEnvelopeV2,
};
use serde_json::Value;

use super::{ApplicationService, failed, management_port, map_management_error, succeeded};

/// Independent login has no routing authority. Only protected input references may cross this
/// boundary; the daemon resolves and consumes the exact session-bound callback once.
pub(crate) fn dispatch(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Ok(action) = serde_json::from_value::<ComputeSubscriptionLoginRequestV1>(request.payload)
    else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    if !action.valid() {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let Some(management) = management_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match management.manage_subscription_login(action) {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(map_management_error(error), request.request_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_application_api::{LOCAL_CONTROL_SCHEMA_V2, PrincipalV1};
    use serde_json::json;

    fn request(payload: Value) -> LocalControlRequestV2 {
        LocalControlRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "login-request".into(),
            principal: PrincipalV1::ambient_local_peer(),
            operation_id: "ManageSubscriptionLogin".into(),
            payload,
            protected_grant: None,
        }
    }

    #[test]
    fn login_is_a_real_dispatch_route_and_raw_oauth_input_fails_before_the_port() {
        let service = ApplicationService::default();
        let response = service.dispatch(request(json!({"action":"start","provider":"claude"})));
        assert_eq!(response.error.unwrap().code, ErrorCode::DaemonUnavailable);
        let response = service.dispatch(request(json!({
            "action":"callback","login_ref":"login/one",
            "input_candidate":{"candidate_ref":"candidate/subscription-login/one","candidate_revision":1},
            "code":"must-not-be-accepted"
        })));
        assert_eq!(response.error.unwrap().code, ErrorCode::InvalidArguments);
    }

    #[test]
    fn existing_authorize_does_not_accept_independent_login_actions() {
        let mut request = request(json!({"action":"start","provider":"codex"}));
        request.operation_id = "AuthorizeComputeConnection".into();
        let response = ApplicationService::default().dispatch(request);
        assert_eq!(response.error.unwrap().code, ErrorCode::InvalidArguments);
    }
}
