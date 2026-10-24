use super::*;
use hiroute_application::control::RoutingFactsPort;
use std::time::Instant;

#[test]
fn cpa_snapshot_revalidation_does_not_block_native_grant_registration() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::routing_lock_tests::",
        "cpa_snapshot_revalidation_does_not_block_native_grant_registration"
    )) {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, "snapshot-lock-source"));
    fs::write(fixture.root.path().join("probe-catalog-lock"), b"probe").unwrap();
    let mut blocked = 0;
    let mut probes = 0;
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let adapter = fixture.adapter();
        scope.spawn(move || {
            sender
                .send(adapter.routing_compilation_snapshot(&WorkspaceId::default()))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            for entry in fs::read_dir(fixture.root.path()).unwrap() {
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
                // The external CPA response is still held. A native registration
                // must not wait for its control deadline while holding up all ACKs.
                let available = fixture.adapter().stores.try_lock().is_ok();
                if !available {
                    blocked += 1;
                } else {
                    let registration = hiroute_local_storage::ApplyCapabilityRegistrationV1::from_protected_launcher(
                        format!("native-grant-lock-probe-{probes:040}"), "desktop", WorkspaceId::default(),
                        "ListSessionsV2", hiroute_domain::CanonicalDigest::of_bytes(b"bounded probe"),
                        hiroute_domain::RevisionSetV1 { target: 0, dependencies: Default::default() },
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 + 60,
                    ).unwrap();
                    fixture
                        .runtime()
                        .register_apply_capability(registration)
                        .unwrap();
                }
                fs::write(path.with_extension("resume"), b"resume").unwrap();
            }
            if let Ok(result) = receiver.try_recv() {
                assert!(result.is_ok());
                break;
            }
            assert!(Instant::now() < deadline, "routing lock probe stalled");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert!(
        probes >= 2,
        "must exercise initial and final CPA observations"
    );
    assert_eq!(
        blocked, 0,
        "external CPA observation held shared storage and blocked native grants"
    );
}
