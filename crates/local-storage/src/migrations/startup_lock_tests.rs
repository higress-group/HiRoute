use super::*;
use crate::LocalStorageSet;

fn locked(root: &Path) {
    assert!(
        matches!(acquire_startup_lock(root), Err(LocalStorageError::Recovery { recovery, .. }) if recovery.cause == hiroute_host_runtime::StorageRecoveryCause::Locked),
        "a real store owner must continue to exclude a second writer"
    );
}

#[test]
fn duplicate_os_handle_does_not_extend_released_startup_ownership() {
    let root = tempfile::tempdir().unwrap();
    let owner = acquire_startup_lock(root.path()).unwrap();
    // dup and fork retain the same Unix open-file description. Keep its
    // descriptor alive deterministically, without a fork/exec timing race.
    let inherited = owner.file.try_clone().unwrap();
    locked(root.path());
    drop(owner);
    let next = acquire_startup_lock(root.path())
        .expect("the actual owner is gone even though a duplicate descriptor remains");
    drop(inherited);
    locked(root.path());
    drop(next);
    assert!(acquire_startup_lock(root.path()).is_ok());
}

#[test]
fn foreign_process_cleanup_cannot_unlock_the_retained_owner() {
    let root = tempfile::tempdir().unwrap();
    let owner = acquire_startup_lock(root.path()).unwrap();
    // Model the copied guard as observed in another process. The duplicate
    // still refers to the real owner's flock; no unsafe post-fork work runs.
    let inherited = StartupLock {
        file: owner.file.try_clone().unwrap(),
        owner_pid: owner.owner_pid.wrapping_add(1),
    };
    drop(inherited);
    locked(root.path());
    drop(owner);
    assert!(acquire_startup_lock(root.path()).is_ok());
}

#[test]
fn each_store_part_keeps_ownership_until_its_last_real_owner_drops() {
    for keep in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("store");
        let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        let inherited = stores
            .control()
            .startup_lock
            .as_ref()
            .unwrap()
            .file
            .try_clone()
            .unwrap();
        let (control, runtime, secrets) = stores.into_parts();
        let (mut control, mut runtime, mut secrets) = (Some(control), Some(runtime), Some(secrets));
        if keep != 0 {
            drop(control.take());
        }
        if keep != 1 {
            drop(runtime.take());
        }
        if keep != 2 {
            drop(secrets.take());
        }
        locked(&root);
        drop((control, runtime, secrets));
        let reopened = LocalStorageSet::open_for_daemon_startup(&root)
            .expect("last real store owner released startup ownership");
        drop(inherited);
        locked(&root);
        drop(reopened);
        assert!(LocalStorageSet::open_for_daemon_startup(&root).is_ok());
    }
}
