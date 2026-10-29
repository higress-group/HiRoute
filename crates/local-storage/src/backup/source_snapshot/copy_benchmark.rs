//! Explicit native timing probe; never reads or upgrades a user's storage.
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
#[ignore = "native macOS small-file snapshot timing; run explicitly with --nocapture"]
fn complete_small_file_snapshot_copy_timing() {
    const FILES: usize = 4096;
    const BYTES: usize = 512;
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let target = temporary.path().join("snapshot");
    prepare_owner_directory(&source).unwrap();
    for index in 0..FILES {
        let path = source.join(format!("body-{index:05}"));
        fs::write(&path, vec![(index % 251) as u8; BYTES]).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    hiroute_platform::durable::directory_barrier(&source).unwrap();
    let started = std::time::Instant::now();
    copy_tree(&source, &target).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(fs::read_dir(&target).unwrap().count(), FILES);
    for index in 0..FILES {
        let original = source.join(format!("body-{index:05}"));
        let copy = target.join(format!("body-{index:05}"));
        assert_eq!(fs::read(&copy).unwrap(), vec![(index % 251) as u8; BYTES]);
        assert_eq!(
            fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_ne!(
            fs::metadata(&original).unwrap().ino(),
            fs::metadata(&copy).unwrap().ino()
        );
    }
    fs::write(source.join("body-00000"), b"later source change").unwrap();
    assert_eq!(fs::read(target.join("body-00000")).unwrap(), vec![0; BYTES]);
    println!(
        "snapshot_copy_timing files={FILES} bytes_per_file={BYTES} elapsed_ms={}",
        elapsed.as_millis()
    );
}
