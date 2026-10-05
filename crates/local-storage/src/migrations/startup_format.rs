//! Reject unsupported source formats before any database, key or backup mutation.
use super::*;

pub(crate) fn validate_startup_format(root: &Path) -> Result<(), LocalStorageError> {
    let live = root.join("live");
    let paths = live_database_paths(&live);
    let present = paths.map(|path| path.exists());
    if present == [false; 3] {
        // Only a fresh directory may initialize. A missing live set with durable facts is
        // damage or an interrupted restore, never an invitation to create empty databases.
        if fs::read_dir(root)?.any(|entry| {
            entry.map_or(true, |entry| {
                let name = entry.file_name();
                name != "startup.lock"
                    && !(name == "diagnostics" && bootstrap_diagnostics_directory(&entry.path()))
                    && !(name == "live"
                        && fs::read_dir(entry.path())
                            .is_ok_and(|mut entries| entries.next().is_none()))
            })
        }) || crate::upgrade_backup_root(root)?.exists()
        {
            return Err(LocalStorageError::InvalidData);
        }
        return Ok(());
    }
    if present != [true; 3] {
        return Err(LocalStorageError::InvalidData);
    }
    for path in live_database_paths(&live) {
        validate_owner_file(&path)?;
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        // Schema 23 is the first stable format; 24 preserves that same marker.
        // A mixed supported live set is only a candidate for the existing coordinator:
        // it must still prove its complete source backup, store identities, key and phase.
        if !matches!(version, 23 | 24 | 25 | LATEST_SCHEMA_VERSION) {
            return Err(LocalStorageError::UpgradeSourceUnsupported);
        }
        let marker = connection.query_row(
            "SELECT format_version, converted FROM stable_storage_format WHERE singleton=1",
            [],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
        );
        if !matches!(marker, Ok((1, 1))) {
            return Err(LocalStorageError::UpgradeSourceUnsupported);
        }
    }
    Ok(())
}

// The production diagnostics runtime starts before storage and retains startup failures.
// Recognize only its real private directory; never follow a link or modify its contents.
fn bootstrap_diagnostics_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o700 || metadata.uid() != rustix::process::getuid().as_raw()
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_diagnostics_can_precede_first_store_without_being_discarded() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("storage");
        prepare_storage_root(&root).unwrap();
        prepare_storage_root(&root.join("diagnostics")).unwrap();
        prepare_storage_root(&root.join("diagnostics/daemon")).unwrap();
        let log = root.join("diagnostics/daemon/current.jsonl");
        fs::write(&log, b"retained startup diagnostic\n").unwrap();
        set_owner_file_permissions(&log).unwrap();
        drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        assert_eq!(fs::read(log).unwrap(), b"retained startup diagnostic\n");
        assert!(root.join("live/control.db").is_file());
    }

    #[test]
    fn unknown_bootstrap_data_is_preserved_and_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("storage");
        prepare_storage_root(&root).unwrap();
        fs::write(root.join("old-control.db"), b"old durable facts").unwrap();
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
        assert_eq!(
            fs::read(root.join("old-control.db")).unwrap(),
            b"old durable facts"
        );
        assert!(!root.join("master-key").exists());
        assert!(!root.join("live/control.db").exists());
    }

    #[cfg(unix)]
    #[test]
    fn diagnostics_symlink_does_not_authorize_first_store() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("storage");
        prepare_storage_root(&root).unwrap();
        let outside = temporary.path().join("outside");
        prepare_storage_root(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("diagnostics")).unwrap();
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
        assert!(!root.join("master-key").exists());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn incomplete_or_unmarked_current_directories_are_preserved() {
        for missing_database in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("storage");
            drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
            if missing_database {
                fs::rename(root.join("live/runtime.db"), root.join("runtime-kept.db")).unwrap();
            } else {
                Connection::open(root.join("live/control.db"))
                    .unwrap()
                    .execute("DROP TABLE stable_storage_format", [])
                    .unwrap();
            }
            let key = fs::read(root.join("master-key")).unwrap();
            assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
            assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
            if missing_database {
                assert!(!root.join("live/runtime.db").exists());
                assert!(root.join("runtime-kept.db").is_file());
            }
        }
    }

    #[test]
    fn startup_exclusion_lives_until_the_last_store_is_dropped() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("storage");
        let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
        let (control, runtime, secrets) = stores.into_parts();
        drop(control);
        drop(runtime);
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
        drop(secrets);
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_ok());
    }

    #[test]
    fn unsupported_source_is_rejected_without_changing_database_or_key_bytes() {
        for version in [0, 1, 22, LATEST_SCHEMA_VERSION + 1] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("storage");
            drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
            let paths = live_database_paths(&root.join("live"));
            for path in &paths {
                let db = Connection::open(path).unwrap();
                db.pragma_update(None, "user_version", version).unwrap();
            }
            let before = paths.clone().map(|path| fs::read(path).unwrap());
            let key = fs::read(root.join("master-key")).unwrap();
            assert!(matches!(
                crate::LocalStorageSet::open_for_daemon_startup(&root),
                Err(LocalStorageError::UpgradeSourceUnsupported)
            ));
            assert_eq!(paths.map(|path| fs::read(path).unwrap()), before);
            assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
        }
    }

    #[test]
    fn current_format_reopens_and_unfinished_format_is_preserved_and_refused() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("storage");
        drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        let path = root.join("live/control.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE stable_storage_format SET converted=0", [])
            .unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            crate::LocalStorageSet::open_for_daemon_startup(&root),
            Err(LocalStorageError::UpgradeSourceUnsupported)
        ));
        assert_eq!(fs::read(path).unwrap(), before);
    }
}
