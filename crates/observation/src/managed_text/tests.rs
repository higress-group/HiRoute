use super::*;
use crate::{DigestAuthority, LocalObservationStore};

fn input(event: &str, time: i64) -> ManagedTextInput {
    ManagedTextInput {
        scope: ManagedTextScope {
            workspace_id: WorkspaceId::default(),
            task_id: "task-a".into(),
            run_id: "run-a".into(),
        },
        purpose: ManagedTextPurpose::Result,
        source_event_id: event.into(),
        source_revision: 1,
        original_created_at_ms: time,
        import_origin: None,
    }
}

fn store(root: &std::path::Path) -> LocalObservationStore {
    LocalObservationStore::open(root, DigestAuthority::new([7; 32])).unwrap()
}

fn complete(store: &LocalObservationStore, input: &ManagedTextInput) -> ManagedTextRef {
    let reference = store.managed_text_put(input, 100).unwrap();
    store
        .managed_text_append(&input.scope, &reference, 0, b"actual result", 100)
        .unwrap();
    store
        .managed_text_finish(&input.scope, &reference, 1, 100)
        .unwrap()
}

#[test]
fn retained_visibility_reader_observes_later_commits_without_pinning_wal() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("reader-a", 10);
    let reference = store.managed_text_put(&event, 100).unwrap();
    let mut reader = store.managed_text_reader().unwrap();
    assert_eq!(
        reader.resolve(&event.scope, &reference, 100).unwrap().state,
        ManagedTextState::Pending
    );
    let completed = complete(&store, &event);
    assert_eq!(
        reader.resolve(&event.scope, &reference, 100).unwrap(),
        completed
    );
    assert!(reader.connection.is_autocommit());
    let checkpoint_busy: i64 = store
        .connection
        .lock()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(checkpoint_busy, 0);

    let preview = store
        .managed_text_delete_preview(&event.scope, 100, 100)
        .unwrap();
    let deletion = store.managed_text_delete_apply(&preview).unwrap();
    let hidden = reader.resolve(&event.scope, &reference, 100).unwrap();
    assert_eq!(hidden.state, ManagedTextState::Deleted);
    assert_eq!(hidden.visibility_generation, deletion.visibility_generation);

    let later = input("reader-b", 101);
    let later_ref = store.managed_text_put(&later, 200).unwrap();
    assert_eq!(
        reader.resolve(&later.scope, &later_ref, 200).unwrap(),
        later_ref
    );
    assert_eq!(
        reader
            .resolve(&later.scope, &later_ref, 101 + RETENTION_MS)
            .unwrap()
            .state,
        ManagedTextState::Expired
    );
    assert!(reader.connection.is_autocommit());
}

#[test]
fn retained_visibility_reader_keeps_scope_and_reference_validation() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("reader-scope", 10);
    let reference = complete(&store, &event);
    let mut reader = store.managed_text_reader().unwrap();
    let mut other = event.scope.clone();
    other.run_id = "another-run".into();
    assert_eq!(
        reader.resolve(&other, &reference, 100),
        Err(ManagedTextError::ScopeMismatch)
    );
    let mut forged = reference.clone();
    forged.scope = other.clone();
    assert_eq!(
        reader.resolve(&other, &forged, 100),
        Err(ManagedTextError::Unavailable)
    );
    forged = reference.clone();
    forged.opaque_id.clear();
    assert_eq!(
        reader.resolve(&event.scope, &forged, 100),
        Err(ManagedTextError::Invalid)
    );
    assert_eq!(
        reader.resolve(&event.scope, &reference, 100).unwrap(),
        reference
    );
}

#[test]
fn event_retries_preserve_deadline_and_conflicting_content_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("event-a", 10);
    let reference = complete(&store, &event);
    assert_eq!(store.managed_text_put(&event, 200).unwrap(), reference);
    assert_eq!(reference.original_retention_deadline_ms, 10 + RETENTION_MS);
    store
        .managed_text_append(&event.scope, &reference, 0, b"actual result", 200)
        .unwrap();
    assert_eq!(
        store.managed_text_append(&event.scope, &reference, 0, b"different", 200),
        Err(ManagedTextError::Conflict)
    );
    let mut changed = event.clone();
    changed.original_created_at_ms = 11;
    assert_eq!(
        store.managed_text_put(&changed, 200),
        Err(ManagedTextError::Conflict)
    );
    assert_eq!(
        store
            .managed_text_resolve(&event.scope, &reference, 10 + RETENTION_MS)
            .unwrap()
            .state,
        ManagedTextState::Expired
    );
    assert_eq!(
        store.managed_text_read(&event.scope, &reference, 0, 1, 10 + RETENTION_MS),
        Err(ManagedTextError::Unavailable)
    );
}

#[test]
fn caller_scope_and_forged_reference_cannot_retrieve_other_run() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("event-a", 10);
    let reference = complete(&store, &event);
    let mut other = event.scope.clone();
    other.run_id = "run-b".into();
    assert_eq!(
        store.managed_text_read(&other, &reference, 0, 1, 100),
        Err(ManagedTextError::ScopeMismatch)
    );
    let mut forged = reference.clone();
    forged.scope = other.clone();
    assert_eq!(
        store.managed_text_read(&other, &forged, 0, 1, 100),
        Err(ManagedTextError::Unavailable)
    );
}

#[test]
fn bounded_chunks_page_forward_and_finish_requires_contiguous_committed_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("event-a", 10);
    let reference = store.managed_text_put(&event, 100).unwrap();
    assert_eq!(
        store.managed_text_append(&event.scope, &reference, 1, b"gap", 100),
        Err(ManagedTextError::Conflict)
    );
    assert_eq!(
        store.managed_text_append(&event.scope, &reference, 0, &vec![0; CHUNK_BYTES + 1], 100),
        Err(ManagedTextError::Invalid)
    );
    for ordinal in 0..18 {
        store
            .managed_text_append(
                &event.scope,
                &reference,
                ordinal,
                &vec![ordinal as u8; CHUNK_BYTES],
                100,
            )
            .unwrap();
    }
    assert_eq!(
        store.managed_text_finish(&event.scope, &reference, 19, 100),
        Err(ManagedTextError::Conflict)
    );
    let reference = store
        .managed_text_finish(&event.scope, &reference, 18, 100)
        .unwrap();
    let first = store
        .managed_text_read(&event.scope, &reference, 0, 16, 100)
        .unwrap();
    assert_eq!(first.bytes.len(), PAGE_BYTES);
    assert_eq!(first.next_chunk, Some(16));
    let last = store
        .managed_text_read(&event.scope, &reference, 16, 16, 100)
        .unwrap();
    assert_eq!(last.bytes.len(), 2 * CHUNK_BYTES);
    assert_eq!(last.next_chunk, None);
}

#[test]
fn deletion_survives_restart_blocks_late_import_and_preserves_other_run() {
    let directory = tempfile::tempdir().unwrap();
    let first = store(directory.path());
    let event = input("event-a", 10);
    let reference = complete(&first, &event);
    let mut other = event.clone();
    other.scope.run_id = "run-b".into();
    let other_ref = complete(&first, &other);
    let preview = first
        .managed_text_delete_preview(&event.scope, 100, 100)
        .unwrap();
    assert_eq!(preview.reference_count, 1);
    let result = first.managed_text_delete_apply(&preview).unwrap();
    assert!(result.object_gc_pending && result.native_gc_pending);
    assert_eq!(first.managed_text_delete_apply(&preview).unwrap(), result);
    drop(first);
    let restarted = store(directory.path());
    let jobs = restarted
        .managed_text_pending_native_cleanup(&event.scope, 0, 50)
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].through_ms, 100);
    assert_eq!(
        restarted
            .managed_text_resolve(&event.scope, &reference, 100)
            .unwrap()
            .state,
        ManagedTextState::Deleted
    );
    assert_eq!(
        restarted.managed_text_read(&event.scope, &reference, 0, 1, 100),
        Err(ManagedTextError::Unavailable)
    );
    assert_eq!(
        restarted.managed_text_put(&input("late", 99), 100),
        Err(ManagedTextError::Unavailable)
    );
    let mut import = input("renamed-old-history", 10);
    import.import_origin = Some(reference);
    assert_eq!(
        restarted.managed_text_put(&import, 100),
        Err(ManagedTextError::Unavailable)
    );
    assert_eq!(restarted.managed_text_gc(100, 200).unwrap(), 1);
    restarted
        .managed_text_native_gc_ack(&event.scope, result.visibility_generation)
        .unwrap();
    let result = restarted.managed_text_delete_apply(&preview).unwrap();
    assert!(!result.object_gc_pending && !result.native_gc_pending);
    assert!(
        restarted
            .managed_text_pending_native_cleanup(&event.scope, 0, 50)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restarted
            .managed_text_read(&other.scope, &other_ref, 0, 1, 100)
            .unwrap()
            .bytes,
        b"actual result"
    );
    assert!(restarted.managed_text_put(&input("new", 101), 101).is_ok());
}

#[test]
fn changed_preview_is_stale_and_cancel_has_no_effect() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("event-a", 10);
    let reference = complete(&store, &event);
    let preview = store
        .managed_text_delete_preview(&event.scope, 100, 100)
        .unwrap();
    assert_eq!(
        store
            .managed_text_read(&event.scope, &reference, 0, 1, 100)
            .unwrap()
            .bytes,
        b"actual result"
    );
    complete(&store, &input("event-b", 11));
    assert_eq!(
        store.managed_text_delete_apply(&preview),
        Err(ManagedTextError::Stale)
    );
    assert_eq!(
        store
            .managed_text_resolve(&event.scope, &reference, 100)
            .unwrap()
            .state,
        ManagedTextState::Complete
    );
}

#[test]
fn import_must_prove_original_deadline_and_current_visibility() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let event = input("event-a", 10);
    let reference = complete(&store, &event);
    let mut import = input("import", 11);
    import.import_origin = Some(reference);
    assert_eq!(
        store.managed_text_put(&import, 100),
        Err(ManagedTextError::Unavailable)
    );
    import.original_created_at_ms = 10;
    let imported = store.managed_text_put(&import, 100).unwrap();
    assert_eq!(imported.original_retention_deadline_ms, 10 + RETENTION_MS);
}

#[test]
fn interrupted_chunk_is_hidden_retryable_and_collected_after_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let first = store(directory.path());
    let event = input("event-a", 10);
    let reference = first.managed_text_put(&event, 100).unwrap();
    // Persist exactly the pre-I/O journal and a partially written file, as a
    // process interruption would leave them. Neither is a committed body chunk.
    use sha2::{Digest, Sha256};
    let expected = format!("{:x}", Sha256::digest(b"actual result"));
    first.connection.lock().execute(
        "INSERT INTO managed_text_chunks(ref_id,ordinal,digest,size,ready) VALUES(?1,0,?2,13,0)",
        rusqlite::params![reference.opaque_id, expected],
    ).unwrap();
    std::fs::write(first.managed_chunk_path(&reference.opaque_id, 0), b"act").unwrap();
    drop(first);
    let restarted = store(directory.path());
    assert!(
        restarted
            .managed_text_read(&event.scope, &reference, 0, 1, 100)
            .unwrap()
            .bytes
            .is_empty()
    );
    assert_eq!(
        restarted.managed_text_finish(&event.scope, &reference, 0, 100),
        Err(ManagedTextError::Conflict)
    );
    restarted
        .managed_text_append(&event.scope, &reference, 0, b"actual result", 100)
        .unwrap();
    restarted
        .managed_text_finish(&event.scope, &reference, 1, 100)
        .unwrap();
    assert_eq!(
        restarted
            .managed_text_read(&event.scope, &reference, 0, 1, 100)
            .unwrap()
            .bytes,
        b"actual result"
    );
    assert_eq!(restarted.managed_text_gc(10 + RETENTION_MS, 1).unwrap(), 1);
    assert!(
        !restarted
            .managed_chunk_path(&reference.opaque_id, 0)
            .exists()
    );
}

#[test]
fn expiry_enqueues_native_cleanup_once_and_does_not_extend_long_run_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path());
    let old = input("old", 10);
    let old_ref = complete(&store, &old);
    let new = input("new", 20);
    let new_ref = complete(&store, &new);
    let result = store
        .managed_text_expire_scope(&old.scope, 10 + RETENTION_MS)
        .unwrap()
        .unwrap();
    assert_eq!(result.logically_hidden, 1);
    assert!(result.native_gc_pending);
    assert_eq!(
        store
            .managed_text_resolve(&old.scope, &old_ref, 10 + RETENTION_MS)
            .unwrap()
            .state,
        ManagedTextState::Expired
    );
    let new_ref = store
        .managed_text_resolve(&new.scope, &new_ref, 10 + RETENTION_MS)
        .unwrap();
    assert_eq!(
        store
            .managed_text_read(&new.scope, &new_ref, 0, 1, 10 + RETENTION_MS)
            .unwrap()
            .bytes,
        b"actual result"
    );
    assert!(
        store
            .managed_text_expire_scope(&old.scope, 10 + RETENTION_MS)
            .unwrap()
            .is_none()
    );
}

#[test]
fn restart_discovers_expired_scopes_and_durably_schedules_native_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let first = input("expiry-a", 10);
    let mut second = input("expiry-b", 20);
    second.scope.run_id = "run-b".into();
    {
        let store = store(directory.path());
        complete(&store, &first);
        complete(&store, &second);
    }
    let store = store(directory.path());
    let now = RETENTION_MS + 100;
    assert_eq!(store.managed_text_expire_pending(now, 1).unwrap(), 1);
    assert_eq!(store.managed_text_expire_pending(now, 1).unwrap(), 1);
    assert_eq!(store.managed_text_expire_pending(now, 1).unwrap(), 0);
    for event in [&first, &second] {
        let jobs = store
            .managed_text_pending_native_cleanup(&event.scope, 0, 10)
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].through_ms, 100);
    }
    assert_eq!(store.managed_text_gc(now, 1).unwrap(), 1);
    assert_eq!(store.managed_text_gc(now, 1).unwrap(), 1);
    assert_eq!(store.managed_text_gc(now, 1).unwrap(), 0);
    assert_eq!(
        store
            .managed_text_pending_native_cleanup(&first.scope, 0, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn native_cleanup_page_is_keyset_bounded_and_exact_ack_is_restart_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let first = input("ack-first", 10);
    let mut second = input("ack-second", 11);
    second.scope.task_id = "task-b".into();
    second.scope.run_id = "run-b".into();
    let first_job;
    let second_job;
    {
        let store = store(directory.path());
        complete(&store, &first);
        complete(&store, &second);
        for event in [&first, &second] {
            let preview = store
                .managed_text_delete_preview(&event.scope, 100, 100)
                .unwrap();
            store.managed_text_delete_apply(&preview).unwrap();
        }
        let page = store
            .managed_text_pending_native_cleanup_page(None, 1)
            .unwrap();
        assert_eq!(page.len(), 1);
        let cursor = ManagedTextNativeCleanupCursor {
            scope: page[0].scope.clone(),
            visibility_generation: page[0].visibility_generation,
        };
        let next = store
            .managed_text_pending_native_cleanup_page(Some(&cursor), 1)
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_ne!(page[0].scope, next[0].scope);
        assert!(
            store
                .managed_text_pending_native_cleanup_page(
                    Some(&ManagedTextNativeCleanupCursor {
                        scope: next[0].scope.clone(),
                        visibility_generation: next[0].visibility_generation,
                    }),
                    1,
                )
                .unwrap()
                .is_empty()
        );
        first_job = page[0].clone();
        second_job = next[0].clone();
        store.managed_text_native_gc_ack_exact(&first_job).unwrap();
    }

    let restarted = store(directory.path());
    // Crash after the first of several claim jobs: replaying that exact triple is success, so the
    // consumer can continue to later jobs instead of becoming permanently stuck at the prefix.
    restarted
        .managed_text_native_gc_ack_exact(&first_job)
        .unwrap();
    restarted
        .managed_text_native_gc_ack_exact(&second_job)
        .unwrap();
    let mut wrong = second_job.clone();
    wrong.through_ms += 1;
    assert_eq!(
        restarted.managed_text_native_gc_ack_exact(&wrong),
        Err(ManagedTextError::Unavailable)
    );
    assert!(
        restarted
            .managed_text_pending_native_cleanup_page(None, 10)
            .unwrap()
            .is_empty()
    );
}
