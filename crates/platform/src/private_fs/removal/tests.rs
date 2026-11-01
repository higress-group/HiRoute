use super::*;

#[test]
fn read_only_retired_copy_can_be_removed() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("retired");
    fs::create_dir(&root).unwrap();
    let path = root.join("read-only-copy");
    fs::write(&path, b"copy").unwrap();
    let mut mode = fs::metadata(&path).unwrap().permissions();
    mode.set_readonly(true);
    fs::set_permissions(&path, mode).unwrap();
    let mut removal = DirectoryRemoval::open(&root, directory_identity(&root).unwrap()).unwrap();
    assert!(removal.step(128, Duration::from_secs(10)).unwrap().done);
}

#[test]
fn large_flat_tree_is_bounded_and_resumes_with_linear_progress() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("retired");
    fs::create_dir(&root).unwrap();
    for index in 0..10_001 {
        fs::write(root.join(index.to_string()), b"backup").unwrap();
    }
    let expected = directory_identity(&root).unwrap();
    let mut removal = DirectoryRemoval::open(&root, expected).unwrap();
    let first = removal.step(128, Duration::from_secs(10)).unwrap();
    assert_eq!(first.removed, 128);
    assert!(!first.done);
    drop(removal);
    let mut removal = DirectoryRemoval::open(&root, expected).unwrap();
    let mut removed = first.removed;
    loop {
        let progress = removal.step(128, Duration::from_secs(10)).unwrap();
        assert!((1..=128).contains(&progress.removed));
        removed += progress.removed;
        if progress.done {
            break;
        }
    }
    assert_eq!(removed, 10_002);
    assert!(!root.exists());
}

#[test]
fn replaced_root_and_descendant_are_never_adopted() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("retired");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("child")).unwrap();
    fs::write(root.join("child/old"), b"old").unwrap();
    let expected = directory_identity(&root).unwrap();
    let mut removal = DirectoryRemoval::open(&root, expected).unwrap();
    removal.step(1, Duration::from_secs(10)).unwrap();
    fs::rename(root.join("child"), root.join("moved")).unwrap();
    fs::create_dir(root.join("child")).unwrap();
    fs::write(root.join("child/keep"), b"keep").unwrap();
    assert!(removal.step(128, Duration::from_secs(10)).is_err());
    assert_eq!(fs::read(root.join("child/keep")).unwrap(), b"keep");
    drop(removal);
    let mut removal = DirectoryRemoval::open(&root, expected).unwrap();
    fs::rename(&root, fixture.path().join("moved")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("keep"), b"keep").unwrap();
    assert!(removal.step(128, Duration::from_secs(10)).is_err());
    assert!(DirectoryRemoval::open(&root, expected).is_err());
    assert_eq!(fs::read(root.join("keep")).unwrap(), b"keep");
}

#[test]
fn deep_or_unknown_entry_is_preserved_without_false_completion() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("retired");
    fs::create_dir(&root).unwrap();
    let mut path = root.clone();
    for _ in 0..MAX_DEPTH {
        path.push("child");
        fs::create_dir(&path).unwrap();
    }
    fs::write(path.join("keep"), b"keep").unwrap();
    let mut removal = DirectoryRemoval::open(&root, directory_identity(&root).unwrap()).unwrap();
    for _ in 0..2 {
        assert!(removal.step(128, Duration::from_secs(10)).is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"keep");
    }
}

#[cfg(unix)]
#[test]
fn links_are_unlinked_without_reading_the_target() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    let root = fixture.path().join("retired");
    fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    std::os::unix::fs::symlink(fixture.path().join("absent"), root.join("dangling")).unwrap();
    let mut removal = DirectoryRemoval::open(&root, directory_identity(&root).unwrap()).unwrap();
    assert!(removal.step(128, Duration::from_secs(10)).unwrap().done);
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}
