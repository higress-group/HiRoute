use std::{
    io,
    process::Output,
    time::{Duration, Instant},
};

pub fn output_with_file_busy_retry(
    mut output: impl FnMut() -> io::Result<Output>,
) -> io::Result<Output> {
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match output() {
            // Execution has not started. A concurrently forked fixture can briefly retain
            // the freshly materialized script's writer; never retry a child result.
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn held_writer_matches_original_failure_then_runs_launcher_only_once() {
        use std::os::unix::fs::PermissionsExt;

        let root = super::super::private_tempdir().unwrap();
        let script = root.path().join("native-launcher");
        std::fs::write(
            &script,
            b"#!/bin/sh\nprintf 'launched\\n' >> \"$0.started\"\nprintf '%s' \"$1\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut writer = Some(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&script)
                .unwrap(),
        );
        let literal = "a path with spaces; $(not-a-command)";
        let mut attempts = 0;
        let mut original_error = None;
        let output = output_with_file_busy_retry(|| {
            attempts += 1;
            let result = std::process::Command::new(&script).arg(literal).output();
            if attempts == 1 {
                original_error = result.as_ref().err().and_then(io::Error::raw_os_error);
                // Release only after reproducing the original direct Command failure.
                // No timed writer thread or successful isolated rerun establishes this proof.
                drop(writer.take());
            }
            result
        })
        .unwrap();
        assert_eq!(original_error, Some(nix::errno::Errno::ETXTBSY as i32));
        assert!(attempts >= 2);
        assert!(output.status.success());
        assert_eq!(output.stdout, literal.as_bytes());
        assert_eq!(
            std::fs::read(root.path().join("native-launcher.started")).unwrap(),
            b"launched\n"
        );
    }

    #[test]
    fn non_file_busy_errors_and_child_failures_are_never_retried() {
        let mut attempts = 0;
        let error = output_with_file_busy_retry(|| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts, 1);

        let mut attempts = 0;
        let output = output_with_file_busy_retry(|| {
            attempts += 1;
            std::process::Command::new("/bin/sh")
                .args(["-c", "printf 'child failure' >&2; exit 7"])
                .output()
        })
        .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stderr, b"child failure");
        assert_eq!(attempts, 1);
    }

    #[test]
    fn persistent_file_busy_error_is_bounded_and_returned() {
        let started = Instant::now();
        let mut attempts = 0;
        let error = output_with_file_busy_retry(|| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::ExecutableFileBusy))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy);
        assert!(attempts >= 2);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
