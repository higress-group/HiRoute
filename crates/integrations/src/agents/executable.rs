//! Read-only, bounded probes of explicitly selected or PATH-resolved installations.
//! No configuration helper or caller-supplied arguments are executed. Raw output stays private.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;

pub(super) struct ExecutableObservationV1 {
    pub version: String,
    pub canonical_path: String,
}

pub(super) enum ExecutableProbe {
    Installed(ExecutableObservationV1),
    NotFound,
    Unknown(ProbeFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProbeFailure {
    NotExecutable,
    Unavailable,
    LaunchFailed(Option<i32>),
    PipeSetupFailed(i32),
    PipeReadFailed(Option<i32>),
    WaitFailed(Option<i32>),
    Failed,
    TimedOut,
    OutputLimit,
}

pub(super) fn executable_probe(path: &Path) -> ExecutableProbe {
    probe(path, Duration::from_secs(2))
}

fn probe(path: &Path, timeout: Duration) -> ExecutableProbe {
    let canonical = match resolve(path) {
        Ok(Some(path)) => path,
        Ok(None) => return ExecutableProbe::NotFound,
        Err(reason) => return ExecutableProbe::Unknown(reason),
    };
    let Some(canonical_path) = canonical.to_str().map(str::to_owned) else {
        return ExecutableProbe::Unknown(ProbeFailure::Unavailable);
    };
    let version = match bounded_version(&canonical, timeout) {
        Ok(bytes) => parse_version(&bytes).unwrap_or_default(),
        Err(ProbeFailure::LaunchFailed(code)) => {
            return ExecutableProbe::Unknown(ProbeFailure::LaunchFailed(code));
        }
        Err(ProbeFailure::NotExecutable) => {
            return ExecutableProbe::Unknown(ProbeFailure::NotExecutable);
        }
        // The process started: version diagnostics cannot establish capability or deny admission.
        Err(_) => String::new(),
    };
    ExecutableProbe::Installed(ExecutableObservationV1 {
        version,
        canonical_path,
    })
}

pub(super) fn resolve(path: &Path) -> Result<Option<PathBuf>, ProbeFailure> {
    let candidates = if path.is_absolute() || path.components().count() > 1 {
        vec![path.to_owned()]
    } else {
        let mut directories = std::env::var_os("PATH")
            .map(|search| std::env::split_paths(&search).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME") {
            directories.push(PathBuf::from(home).join(".local/bin"));
        }
        directories.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);
        #[cfg(target_os = "macos")]
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        directories.into_iter().map(|dir| dir.join(path)).collect()
    };
    for candidate in candidates {
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                let canonical =
                    std::fs::canonicalize(&candidate).map_err(|_| ProbeFailure::Unavailable)?;
                check_runnable(&canonical)?;
                return Ok(Some(canonical));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(ProbeFailure::Unavailable),
        }
    }
    Ok(None)
}

#[cfg(unix)]
fn check_runnable(path: &Path) -> Result<(), ProbeFailure> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).map_err(|_| ProbeFailure::Unavailable)?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(ProbeFailure::NotExecutable);
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_runnable(path: &Path) -> Result<(), ProbeFailure> {
    let metadata = std::fs::metadata(path).map_err(|_| ProbeFailure::Unavailable)?;
    if !metadata.is_file() {
        return Err(ProbeFailure::NotExecutable);
    }
    Ok(())
}

#[cfg(unix)]
fn bounded_version(path: &Path, timeout: Duration) -> Result<Vec<u8>, ProbeFailure> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn nonblocking(fd: impl AsFd) -> Result<(), ProbeFailure> {
        let flags = fcntl(&fd, FcntlArg::F_GETFL)
            .map_err(|error| ProbeFailure::PipeSetupFailed(error as i32))?;
        fcntl(
            &fd,
            FcntlArg::F_SETFL(OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|error| ProbeFailure::PipeSetupFailed(error as i32))?;
        Ok(())
    }
    fn drain(
        reader: &mut impl Read,
        target: &mut Vec<u8>,
        total: &mut usize,
    ) -> Result<(), ProbeFailure> {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    *total += count;
                    if *total > OUTPUT_LIMIT {
                        return Err(ProbeFailure::OutputLimit);
                    }
                    target.extend_from_slice(&chunk[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ProbeFailure::PipeReadFailed(error.raw_os_error())),
            }
        }
    }
    let mut command = Command::new(path);
    command
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(search) = std::env::var_os("PATH") {
        command.env("PATH", search);
    }
    let started = Instant::now();
    // ETXTBSY means exec did not start a child program. A concurrent installer or a fork
    // briefly retaining a CLOEXEC writer can cause this. Retry only this read-only probe,
    // inside both its original deadline and a small launch budget; never retry an Agent task.
    let launch_budget = timeout.min(Duration::from_millis(100));
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error)
                if error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32)
                    && started.elapsed() < launch_budget =>
            {
                std::thread::sleep(Duration::from_millis(5));
                check_runnable(path)?;
            }
            Err(error) => return Err(ProbeFailure::LaunchFailed(error.raw_os_error())),
        }
    };
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or(ProbeFailure::Unavailable)?;
        let mut stderr = child.stderr.take().ok_or(ProbeFailure::Unavailable)?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let (mut out, mut err, mut total) = (Vec::new(), Vec::new(), 0);
        loop {
            drain(&mut stdout, &mut out, &mut total)?;
            drain(&mut stderr, &mut err, &mut total)?;
            if let Some(status) = child
                .try_wait()
                .map_err(|error| ProbeFailure::WaitFailed(error.raw_os_error()))?
            {
                drain(&mut stdout, &mut out, &mut total)?;
                drain(&mut stderr, &mut err, &mut total)?;
                return if status.success() {
                    Ok(if out.is_empty() { err } else { out })
                } else {
                    Err(ProbeFailure::Failed)
                };
            }
            if started.elapsed() >= timeout {
                return Err(ProbeFailure::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    // Reap the probe, including descendants retaining pipe handles, on every outcome.
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(not(unix))]
fn bounded_version(_path: &Path, _timeout: Duration) -> Result<Vec<u8>, ProbeFailure> {
    Err(ProbeFailure::Unavailable)
}

fn parse_version(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()?
        .split_whitespace()
        .find(|word| {
            if word.len() > 64
                || !word
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b))
            {
                return false;
            }
            let numeric = word.split(['-', '+']).next().unwrap_or("");
            let pieces = numeric.split('.').collect::<Vec<_>>();
            pieces.len() == 3
                && pieces
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;
