use super::{CpaExit, CpaProcessError, CpaProcessHandle};
use rustix::process::{Pid, Signal};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
use hiroute_platform::macos_process::ProcessIdentity as NativeProcess;
#[cfg(target_os = "linux")]
use linux::NativeProcess;

pub(super) fn pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    match AttachedHandle::open(pid) {
        Ok(handle) => handle.process.is_live().map_err(CpaProcessError::Inspect),
        Err(CpaProcessError::NotRunning) => Ok(false),
        // An inaccessible process may still own the record.
        Err(CpaProcessError::Inspect(error)) if error.kind() == io::ErrorKind::PermissionDenied => {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub(super) struct AttachedHandle {
    pid: Pid,
    process: NativeProcess,
}

impl AttachedHandle {
    pub(super) fn open(raw_pid: u32) -> Result<Self, CpaProcessError> {
        let pid = i32::try_from(raw_pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or(CpaProcessError::NotRunning)?;
        let process = NativeProcess::open(pid.as_raw_nonzero().get())
            .map_err(CpaProcessError::Inspect)?
            .ok_or(CpaProcessError::NotRunning)?;
        Ok(Self { pid, process })
    }

    fn signal(&self, signal: Signal) -> Result<(), CpaProcessError> {
        if !self.process.is_live().map_err(CpaProcessError::Inspect)? {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        let result = self.process.send_signal(signal);
        #[cfg(target_os = "macos")]
        let result = self.process.send_signal(signal.as_raw());
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) => {
                Ok(())
            }
            Err(error) => Err(CpaProcessError::Terminate(error)),
        }
    }
}

impl CpaProcessHandle for AttachedHandle {
    fn pid(&self) -> u32 {
        self.pid.as_raw_nonzero().get() as u32
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        Ok((!self.process.is_live().map_err(CpaProcessError::Inspect)?)
            .then_some(CpaExit { code: None }))
    }

    fn has_work_dir(&self, expected: &Path) -> Result<bool, CpaProcessError> {
        self.process
            .has_work_dir(expected)
            .map_err(CpaProcessError::Inspect)
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        let deadline = Instant::now() + timeout;
        let force_at = deadline - timeout.min(Duration::from_millis(250));
        self.signal(Signal::TERM)?;
        while Instant::now() < force_at && self.try_exit()?.is_none() {
            std::thread::sleep(
                force_at
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
        self.signal(Signal::KILL)?;
        loop {
            if let Some(exit) = self.try_exit()? {
                return Ok(exit);
            }
            if Instant::now() >= deadline {
                return Err(CpaProcessError::ShutdownTimeout);
            }
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
    }
}
