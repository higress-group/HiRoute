use super::*;
use hiroute_domain::GatewayPublicationRevision;
use hiroute_gateway::server::publication::PublicationFailpoint;
use hiroute_gateway::server::{dispatch::GatewayRequestAuthority, request_plan::IngressProtocol};

fn record(revision: u64) -> PublicationRecordV1 {
    let mut publication = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(
        include_bytes!("../../../../e2e/product/fixtures/routing/current-publication.v3.json"),
    )
    .unwrap();
    publication.publication_revision = GatewayPublicationRevision::new(revision).unwrap();
    PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication).unwrap()
}

#[test]
fn publication_readiness_requires_complete_identity_and_resumed_requests() {
    let root = tempfile::tempdir().unwrap();
    let target = GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(root.path().join("lkg.json")).unwrap(),
    ));
    assert!(!target.observes_verified(None).unwrap());
    target.resume_requests().unwrap();
    assert!(target.observes_verified(None).unwrap());
    let first = record(1);
    target.activate_verified(&first).unwrap();
    assert!(target.observes_verified(Some(&first)).unwrap());
    assert!(!target.observes_verified(None).unwrap()); // Orphan target is not Empty.
    assert!(!target.observes_verified(Some(&record(2))).unwrap());
    // Keep the digest unchanged but change identity: no digest-only shortcut is allowed.
    let mut wrong = first.clone();
    wrong.publication_revision = GatewayPublicationRevision::new(2).unwrap();
    assert!(target.observes_verified(Some(&wrong)).is_err());
    wrong = first.clone();
    wrong.workspace_id = hiroute_domain::WorkspaceId::parse("personal/other").unwrap();
    assert!(target.observes_verified(Some(&wrong)).is_err());
    wrong = first.clone();
    std::sync::Arc::make_mut(&mut wrong.bytes).push(b' ');
    assert!(target.observes_verified(Some(&wrong)).is_err());
    let pinned = RuntimePublicationFeed::pin(&target).unwrap();
    target.suspend_requests().unwrap();
    assert!(target.verify_installed(&first).unwrap());
    assert!(!target.observes_verified(Some(&first)).unwrap());
    assert!(RuntimePublicationFeed::pin(&target).is_none());
    target.activate_verified(&record(2)).unwrap();
    assert_eq!(pinned.publication_revision(), 1);
    target.resume_requests().unwrap();
    assert!(target.observes_verified(Some(&record(2))).unwrap());
}

#[test]
fn restored_execution_lkg_reattaches_pricing_from_verified_product_record() {
    let root = tempfile::tempdir().unwrap();
    let lkg = root.path().join("lkg.json");
    let record = record(1);
    let model_alias = record
        .verify()
        .unwrap()
        .aliases
        .iter()
        .find(|alias| {
            alias
                .protocols
                .contains(&hiroute_domain::AgentIngressProtocolV1::Responses)
        })
        .unwrap()
        .served_model_id
        .as_str()
        .to_owned();

    let first = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(&lkg).unwrap(),
    )));
    first.activate_verified(&record).unwrap();
    first.resume_requests().unwrap();
    let authorized = GatewayRequestAuthority::new(Arc::clone(&first))
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer codex-grant-verifier"),
            format!(r#"{{"model":"{model_alias}"}}"#).as_bytes(),
            std::time::Instant::now(),
        )
        .unwrap();
    assert!(!authorized.pricing_bindings().is_empty());
    drop(authorized);
    drop(first);

    let restored = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(&lkg).unwrap(),
    )));
    restored.resume_requests().unwrap();
    let before_reattach = GatewayRequestAuthority::new(Arc::clone(&restored))
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer codex-grant-verifier"),
            format!(r#"{{"model":"{model_alias}"}}"#).as_bytes(),
            std::time::Instant::now(),
        )
        .unwrap();
    assert!(before_reattach.pricing_bindings().is_empty());
    drop(before_reattach);

    restored.activate_verified(&record).unwrap();
    let after_reattach = GatewayRequestAuthority::new(restored)
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer codex-grant-verifier"),
            format!(r#"{{"model":"{model_alias}"}}"#).as_bytes(),
            std::time::Instant::now(),
        )
        .unwrap();
    assert!(!after_reattach.pricing_bindings().is_empty());
}

#[test]
fn publication_readiness_rejects_uncertainty_even_with_the_previous_live_target() {
    let root = tempfile::tempdir().unwrap();
    let target = GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(root.path().join("lkg.json")).unwrap(),
    ));
    let before = record(1);
    target.activate_verified(&before).unwrap();
    target.resume_requests().unwrap();
    let after = record(2);
    let snapshot = hiroute_integrations::gateway::project_publication(
        &after.verify().unwrap().gateway_snapshot().unwrap(),
    )
    .unwrap();
    {
        let _mutation = target.mutation.lock().unwrap();
        let GatewayPrepareOutcome::Prepared(prepared) = target.installer.prepare(snapshot).unwrap()
        else {
            panic!("expected new publication")
        };
        assert!(
            target
                .installer
                .publish_with_failpoint(prepared, PublicationFailpoint::AfterDurableLkg)
                .is_err()
        );
    }
    assert!(target.installer.durability_uncertain());
    assert!(target.observes_verified(Some(&before)).is_err());
    assert!(target.observes_verified(Some(&after)).is_err());
    assert!(target.observes_verified(None).is_err());
    assert!(RuntimePublicationFeed::pin(&target).is_none());
    assert!(target.resume_requests().is_err());
}

#[test]
fn publication_readiness_busy_install_is_nonblocking_and_pin_uses_no_mutation_lock() {
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;
    let root = tempfile::tempdir().unwrap();
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(root.path().join("lkg.json")).unwrap(),
    )));
    let first = record(1);
    target.activate_verified(&first).unwrap();
    target.resume_requests().unwrap();
    let entered = Arc::new(Barrier::new(2));
    let (release, wait) = mpsc::channel();
    let writer = {
        let target = target.clone();
        let entered = entered.clone();
        std::thread::spawn(move || {
            let _mutation = target.mutation.lock().unwrap();
            entered.wait();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        })
    };
    entered.wait();
    let (send, receive) = mpsc::channel();
    let reader = {
        let target = target.clone();
        let first = first.clone();
        std::thread::spawn(move || {
            send.send((
                target.observes_verified(Some(&first)),
                RuntimePublicationFeed::pin(target.as_ref()).is_some(),
            ))
            .unwrap()
        })
    };
    let observed = receive.recv_timeout(Duration::from_secs(2));
    release.send(()).unwrap();
    writer.join().unwrap();
    reader.join().unwrap();
    assert_eq!(
        observed.unwrap(),
        (Err(PublicationTargetError::Unavailable), true)
    );
    assert!(target.observes_verified(Some(&first)).unwrap());
}

#[test]
fn publication_readiness_no_new_calls_requires_installed_verified_record() {
    let root = tempfile::tempdir().unwrap();
    let target = GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(root.path().join("lkg.json")).unwrap(),
    ));
    let mut empty = record(1).verify().unwrap();
    empty.aliases.clear();
    empty.grants.clear();
    let empty = PublicationRecordV1::from_publication(empty.workspace_id.clone(), &empty).unwrap();
    target.resume_requests().unwrap();
    assert!(!target.observes_verified(Some(&empty)).unwrap());
    target.activate_verified(&empty).unwrap();
    assert!(target.observes_verified(Some(&empty)).unwrap());
    target.activate_verified(&record(2)).unwrap();
    assert!(!target.observes_verified(Some(&empty)).unwrap());
}
