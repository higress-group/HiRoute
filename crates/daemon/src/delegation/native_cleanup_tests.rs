use std::os::unix::fs::{MetadataExt, PermissionsExt};

use hiroute_domain::WorkspaceId;
use hiroute_domain::delegation::{
    DELEGATION_NATIVE_ROOT_MARKER_FILE_V1, DelegationNativeCleanupClaimV1,
    DelegationNativeCleanupJobV1, DelegationNativeFilesystemIdentityV1,
    DelegationNativeRootStateV1, WorkerHarnessV1,
};

use super::*;

fn claimed_root(base: &std::path::Path, root: &std::path::Path) -> DelegationNativeRootV1 {
    let base_metadata = std::fs::metadata(base).unwrap();
    let root_metadata = std::fs::metadata(root).unwrap();
    let mut record = DelegationNativeRootV1 {
        workspace_id: WorkspaceId::default(),
        task_id: "task-native-delete".into(),
        root_generation: 1,
        harness: WorkerHarnessV1::CodexCli,
        workspace_root_identity: "workspace-root".into(),
        relative_root: root.file_name().unwrap().to_str().unwrap().into(),
        creation_nonce: "creation-native-delete".into(),
        state: DelegationNativeRootStateV1::Deleting,
        use_revision: 1,
        managed_base_path: Some(base.to_str().unwrap().into()),
        filesystem_identity: Some(DelegationNativeFilesystemIdentityV1 {
            scheme: "unix-dev-inode-v1".into(),
            base_device: base_metadata.dev(),
            base_inode: base_metadata.ino(),
            root_device: root_metadata.dev(),
            root_inode: root_metadata.ino(),
            marker_device: 0,
            marker_inode: 0,
        }),
        cleanup_claim: Some(DelegationNativeCleanupClaimV1 {
            claim_id: "claim-native-delete".into(),
            root_generation: 1,
            expected_use_revision: 1,
            checked_at_ms: 1,
            jobs: vec![DelegationNativeCleanupJobV1 {
                workspace_id: WorkspaceId::default(),
                task_id: "task-native-delete".into(),
                run_id: "run-native-delete".into(),
                visibility_generation: 1,
                through_ms: 1,
            }],
        }),
        deletion_batches: 0,
        last_cleanup_failure: None,
    };
    let marker_path = root.join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1);
    std::fs::write(
        &marker_path,
        serde_json::to_vec(&record.ownership_marker()).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&marker_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let marker = std::fs::metadata(&marker_path).unwrap();
    let identity = record.filesystem_identity.as_mut().unwrap();
    identity.marker_device = marker.dev();
    identity.marker_inode = marker.ino();
    record
}

fn private_directory(path: &std::path::Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn deletion_is_bounded_and_does_not_follow_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    // macOS exposes /var through a symlink. Production records the canonical managed base,
    // so the fixture must exercise the same no-follow representation.
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let outside = base_path.join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, root_path.join("outside-link")).unwrap();
    for index in 0..70 {
        std::fs::write(root_path.join(format!("entry-{index:03}")), b"x").unwrap();
    }
    let root = claimed_root(&base_path, &root_path);
    assert_eq!(
        delete_native_root_batch(&root, 64),
        NativeDeletionOutcome::Deferred {
            processed_entries: 64
        }
    );
    assert!(root_path.exists());
    assert!(outside.exists());
    assert!(matches!(
        delete_native_root_batch(&root, 64),
        NativeDeletionOutcome::Removed { .. }
    ));
    assert!(!root_path.exists());
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[test]
fn supported_nested_tree_makes_progress_across_small_batches() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let root = claimed_root(&base_path, &root_path);
    let mut child = root_path.clone();
    for depth in 0..DELETE_DIRECTORY_DEPTH_LIMIT {
        child = child.join(format!("depth-{depth:02}"));
        private_directory(&child);
    }
    std::fs::write(child.join("leaf"), b"x").unwrap();

    let mut batches = 0;
    loop {
        batches += 1;
        match delete_native_root_batch(&root, 4) {
            NativeDeletionOutcome::Deferred { processed_entries } => {
                assert_eq!(processed_entries, 4);
                assert!(root_path.exists());
            }
            NativeDeletionOutcome::Removed { processed_entries } => {
                assert!((1..=4).contains(&processed_entries));
                break;
            }
            outcome => panic!("supported nested root did not make progress: {outcome:?}"),
        }
        assert!(batches < 16, "supported nested root did not converge");
    }
    assert!(batches > 1);
    assert!(!root_path.exists());
}

#[test]
fn final_marker_waits_for_a_root_removal_budget_unit() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let root = claimed_root(&base_path, &root_path);

    assert_eq!(
        delete_native_root_batch(&root, 1),
        NativeDeletionOutcome::Deferred {
            processed_entries: 0,
        }
    );
    assert!(
        root_path
            .join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1)
            .exists()
    );
    assert_eq!(
        delete_native_root_batch(&root, 2),
        NativeDeletionOutcome::Removed {
            processed_entries: 2,
        }
    );
    assert!(!root_path.exists());
}

#[test]
fn excessive_directory_depth_is_bounded_and_retained() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let root = claimed_root(&base_path, &root_path);
    let mut child = root_path.clone();
    for depth in 0..=DELETE_DIRECTORY_DEPTH_LIMIT {
        child = child.join(format!("depth-{depth:02}"));
        private_directory(&child);
    }
    std::fs::write(child.join("must-remain"), b"preserved").unwrap();

    assert_eq!(
        delete_native_root_batch(&root, DELETE_BATCH_LIMIT),
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::UnsafeEntry,
            processed_entries: 0,
        }
    );
    assert_eq!(
        std::fs::read(child.join("must-remain")).unwrap(),
        b"preserved"
    );
    assert!(
        root_path
            .join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1)
            .exists()
    );
}

#[test]
fn replacement_identity_blocks_deletion_but_accessible_modes_do_not() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let root = claimed_root(&base_path, &root_path);
    std::fs::remove_file(root_path.join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1)).unwrap();
    std::fs::remove_dir(&root_path).unwrap();
    private_directory(&root_path);
    std::fs::write(root_path.join("replacement"), b"keep").unwrap();
    let replacement = std::fs::metadata(&root_path).unwrap();
    let mut same_root_identity = root.clone();
    let identity = same_root_identity.filesystem_identity.as_mut().unwrap();
    identity.root_device = replacement.dev();
    identity.root_inode = replacement.ino();
    assert_eq!(
        delete_native_root_batch(&same_root_identity, 64),
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
            processed_entries: 0,
        }
    );
    assert!(root_path.join("replacement").exists());

    let permitted = claimed_root(&base_path, &root_path);
    std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        delete_native_root_batch(&permitted, 64),
        NativeDeletionOutcome::Removed { .. }
    ));
    assert!(!root_path.exists());
}

#[test]
fn wrong_marker_content_blocks_before_deleting_any_entry() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let mut root = claimed_root(&base_path, &root_path);
    let marker_path = root_path.join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1);
    std::fs::write(&marker_path, br#"{"schema":"unrelated"}"#).unwrap();
    let marker = std::fs::metadata(&marker_path).unwrap();
    let identity = root.filesystem_identity.as_mut().unwrap();
    identity.marker_device = marker.dev();
    identity.marker_inode = marker.ino();
    std::fs::write(root_path.join("must-remain"), b"preserved").unwrap();

    assert_eq!(
        delete_native_root_batch(&root, 64),
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
            processed_entries: 0,
        }
    );
    assert_eq!(
        std::fs::read(root_path.join("must-remain")).unwrap(),
        b"preserved"
    );

    // Even byte-for-byte marker contents do not adopt a replacement marker file.
    let replacement = root_path.join("replacement-marker");
    std::fs::write(
        &replacement,
        serde_json::to_vec(&root.ownership_marker()).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(&replacement, &marker_path).unwrap();
    assert_eq!(
        delete_native_root_batch(&root, 64),
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
            processed_entries: 0,
        }
    );
    assert!(root_path.join("must-remain").exists());
}

#[test]
fn blocked_after_partial_deletion_reports_consumed_budget() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let base_path = std::fs::canonicalize(directory.path()).unwrap();
    let root_path = base_path.join("owned-root");
    private_directory(&root_path);
    let child = root_path.join("child");
    private_directory(&child);
    let removed_before_block = child.join("removed-before-block");
    std::fs::write(&removed_before_block, b"x").unwrap();
    let root = claimed_root(&base_path, &root_path);

    // Reading/traversing remains possible, but removing the emptied child from this root is
    // denied. Both the child entry and its file consumed this tick's global budget.
    std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(
        delete_native_root_batch(&root, 64),
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
            processed_entries: 2,
        }
    );
    assert!(!removed_before_block.exists());
    assert!(child.exists());
    std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
