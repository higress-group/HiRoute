//! Typed compute-management calls over the existing Local Control transport.
//!
//! Local saves use the owner-only transport. Check operations retain their separate native
//! probe authorization; no method accepts credential bytes or a protected input locator.

use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, ApplyResultV1,
    CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1, CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1,
    CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1, CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1,
    ComputeCandidateQueryV2, ComputeCandidateRefV2, ComputeCandidateViewV2,
    ComputeConnectionApplyRequestV1, ComputeManagementChangeV2, ComputeSavePreviewRequestV2,
    ComputeSavePreviewV2, ComputeSaveResultQueryV2, ComputeSaveResultV2,
    GET_COMPUTE_CANDIDATE_OPERATION_V2, GET_COMPUTE_SAVE_RESULT_OPERATION_V2,
    LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MachineEnvelopeV2,
    ModelConnectionCheckViewV1, NativeModelConnectionCancelRequestV1,
    NativeModelConnectionCheckRequestV1, OperationReferenceV1,
    PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1, PREVIEW_COMPUTE_SAVE_OPERATION_V2,
    PrepareDiscoveredModelConnectionRequestV1, ProtectedClientGrantV2,
    RegisteredModelConnectionCheckRequestV1, SavedModelConnectionCheckRequestV1,
};

use crate::{Client, ClientFailure, FailureCode};

impl Client {
    pub async fn get_compute_candidate(
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

    pub async fn preview_compute_save(
        &self,
        request_id: &str,
        change: ComputeManagementChangeV2,
    ) -> Result<MachineEnvelopeV2<ComputeSavePreviewV2>, ClientFailure> {
        self.query(
            PREVIEW_COMPUTE_SAVE_OPERATION_V2,
            request_id,
            &ComputeSavePreviewRequestV2 { change },
        )
        .await
    }

    pub async fn apply_compute_save(
        &self,
        request_id: &str,
        request: ComputeConnectionApplyRequestV1,
    ) -> Result<MachineEnvelopeV2<ApplyResultV1>, ClientFailure> {
        self.query(APPLY_COMPUTE_SAVE_OPERATION_V2, request_id, &request)
            .await
    }

    pub async fn get_compute_save_result(
        &self,
        request_id: &str,
        operation: OperationReferenceV1,
    ) -> Result<MachineEnvelopeV2<ComputeSaveResultV2>, ClientFailure> {
        self.query(
            GET_COMPUTE_SAVE_RESULT_OPERATION_V2,
            request_id,
            &ComputeSaveResultQueryV2 { operation },
        )
        .await
    }

    pub async fn check_native_model_connection(
        &self,
        request_id: &str,
        request: NativeModelConnectionCheckRequestV1,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ModelConnectionCheckViewV1>, ClientFailure> {
        self.compute_management_call(
            request_id,
            CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1,
            request,
            grant,
        )
        .await
    }

    pub async fn check_registered_model_connection(
        &self,
        request_id: &str,
        request: RegisteredModelConnectionCheckRequestV1,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ModelConnectionCheckViewV1>, ClientFailure> {
        self.compute_management_call(
            request_id,
            CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1,
            request,
            grant,
        )
        .await
    }

    pub async fn prepare_discovered_model_connection(
        &self,
        request_id: &str,
        request: PrepareDiscoveredModelConnectionRequestV1,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ComputeCandidateViewV2>, ClientFailure> {
        self.compute_management_call(
            request_id,
            PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
            request,
            grant,
        )
        .await
    }

    /// Local OS-authenticated management entry; Desktop continues to use its scoped grant method.
    pub async fn prepare_local_discovered_model_connection(
        &self,
        request_id: &str,
        request: PrepareDiscoveredModelConnectionRequestV1,
    ) -> Result<MachineEnvelopeV2<ComputeCandidateViewV2>, ClientFailure> {
        self.query(
            PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
            request_id,
            &request,
        )
        .await
    }

    pub async fn check_saved_model_connection(
        &self,
        request_id: &str,
        request: SavedModelConnectionCheckRequestV1,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<ModelConnectionCheckViewV1>, ClientFailure> {
        self.compute_management_call(
            request_id,
            CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1,
            request,
            grant,
        )
        .await
    }

    pub async fn cancel_native_model_connection_check(
        &self,
        request_id: &str,
        check_id: String,
    ) -> Result<MachineEnvelopeV2<()>, ClientFailure> {
        self.query(
            CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1,
            request_id,
            &NativeModelConnectionCancelRequestV1 { check_id },
        )
        .await
    }

    pub async fn check_native_model_connection_v2(
        &self,
        request_id: &str,
        request: hiroute_application_api::NativeModelConnectionCheckRequestV2,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<hiroute_application_api::ModelConnectionCheckViewV2>, ClientFailure>
    {
        self.compute_management_call(
            request_id,
            hiroute_application_api::CHECK_NATIVE_MODEL_CONNECTION_V2,
            request,
            grant,
        )
        .await
    }
    pub async fn check_registered_model_connection_v2(
        &self,
        request_id: &str,
        request: hiroute_application_api::RegisteredModelConnectionCheckRequestV2,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<hiroute_application_api::ModelConnectionCheckViewV2>, ClientFailure>
    {
        self.compute_management_call(
            request_id,
            hiroute_application_api::CHECK_REGISTERED_MODEL_CONNECTION_V2,
            request,
            grant,
        )
        .await
    }
    pub async fn check_saved_model_connection_v2(
        &self,
        request_id: &str,
        request: hiroute_application_api::SavedModelConnectionCheckRequestV2,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<hiroute_application_api::ModelConnectionCheckViewV2>, ClientFailure>
    {
        self.compute_management_call(
            request_id,
            hiroute_application_api::CHECK_SAVED_MODEL_CONNECTION_V2,
            request,
            grant,
        )
        .await
    }
    async fn compute_management_call<P, R>(
        &self,
        request_id: &str,
        operation_id: &str,
        payload: P,
        grant: ProtectedClientGrantV2,
    ) -> Result<MachineEnvelopeV2<R>, ClientFailure>
    where
        P: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let payload = serde_json::to_value(payload)
            .map_err(|_| ClientFailure::before_send(FailureCode::FrameInvalid))?;
        self.call_typed(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.into(),
            operation_id: operation_id.into(),
            payload,
            protected_grant: Some(grant),
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_names_are_the_fixed_shared_methods() {
        assert_eq!(GET_COMPUTE_CANDIDATE_OPERATION_V2, "GetComputeCandidate");
        assert_eq!(PREVIEW_COMPUTE_SAVE_OPERATION_V2, "PreviewComputeSave");
        assert_eq!(APPLY_COMPUTE_SAVE_OPERATION_V2, "ApplyComputeSave");
        assert_eq!(GET_COMPUTE_SAVE_RESULT_OPERATION_V2, "GetComputeSaveResult");
        assert_eq!(
            CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1,
            "CheckSavedModelConnection"
        );
        assert_eq!(
            PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
            "PrepareDiscoveredModelConnection"
        );
    }
}
