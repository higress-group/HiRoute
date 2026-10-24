use super::*;
use std::fs::{self, FileTimes, Permissions};
use std::os::unix::fs::PermissionsExt;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private-executable-name");
    fs::write(&path, b"private-original-body\n").unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
    (root, path)
}

#[test]
fn executable_diagnostics_distinguish_ctime_from_content_permissions_and_identity() {
    let (_root, path) = fixture();
    let original = fs::metadata(&path).unwrap();
    let before = snapshot(&path);
    assert_eq!(before["state"], "stable");
    assert_eq!(before["content"]["state"], "complete");
    // Deterministic ctime-only input, not a claim about the unobserved original writer.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let after = loop {
        fs::set_permissions(&path, original.permissions()).unwrap();
        let after = snapshot(&path);
        if before["stat"]["ctime"] != after["stat"]["ctime"] {
            break after;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ctime did not advance"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(after["state"], "stable");
    assert_eq!(before["content"], after["content"]);
    assert_eq!(before["extended"], after["extended"]);
    for field in ["dev", "ino", "uid", "gid", "mode", "nlink", "len", "mtime"] {
        assert_eq!(before["stat"][field], after["stat"][field], "{field}");
    }

    fs::write(&path, b"private-modified-body\n").unwrap();
    File::open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(original.modified().unwrap()))
        .unwrap();
    let changed = snapshot(&path);
    assert_eq!(before["stat"]["len"], changed["stat"]["len"]);
    assert_eq!(before["stat"]["mtime"], changed["stat"]["mtime"]);
    assert_ne!(before["content"], changed["content"]);
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    assert_ne!(before["stat"]["mode"], snapshot(&path)["stat"]["mode"]);

    // Hold the old inode through replacement, avoiding filesystem inode reuse.
    let _old = File::open(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"private-original-body\n").unwrap();
    let replaced = snapshot(&path);
    assert_eq!(before["content"], replaced["content"]);
    assert_ne!(before["stat"]["ino"], replaced["stat"]["ino"]);

    let mut trace = super::super::Trace::default();
    trace.record("native_executable_content", &before);
    trace.record("native_executable_content", &changed);
    assert_eq!(trace.report()["rows"][0]["sampling"]["snapshot"], "stable");
    assert_eq!(trace.report()["rows"][0]["sampling"]["content"], "complete");
    let output = trace.report().to_string();
    assert!(!output.contains("private-"));
    assert!(!output.contains(path.to_str().unwrap()));
    assert!(output.contains("sha256:"));
}

#[test]
fn executable_diagnostics_report_limits_links_and_in_sample_mutation() {
    let (root, path) = fixture();
    let changed = snapshot_with(&path, || {
        fs::write(&path, b"changed while reading").unwrap()
    });
    assert_eq!(changed["state"], "changed");
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert_eq!(snapshot(&alias)["state"], "unavailable");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(CONTENT_LIMIT + 1)
        .unwrap();
    let over = snapshot(&path);
    assert_eq!(over["state"], "stable");
    assert_eq!(over["content"]["state"], "limit");
    assert!(over["content"].get("digest").is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn executable_diagnostics_include_native_acl_flags_and_unknown_xattrs() {
    let (_root, path) = fixture();
    let before = snapshot(&path);
    assert_eq!(before["extended"]["acl_flags"]["state"], "complete");
    assert_eq!(before["extended"]["xattrs"]["state"], "complete");
    let file = File::open(&path).unwrap();
    let value = b"private-attribute-value";
    rustix::fs::fsetxattr(
        &file,
        c"user.hiroute-test",
        value,
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    let attr = snapshot(&path);
    assert_ne!(before["extended"]["xattrs"], attr["extended"]["xattrs"]);
    assert_eq!(before["content"], attr["content"]);
    // This changes a real native ACL on this owned fixture, not chmod's POSIX mode.
    let output = std::process::Command::new("/bin/chmod")
        .args(["+a", "everyone deny execute"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture ACL change failed");
    let acl = snapshot(&path);
    assert_eq!(attr["stat"]["mode"], acl["stat"]["mode"]);
    assert_ne!(attr["extended"]["acl_flags"], acl["extended"]["acl_flags"]);
    assert_eq!(attr["content"], acl["content"]);
    let mut trace = super::super::Trace::default();
    trace.record("native_executable_content", &attr);
    trace.record("native_executable_content", &acl);
    let output = trace.report().to_string();
    assert!(!output.contains("private-"));
    assert!(!output.contains("user.hiroute-test"));
    assert!(!output.contains("everyone"));
}
