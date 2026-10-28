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
        inventories, 4,
        "the initial and final batch discoveries each perform the two pinned stock inventory probes"
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
                if probes == 3 {
                    // Two inventory probes belong to the initial discovery. Change CPA at the
                    // first probe of the final discovery, after the captured candidates exist.
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
    assert!(
        probes >= 3,
        "the captured batch must reach final revalidation"
    );
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
    assert_eq!(probes, 4);
}

#[test]
fn trial_failed_initial_cpa_batch_cannot_be_promoted_by_a_later_read() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_failed_initial_cpa_batch_cannot_be_promoted_by_a_later_read"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let revisions = fixture.snapshot().revisions;
    let failure = fixture.root.path().join("catalog-auth-unavailable");
    fs::write(&failure, b"unavailable").unwrap();
    let unavailable = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert!(
        unavailable
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .all(|model| !model.catalog_admitted
                && model.availability != ComputeModelAvailabilityV1::Available)
    );
    assert!(unavailable.routing.facts.candidates.is_empty());
    fs::remove_file(failure).unwrap();
    let recovered = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(fixture.snapshot().revisions, revisions);
    assert!(
        recovered
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .any(|model| model.catalog_admitted
                && model.availability == ComputeModelAvailabilityV1::Available)
    );
    assert!(
        unavailable
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .all(|model| !model.catalog_admitted)
    );
}

#[test]
fn trial_capture_rejects_binding_health_change_during_cpa_finish() {
    use hiroute_domain::{
        ComputeRuntimeStateStoreV1, RuntimeClockSampleV1, RuntimeStateIdentityV1, RuntimeStateV1,
    };
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_capture_rejects_binding_health_change_during_cpa_finish"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let stable = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    let member = stable
        .management
        .sources
        .iter()
        .flat_map(|source| &source.models)
        .find(|member| {
            member.catalog_admitted && member.availability == ComputeModelAvailabilityV1::Available
        })
        .expect("available subscription binding");
    let binding_id = member.binding_id.clone();
    let binding_revision = member.revision;
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
                if probes == 3 {
                    let identity = RuntimeStateIdentityV1::binding(binding_id.clone()).unwrap();
                    let disabled = RuntimeStateV1::disabled(
                        identity,
                        1,
                        RuntimeClockSampleV1::from_unix_millis(1_000).unwrap(),
                    )
                    .unwrap();
                    fixture
                        .adapter()
                        .stores_lock()
                        .unwrap()
                        .runtime()
                        .compare_and_set_runtime_state(0, &disabled)
                        .unwrap();
                    assert_eq!(fixture.snapshot().revisions, revisions);
                }
                fs::write(path.with_extension("resume"), b"resume").unwrap();
            }
            if let Ok(result) = receiver.try_recv() {
                assert!(
                    matches!(result, Err(ControlReadError::SnapshotChanged)),
                    "old binding health escaped"
                );
                break;
            }
            assert!(Instant::now() < deadline, "binding health probe stalled");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(probes, 4);
    fs::remove_file(root.join("probe-catalog-lock")).unwrap();
    let changed = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    let member = changed
        .management
        .sources
        .iter()
        .flat_map(|source| &source.models)
        .find(|member| member.binding_id == binding_id)
        .unwrap();
    assert_eq!(member.revision, binding_revision);
    assert_eq!(member.availability, ComputeModelAvailabilityV1::Disabled);
}

#[test]
fn independent_routing_and_management_reads_are_a_cpa_mismatch_negative_control() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "independent_routing_and_management_reads_are_a_cpa_mismatch_negative_control"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let revisions = fixture.snapshot().revisions;
    let options = fixture
        .adapter()
        .routing_compilation_snapshot(&WorkspaceId::default())
        .unwrap();
    let candidate = options
        .facts
        .candidates
        .iter()
        .find(|candidate| candidate.is_routable())
        .unwrap();
    let binding_id = candidate.binding.binding_id.clone();
    fs::write(
        fixture.root.path().join("catalog-auth-unavailable"),
        b"changed",
    )
    .unwrap();
    let management = fixture
        .adapter()
        .compute_management_snapshot_v3(
            &hiroute_application_api::ComputeManagementQueryV2::default(),
        )
        .unwrap();
    let member = management
        .sources
        .iter()
        .flat_map(|source| &source.models)
        .find(|member| member.binding_id == binding_id)
        .unwrap();
    assert_ne!(
        member.presentation.availability,
        ComputeModelAvailabilityV1::Available
    );
    assert_eq!(
        fixture.snapshot().revisions,
        revisions,
        "the outer service revisions cannot detect this independent CPA change"
    );
    assert!(
        candidate.is_routable(),
        "the earlier options read remains independently valid"
    );
    // The old frontend could combine those independently valid values. The new owner must
    // publish a conservative result after the failed batch, never the earlier candidate.
    let combined = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert!(
        !combined
            .routing
            .facts
            .candidates
            .iter()
            .any(|candidate| candidate.binding.binding_id == binding_id && candidate.is_routable())
    );
    assert!(
        !combined
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .any(|member| member.binding_id == binding_id && member.catalog_admitted)
    );
}

#[test]
fn trial_capture_rejects_revoked_retained_subscription_validation() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::trial_snapshot_tests::",
        "trial_capture_rejects_revoked_retained_subscription_validation"
    )) {
        return;
    }
    let fixture = fixture_with_saved_subscription();
    let stored = fixture.snapshot();
    let source = &stored.sources[0];
    let approval = hiroute_domain::OperationId::parse(
        &source.validation.as_ref().unwrap().approval_operation_id,
    )
    .unwrap();
    let stable = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert!(
        stable
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .any(|member| member.catalog_admitted
                && member.availability == ComputeModelAvailabilityV1::Available)
    );
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
                if probes == 3 {
                    fixture
                        .adapter()
                        .stores_lock()
                        .unwrap()
                        .control()
                        .release_compute_subscription_validation(&approval)
                        .unwrap();
                    assert_eq!(fixture.snapshot().revisions, stored.revisions);
                }
                fs::write(path.with_extension("resume"), b"resume").unwrap();
            }
            if let Ok(result) = receiver.try_recv() {
                assert!(
                    matches!(result, Err(ControlReadError::SnapshotChanged)),
                    "revoked validation escaped"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "retained validation probe stalled"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(probes, 4);
    fs::remove_file(root.join("probe-catalog-lock")).unwrap();
    let revoked = fixture
        .adapter()
        .trial_routing_snapshot(&WorkspaceId::default())
        .unwrap();
    assert!(
        revoked
            .management
            .sources
            .iter()
            .flat_map(|source| &source.models)
            .all(|member| !member.catalog_admitted
                && member.availability != ComputeModelAvailabilityV1::Available)
    );
}
