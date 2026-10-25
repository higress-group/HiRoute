//! Narrow ownership for a synchronous native check and its read-only CLI baseline.
//! Exit observation retains the group leader until all mutating group signals are finished.
#[cfg(windows)]
use hiroute_platform::windows::job::Child;
#[cfg(unix)]
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
#[cfg(unix)]
use std::process::Child;
use std::time::{Duration, Instant};

use hiroute_domain::delegation::NativeDependencyFailureReasonV1 as Reason;

pub struct NativeProbeProcess {
    child: Child,
    #[cfg(unix)]
    group: Pid,
    signals_closed: bool,
    stopped: bool,
}

impl NativeProbeProcess {
    /// Unix callers spawn with process_group(0); Windows callers retain an atomic Job child.
    pub fn new(child: Child) -> Self {
        Self {
            #[cfg(unix)]
            group: Pid::from_child(&child),
            child,
            signals_closed: false,
            stopped: false,
        }
    }

    pub fn observe(&self) -> io::Result<Option<bool>> {
        if self.signals_closed {
            return Err(io::Error::other("native process ownership closed"));
        }
        #[cfg(windows)]
        return self
            .child
            .exit_code()
            .map(|code| code.map(|code| code == 0));
        #[cfg(unix)]
        waitid(
            WaitId::Pid(self.group),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map(|value| value.map(|status| status.exit_status() == Some(0)))
        .map_err(Into::into)
    }

    #[cfg(unix)]
    fn signal(&self, signal: Signal) -> io::Result<()> {
        self.observe()?;
        match kill_process_group(self.group, signal) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            // Darwin may refuse a group that exits between observe and signal.
            // This is provisional: stop still requires reap and proven group absence.
            Err(rustix::io::Errno::PERM) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(unix)]
    pub fn stop(&mut self) -> io::Result<()> {
        self.stop_with_grace_wait(std::thread::sleep)
    }

    #[cfg(unix)]
    fn stop_with_grace_wait(&mut self, wait: impl FnOnce(Duration)) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        if self.signals_closed {
            return Err(io::Error::other("native process stop incomplete"));
        }
        let running = self.observe()?.is_none();
        self.signal(Signal::TERM)?;
        // A completed check has no remaining work to drain. Keep its waitable
        // leader as the ownership anchor, kill any surviving group members,
        // and still prove reap/group absence below. A live check keeps its grace.
        if running {
            wait(Duration::from_millis(100));
        }
        // The leader is still waitable even when it exited before a surviving tool.
        self.signal(Signal::KILL)?;
        self.signals_closed = true;
        let start = Instant::now();
        loop {
            if self.child.try_wait()?.is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(1) {
                return Err(io::Error::other("native process reap deadline"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Never mutate this numeric PID/PGID again. A reused group yields conservative unknown.
        loop {
            if matches!(
                test_kill_process_group(self.group),
                Err(rustix::io::Errno::SRCH)
            ) {
                self.stopped = true;
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(1) {
                return Err(io::Error::other("native process scope remains"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(windows)]
    pub fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.child.terminate()?;
        let start = Instant::now();
        while !self.child.empty()? || self.child.exit_code()?.is_none() {
            if start.elapsed() >= Duration::from_secs(1) {
                return Err(io::Error::other("native process scope remains"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.signals_closed = true;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for NativeProbeProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Shared bounded one-shot output capture; the caller selects the command/environment.
#[cfg(unix)]
pub(super) fn capture(
    mut command: std::process::Command,
    _inherit: bool,
    timeout: std::time::Duration,
    output_limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>, Reason> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let capture = tempfile::NamedTempFile::new().map_err(|_| Reason::Unavailable)?;
    let child = command
        .stdin(Stdio::null())
        .stdout(capture.reopen().map_err(|_| Reason::Unavailable)?)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| Reason::Unavailable)?;
    let mut child = super::NativeProbeProcess::new(child);
    let result = (|| {
        loop {
            if start.elapsed() >= timeout {
                return Err(Reason::Timeout);
            }
            match child.observe().map_err(|_| Reason::Unavailable)? {
                Some(true) => break,
                Some(false) => return Err(Reason::ProcessFailed),
                None => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        super::native_probe_process::read_bounded(capture.path(), output_limit)
            .map_err(|_| Reason::InvalidOutput)
    })();
    // A failed check also stops and accounts for its owned scope before returning.
    child.stop().map_err(|_| Reason::CleanupFailed)?;
    result
}

#[cfg(windows)]
pub(super) fn capture(
    command: std::process::Command,
    inherit: bool,
    timeout: std::time::Duration,
    output_limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>, Reason> {
    super::windows_runtime::bounded(&command, inherit, timeout, output_limit).map_err(|error| {
        if super::windows_runtime::cleanup_unknown(&error) {
            Reason::CleanupFailed
        } else if super::windows_runtime::process_exit_code(&error).is_some() {
            Reason::ProcessFailed
        } else {
            match error.kind() {
                std::io::ErrorKind::TimedOut => Reason::Timeout,
                std::io::ErrorKind::InvalidData => Reason::InvalidOutput,
                _ => Reason::Unavailable,
            }
        }
    })
}

/// Rechecks the limit during the final read, including children that write and exit between polls.
pub(super) fn read_bounded(path: &Path, limit: usize) -> io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("native output bound"));
    }
    Ok(bytes)
}

/// Read an owned capture only after every writer in its process scope has stopped.
/// The duplicate retains the original file object, including exclusive Windows
/// sharing. Its seek pointer is shared, so this must never inspect a live writer.
pub(super) fn read_bounded_file(
    file: &File,
    limit: usize,
) -> io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let cap = u64::try_from(limit)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "native output limit"))?;
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(cap).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("native output bound"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod output_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn retained_capture_starts_at_zero_and_rechecks_the_exact_final_bound() {
        let root = super::super::probe_files::temporary("hiroute-held-output-").unwrap();
        let mut capture = super::super::probe_files::file(&root.path().join("capture")).unwrap();
        assert!(read_bounded_file(&capture, 0).unwrap().is_empty());
        capture.write_all(b"fixed capture").unwrap();
        capture.flush().unwrap();
        let length = b"fixed capture".len();
        assert_eq!(
            read_bounded_file(&capture, length).unwrap().as_slice(),
            b"fixed capture"
        );
        assert_eq!(
            read_bounded_file(&capture, length - 1).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            read_bounded_file(&capture, 0).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            read_bounded_file(&capture, length).unwrap().as_slice(),
            b"fixed capture"
        );
        if usize::BITS == 64 {
            assert_eq!(
                read_bounded_file(&capture, usize::MAX).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn retained_capture_preserves_actual_non_limit_read_errors() {
        let root = super::super::probe_files::temporary("hiroute-write-only-output-").unwrap();
        let path = root.path().join("write-only");
        let mut capture = File::options()
            .create_new(true)
            .write(true)
            .open(path)
            .unwrap();
        capture.write_all(b"fixed").unwrap();
        capture.flush().unwrap();
        let mut direct = capture.try_clone().unwrap();
        direct.seek(SeekFrom::Start(0)).unwrap();
        let expected = direct.read(&mut [0; 1]).unwrap_err();
        let error = read_bounded_file(&capture, 128).unwrap_err();
        assert_eq!(error.kind(), expected.kind());
        assert_eq!(error.raw_os_error(), expected.raw_os_error());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn await_exit(child: &NativeProbeProcess) {
        let start = Instant::now();
        loop {
            if child.observe().unwrap().is_some() {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn successful_probe_exit_remains_waitable_until_group_stop() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .unwrap();
        let mut child = NativeProbeProcess::new(child);
        await_exit(&child);
        assert_eq!(child.observe().unwrap(), Some(true));
        // A second WNOWAIT observation proves the first poll did not reap the leader.
        assert_eq!(child.observe().unwrap(), Some(true));
        child
            .stop_with_grace_wait(|_| panic!("completed probe must not wait for a live leader"))
            .unwrap();
        assert!(child.observe().is_err());
    }

    #[test]
    fn running_probe_keeps_termination_grace_and_reaps_owned_group() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 90"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut child = NativeProbeProcess::new(child);
        assert_eq!(child.observe().unwrap(), None);
        let mut waited = false;
        child
            .stop_with_grace_wait(|duration| {
                assert_eq!(duration, Duration::from_millis(100));
                waited = true;
                std::thread::sleep(duration);
            })
            .unwrap();
        assert!(waited);
        assert!(child.stopped);
        child.stop().unwrap();
    }

    #[test]
    fn leader_exit_does_not_abandon_term_ignoring_tool() {
        let root = tempfile::tempdir().unwrap();
        let pidfile = root.path().join("tool.pid");
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                "trap '' TERM; sleep 90 & echo $! > \"$1\"; exit 0",
                "probe",
            ])
            .arg(&pidfile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut child = NativeProbeProcess::new(child);
        await_exit(&child);
        let tool = std::fs::read_to_string(pidfile).unwrap();
        // An orphan zombie may conservatively leave scope unknown, but no live
        // owned tool may survive and no grace belongs to the exited leader.
        let _ = child.stop_with_grace_wait(|_| panic!("exited leader has no graceful work"));
        let state = Command::new("/bin/ps")
            .args(["-p", tool.trim(), "-o", "stat="])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&state.stdout);
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "owned tool remains active: {state}"
        );
    }

    #[test]
    fn fast_exit_output_cannot_bypass_final_read_bound() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("out");
        std::fs::write(&path, vec![b'x'; 1025]).unwrap();
        assert!(read_bounded(&path, 1024).is_err());
        assert_eq!(read_bounded(&path, 1025).unwrap().len(), 1025);
    }
}
