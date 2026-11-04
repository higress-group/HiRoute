//! Same-user Task action lifetime and bounded runtime recovery.
//!
//! Scheduler RestartOnFailure covers action-launch failures, but an action that
//! starts and later exits can still produce TaskCompleted, even for an abnormal
//! exit status. Own the business child rather than relying on that distinction.
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use hiroute_host_runtime::{StandaloneLayout, read_standalone_install_record};
use hiroute_platform::windows::{background, job, security, user_path};

const RETRIES: usize = 3;
const RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// Foreground and SCM dispatch retain their existing owners. Only a verified
/// installed Task image becomes the supervisor; the persisted definition and
/// registered action arguments keep their existing contract.
pub(super) fn run_if_installed_task() -> Result<bool, String> {
    let layout = StandaloneLayout::from_environment().map_err(|_| "Task layout unavailable")?;
    let record = read_standalone_install_record(&layout.marker_path)
        .map_err(|_| "Task installation unavailable")?;
    if hiroute_host_runtime::standalone_maintenance_pending(&layout)
        .map_err(|_| "Task maintenance state unavailable")?
    {
        return Err("Task installation maintenance pending".into());
    }
    match std::fs::symlink_metadata(&record.service_definition) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Task definition unavailable".into()),
        Ok(_) => {}
    }
    let definition = background::read_definition(&record.service_definition)
        .map_err(|_| "Task definition invalid")?;
    if definition.manager != background::Manager::Task {
        return Ok(false);
    }
    definition
        .verify_image()
        .map_err(|_| "Task image verification failed")?;
    same_image(&definition.daemon)?;
    job::initialize_resident().map_err(|_| "Task process-tree lifetime unavailable")?;
    let arguments: Vec<OsString> = ["--role", "all", "--standalone", "--windows-task-runtime"]
        .into_iter()
        .map(OsString::from)
        .collect();
    for attempt in 0..=RETRIES {
        // Installation maintenance must stop this action before replacing the
        // image. Never recover by launching changed bytes at a retained path.
        definition
            .verify_image()
            .map_err(|_| "Task recovery image verification failed")?;
        let environment = task_environment(std::env::vars_os().collect());
        let launched = job::Child::spawn(job::Launch {
            program: &definition.daemon,
            args: &arguments,
            env: &environment,
            cwd: definition.daemon.parent(),
            stdin: job::Stream::Null,
            stdout: job::Stream::Null,
            stderr: job::Stream::Null,
        });
        if let Ok(child) = launched {
            let code = loop {
                if let Some(code) = child.exit_code().map_err(|_| "Task child wait failed")? {
                    break code;
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            while !child.empty().map_err(|_| "Task child Job query failed")?
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            let empty = child.empty().map_err(|_| "Task child Job query failed")?;
            if !empty {
                child.terminate().map_err(|_| "Task child cleanup failed")?;
                let deadline = Instant::now() + Duration::from_secs(5);
                while !child
                    .empty()
                    .map_err(|_| "Task child cleanup query failed")?
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if !child
                    .empty()
                    .map_err(|_| "Task child cleanup query failed")?
                {
                    return Err("Task recovery refused a live child tree".into());
                }
            }
            if code == 0 && empty {
                return Ok(true);
            }
        }
        if attempt == RETRIES {
            return Err("Task runtime recovery exhausted".into());
        }
        // Public stop can terminate the exact Scheduler action during backoff.
        // Its resident Job then closes every owned child; no detached waiter or
        // timer can start another process after that stop.
        std::thread::sleep(RETRY_INTERVAL);
    }
    unreachable!()
}

fn task_environment(mut inherited: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    match user_path::current_environment_path() {
        Ok(path) => {
            inherited.retain(|(key, _)| !key.eq_ignore_ascii_case("PATH"));
            if let Some(path) = path {
                inherited.push(("PATH".into(), path));
            }
        }
        Err(error) => {
            eprintln!(
                "HiRoute Task could not refresh PATH (OS error {:?})",
                error.raw_os_error()
            );
        }
    }
    inherited
}

fn same_image(expected: &Path) -> Result<(), String> {
    let actual = std::env::current_exe().map_err(|_| "Task current image unavailable")?;
    let actual = security::open_identity(&actual, false)
        .map_err(|_| "Task current image identity unavailable")?;
    let expected = security::open_identity(expected, false)
        .map_err(|_| "Task installed image identity unavailable")?;
    if security::file_identity(&actual).map_err(|_| "Task current FileID unavailable")?
        != security::file_identity(&expected).map_err(|_| "Task installed FileID unavailable")?
    {
        return Err("Task current and installed images differ".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_child_uses_registered_path_instead_of_inherited_path() {
        let inherited = vec![
            ("Path".into(), "C:\\HiRoute-test-stale-parent".into()),
            ("HIROUTE_TEST_UNRELATED".into(), "keep".into()),
        ];
        let registered = user_path::current_environment_path()
            .expect("query registered environment")
            .expect("registered Windows PATH");
        let child = task_environment(inherited);
        let paths: Vec<_> = child
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value)
            .collect();
        assert_eq!(paths, [&registered]);
        assert!(child.contains(&("HIROUTE_TEST_UNRELATED".into(), "keep".into())));
    }
}
