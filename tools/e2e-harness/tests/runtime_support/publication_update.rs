use hiroute_gateway::server::publication::GatewayPublicationSnapshotV3;
use serde_json::json;

use super::{RuntimeFixture, request};

impl RuntimeFixture {
    pub fn publication(&self) -> GatewayPublicationSnapshotV3 {
        serde_json::from_slice(
            &std::fs::read(self._directory.path().join("publication.json")).unwrap(),
        )
        .unwrap()
    }

    pub fn publish(&self, snapshot: &mut GatewayPublicationSnapshotV3) {
        let (address, nonce) = self
            .runtime_state_control
            .as_ref()
            .expect("publication test control is enabled");
        snapshot.payload_digest = snapshot.canonical_digest().unwrap();
        snapshot.validate().unwrap();
        let response = request(
            *address,
            "POST",
            "/_hiroute/e2e-control/v1",
            &[("X-HiRoute-E2E-Control-Nonce", nonce)],
            &serde_json::to_vec(&json!({
                "schema_version":"hiroute.gateway.e2e-control-request/v1",
                "request_id":format!("publish-{}", snapshot.publication_revision),
                "command":{"kind":"publication_update","snapshot":snapshot}
            }))
            .unwrap(),
        );
        assert_eq!(response.status, 200, "{:?}", response.body);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["code"], "PUBLICATION_APPLIED", "{body}");
        assert_eq!(
            body["active"]["publication_revision"],
            snapshot.publication_revision
        );
    }
}
