use std::os::unix::fs::symlink;
use std::sync::Barrier;

use super::*;

#[test]
fn recovery_rejects_symlink_replacements_at_the_root_or_its_parent() {
    let outside = TestRoot::new("recovery-outside");
    fs::create_dir(&outside.0).unwrap();
    fs::create_dir(outside.0.join("replay")).unwrap();
    let marker = outside.0.join("keep");
    fs::write(&marker, b"outside replay").unwrap();

    for replace_parent in [false, true] {
        let parent = TestRoot::new("recovery-parent");
        let root = parent.0.join("replay");
        let manager = ReplayManager::open(config(&root, 64, 16)).unwrap();
        fs::remove_dir(&root).unwrap();
        let replaced = if replace_parent {
            fs::remove_dir(&parent.0).unwrap();
            &parent.0
        } else {
            &root
        };
        symlink(&outside.0, replaced).unwrap();
        assert!(manager.begin_request(budget()).is_err());
        assert!(fs::symlink_metadata(replaced).unwrap().is_symlink());
        assert_eq!(fs::read(&marker).unwrap(), b"outside replay");

        fs::remove_file(replaced).unwrap();
        if replace_parent {
            fs::create_dir(&parent.0).unwrap();
        }
        let store = manager.begin_request(budget()).unwrap();
        assert!(store.request_directory().is_dir());
        drop(store);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
}

#[test]
fn concurrent_requests_reuse_the_recovered_root_and_remove_only_expired_orphans() {
    let root = TestRoot::new("recovery-concurrent");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).unwrap();
    fs::remove_dir(&root.0).unwrap();
    fs::create_dir(&root.0).unwrap();
    let expired = root.0.join("req-expired");
    let fresh = root.0.join("req-fresh");
    let unrelated = root.0.join("unrelated");
    for entry in [&expired, &fresh, &unrelated] {
        fs::create_dir(entry).unwrap();
        fs::write(entry.join("marker"), b"retained or expired").unwrap();
    }
    let old = SystemTime::now() - Duration::from_secs(120);
    for entry in [&expired, &unrelated] {
        fs::File::open(entry).unwrap().set_modified(old).unwrap();
    }

    let barrier = Barrier::new(2);
    let [first, second] = std::thread::scope(|scope| {
        let begin = || {
            barrier.wait();
            manager.begin_request(budget()).unwrap()
        };
        let first = scope.spawn(begin);
        let second = scope.spawn(begin);
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_ne!(first.request_directory(), second.request_directory());
    assert!(first.request_directory().is_dir() && second.request_directory().is_dir());
    assert!(!expired.exists());
    assert!(fresh.join("marker").is_file());
    assert!(unrelated.join("marker").is_file());

    drop(first);
    drop(second);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
}
