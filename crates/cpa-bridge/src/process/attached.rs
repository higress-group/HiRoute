use super::{CpaExit, CpaProcessError, CpaProcessHandle};
use rustix::process::{Pid, Signal, kill_process};
use std::time::{Duration, Instant};

pub(super) fn pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    match AttachedHandle::open(pid) {
        Ok(handle) => handle.process.is_live(handle.pid),
        Err(CpaProcessError::NotRunning) => Ok(false),
        // An inaccessible process may still own the record.
        Err(CpaProcessError::Inspect(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub(super) struct AttachedHandle {
    pid: Pid,
    process: ProcessIdentity,
}

impl AttachedHandle {
    pub(super) fn open(raw_pid: u32) -> Result<Self, CpaProcessError> {
        let pid = i32::try_from(raw_pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or(CpaProcessError::NotRunning)?;
        Ok(Self {
            pid,
            process: ProcessIdentity::open(pid)?,
        })
    }
}

impl CpaProcessHandle for AttachedHandle {
    fn pid(&self) -> u32 {
        self.pid.as_raw_nonzero().get() as u32
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        Ok((!self.process.is_live(self.pid)?).then_some(CpaExit { code: None }))
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        let deadline = Instant::now() + timeout;
        let force_at = deadline - timeout.min(Duration::from_millis(250));
        self.process.signal(self.pid, Signal::TERM)?;
        while Instant::now() < force_at && self.process.is_live(self.pid)? {
            std::thread::sleep(
                force_at
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
        self.process.signal(self.pid, Signal::KILL)?;
        while self.process.is_live(self.pid)? {
            if Instant::now() >= deadline {
                return Err(CpaProcessError::ShutdownTimeout);
            }
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
        Ok(CpaExit { code: None })
    }
}

enum ProcessIdentity {
    #[cfg(target_os = "linux")]
    PidFd(std::os::fd::OwnedFd),
    StartTime((u64, u64)),
}

impl ProcessIdentity {
    fn open(pid: Pid) -> Result<Self, CpaProcessError> {
        #[cfg(target_os = "linux")]
        match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
            Ok(fd) => return Ok(Self::PidFd(fd)),
            Err(rustix::io::Errno::NOSYS | rustix::io::Errno::PERM) => {}
            Err(rustix::io::Errno::SRCH) => return Err(CpaProcessError::NotRunning),
            Err(error) => return Err(CpaProcessError::Inspect(error.into())),
        }
        process_start_time(pid)?
            .map(Self::StartTime)
            .ok_or(CpaProcessError::NotRunning)
    }

    fn is_live(&self, pid: Pid) -> Result<bool, CpaProcessError> {
        match self {
            #[cfg(target_os = "linux")]
            Self::PidFd(fd) => {
                use rustix::event::{PollFd, PollFlags, Timespec, poll};
                let mut descriptors = [PollFd::new(fd, PollFlags::IN)];
                match poll(&mut descriptors, Some(&Timespec::default())) {
                    Ok(count) => Ok(count == 0),
                    Err(error) => Err(CpaProcessError::Inspect(error.into())),
                }
            }
            Self::StartTime(start) => Ok(process_start_time(pid)?.as_ref() == Some(start)),
        }
    }

    fn signal(&self, pid: Pid, signal: Signal) -> Result<(), CpaProcessError> {
        if !self.is_live(pid)? {
            return Ok(());
        }
        let result = match self {
            #[cfg(target_os = "linux")]
            Self::PidFd(fd) => rustix::process::pidfd_send_signal(fd, signal),
            Self::StartTime(_) => kill_process(pid, signal),
        };
        match result {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(CpaProcessError::Terminate(error.into())),
        }
    }
}

#[cfg(target_os = "macos")]
fn process_start_time(pid: Pid) -> Result<Option<(u64, u64)>, CpaProcessError> {
    hiroute_platform::unix_process::process_start_time(pid.as_raw_nonzero().get() as u32)
        .map_err(CpaProcessError::Inspect)
}

#[cfg(target_os = "linux")]
fn process_start_time(pid: Pid) -> Result<Option<(u64, u64)>, CpaProcessError> {
    let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CpaProcessError::Inspect(error)),
    };
    let parse = || {
        let mut fields = stat.rsplit_once(')')?.1.split_ascii_whitespace();
        let state = fields.next()?;
        let start = fields.nth(18)?.parse().ok()?;
        Some((state != "Z" && state != "X").then_some((start, 0)))
    };
    parse().ok_or_else(|| CpaProcessError::Inspect(std::io::ErrorKind::InvalidData.into()))
}
