//! Real-filesystem counterexamples for the safe file adapter (V6).
//!
//! Every case builds the unsafe object on disk and asserts that the operation is refused
//! before reading or writing, and that an external sentinel file is unchanged. No test
//! replaces the platform with a mock.

#![cfg(unix)]

mod support;

use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::time::{Duration, Instant};

use hiroute_diagnostics::files::{
    CORRELATION_KEY_FILE, CURRENT_LOG_FILE, FileSafetyError, PrivateDir, SETTINGS_FILE,
    SETTINGS_LOCK_FILE, WRITER_LOCK_FILE, is_allowed_name, previous_log_file,
};

fn private_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = support::private_tempdir();
    let root = temp.path().join("diagnostics");
    std::fs::create_dir(&root).expect("create root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    (temp, root)
}

fn sentinel(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let path = temp.path().join("external-sentinel.txt");
    std::fs::write(&path, b"external-sentinel-content").expect("write sentinel");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    path
}

#[test]
fn symlinked_settings_file_is_refused_and_sentinel_untouched() {
    let (temp, root) = private_root();
    let outside = sentinel(&temp);
    symlink(&outside, root.join(SETTINGS_FILE)).expect("create symlink");
    let dir = PrivateDir::open_existing(&root).expect("open root");
    assert!(matches!(
        dir.open_read(SETTINGS_FILE),
        Err(FileSafetyError::UnsafeFile)
    ));
    let error = dir.create_new(SETTINGS_FILE).expect_err("must refuse");
    assert!(matches!(
        error,
        FileSafetyError::AlreadyExists | FileSafetyError::UnsafeFile
    ));
    assert_eq!(
        std::fs::read(&outside).expect("read sentinel"),
        b"external-sentinel-content"
    );
    assert_eq!(
        std::fs::metadata(&outside)
            .expect("stat")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn fifo_at_log_path_is_refused_without_blocking() {
    let (temp, root) = private_root();
    let fifo = root.join(CURRENT_LOG_FILE);
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(status.success());
    let _outside = sentinel(&temp);
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let started = std::time::Instant::now();
    let error = dir
        .open_append_or_create(CURRENT_LOG_FILE)
        .expect_err("must refuse fifo");
    assert!(matches!(
        error,
        FileSafetyError::UnsafeFile | FileSafetyError::Io { .. }
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let metadata = std::fs::symlink_metadata(&fifo).expect("stat fifo");
    assert!(metadata.file_type().is_fifo());
}

#[test]
fn hard_linked_log_file_is_refused() {
    let (_temp, root) = private_root();
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut file = dir.create_new(CURRENT_LOG_FILE).expect("create");
    file.append(b"{}\n").expect("append");
    drop(file);
    std::fs::hard_link(root.join(CURRENT_LOG_FILE), root.join("outside-link.jsonl"))
        .expect("hard link");
    let error = dir
        .open_append_or_create(CURRENT_LOG_FILE)
        .expect_err("must refuse multiply linked file");
    assert_eq!(error, FileSafetyError::UnsafeFile);
}

#[test]
fn readable_file_and_directory_modes_are_preserved() {
    let (temp, root) = private_root();
    let _outside = sentinel(&temp);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("chmod dir");
    PrivateDir::open_existing(&root).expect("accessible directory");
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut file = dir.create_new(SETTINGS_FILE).expect("create");
    file.append(b"x").expect("append");
    drop(file);
    std::fs::set_permissions(
        root.join(SETTINGS_FILE),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("chmod file");
    assert_eq!(
        dir.open_read(SETTINGS_FILE)
            .unwrap()
            .unwrap()
            .read_prefix(10)
            .unwrap(),
        b"x"
    );
    assert_eq!(
        std::fs::metadata(root.join(SETTINGS_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn symlinked_root_directory_is_refused() {
    let (temp, root) = private_root();
    let link = temp.path().join("diagnostics-link");
    symlink(&root, &link).expect("symlink dir");
    assert_eq!(
        PrivateDir::open_existing(&link).expect_err("symlinked root must be refused"),
        FileSafetyError::UnsafeDirectory
    );
}

#[test]
fn second_writer_lock_is_reported_as_locked_and_never_takes_over() {
    let (_temp, root) = private_root();
    let first = PrivateDir::open_existing(&root).expect("open root");
    let second = PrivateDir::open_existing(&root).expect("open root");
    let held = first.open_lock(WRITER_LOCK_FILE).expect("open lock");
    held.try_lock_exclusive().expect("take lock");
    let contender = second.open_lock(WRITER_LOCK_FILE).expect("open lock");
    assert_eq!(
        contender.try_lock_exclusive().expect_err("second writer"),
        FileSafetyError::Locked
    );
    // The loser must not modify or remove the winner's files.
    let mut current = first
        .open_append_or_create(CURRENT_LOG_FILE)
        .expect("append");
    current.append(b"first-writer").expect("append");
    drop(current);
    for name in [CURRENT_LOG_FILE, WRITER_LOCK_FILE] {
        assert!(second.open_read(name).expect("open").is_some());
    }
}

#[test]
fn rename_and_remove_reject_replaced_identity_and_unknown_names() {
    let (_temp, root) = private_root();
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut file = dir.create_new(&previous_log_file(1)).expect("create");
    file.append(b"data").expect("append");
    let identity = file.identity();
    drop(file);
    // Replace the file behind the recorded identity.
    let existing = dir
        .open_read(&previous_log_file(1))
        .expect("open")
        .expect("present");
    dir.remove_verified(&previous_log_file(1), existing.identity())
        .expect("remove");
    let mut replacement = dir.create_new(&previous_log_file(1)).expect("create");
    replacement.append(b"replacement").expect("append");
    drop(replacement);
    assert_eq!(
        dir.rename_verified(&previous_log_file(1), &previous_log_file(2), identity)
            .expect_err("identity changed"),
        FileSafetyError::IdentityChanged
    );
    assert_eq!(
        dir.remove_verified(&previous_log_file(1), identity)
            .expect_err("identity changed"),
        FileSafetyError::IdentityChanged
    );
    // Unknown names are outside the allowed set and are never listed as ours.
    assert!(!is_allowed_name("attacker.jsonl"));
    assert!(!is_allowed_name("previous-9.jsonl"));
    let mut unknown = root.join("attacker.jsonl");
    std::fs::write(&unknown, b"unknown-file").expect("write unknown");
    std::fs::set_permissions(&unknown, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    unknown.pop();
    let error = dir
        .remove_verified("attacker.jsonl", identity)
        .expect_err("unknown file");
    assert!(matches!(
        error,
        FileSafetyError::IdentityChanged | FileSafetyError::Io { .. } | FileSafetyError::UnsafeFile
    ));
    assert!(root.join("attacker.jsonl").exists());
}

#[test]
fn rotation_never_follows_a_symlinked_previous_file() {
    let (temp, root) = private_root();
    let outside = sentinel(&temp);
    symlink(&outside, root.join(previous_log_file(4))).expect("symlink previous-4");
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut current = dir.create_new(CURRENT_LOG_FILE).expect("create current");
    current
        .append(b"{\"schema\":\"hiroute.diagnostic-event/v1\"}\n")
        .expect("append");
    drop(current);
    // A rotation that needs to drop previous-4 must refuse the symlink and leave it alone.
    assert!(matches!(
        dir.open_read(&previous_log_file(4)),
        Err(FileSafetyError::UnsafeFile)
    ));
    assert_eq!(
        std::fs::read(&outside).expect("read sentinel"),
        b"external-sentinel-content"
    );
}

#[test]
fn allowed_names_cover_exactly_the_owned_set() {
    assert!(is_allowed_name(SETTINGS_FILE));
    assert!(is_allowed_name(SETTINGS_LOCK_FILE));
    assert!(is_allowed_name(CORRELATION_KEY_FILE));
    assert!(is_allowed_name(WRITER_LOCK_FILE));
    assert!(is_allowed_name(CURRENT_LOG_FILE));
    for index in 1..=4 {
        assert!(is_allowed_name(&previous_log_file(index)));
    }
    assert!(!is_allowed_name("settings.json.bak"));
    assert!(!is_allowed_name("../settings.json"));
    assert!(!is_allowed_name("keychain.db"));
}

/// The app root itself can be missing on a first start. Only the missing trail below the
/// nearest existing ancestor may be created, every created level private, and no
/// pre-existing level may be chmodded to pass the checks.
#[test]
fn missing_parent_chain_is_created_without_touching_existing_levels() {
    let temp = support::private_tempdir();
    let existing = temp.path().join("existing-parent");
    std::fs::create_dir(&existing).expect("create existing parent");
    // Group/other readable but not writable: allowed for an ancestor and never chmodded.
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o711)).expect("chmod");
    let root = existing.join("app-local-data").join("diagnostics");

    let dir = PrivateDir::open_or_create(&root).expect("create missing chain");
    assert_eq!(
        std::fs::metadata(&existing)
            .expect("stat")
            .permissions()
            .mode()
            & 0o777,
        0o711,
        "an existing ancestor must never be chmodded"
    );
    for level in [existing.join("app-local-data"), root.clone()] {
        let meta = std::fs::metadata(&level).expect("stat created level");
        assert!(meta.is_dir());
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o700,
            "created levels are private"
        );
    }
    assert_eq!(dir.path(), root.as_path());
}

/// Existing local directory permissions are not an admission or automatic chmod trigger.
#[test]
fn writable_non_sticky_ancestor_is_accepted_without_chmod() {
    let temp = support::private_tempdir();
    let loose = temp.path().join("loose-parent");
    std::fs::create_dir(&loose).expect("create loose parent");
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    PrivateDir::open_or_create(&loose.join("diagnostics")).unwrap();
    assert_eq!(
        std::fs::metadata(&loose).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert_eq!(
        std::fs::metadata(loose.join("diagnostics"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

/// An ancestor replaced by a symlink must be refused before anything is opened or created
/// through it, and the external directory keeps its content and permissions.
#[test]
fn ancestor_symlink_to_external_private_directory_is_refused() {
    let (temp, root) = private_root();
    let outside = temp.path().join("outside-private");
    std::fs::create_dir(&outside).expect("create outside");
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let external_sentinel = outside.join("sentinel.txt");
    std::fs::write(&external_sentinel, b"external-sentinel-content").expect("write sentinel");
    std::fs::set_permissions(&external_sentinel, std::fs::Permissions::from_mode(0o600))
        .expect("chmod sentinel");
    symlink(&outside, root.join("escape")).expect("symlink ancestor");

    assert_eq!(
        PrivateDir::open_or_create(&root.join("escape").join("diagnostics"))
            .expect_err("a symlinked ancestor must be refused"),
        FileSafetyError::UnsafeDirectory
    );
    assert_eq!(
        PrivateDir::open_existing(&root.join("escape")).expect_err("leaf symlink refused"),
        FileSafetyError::UnsafeDirectory
    );
    assert!(
        !outside.join("diagnostics").exists(),
        "nothing may be created through the link"
    );
    assert_eq!(
        std::fs::read(&external_sentinel).expect("read sentinel"),
        b"external-sentinel-content"
    );
    assert_eq!(
        std::fs::metadata(&outside)
            .expect("stat outside")
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "the external directory permissions must stay untouched"
    );
}

/// Chmod does not block an open handle; hardlink replacement still prevents further writes.
#[test]
fn append_rechecks_the_open_file_after_chmod_and_hardlink() {
    let (_temp, root) = private_root();
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut file = dir.create_new(CURRENT_LOG_FILE).expect("create");
    file.append(b"first\n").expect("append");
    std::fs::set_permissions(
        root.join(CURRENT_LOG_FILE),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("chmod");
    file.append(b"second\n")
        .expect("chmod does not revoke access");
    std::fs::set_permissions(
        root.join(CURRENT_LOG_FILE),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("chmod back");
    file.append(b"third\n").expect("append after repair");
    // Both permission modes allow writing to the same stable file.
    assert_eq!(
        std::fs::read(root.join(CURRENT_LOG_FILE)).expect("raw read"),
        b"first\nsecond\nthird\n"
    );
    std::fs::hard_link(root.join(CURRENT_LOG_FILE), root.join("outside-link.jsonl"))
        .expect("hard link");
    assert_eq!(
        file.append(b"fourth\n")
            .expect_err("multiply linked file refused"),
        FileSafetyError::UnsafeFile
    );
    drop(file);
    assert_eq!(
        std::fs::read(root.join(CURRENT_LOG_FILE)).expect("raw read"),
        b"first\nsecond\nthird\n",
        "the multiply linked file stayed untouched"
    );
}

/// A regular file replaced by a FIFO must be refused in bounded time: the swap cannot make
/// rename/remove block on an open that waits for a writer.
#[test]
fn fifo_swap_is_refused_without_blocking_rename_and_remove() {
    let (_temp, root) = private_root();
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut file = dir.create_new(&previous_log_file(1)).expect("create");
    file.append(b"data").expect("append");
    let identity = file.identity();
    drop(file);
    let path = root.join(previous_log_file(1));
    std::fs::remove_file(&path).expect("remove");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo");
    assert!(status.success());

    let started = Instant::now();
    let error = dir
        .remove_verified(&previous_log_file(1), identity)
        .expect_err("fifo swapped in");
    assert_eq!(error, FileSafetyError::UnsafeFile);
    let error = dir
        .rename_verified(&previous_log_file(1), &previous_log_file(2), identity)
        .expect_err("fifo swapped in");
    assert_eq!(error, FileSafetyError::UnsafeFile);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a FIFO swap must not block the operation"
    );
    assert!(
        std::fs::symlink_metadata(&path)
            .expect("stat")
            .file_type()
            .is_fifo(),
        "the refused FIFO is left in place"
    );
    assert!(!root.join(previous_log_file(2)).exists());
}

/// Renaming onto a symlink or FIFO target would silently replace an object we do not own;
/// only a private regular file (atomic settings replacement) or an absent target is
/// accepted.
#[test]
fn rename_onto_an_unsafe_target_is_refused() {
    let (temp, root) = private_root();
    let outside = sentinel(&temp);
    let dir = PrivateDir::open_existing(&root).expect("open root");
    let mut source = dir
        .create_new(&previous_log_file(1))
        .expect("create source");
    source.append(b"data\n").expect("append");
    let identity = source.identity();
    drop(source);
    symlink(&outside, root.join(previous_log_file(2))).expect("symlink target");
    let error = dir
        .rename_verified(&previous_log_file(1), &previous_log_file(2), identity)
        .expect_err("symlinked target refused");
    assert_eq!(error, FileSafetyError::UnsafeFile);
    assert_eq!(
        std::fs::read(&outside).expect("read sentinel"),
        b"external-sentinel-content"
    );
    std::fs::remove_file(root.join(previous_log_file(2))).expect("remove symlink");
    let fifo = root.join(previous_log_file(2));
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(status.success());
    let started = Instant::now();
    let error = dir
        .rename_verified(&previous_log_file(1), &previous_log_file(2), identity)
        .expect_err("fifo target refused");
    assert_eq!(error, FileSafetyError::UnsafeFile);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(root.join(previous_log_file(1)).exists());
}
