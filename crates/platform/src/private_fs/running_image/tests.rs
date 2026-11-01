use super::*;
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn await_file(path: &Path) -> bool {
    let started = Instant::now();
    while !path.exists() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    path.exists()
}

#[test]
fn replacement_path_cannot_prove_a_running_image() {
    const CHILD: &str = "HIROUTE_RUNNING_IMAGE_CHILD";
    const TEST: &str =
        "private_fs::running_image::tests::replacement_path_cannot_prove_a_running_image";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = std::path::PathBuf::from(root);
        assert!(
            open_running_executable().is_ok(),
            "original mapped image must be admitted"
        );
        fs::write(root.join("ready"), b"ready").unwrap();
        assert!(await_file(&root.join("replaced")));
        assert_eq!(std::env::current_exe().unwrap(), root.join("image"));
        assert!(
            open_running_executable().is_err(),
            "a new disk image is not the running process"
        );
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    // Canonicalize the parent so /var versus /private/var cannot affect the
    // child's original executable-path assertion.
    let root = fixture.path().canonicalize().unwrap();
    let executable = root.join("image");
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut child = Command::new(&executable)
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, &root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if !await_file(&root.join("ready")) {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "image probe did not start: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::rename(&executable, root.join("old-image")).unwrap();
    fs::write(&executable, b"future image that has never started").unwrap();
    fs::write(root.join("replaced"), b"replaced").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
