use super::*;
use crate::gateway_ports::GatewayPublicationAdapter;
use hiroute_application::client_access::ClientAccessPort;
use hiroute_application::publication::{PublicationTargetError, PublicationTargetPort};
use hiroute_application_api::ClientGatewayStateV1;
use hiroute_domain::OperationState;
use hiroute_gateway::server::publication::GatewayPublicationInstaller;
use std::sync::Arc;

#[test]
fn service_status_requires_resumed_target_and_no_retained_writer_claim() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::service_status::service_status_requires_resumed_target_and_no_retained_writer_claim",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        root.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    assert_eq!(
        runtime.adapter.service_status().unwrap().gateway,
        ClientGatewayStateV1::NotComposed
    );
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(root.path().join("lkg.json")).unwrap(),
    )));
    *runtime.adapter.publication_target.lock().unwrap() = Some(target.clone());
    assert_eq!(
        runtime.adapter.service_status().unwrap().gateway,
        ClientGatewayStateV1::Unavailable
    );
    target.resume_requests().unwrap();
    assert_eq!(
        runtime.adapter.service_status().unwrap().gateway,
        ClientGatewayStateV1::Empty
    );
    let publication =
        serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(include_bytes!(
            "../../../../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
    let record =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    {
        let stores = runtime.adapter.stores_lock().unwrap();
        stores.control().prepare_publication(&record, None).unwrap();
        stores
            .control()
            .mark_publication_active(
                &WorkspaceId::default(),
                record.publication_revision,
                &record.digest,
            )
            .unwrap();
    }
    assert_eq!(
        runtime.adapter.service_status().unwrap().gateway,
        ClientGatewayStateV1::Unavailable
    );
    target.activate_verified(&record).unwrap();
    let ready = runtime.adapter.service_status().unwrap();
    assert_eq!(ready.gateway, ClientGatewayStateV1::Ready);
    assert!(ready.recovery_ready && ready.mutation_available);
    target.suspend_requests().unwrap();
    assert!(target.verify_installed(&record).unwrap());
    let suspended = runtime.adapter.service_status().unwrap();
    assert_eq!(suspended.gateway, ClientGatewayStateV1::Unavailable);
    assert!(!suspended.mutation_available);
    target.resume_requests().unwrap();
    let (mut operation, _, _) =
        routing_operation(&runtime.adapter, publication, None, "status-claim");
    assert!(!runtime.adapter.service_status().unwrap().recovery_ready);
    operation.transition(OperationState::RollingBack).unwrap();
    operation
        .transition(OperationState::NeedsAttention)
        .unwrap();
    {
        let stores = runtime.adapter.stores_lock().unwrap();
        stores.control().finish_operation(&mut operation).unwrap();
        let recoverable = stores.control().recoverable_operations().unwrap();
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].operation_id, operation.operation_id);
        assert!(stores.control().writer_claim_operation().unwrap().is_some());
    }
    let blocked = runtime.adapter.service_status().unwrap();
    assert!(!blocked.recovery_ready && !blocked.mutation_available);
    assert_eq!(blocked.gateway, ClientGatewayStateV1::Unavailable);
}

#[test]
fn service_status_observes_product_and_target_in_one_store_read_scope() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::service_status::service_status_observes_product_and_target_in_one_store_read_scope",
    ) {
        return;
    }
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;
    struct ObservationBarrier {
        entered: Arc<Barrier>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl PublicationTargetPort for ObservationBarrier {
        fn observes_verified(
            &self,
            record: Option<&PublicationRecordV1>,
        ) -> Result<bool, PublicationTargetError> {
            assert!(record.is_none());
            self.entered.wait();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(true)
        }
        fn activate_verified(&self, _: &PublicationRecordV1) -> Result<(), PublicationTargetError> {
            panic!("read-only observation")
        }
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        root.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let (release, wait) = mpsc::channel();
    *runtime.adapter.publication_target.lock().unwrap() = Some(Arc::new(ObservationBarrier {
        entered: entered.clone(),
        release: Mutex::new(wait),
    }));
    let adapter = runtime.adapter.clone();
    let observer = std::thread::spawn(move || adapter.service_status().unwrap());
    entered.wait();
    // A concurrent activation/admission cannot replace product/claim facts mid-observation.
    let stores_busy = runtime.adapter.stores.try_lock().is_err();
    let composition_busy = runtime.adapter.publication_target.try_lock().is_err();
    release.send(()).unwrap();
    let status = observer.join().unwrap();
    assert!(stores_busy && composition_busy);
    assert_eq!(status.gateway, ClientGatewayStateV1::Empty);
    assert!(status.recovery_ready);
}
