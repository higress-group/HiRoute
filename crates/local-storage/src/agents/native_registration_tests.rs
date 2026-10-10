//! Registering an unused native target is not permission to read, write or recover it.
use super::*;

#[test]
fn accessible_writable_target_and_neighbor_keep_independent_effects() {
    let root = crate::test_tempdir().unwrap();
    let unsafe_parent = root.path().join("unsafe-native");
    let safe_parent = root.path().join("safe-native");
    fs::create_dir(&unsafe_parent).unwrap();
    fs::create_dir(&safe_parent).unwrap();
    fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o775)).unwrap();
    fs::set_permissions(&safe_parent, fs::Permissions::from_mode(0o700)).unwrap();
    let paths = [
        unsafe_parent.join("settings.json"),
        safe_parent.join("settings.json"),
    ];
    let intents = ["agent.unselected", "agent.selected"].map(|agent| {
        intent_for_agent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            None,
            agent,
            "profile.fixture",
        )
    });
    let mut store = ManagedArtifactStore::open_with_external_targets(
        &crate::test_storage_authority(),
        root.path().join("artifacts"),
        root.path().join("restore"),
        intents
            .iter()
            .zip(&paths)
            .map(|(intent, path)| (intent.target().to_owned(), path.clone())),
    )
    .unwrap();
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    assert!(
        store
            .read_native_target(intents[0].target())
            .unwrap()
            .is_none()
    );
    let first = store
        .stage_native_target(&operation, &intents[0], Some(b"first target"), false)
        .unwrap();
    assert!(!paths[0].exists(), "staging does not activate the target");
    store.activate_artifact(&first).unwrap();
    assert_eq!(fs::read(&paths[0]).unwrap(), b"first target");
    store
        .bind_external_target("another-target", unsafe_parent.join("other-settings.json"))
        .unwrap();
    let effect = store
        .stage_native_target(&operation, &intents[1], Some(b"safe neighbor"), false)
        .unwrap();
    store.activate_artifact(&effect).unwrap();
    assert_eq!(fs::read(&paths[1]).unwrap(), b"safe neighbor");
    assert_eq!(
        fs::metadata(&unsafe_parent).unwrap().permissions().mode() & 0o777,
        0o775
    );
    assert_eq!(fs::read_dir(&unsafe_parent).unwrap().count(), 1);
}

#[test]
fn existing_native_directory_modes_allow_activation_restore_and_reopen() {
    for activated in [false, true] {
        let root = crate::test_tempdir().unwrap();
        let parent = root.path().join("native");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("settings.json");
        let intent = intent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            None,
        );
        let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
        let open = || {
            ManagedArtifactStore::open_with_external_targets(
                &crate::test_storage_authority(),
                root.path().join("artifacts"),
                root.path().join("restore"),
                [(intent.target().to_owned(), path.clone())],
            )
        };
        let store = open().unwrap();
        let effect = store
            .stage_native_target(&operation, &intent, Some(b"managed content"), false)
            .unwrap();
        if activated {
            store.activate_artifact(&effect).unwrap();
        }
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o775)).unwrap();
        let restore_key = root.path().join("restore/.restore-key");
        let key_before = fs::read(&restore_key).unwrap();
        drop(store);
        let store = open().unwrap();
        if !activated {
            store.activate_artifact(&effect).unwrap();
        }
        assert_eq!(
            store
                .read_native_target(intent.target())
                .unwrap()
                .unwrap()
                .as_slice(),
            b"managed content"
        );
        assert_eq!(
            store.compensate_artifact(&effect).unwrap(),
            CompensationOutcome::Compensated
        );
        assert!(store.read_native_target(intent.target()).unwrap().is_none());
        drop(store);
        open().unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read(&restore_key).unwrap(), key_before);
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o775
        );
    }
}
