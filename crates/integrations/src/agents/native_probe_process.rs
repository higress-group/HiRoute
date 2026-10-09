//! Narrow ownership for a synchronous native check and its read-only CLI baseline.
//! Exit observation retains the group leader until all mutating group signals are finished.
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
};
use std::io::{self, Read};
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

pub struct NativeProbeProcess {
    child: Child,
    group: Pid,
    signals_closed: bool,
    stopped: bool,
}

impl NativeProbeProcess {
    /// The caller must spawn this child with process_group(0).
    pub fn new(child: Child) -> Self {
        Self {
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
        waitid(
            WaitId::Pid(self.group),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map(|value| value.map(|status| status.exit_status() == Some(0)))
        .map_err(Into::into)
    }

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

    pub fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        if self.signals_closed {
            return Err(io::Error::other("native process stop incomplete"));
        }
        self.signal(Signal::TERM)?;
        std::thread::sleep(Duration::from_millis(100));
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
}

impl Drop for NativeProbeProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
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

#[cfg(test)]
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
        child.stop().unwrap();
        assert!(child.observe().is_err());
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
        let _ = child.stop(); // An orphan zombie may conservatively leave scope unknown.
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
