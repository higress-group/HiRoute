use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    state: PathBuf,
    storage: PathBuf,
    backups: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        crate::prepare_owner_directory(&state).unwrap();
        let storage = state.join("storage");
        crate::prepare_owner_directory(&storage).unwrap();
        let backups = crate::upgrade_backup_root(&storage)
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        crate::prepare_owner_directory(&backups).unwrap();
        Self {
            _temp: temp,
            state,
            storage,
            backups,
        }
    }
    fn file(&self, path: &Path, data: &[u8]) {
        hiroute_platform::durable::write_private_new(path, data).unwrap();
    }
    fn controller(&self) -> UpgradeCleanup {
        UpgradeCleanup::new(&self.storage)
    }
}
fn drain(cleanup: &UpgradeCleanup) {
    // The largest fixture has 1,001 files. A cooperative time budget may
    // legitimately stop after one deletion; follow the scheduled due time
    // instead of imposing wall-clock throughput on a loaded filesystem.
    for _ in 0..2_048 {
        let due = cleanup.state.lock().unwrap().due;
        if cleanup.cycle_at(due).unwrap() {
            return;
        }
    }
    panic!("cleanup did not finish");
}

#[test]
fn readiness_is_required_and_live_data_is_unchanged() {
    let f = Fixture::new();
    f.file(&f.backups.join("core.db"), b"old authority");
    f.file(&f.storage.join("control.db"), b"current authority");
    f.file(
        &f.storage.join("observation-source-backup.json"),
        b"obsolete link",
    );
    crate::prepare_owner_directory(&f.storage.join("observation")).unwrap();
    f.file(
        &f.storage.join("observation/activity.db"),
        b"current history",
    );
    let cleanup = f.controller();
    assert!(!cleanup.cycle().unwrap());
    assert!(f.backups.join("core.db").exists());
    // Reopening after an interrupted or failed startup still supplies no authority.
    drop(cleanup);
    let cleanup = f.controller();
    assert!(!cleanup.cycle().unwrap());
    cleanup.service_ready();
    drain(&cleanup);
    assert!(!f.backups.exists());
    assert!(!f.storage.join("observation-source-backup.json").exists());
    assert_eq!(
        fs::read(f.storage.join("control.db")).unwrap(),
        b"current authority"
    );
    assert_eq!(
        fs::read(f.storage.join("observation/activity.db")).unwrap(),
        b"current history"
    );
    // Once complete, even a newer generation at the same source name is preserved.
    crate::prepare_owner_directory(&f.backups).unwrap();
    f.file(&f.backups.join("new"), b"new recovery");
    assert!(cleanup.cycle().unwrap());
    assert!(f.backups.join("new").exists());
}

#[test]
fn interruption_before_after_rename_and_mid_delete_is_recoverable() {
    for phase in 0..3 {
        let f = Fixture::new();
        for i in 0..300 {
            f.file(&f.backups.join(i.to_string()), b"backup");
        }
        let cleanup = f.controller();
        let mut job = cleanup.submit(Source::Backups).unwrap().unwrap();
        if phase > 0 {
            let payload = job.path.join("data");
            hiroute_platform::durable::rename(&f.backups, &payload).unwrap();
            if phase > 1 {
                assert!(!cleanup.advance(&mut job).unwrap());
            }
        }
        drop(job);
        drop(cleanup);
        let cleanup = f.controller();
        assert!(!cleanup.cycle().unwrap());
        cleanup.service_ready();
        drain(&cleanup);
        assert!(!f.backups.exists());
        assert_eq!(fs::read_dir(cleanup.jobs().unwrap()).unwrap().count(), 0);
    }
}

#[test]
fn old_claim_does_not_remove_a_replacement_source() {
    let f = Fixture::new();
    f.file(&f.backups.join("old"), b"old");
    let cleanup = f.controller();
    let mut job = cleanup.submit(Source::Backups).unwrap().unwrap();
    fs::rename(&f.backups, f.state.as_path().join("kept")).unwrap();
    crate::prepare_owner_directory(&f.backups).unwrap();
    f.file(&f.backups.join("new"), b"new");
    assert!(cleanup.advance(&mut job).unwrap());
    assert_eq!(fs::read(f.backups.join("new")).unwrap(), b"new");
}

#[test]
fn batches_back_off_and_make_bounded_progress() {
    let f = Fixture::new();
    for i in 0..1_001 {
        f.file(&f.backups.join(i.to_string()), b"backup");
    }
    let cleanup = f.controller();
    cleanup.service_ready();
    let now = cleanup.state.lock().unwrap().due;
    cleanup.cycle_at(now).unwrap(); // publish claim
    let job = cleanup
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .unwrap()
        .path
        .clone();
    let mut previous_due = cleanup.state.lock().unwrap().due;
    cleanup.cycle_at(previous_due).unwrap();
    let payload = job.join("data");
    let mut count = fs::read_dir(&payload).unwrap().count();
    assert!((873..1_001).contains(&count));
    for _ in 0..100 {
        cleanup.cycle_at(previous_due).unwrap();
    }
    assert_eq!(fs::read_dir(&payload).unwrap().count(), count);
    while count > 0 {
        let due = cleanup.state.lock().unwrap().due;
        assert!(due >= previous_due + INTERVAL);
        cleanup.cycle_at(due - Duration::from_nanos(1)).unwrap();
        assert_eq!(fs::read_dir(&payload).unwrap().count(), count);
        cleanup.cycle_at(due).unwrap();
        let remaining = match fs::read_dir(&payload) {
            Ok(entries) => entries.count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("cannot inspect retired payload: {error}"),
        };
        assert!(remaining < count, "a due batch must make progress");
        assert!(count - remaining <= BATCH, "each batch stays bounded");
        count = remaining;
        previous_due = due;
    }
    drain(&cleanup);
    assert!(!payload.exists());
    assert!(!job.exists());
}

fn handoff(f: &Fixture, target: &[u8], owned: bool) -> (PathBuf, Vec<u8>) {
    let parent = f.state.as_path().join(if owned {
        "upgrade-packages"
    } else {
        "external"
    });
    crate::prepare_owner_directory(&parent).unwrap();
    let package = parent.join("old-package-test");
    crate::prepare_owner_directory(&package).unwrap();
    f.file(&package.join("hirouted"), b"old image");
    let record = RetainedRecoveryPackage {
        schema: "hiroute.retained-recovery-package/v1".into(),
        storage_root: f.storage.clone(),
        package_root: package.clone(),
        target_daemon_sha256: format!("{:x}", Sha256::digest(target)),
        files: Default::default(),
    };
    let bytes = serde_json::to_vec(&record).unwrap();
    f.file(
        &f.state
            .as_path()
            .join(hiroute_host_runtime::RETAINED_PACKAGE_FILE),
        &bytes,
    );
    (package, bytes)
}

#[test]
fn matching_private_package_expires_but_external_and_future_images_do_not() {
    for (owned, matches) in [(true, true), (false, true), (true, false)] {
        let f = Fixture::new();
        let (package, _) = handoff(&f, b"new image", owned);
        let image = f.state.as_path().join("running-image");
        f.file(
            &image,
            if matches {
                b"new image"
            } else {
                b"older image"
            },
        );
        let cleanup = f.controller();
        cleanup.state.lock().unwrap().image = Some(fs::File::open(image).unwrap());
        cleanup.service_ready();
        drain(&cleanup);
        assert_eq!(package.exists(), !(owned && matches));
        assert_eq!(
            f.state
                .as_path()
                .join(hiroute_host_runtime::RETAINED_PACKAGE_FILE)
                .exists(),
            !matches
        );
    }
}

#[test]
fn empty_interrupted_wrapper_is_removed_and_unknown_material_is_preserved() {
    let f = Fixture::new();
    let cleanup = f.controller();
    crate::prepare_owner_directory(&cleanup.jobs().unwrap()).unwrap();
    let wrapper = cleanup.jobs().unwrap().join("retire-empty");
    crate::prepare_owner_directory(&wrapper).unwrap();
    assert!(cleanup.load(wrapper.clone()).unwrap().is_none());
    assert!(!wrapper.exists());
    crate::prepare_owner_directory(&wrapper).unwrap();
    f.file(&wrapper.join("unexpected"), b"keep");
    assert!(cleanup.load(wrapper.clone()).is_err());
    assert_eq!(fs::read(wrapper.join("unexpected")).unwrap(), b"keep");
}

#[test]
fn replaced_installer_handoff_is_preserved_with_its_package() {
    let f = Fixture::new();
    let (package, bytes) = handoff(&f, b"new image", true);
    let image = f.state.as_path().join("running-image");
    f.file(&image, b"new image");
    let cleanup = f.controller();
    cleanup.state.lock().unwrap().image = Some(fs::File::open(image).unwrap());
    cleanup.service_ready();
    cleanup.cycle().unwrap(); // capture this generation's handoff
    let record = f
        .state
        .as_path()
        .join(hiroute_host_runtime::RETAINED_PACKAGE_FILE);
    fs::rename(&record, f.state.as_path().join("old-handoff")).unwrap();
    f.file(&record, &bytes); // even identical bytes cannot adopt a newer identity
    drain(&cleanup);
    assert!(package.join("hirouted").exists());
    assert_eq!(fs::read(record).unwrap(), bytes);
}

#[test]
fn failed_cleanup_stays_pending_and_backs_off_until_repaired() {
    let f = Fixture::new();
    let cleanup = f.controller();
    crate::prepare_owner_directory(&cleanup.jobs().unwrap()).unwrap();
    let wrapper = cleanup.jobs().unwrap().join("retire-incomplete");
    crate::prepare_owner_directory(&wrapper).unwrap();
    f.file(&wrapper.join("unexpected"), b"preserve");
    cleanup.service_ready();
    let now = cleanup.state.lock().unwrap().due;
    cleanup.cycle_at(now).unwrap(); // discover, without admitting unknown material
    let due = cleanup.state.lock().unwrap().due;
    assert!(cleanup.cycle_at(due).is_err());
    assert!(cleanup.state.lock().unwrap().due >= due + Duration::from_secs(5));
    for seconds in 1..5 {
        assert!(
            !cleanup
                .cycle_at(due + Duration::from_secs(seconds))
                .unwrap()
        );
    }
    assert_eq!(fs::read(wrapper.join("unexpected")).unwrap(), b"preserve");
    assert_eq!(
        cleanup.state.lock().unwrap().pending.as_ref(),
        Some(&wrapper)
    );
    fs::remove_file(wrapper.join("unexpected")).unwrap();
    drain(&cleanup);
    assert!(!wrapper.exists());
    assert!(!f.backups.exists());
}

#[cfg(unix)]
#[test]
fn native_storage_names_do_not_share_retirement_authority() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new();
    let roots = [b"store-\x80".to_vec(), b"store-\x81".to_vec()]
        .map(|name| f.state.join(std::ffi::OsString::from_vec(name)));
    let controllers = roots.map(|root| {
        crate::prepare_owner_directory(&root).unwrap();
        let cleanup = UpgradeCleanup::new(&root);
        let backups = cleanup.source(&Source::Backups).unwrap();
        crate::prepare_owner_directory(&backups).unwrap();
        f.file(&backups.join("source"), b"keep original");
        cleanup
    });
    let other = controllers[1].submit(Source::Backups).unwrap().unwrap();
    let payload = other.path.join("data");
    fs::rename(controllers[1].source(&Source::Backups).unwrap(), &payload).unwrap();
    controllers[0].service_ready();
    drain(&controllers[0]);
    assert_eq!(fs::read(payload.join("source")).unwrap(), b"keep original");
    assert!(other.path.join("task.json").exists());
}
