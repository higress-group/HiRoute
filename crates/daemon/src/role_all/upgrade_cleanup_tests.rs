use super::*;
use std::fs;

#[test]
fn failed_control_start_retains_recovery_and_ready_service_cleans_it() {
    if crate::test_support::isolated_agent_home(
        "role_all::upgrade_cleanup_tests::failed_control_start_retains_recovery_and_ready_service_cleans_it",
    ) {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let storage = fixture.path().join("storage");
    let runtime = fixture.path().join("runtime");
    // Establish a real, fully admitted current Core/Observation group first.
    drop(
        ProductionControlRuntime::prepare_for_role_all_with_codex_desktop_engine(
            &storage,
            crate::release_catalog::fixture_catalog(),
            None,
            None,
            Default::default(),
        )
        .unwrap(),
    );
    let backups = hiroute_local_storage::upgrade_backup_root(&storage)
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    hiroute_platform::private_fs::prepare_storage_directory(&backups).unwrap();
    hiroute_platform::durable::write_private_new(&backups.join("retention-probe"), b"recovery")
        .unwrap();
    let before_key = fs::read(storage.join("master-key")).unwrap();
    let before_hmac = fs::read(storage.join("observation-hmac-key")).unwrap();
    // Migration/Observation/publication can succeed while Control still fails.
    fs::write(&runtime, b"not a runtime directory").unwrap();
    let listener = crate::test_support::ListenerPortLease::reserve().unwrap();
    let config = || {
        RoleAllConfig::new(
            storage.clone(),
            runtime.clone(),
            listener.address,
            fixture.path().join("gateway-lkg.json"),
        )
    };
    assert!(matches!(
        start_role_all(config()),
        Err(RoleAllError::Component("Local Control", _))
    ));
    assert_eq!(
        fs::read(backups.join("retention-probe")).unwrap(),
        b"recovery"
    );
    fs::remove_file(&runtime).unwrap();
    let mut role = start_role_all(config()).unwrap();
    assert_eq!(
        role.phases(),
        (ManagedControlPhase::Ready, ManagedGatewayPhase::Ready)
    );
    let started = Instant::now();
    while backups.exists() && started.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !backups.exists(),
        "ready service must retire recovery material"
    );
    assert_eq!(fs::read(storage.join("master-key")).unwrap(), before_key);
    assert_eq!(
        fs::read(storage.join("observation-hmac-key")).unwrap(),
        before_hmac
    );
    assert!(storage.join("observation/activity.db").exists());
    role.shutdown();
    role.join(Duration::from_secs(5)).unwrap();
}
