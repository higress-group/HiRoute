use super::*;

#[test]
fn missing_worker_dependencies_leave_normal_gateway_publication_available() {
    use hiroute_application::publication::PublicationTargetPort;
    use hiroute_gateway::server::composition::RuntimePublicationFeed;
    use hiroute_gateway::server::publication::GatewayPublicationInstaller;
    use std::sync::Arc;

    let publication = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(
        include_str!("../../../../../e2e/product/fixtures/routing/current-publication.v3.json")
            .as_bytes(),
    )
    .unwrap();
    let record = hiroute_domain::PublicationRecordV1::from_publication(
        publication.workspace_id.clone(),
        &publication,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let installer =
        Arc::new(GatewayPublicationInstaller::open(directory.path().join("lkg.json")).unwrap());
    let gateway = crate::gateway_ports::GatewayPublicationAdapter::new(installer);
    gateway.activate_verified(&record).unwrap();
    gateway.resume_requests().unwrap();
    let before = gateway.pin().unwrap();
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
        let config = crate::delegation::installation::WorkerInstallationConfig {
            harness,
            adapter: PathBuf::from("/missing-worker-adapter"),
            harness_binary: PathBuf::from("/missing-worker-harness"),
            node_binary: None,
        };
        assert_eq!(
            crate::delegation::installation::check_installation(&config),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
    }
    let after = gateway
        .pin()
        .expect("ordinary requests still have an active publication");
    assert_eq!(before.payload_digest(), after.payload_digest());
    assert_eq!(after.authority_id(), publication.authority_id);
}
