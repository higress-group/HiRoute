use super::*;
use std::{
    fs,
    io::{Read, Write},
};

#[test]
fn private_materials_are_exclusive_and_read_without_permission_adoption() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("private");
    create_directory(&root).unwrap();
    assert!(create_directory(&root).is_err());
    let path = root.join("key");
    let mut file = create_file(&path, FileMode::Data).unwrap();
    file.write_all(b"original").unwrap();
    drop(file);
    assert!(create_file(&path, FileMode::Data).is_err());
    let mut file = open_private_file(&path).unwrap();
    verify_single_link(&file).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "original");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    #[cfg(windows)]
    crate::windows::security::verify_owner_only_handle(&file, false).unwrap();
}

#[test]
fn owned_cleanup_is_idempotent_and_preserves_neighbor_materials() {
    let fixture = tempfile::tempdir().unwrap();
    let neighbor = fixture.path().join("native-history");
    fs::write(&neighbor, b"keep").unwrap();
    let path = fixture.path().join("run");
    let mut owned = OwnedDirectory::create(&path).unwrap();
    assert!(OwnedDirectory::create(&path).is_err());
    create_directory(&path.join("nested")).unwrap();
    write_material(&path.join("nested/data"), b"private", FileMode::Data).unwrap();
    owned.remove().unwrap();
    owned.remove().unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read(neighbor).unwrap(), b"keep");
}

#[test]
fn storage_preparation_reopens_without_adopting_a_file() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("nested/storage");
    prepare_storage_directory(&path).unwrap();
    prepare_storage_directory(&path).unwrap();
    let file = path.join("foreign");
    write_material(&file, b"preserve", FileMode::Data).unwrap();
    assert!(matches!(
        prepare_storage_directory(&file),
        Err(DirectoryError::NotDirectory)
    ));
    #[cfg(windows)]
    {
        let unnormalized = path.join("..").join("storage").join("foreign");
        assert!(matches!(
            prepare_storage_directory(&unnormalized),
            Err(DirectoryError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
        ));
    }
    assert_eq!(fs::read(file).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn unix_storage_preserves_existing_mode_and_materials_keep_executable_intent() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("existing");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
    prepare_storage_directory(&root).unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o750
    );
    let executable = root.join("launcher");
    write_material(&executable, b"#!/bin/sh\nexit 0\n", FileMode::Executable).unwrap();
    assert_eq!(
        fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn protected_reads_reject_links_and_special_files_without_touching_targets() {
    use std::os::unix::fs::symlink;
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    write_material(&source, b"keep", FileMode::Data).unwrap();
    let link = fixture.path().join("link");
    symlink(&source, &link).unwrap();
    assert!(open_regular_file(&link).is_err());
    assert!(open_private_file(&link).is_err());
    assert!(open_regular_file(fixture.path()).is_err());
    let fifo = fixture.path().join("fifo");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRUSR).unwrap();
    assert!(open_regular_file(&fifo).is_err());
    let alias = fixture.path().join("alias");
    fs::hard_link(&source, &alias).unwrap();
    assert!(verify_single_link(&open_private_file(&source).unwrap()).is_err());
    assert_eq!(fs::read(source).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn owned_cleanup_rejects_replacement_and_never_follows_child_links() {
    use std::os::unix::fs::symlink;
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("run");
    let original = fixture.path().join("original");
    let mut owned = OwnedDirectory::create(&path).unwrap();
    fs::rename(&path, &original).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("foreign"), b"keep").unwrap();
    assert!(owned.remove().is_err());
    assert_eq!(fs::read(path.join("foreign")).unwrap(), b"keep");
    fs::remove_dir_all(&path).unwrap();
    fs::rename(&original, &path).unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("history"), b"keep").unwrap();
    symlink(&outside, path.join("link")).unwrap();
    owned.remove().unwrap();
    assert_eq!(fs::read(outside.join("history")).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn windows_owned_root_stays_pinned_and_private_read_does_not_repair_borrowed_acl() {
    use crate::{file_security, windows::security};
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("run");
    let mut owned = OwnedDirectory::create(&path).unwrap();
    assert!(fs::rename(&path, fixture.path().join("moved")).is_err());
    let borrowed = fixture.path().join("borrowed");
    let mut permissions = file_security::private_defaults(false).unwrap();
    // Use the concrete file-read mask: native creation maps generic GR to FR.
    permissions.descriptor.push_str("(A;;FR;;;WD)");
    crate::durable::write_windows_atomic(&borrowed, b"preserve", &permissions).unwrap();
    let file = security::open_identity(&borrowed, false).unwrap();
    let before = file_security::capture(&file).unwrap();
    assert!(open_private_file(&borrowed).is_err());
    assert_eq!(file_security::capture(&file).unwrap(), before);
    owned.remove().unwrap();
}
