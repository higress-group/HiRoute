//! Admission for borrowing Claude settings requires the native host-managed provider flag.
//! Older CLIs silently ignore that flag, so ACP success cannot prove routing ownership.
//! This metadata check is only for a selected borrowed Worker launch, never discovery/probing.
use hiroute_domain::delegation::{
    DelegationErrorV1, NativeDependencyCheckV1, NativeDependencyFailureReasonV1 as Reason,
    NativeDependencyFailureV1,
};
use std::path::Path;
use std::time::Duration;

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(unix)]
const OUTPUT_LIMIT: usize = 4096;

pub(super) fn require_host_managed_provider(
    binary: &Path,
    search_path: &str,
) -> Result<(), DelegationErrorV1> {
    require_version(binary, search_path, VERSION_TIMEOUT)
}

fn require_version(
    binary: &Path,
    search_path: &str,
    timeout: Duration,
) -> Result<(), DelegationErrorV1> {
    let output = bounded_version(binary, search_path, timeout).map_err(|error| match error {
        DelegationErrorV1::DependencyCheckFailed(_) => error,
        _ => failure(Reason::Unavailable),
    })?;
    if supports_host_managed_provider(&output) {
        Ok(())
    } else {
        Err(failure(Reason::Unsupported))
    }
}

fn failure(reason: Reason) -> DelegationErrorV1 {
    DelegationErrorV1::DependencyCheckFailed(NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::ClaudeVersion,
        reason,
    })
}

fn supports_host_managed_provider(output: &[u8]) -> bool {
    let Some(version) = std::str::from_utf8(output)
        .ok()
        .and_then(|text| text.trim().strip_suffix(" (Claude Code)"))
    else {
        return false;
    };
    let version = version.strip_prefix("claude ").unwrap_or(version);
    let mut parts = version.split('.');
    let mut number = || {
        let part = parts.next()?;
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        part.parse::<u32>().ok()
    };
    let Some((major, minor, patch)) = number()
        .zip(number())
        .zip(number())
        .map(|((major, minor), patch)| (major, minor, patch))
    else {
        return false;
    };
    parts.next().is_none() && major == 2 && (minor, patch) >= (1, 231)
}

#[cfg(unix)]
fn bounded_version(
    binary: &Path,
    search_path: &str,
    timeout: Duration,
) -> Result<Vec<u8>, DelegationErrorV1> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    if !binary.is_absolute() || search_path.is_empty() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let probe_home = tempfile::Builder::new()
        .prefix("hiroute-claude-version-")
        .tempdir()
        .map_err(|_| failure(Reason::Unavailable))?;
    let home =
        std::fs::canonicalize(probe_home.path()).map_err(|_| failure(Reason::Unavailable))?;
    for relative in [".codex", ".claude", ".config", ".cache", ".local/share"] {
        std::fs::create_dir_all(home.join(relative)).map_err(|_| failure(Reason::Unavailable))?;
    }
    let started = Instant::now();
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .env_clear()
        .env("PATH", search_path)
        .env("HOME", &home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let child = spawn_version_command(|| command.spawn(), started, timeout)?;
    let mut child = VersionChild {
        child,
        signals_closed: false,
    };
    let result = (|| {
        let mut stdout = child
            .child
            .stdout
            .take()
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        let flags = fcntl(&stdout, FcntlArg::F_GETFL)
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        fcntl(
            &stdout,
            FcntlArg::F_SETFL(OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        let mut output = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            if started.elapsed() >= timeout {
                return Err(failure(Reason::Timeout));
            }
            // Observe exit without reaping: the leader must reserve its PID/PGID
            // until cleanup has stopped the group, including surviving descendants.
            let status = observe_version_child(&child)?;
            loop {
                if started.elapsed() >= timeout {
                    return Err(failure(Reason::Timeout));
                }
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output.len() + count > OUTPUT_LIMIT {
                            return Err(failure(Reason::InvalidOutput));
                        }
                        output.extend_from_slice(&buffer[..count]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(DelegationErrorV1::CapabilityUnavailable),
                }
            }
            if let Some(success) = status {
                return if success {
                    Ok(output)
                } else {
                    Err(failure(Reason::ProcessFailed))
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if stop_version_child(&mut child).is_err() {
        // Do not remove a live/unknown descendant's working directory.
        let _ = probe_home.keep();
        return Err(failure(Reason::CleanupFailed));
    }
    result
}

#[cfg(unix)]
fn spawn_version_command(
    mut launch: impl FnMut() -> std::io::Result<std::process::Child>,
    started: std::time::Instant,
    timeout: Duration,
) -> Result<std::process::Child, DelegationErrorV1> {
    // ETXTBSY means exec did not begin: an installer or concurrent fork may still
    // hold a writer for the selected executable. Match the native discovery probe's
    // bounded launch retry, without retrying a running CLI or an Agent task.
    let retry_budget = timeout.min(Duration::from_millis(100));
    loop {
        if started.elapsed() >= timeout {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        match launch() {
            Ok(child) => return Ok(child),
            Err(error) if error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32) => {
                let remaining = retry_budget.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(DelegationErrorV1::CapabilityUnavailable);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
                if started.elapsed() >= retry_budget {
                    return Err(DelegationErrorV1::CapabilityUnavailable);
                }
            }
            Err(_) => return Err(DelegationErrorV1::CapabilityUnavailable),
        }
    }
}

#[cfg(unix)]
struct VersionChild {
    child: std::process::Child,
    signals_closed: bool,
}

#[cfg(unix)]
fn observe_version_child(child: &VersionChild) -> Result<Option<bool>, DelegationErrorV1> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

    if child.signals_closed {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    waitid(
        WaitId::Pid(Pid::from_child(&child.child)),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    )
    .map(|status| status.map(|status| status.exit_status() == Some(0)))
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)
}

#[cfg(unix)]
fn stop_version_child(child: &mut VersionChild) -> Result<(), DelegationErrorV1> {
    use rustix::io::Errno;
    use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

    // Validate ownership immediately before signalling. WNOWAIT keeps even an exited
    // leader reserved, so the group number cannot belong to an unrelated new process.
    // An already reaped/unknown child fails here, without any numeric PID operation.
    let exited = observe_version_child(child)?.is_some();
    let group = Pid::from_child(&child.child);
    let group_stop = kill_process_group(group, Signal::KILL);
    if !exited && child.child.kill().is_err() && observe_version_child(child)?.is_none() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    child.signals_closed = true;
    child
        .child
        .wait()
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    // No mutating PID operation is allowed after wait. Darwin can report EPERM for
    // a group containing only its exited leader; verify absence read-only after reap.
    match group_stop {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(Errno::PERM) if matches!(test_kill_process_group(group), Err(Errno::SRCH)) => Ok(()),
        Err(_) => Err(DelegationErrorV1::CapabilityUnavailable),
    }
}

#[cfg(not(unix))]
fn bounded_version(
    _binary: &Path,
    _search_path: &str,
    _timeout: Duration,
) -> Result<Vec<u8>, DelegationErrorV1> {
    Err(DelegationErrorV1::CapabilityUnavailable)
}

#[cfg(test)]
#[path = "claude_runtime_tests.rs"]
mod tests;
