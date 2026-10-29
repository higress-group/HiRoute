use crate::LocalStorageSet;
use rusqlite::Connection;

#[test]
fn owned_capture_rejects_control_and_secret_changes_and_rolls_back_local_writes() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let control = Connection::open(root.path().join("live/control.db")).unwrap();
    let secrets = Connection::open(root.path().join("live/secrets.db")).unwrap();
    assert!(
        stores
            .capture_agent_read(false, |current, _| {
                current
                    .connection
                    .borrow()
                    .query_row("SELECT store_uuid FROM storage_meta", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap();
                control
                    .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
                    .unwrap();
                Ok(())
            })
            .is_err()
    );
    assert!(
        stores
            .capture_agent_read(true, |_, current| {
                current
                    .unwrap()
                    .connection
                    .borrow()
                    .query_row("SELECT store_uuid FROM secret_store_meta", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap();
                secrets
                    .execute("UPDATE secret_store_meta SET updated_at=updated_at+1", [])
                    .unwrap();
                Ok(())
            })
            .is_err()
    );
    let uuid: String = stores
        .control
        .connection
        .borrow()
        .query_row("SELECT store_uuid FROM storage_meta", [], |r| r.get(0))
        .unwrap();
    assert!(
        stores
            .capture_agent_read(false, |current, _| {
                current
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE storage_meta SET store_uuid='temporary-local-change'",
                        [],
                    )
                    .unwrap();
                Ok(())
            })
            .is_err()
    );
    let after: String = stores
        .control
        .connection
        .borrow()
        .query_row("SELECT store_uuid FROM storage_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, uuid, "an accidental local write must roll back");
}

#[test]
fn common_epoch_rejects_cross_store_ddl_and_interleaved_commits() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let control = Connection::open(root.path().join("live/control.db")).unwrap();
    let secrets = Connection::open(root.path().join("live/secrets.db")).unwrap();
    for first_control in [true, false] {
        assert!(
            stores
                .capture_agent_read(true, |current, secret| {
                    current
                        .connection
                        .borrow()
                        .query_row("SELECT store_uuid FROM storage_meta", [], |r| {
                            r.get::<_, String>(0)
                        })
                        .unwrap();
                    secret
                        .unwrap()
                        .connection
                        .borrow()
                        .query_row("SELECT store_uuid FROM secret_store_meta", [], |r| {
                            r.get::<_, String>(0)
                        })
                        .unwrap();
                    if first_control {
                        control
                            .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
                            .unwrap();
                        secrets
                            .execute("UPDATE secret_store_meta SET updated_at=updated_at+1", [])
                            .unwrap();
                    } else {
                        secrets
                            .execute("UPDATE secret_store_meta SET updated_at=updated_at+1", [])
                            .unwrap();
                        control
                            .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
                            .unwrap();
                    }
                    Ok(())
                })
                .is_err()
        );
    }
    assert!(
        stores
            .capture_agent_read(true, |_, current| {
                current
                    .unwrap()
                    .connection
                    .borrow()
                    .query_row("SELECT store_uuid FROM secret_store_meta", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap();
                secrets
                    .execute_batch("CREATE TABLE physical_stamp_external_test(id INTEGER)")
                    .unwrap();
                Ok(())
            })
            .is_err()
    );
}

#[test]
fn final_fence_detects_external_change_and_caller_owned_transactions() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let control = Connection::open(root.path().join("live/control.db")).unwrap();
    let (_, stamp) = stores.capture_agent_read(false, |_, _| Ok(())).unwrap();
    assert!(
        stores
            .fence_agent_read(&stamp, |current, _| {
                current
                    .connection
                    .borrow()
                    .query_row("SELECT store_uuid FROM storage_meta", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap();
                control
                    .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
                    .unwrap();
                Ok(())
            })
            .is_err()
    );
    let db = stores.control.connection.borrow();
    let tx = db.unchecked_transaction().unwrap();
    assert!(stores.capture_agent_read(false, |_, _| Ok(())).is_err());
    tx.rollback().unwrap();
}

#[test]
fn same_uuid_reopened_connection_cannot_reuse_a_stamp() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let (_, stamp) = stores.capture_agent_read(true, |_, _| Ok(())).unwrap();
    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert!(reopened.fence_agent_read(&stamp, |_, _| Ok(())).is_err());
}

#[test]
#[cfg(unix)]
fn replacing_the_database_file_invalidates_its_connection_binding() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let (_, stamp) = stores.capture_agent_read(false, |_, _| Ok(())).unwrap();
    let path = root.path().join("live/control.db");
    let saved = root.path().join("live/control-original.db");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::copy(&saved, &path).unwrap();
    assert!(stores.fence_agent_read(&stamp, |_, _| Ok(())).is_err());
}

#[test]
fn physical_fences_reject_each_store_ddl_and_dml_in_both_intervals() {
    for stage in ["capture", "fence"] {
        for target in ["control", "secrets"] {
            for change in ["dml", "ddl"] {
                let root = crate::test_tempdir().unwrap();
                let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
                let path = root.path().join(format!("live/{target}.db"));
                let external = Connection::open(path).unwrap();
                let mutate = || {
                    if change == "ddl" {
                        external
                            .execute_batch("CREATE TABLE concurrent_stamp_change(id INTEGER)")
                            .unwrap();
                    } else if target == "control" {
                        external
                            .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
                            .unwrap();
                    } else {
                        external
                            .execute("UPDATE secret_store_meta SET updated_at=updated_at+1", [])
                            .unwrap();
                    }
                };
                let read_then_mutate =
                    |control: &crate::ControlStore, secret: Option<&crate::LocalSecretStore>| {
                        control
                            .connection
                            .borrow()
                            .query_row("SELECT store_uuid FROM storage_meta", [], |r| {
                                r.get::<_, String>(0)
                            })
                            .unwrap();
                        secret
                            .unwrap()
                            .connection
                            .borrow()
                            .query_row("SELECT store_uuid FROM secret_store_meta", [], |r| {
                                r.get::<_, String>(0)
                            })
                            .unwrap();
                        mutate();
                        Ok(())
                    };
                if stage == "capture" {
                    assert!(
                        stores.capture_agent_read(true, read_then_mutate).is_err(),
                        "{stage}/{target}/{change}"
                    );
                } else {
                    let (_, stamp) = stores.capture_agent_read(true, |_, _| Ok(())).unwrap();
                    assert!(
                        stores.fence_agent_read(&stamp, read_then_mutate).is_err(),
                        "{stage}/{target}/{change}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_barriered_external_commit_during_capture_invalidates_the_whole_bundle() {
    use std::sync::mpsc;
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let path = root.path().join("live/control.db");
    let (release, released) = mpsc::channel();
    let (finished, complete) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let external = Connection::open(path).unwrap();
        released.recv().unwrap();
        external
            .execute("UPDATE storage_meta SET store_uuid=store_uuid", [])
            .unwrap();
        finished.send(()).unwrap();
    });
    let result = stores.capture_agent_read(false, |control, _| {
        control
            .connection
            .borrow()
            .query_row("SELECT store_uuid FROM storage_meta", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap();
        release.send(()).unwrap();
        complete.recv().unwrap();
        Ok(())
    });
    writer.join().unwrap();
    assert!(result.is_err());
}
