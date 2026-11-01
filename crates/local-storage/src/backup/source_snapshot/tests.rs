use super::*;

#[test]
fn current_history_is_not_sized_copied_or_hashed_for_core_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("storage");
    let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let options = crate::StorageStartupOptions::default();
    let before = preflight_space(&root, None, &options).unwrap();
    let observation = root.join("observation");
    prepare_owner_directory(&observation).unwrap();
    let body = observation.join("large-body");
    let sparse = fs::File::create(&body).unwrap();
    // Unix creates a sparse file: counting it would refuse this upgrade even
    // though no history bytes need copying. Windows set_len can allocate space.
    #[cfg(unix)]
    let length = fs2::available_space(&root).unwrap() + 512 * 1024 * 1024;
    #[cfg(windows)]
    let length = 1024 * 1024;
    sparse.set_len(length).unwrap();
    drop(sparse);
    #[cfg(unix)]
    std::os::unix::fs::symlink("absent-history-target", observation.join("must-not-follow"))
        .unwrap();
    assert_eq!(preflight_space(&root, None, &options).unwrap(), before);
    let backup = temporary.path().join("backup");
    let barrier = test_writer_barrier();
    let set = BackupSet::create(
        &barrier,
        &backup,
        &stores.control().connection.borrow(),
        &stores.runtime().connection.borrow(),
        &stores.secrets().connection.borrow(),
    )
    .unwrap();
    set.publish_stopped_source(&barrier, &root, &options)
        .unwrap();
    assert!(!backup.join("source/storage/observation").exists());
    assert_eq!(fs::metadata(&body).unwrap().len(), length);
    let value = manifest(&backup).unwrap();
    assert_eq!(value["schema"], "hiroute.upgrade-source-backup/v3");
    assert_eq!(value["observation"], "not_backed_up");
    verify(&backup).unwrap();
    // Later observation writes are not changes to the Core recovery source.
    fs::write(observation.join("later-body"), b"new observations").unwrap();
    verify_unchanged(&root, &backup).unwrap();
    for guide in ["RECOVERY.md", "恢复说明.md"] {
        let text = fs::read_to_string(backup.join("source").join(guide)).unwrap();
        assert!(text.contains(if guide == "RECOVERY.md" {
            "Observation is NOT included"
        } else {
            "本次快照不包含 Observation"
        }));
    }
    // A core-only manifest must not be used to smuggle in another generation.
    prepare_owner_directory(&backup.join("source/storage/observation")).unwrap();
    assert!(verify(&backup).is_err());
}

#[test]
fn legacy_full_history_verification_remains_required_only_for_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("storage");
    let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    prepare_owner_directory(&root.join("observation")).unwrap();
    fs::write(root.join("observation/body"), b"original history").unwrap();
    let backup = temporary.path().join("backup");
    let barrier = test_writer_barrier();
    let set = BackupSet::create(
        &barrier,
        &backup,
        &stores.control().connection.borrow(),
        &stores.runtime().connection.borrow(),
        &stores.secrets().connection.borrow(),
    )
    .unwrap();
    set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
        .unwrap();
    // Explicit retained v1 shape: its Observation files were copied and hashed.
    // This is a format regression, not a released-package acceptance fixture.
    let source = backup.join("source");
    copy_tree(
        &root.join("observation"),
        &source.join("storage/observation"),
    )
    .unwrap();
    let mut value = manifest(&backup).unwrap();
    value["schema"] = json!("hiroute.upgrade-source-backup/v1");
    value.as_object_mut().unwrap().remove("observation");
    let mut files = BTreeMap::new();
    collect(&source, &source, &mut files, false).unwrap();
    value["files"] = json!(files);
    fs::write(source.join(MANIFEST), serde_json::to_vec(&value).unwrap()).unwrap();
    verify(&backup).unwrap();
    verify_unchanged(&root, &backup).unwrap();
    fs::write(root.join("observation/body"), b"live changed").unwrap();
    assert!(verify_unchanged(&root, &backup).is_err());
    fs::write(source.join("storage/observation/body"), b"damaged backup").unwrap();
    assert!(verify(&backup).is_err());
    let archived = temporary.path().join("archived");
    archive_guide(&backup, &archived, set.set_id()).unwrap();
    hiroute_platform::durable::rename(&backup, &archived).unwrap();
    assert_eq!(
        fs::read(archived.join("source/storage/observation/body")).unwrap(),
        b"damaged backup"
    );
    assert!(
        verify(&archived).is_err(),
        "archival grants no recovery authority"
    );
}

#[test]
fn stopped_current_snapshot_preserves_keys_auxiliary_files_and_complete_manifest() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("storage");
    let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    prepare_owner_directory(&root.join("managed-probe")).unwrap();
    hiroute_platform::durable::write_private_new(
        &root.join("managed-probe/body"),
        b"preserved native state",
    )
    .unwrap();
    set_owner_file_permissions(&root.join("managed-probe/body")).unwrap();
    // Cross three complete 64KiB chunks plus a partial tail. Readonly
    // content must retain both bytes and real native security in the copy.
    let chunked = (0..(3 * 64 * 1024 + 17))
        .map(|index| ((index * 37 + 11) & 0xff) as u8)
        .collect::<Vec<_>>();
    let chunked_path = root.join("managed-probe/chunked-readonly");
    hiroute_platform::durable::write_private_new(&chunked_path, &chunked).unwrap();
    let mut readonly = fs::metadata(&chunked_path).unwrap().permissions();
    readonly.set_readonly(true);
    fs::set_permissions(&chunked_path, readonly).unwrap();
    let backup = temporary.path().join("backup");
    let barrier = test_writer_barrier();
    let set = BackupSet::create(
        &barrier,
        &backup,
        &stores.control().connection.borrow(),
        &stores.runtime().connection.borrow(),
        &stores.secrets().connection.borrow(),
    )
    .unwrap();
    set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
        .unwrap();
    verify(&backup).unwrap();
    verify_unchanged(&root, &backup).unwrap();
    for name in [
        "master-key",
        "managed-probe/body",
        "managed-probe/chunked-readonly",
    ] {
        assert_eq!(
            fs::read(root.join(name)).unwrap(),
            fs::read(backup.join("source/storage").join(name)).unwrap()
        );
        #[cfg(windows)]
        {
            let original =
                hiroute_platform::windows::security::open_identity(&root.join(name), false)
                    .unwrap();
            let copy = hiroute_platform::windows::security::open_identity(
                &backup.join("source/storage").join(name),
                false,
            )
            .unwrap();
            assert_eq!(
                hiroute_platform::file_security::capture(&original).unwrap(),
                hiroute_platform::file_security::capture(&copy).unwrap(),
                "the complete source snapshot must retain native recovery permissions",
            );
        }
    }
    assert_eq!(
        fs::read(backup.join("source/storage/managed-probe/chunked-readonly")).unwrap(),
        chunked
    );
    assert!(
        fs::metadata(backup.join("source/storage/managed-probe/chunked-readonly"))
            .unwrap()
            .permissions()
            .readonly()
    );
    let manifest = fs::read(backup.join("source/source-backup.json")).unwrap();
    set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
        .unwrap();
    assert_eq!(
        fs::read(backup.join("source/source-backup.json")).unwrap(),
        manifest
    );
    fs::write(root.join("managed-probe/body"), b"changed").unwrap();
    assert!(verify_unchanged(&root, &backup).is_err());
    #[cfg(windows)]
    for path in [
        chunked_path,
        backup.join("source/storage/managed-probe/chunked-readonly"),
    ] {
        let mut writable = fs::metadata(&path).unwrap().permissions();
        writable.set_readonly(false);
        fs::set_permissions(path, writable).unwrap();
    }
}
