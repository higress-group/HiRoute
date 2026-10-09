//! Typed shared-client façade for same-UID local-trust Worker commands.
//!
//! Instance identity is derived by the daemon. Plan policy, idempotency, admission, lifecycle
//! and state transitions remain server-owned.

use hiroute_application_api::{
    DelegationAcceptedV1, DelegationCancelV1, DelegationGetV1, DelegationListV1,
    DelegationResultV1, DelegationRunViewV1, DelegationWaitV1, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlWireRequestV2, MachineEnvelopeV2, WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1,
    WORKER_SETTINGS_GET_OPERATION_V1, WORKER_SETTINGS_SET_OPERATION_V1, WorkPlanListV1,
    WorkerCancelRequestV1, WorkerContinueRequestV1, WorkerDependenciesDiscoverRequestV1,
    WorkerDependenciesSelectRequestV1, WorkerDependenciesViewV1, WorkerExecRequestV1,
    WorkerExecutorAvailabilityListV1, WorkerListRequestV1, WorkerPlansRequestV1, WorkerReadDataV1,
    WorkerReadRequestV1, WorkerResidualConfirmRequestV1, WorkerResultRequestV1, WorkerSettingsV1,
    WorkerStatusRequestV1, WorkerWaitRequestV1,
};

use crate::{Client, ClientFailure, FailureCode};

impl Client {
    async fn worker_call<P, R>(
        &self,
        operation: &str,
        request_id: &str,
        payload: &P,
    ) -> Result<MachineEnvelopeV2<R>, ClientFailure>
    where
        P: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let request = LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.into(),
            operation_id: operation.into(),
            payload: serde_json::to_value(payload)
                .map_err(|_| ClientFailure::before_send(FailureCode::FrameInvalid))?,
            protected_grant: None,
        };
        self.call_typed(request).await
    }

    pub async fn worker_plans(
        &self,
        request_id: &str,
        request: &WorkerPlansRequestV1,
    ) -> Result<MachineEnvelopeV2<WorkPlanListV1>, ClientFailure> {
        self.worker_call("WorkerPlans", request_id, request).await
    }

    pub async fn worker_settings(
        &self,
        request_id: &str,
    ) -> Result<MachineEnvelopeV2<WorkerSettingsV1>, ClientFailure> {
        self.worker_call(
            WORKER_SETTINGS_GET_OPERATION_V1,
            request_id,
            &hiroute_application_api::ClientEmptyRequestV1 {},
        )
        .await
    }

    pub async fn set_worker_settings(
        &self,
        request_id: &str,
        settings: &WorkerSettingsV1,
    ) -> Result<MachineEnvelopeV2<WorkerSettingsV1>, ClientFailure> {
        self.worker_call(WORKER_SETTINGS_SET_OPERATION_V1, request_id, settings)
            .await
    }

    pub async fn worker_executor_availability(
        &self,
        request_id: &str,
    ) -> Result<MachineEnvelopeV2<WorkerExecutorAvailabilityListV1>, ClientFailure> {
        self.worker_call(
            WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1,
            request_id,
            &hiroute_application_api::ClientEmptyRequestV1 {},
        )
        .await
    }

    pub async fn worker_list(
        &self,
        request_id: &str,
        request: &WorkerListRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationListV1>, ClientFailure> {
        self.worker_call("WorkerList", request_id, request).await
    }

    pub async fn worker_dependencies_discover(
        &self,
        request_id: &str,
        request: &WorkerDependenciesDiscoverRequestV1,
    ) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, ClientFailure> {
        self.worker_call(
            hiroute_application_api::WORKER_DEPENDENCIES_DISCOVER_OPERATION_V1,
            request_id,
            request,
        )
        .await
    }

    pub async fn select_worker_dependencies(
        &self,
        request_id: &str,
        request: &WorkerDependenciesSelectRequestV1,
    ) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, ClientFailure> {
        self.worker_call(
            hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1,
            request_id,
            request,
        )
        .await
    }

    pub async fn worker_exec(
        &self,
        request_id: &str,
        request: &WorkerExecRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationAcceptedV1>, ClientFailure> {
        self.worker_call("WorkerExec", request_id, request).await
    }

    pub async fn worker_status(
        &self,
        request_id: &str,
        request: &WorkerStatusRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationGetV1>, ClientFailure> {
        self.worker_call("WorkerStatus", request_id, request).await
    }

    pub async fn worker_wait(
        &self,
        request_id: &str,
        request: &WorkerWaitRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationWaitV1>, ClientFailure> {
        self.worker_call("WorkerWait", request_id, request).await
    }

    pub async fn worker_result(
        &self,
        request_id: &str,
        request: &WorkerResultRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationResultV1>, ClientFailure> {
        self.worker_call("WorkerResult", request_id, request).await
    }

    pub async fn worker_read(
        &self,
        request_id: &str,
        request: &WorkerReadRequestV1,
    ) -> Result<MachineEnvelopeV2<WorkerReadDataV1>, ClientFailure> {
        self.worker_call("WorkerRead", request_id, request).await
    }

    pub async fn worker_continue(
        &self,
        request_id: &str,
        request: &WorkerContinueRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationAcceptedV1>, ClientFailure> {
        self.worker_call("WorkerContinue", request_id, request)
            .await
    }

    pub async fn confirm_worker_residual(
        &self,
        request_id: &str,
        request: &WorkerResidualConfirmRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationRunViewV1>, ClientFailure> {
        self.worker_call("WorkerConfirmResidual", request_id, request)
            .await
    }

    pub async fn worker_cancel(
        &self,
        request_id: &str,
        request: &WorkerCancelRequestV1,
    ) -> Result<MachineEnvelopeV2<DelegationCancelV1>, ClientFailure> {
        self.worker_call("WorkerCancel", request_id, request).await
    }
}
