//! Real routing capture around the synthetic CPA process, without provider calls.
use super::*;
use hiroute_application::control::{ControlReadError, RoutingFactsPort};
use hiroute_application_api::{ComputeConnectionAccessKindV1, ComputeModelAvailabilityV1};
use std::time::Instant;

fn fixture_with_saved_subscription() -> Fixture {
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, "trial-capture-save"));
    fixture
}

#[test]
fn trial_capture_uses_routing_candidate_and_safe_source_from_one_batch() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_capture_uses_routing_candidate_and_safe_source_from_one_batch"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    fs::write(fixture.root.path().join("record-model-catalog"), b"record").unwrap();
    let service =
        hiroute_application::ApplicationService::new(fixture.runtime().application_ports());
    let request = hiroute_application_api::LocalControlRequestV2 {
        schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: "trial-capture-read".into(),
        principal: hiroute_application_api::PrincipalV1::ambient_local_peer(),
        operation_id: hiroute_application_api::GET_TRIAL_SETUP_SNAPSHOT_OPERATION_V1.into(),
        payload: serde_json::json!({}),
        protected_grant: None,
    };
    let response = service.dispatch(request);
    let captured: hiroute_application_api::TrialSetupSnapshotV1 =
        serde_json::from_value(response.data.expect("trial read succeeds")).unwrap();
    let source = captured
        .management
        .sources
        .iter()
        .find(|source| source.access_kind == ComputeConnectionAccessKindV1::Subscription)
        .expect("subscription source");
    let member = source
        .models
        .iter()
        .find(|member| member.catalog_admitted)
        .expect("catalog admitted member");
    assert_eq!(member.availability, ComputeModelAvailabilityV1::Available);
    assert!(
        captured
            .options
            .candidates
            .iter()
            .any(|candidate| candidate.binding_id == member.binding_id
                && candidate.model_configuration_id
                    == *member.catalog_configuration_id.as_ref().unwrap()
                && candidate.routable)
    );
    let safe = serde_json::to_value(&captured.management).unwrap();
    let source = &safe["sources"][0];
    for forbidden in [
        "credentials",
        "keys",
        "target",
        "authentication",
        "inventory_path",
        "price_contexts",
    ] {
        assert!(
            source.get(forbidden).is_none(),
            "trial source exposes {forbidden}"
        );
        assert!(
            source["models"]
                .as_array()
                .unwrap()
                .iter()
                .all(|model| model.get(forbidden).is_none()),
            "trial model exposes {forbidden}"
        );
    }
    let events = fs::read_to_string(fixture.root.path().join("catalog.jsonl")).unwrap();
    let inventories = events
        .lines()
        .filter(|line| line.contains("\"event\": \"inventory\""))
        .count();
    assert_eq!(
        inventories, 2,
        "one batch capture and its finish are the only inventory reads"
    );
}

#[test]
fn trial_capture_rejects_cpa_change_with_unchanged_control_revisions_and_retries_cleanly() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_capture_rejects_cpa_change_with_unchanged_control_revisions_and_retries_cleanly"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let revisions = fixture.snapshot().revisions;
    let root = fixture.root.path();
    fs::write(root.join("probe-catalog-lock"), b"probe").unwrap();
    let mut probes = 0;
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let adapter = fixture.adapter();
        scope.spawn(move || {
            sender
                .send(adapter.trial_routing_snapshot(&WorkspaceId::default()))
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            for entry in fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("catalog-probe-")
                    || path.extension().is_some()
                    || path.with_extension("resume").exists()
                {
                    continue;
                }
                probes += 1;
                assert!(
                    fixture.adapter().stores.try_lock().is_ok(),
                    "CPA transport must not hold local storage"
                );
                assert_eq!(fixture.snapshot().revisions, revisions);
                if probes == 2 {
                    fs::write(root.join("catalog-auth-unavailable"), b"changed").unwrap();
                }
                fs::write(path.with_extension("resume"), b"resume").unwrap();
            }
            if let Ok(result) = receiver.try_recv() {
                assert!(
                    matches!(result, Err(ControlReadError::SnapshotChanged)),
                    "stale CPA facts escaped"
                );
                break;
            }
            assert!(Instant::now() < deadline, "trial capture probe stalled");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(probes, 2);
    fs::remove_file(root.join("probe-catalog-lock")).unwrap();
    fs::remove_file(root.join("catalog-auth-unavailable")).unwrap();
    assert_eq!(fixture.snapshot().revisions, revisions);
    let retried = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert!(
        retried
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .any(|model| model.catalog_admitted
                && model.availability == ComputeModelAvailabilityV1::Available)
    );
}

#[test]
fn trial_capture_rejects_catalog_replacement_during_the_cpa_read() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_capture_rejects_catalog_replacement_during_the_cpa_read"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let revisions = fixture.snapshot().revisions;
    let feed = fixture.adapter().model_feed.as_ref().unwrap();
    let before = feed.available_catalog().unwrap();
    let root = fixture.root.path();
    fs::write(root.join("probe-catalog-lock"), b"probe").unwrap();
    let mut probes = 0;
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let adapter = fixture.adapter();
        scope.spawn(move || {
            sender
                .send(adapter.trial_routing_snapshot(&WorkspaceId::default()))
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            for entry in fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("catalog-probe-")
                    || path.extension().is_some()
                    || path.with_extension("resume").exists()
                {
                    continue;
                }
                probes += 1;
                if probes == 1 {
                    let digest = feed.removal_digest().unwrap();
                    feed.remove(&digest, false, |_, _| Ok(())).unwrap();
                    assert!(!Arc::ptr_eq(&before, &feed.available_catalog().unwrap()));
                    assert_eq!(fixture.snapshot().revisions, revisions);
                }
                fs::write(path.with_extension("resume"), b"resume").unwrap();
            }
            if let Ok(result) = receiver.try_recv() {
                assert!(matches!(result, Err(ControlReadError::SnapshotChanged)));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "catalog replacement probe stalled"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(probes, 2);
}
