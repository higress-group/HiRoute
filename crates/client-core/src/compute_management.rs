//! Thin typed compute-management calls over Local Control v2.

use hiroute_application_api::{
    ComputeManagementQueryV2, ComputeManagementSnapshotV2, ComputeScanRequestV1,
    ComputeScanResultV1, MachineEnvelopeV2,
};

use crate::{Client, ClientFailure};

impl Client {
    pub async fn scan_compute(
        &self,
        request_id: &str,
    ) -> Result<MachineEnvelopeV2<ComputeScanResultV1>, ClientFailure> {
        self.query("ScanCompute", request_id, &ComputeScanRequestV1 {})
            .await
    }

    pub async fn compute_management_snapshot(
        &self,
        request_id: &str,
        query: ComputeManagementQueryV2,
    ) -> Result<MachineEnvelopeV2<ComputeManagementSnapshotV2>, ClientFailure> {
        let operation = if query.source_id.is_some() {
            "GetCompute"
        } else {
            "ListCompute"
        };
        self.query(operation, request_id, &query).await
    }
    pub async fn compute_management_snapshot_v3(
        &self,
        request_id: &str,
        query: ComputeManagementQueryV2,
    ) -> Result<
        MachineEnvelopeV2<hiroute_application_api::ComputeManagementSnapshotV3>,
        ClientFailure,
    > {
        let operation = if query.source_id.is_some() {
            hiroute_application_api::GET_COMPUTE_OPERATION_V3
        } else {
            hiroute_application_api::LIST_COMPUTE_OPERATION_V3
        };
        self.query(operation, request_id, &query).await
    }
}
