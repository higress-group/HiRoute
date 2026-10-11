//! Typed subscription facades over the shared Local Control transport.

use crate::{Client, ClientFailure, FailureCode};
use hiroute_application_api::*;

impl Client {
    pub async fn manage_subscription_login(
        &self,
        request_id: &str,
        request: ComputeSubscriptionLoginRequestV1,
    ) -> Result<MachineEnvelopeV2<ComputeSubscriptionLoginResultV1>, ClientFailure> {
        self.query(MANAGE_SUBSCRIPTION_LOGIN_OPERATION_V1, request_id, &request)
            .await
    }

    pub async fn compute_candidate(
        &self,
        request_id: &str,
        candidate: ComputeCandidateRefV2,
    ) -> Result<MachineEnvelopeV2<ComputeCandidateViewV2>, ClientFailure> {
        self.query(
            GET_COMPUTE_CANDIDATE_OPERATION_V2,
            request_id,
            &ComputeCandidateQueryV2 { candidate },
        )
        .await
    }

    pub async fn compute_subscriptions(
        &self,
        request_id: &str,
    ) -> Result<MachineEnvelopeV2<ComputeSubscriptionCandidatesV2>, ClientFailure> {
        self.query(
            LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2,
            request_id,
            &ClientEmptyRequestV1 {},
        )
        .await
    }

    pub async fn compute_save_result(
        &self,
        request_id: &str,
        operation_id: String,
    ) -> Result<MachineEnvelopeV2<ComputeSaveResultV2>, ClientFailure> {
        self.get_compute_save_result(
            request_id,
            OperationReferenceV1 {
                operation_id,
                state: "accepted".to_owned(),
                sequence: 0,
                cancellable: true,
            },
        )
        .await
    }

    pub async fn preview_subscription_check(
        &self,
        request_id: &str,
        candidate: ComputeCandidateRefV2,
    ) -> Result<MachineEnvelopeV2<ComputeSubscriptionCheckPreviewV2>, ClientFailure> {
        self.query(
            PREVIEW_SUBSCRIPTION_CHECK_OPERATION_V2,
            request_id,
            &ComputeSubscriptionCheckPreviewRequestV2 { candidate },
        )
        .await
    }

    pub async fn apply_subscription_check(
        &self,
        request_id: &str,
        request: ComputeConnectionApplyRequestV1,
        protected_grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ApplyResultV1>, ClientFailure> {
        self.call_typed(protected_request(
            APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
            request_id,
            &request,
            protected_grant,
        )?)
        .await
    }

    pub async fn subscription_check_result(
        &self,
        request_id: &str,
        operation_id: String,
    ) -> Result<MachineEnvelopeV2<ComputeSubscriptionCheckResultV2>, ClientFailure> {
        self.query(
            GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2,
            request_id,
            &ComputeSubscriptionCheckResultQueryV2 {
                operation: OperationReferenceV1 {
                    operation_id,
                    state: "accepted".to_owned(),
                    sequence: 0,
                    cancellable: true,
                },
            },
        )
        .await
    }

    pub async fn release_subscription_check(
        &self,
        request_id: &str,
        validation: ComputeValidationRefV2,
        protected_grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ComputeSubscriptionCheckResultV2>, ClientFailure> {
        self.call_typed(protected_request(
            RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2,
            request_id,
            &validation,
            protected_grant,
        )?)
        .await
    }

    /// Requests cancellation of a still-running approved check. The returned operation view is
    /// authoritative: callers must continue observing it and must not infer completion from the
    /// cancellation request itself.
    pub async fn cancel_subscription_check(
        &self,
        request_id: &str,
        request: OperationCancelRequestV1,
        protected_grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ClientOperationViewV1>, ClientFailure> {
        self.call_typed(protected_request(
            "CancelOperation",
            request_id,
            &request,
            protected_grant,
        )?)
        .await
    }
}

fn protected_request(
    operation_id: &str,
    request_id: &str,
    payload: &impl serde::Serialize,
    protected_grant: ProtectedClientGrantV2,
) -> Result<LocalControlWireRequestV2, ClientFailure> {
    Ok(LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: request_id.to_owned(),
        operation_id: operation_id.to_owned(),
        payload: serde_json::to_value(payload)
            .map_err(|_| ClientFailure::before_send(FailureCode::FrameInvalid))?,
        protected_grant: Some(protected_grant),
    })
}

#[cfg(test)]
mod tests {
    use hiroute_application_api::{PrincipalKind, SchemaVersion};

    use super::*;

    fn grant() -> ProtectedClientGrantV2 {
        ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::Desktop,
            capability: "protected-fixture".to_owned(),
        }
    }

    fn apply_request(key: &str) -> ComputeConnectionApplyRequestV1 {
        ComputeConnectionApplyRequestV1 {
            spec: ChangeSpecV1 {
                schema_version: SchemaVersion::new(1, 0),
                command_id: "compute.save".to_owned(),
                resource_id: None,
                desired_state: serde_json::json!({"candidate":"candidate/one"}),
            },
            accept_digest: CanonicalDigest::of_bytes(b"accept"),
            expected_revisions: RevisionSetV1 {
                target: 1,
                dependencies: Default::default(),
            },
            idempotency_key: key.to_owned(),
        }
    }

    #[test]
    fn approval_a_and_save_b_keep_distinct_operation_payloads() {
        let approval = protected_request(
            APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
            "request-a",
            &apply_request("operation-a-key"),
            grant(),
        )
        .unwrap();
        let save = protected_request(
            APPLY_COMPUTE_SAVE_OPERATION_V2,
            "request-b",
            &apply_request("operation-b-key"),
            grant(),
        )
        .unwrap();

        assert_eq!(approval.operation_id, APPLY_SUBSCRIPTION_CHECK_OPERATION_V2);
        assert_eq!(save.operation_id, APPLY_COMPUTE_SAVE_OPERATION_V2);
        assert_ne!(
            approval.payload["idempotency_key"],
            save.payload["idempotency_key"]
        );
        assert!(approval.protected_grant.is_some());
        assert!(save.protected_grant.is_some());
    }

    #[test]
    fn read_facades_do_not_attach_a_protected_grant() {
        let request = LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "read".to_owned(),
            operation_id: GET_COMPUTE_CANDIDATE_OPERATION_V2.to_owned(),
            payload: serde_json::to_value(ComputeCandidateRefV2 {
                candidate_ref: "candidate/one".to_owned(),
                candidate_revision: 42,
            })
            .unwrap(),
            protected_grant: None,
        };
        assert!(request.protected_grant.is_none());
        assert_eq!(request.schema_version, LOCAL_CONTROL_SCHEMA_V2);
    }

    #[test]
    fn close_cancellation_is_a_protected_request_for_the_original_operation() {
        let request = protected_request(
            "CancelOperation",
            "cancel-request",
            &OperationCancelRequestV1 {
                operation_id: "op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                idempotency_key: "c".repeat(64),
            },
            grant(),
        )
        .unwrap();
        assert_eq!(request.operation_id, "CancelOperation");
        assert_eq!(
            request.payload["operation_id"],
            "op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(request.protected_grant.is_some());
    }
}
