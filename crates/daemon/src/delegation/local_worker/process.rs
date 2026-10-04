use super::super::platform::{WorkerObservation as Observation, WorkerStopScope as Scope};
use std::io;
#[cfg(unix)]
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

/// Never expose a numeric process identifier as an operation capability.
pub(super) struct Process {
    child: Child,
    reaped: Option<Observation>,
    #[cfg(unix)]
    group: rustix::process::Pid,
    #[cfg(unix)]
    graceful_stop_started: Option<Instant>,
    stop_sent: bool,
}

impl Process {
    pub(super) fn configure(command: &mut Command) {
        command.kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
    }

    pub(super) fn new(child: Child) -> io::Result<Self> {
        #[cfg(unix)]
        let group = child
            .id()
            .and_then(|id| rustix::process::Pid::from_raw(id as i32))
            .filter(|pid| pid.as_raw_nonzero().get() > 1)
            .ok_or_else(|| io::Error::other("process identity unavailable"))?;
        Ok(Self {
            child,
            reaped: None,
            #[cfg(unix)]
            group,
            #[cfg(unix)]
            graceful_stop_started: None,
            stop_sent: false,
        })
    }

    pub(super) fn pipes(
        &mut self,
    ) -> Option<(tokio::process::ChildStdin, tokio::process::ChildStdout)> {
        Some((self.child.stdin.take()?, self.child.stdout.take()?))
    }

    pub(super) fn scope(&self) -> Scope {
        if cfg!(unix) {
            Scope::ProcessGroup
        } else {
            Scope::Root
        }
    }

    pub(super) fn observe(&mut self) -> Observation {
        if let Some(status) = self.reaped {
            return status;
        }
        #[cfg(unix)]
        {
            use rustix::process::{WaitId, WaitIdOptions, waitid};
            // WNOWAIT preserves the unreaped group leader, preventing PGID reuse.
            match waitid(
                WaitId::Pid(self.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            ) {
                Ok(None) => Observation::Running,
                Ok(Some(status)) => Observation::Exited {
                    code: status.exit_status(),
                },
                Err(_) => Observation::Unknown,
            }
        }
        #[cfg(not(unix))]
        {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    let observation = Observation::Exited {
                        code: status.code(),
                    };
                    self.reaped = Some(observation);
                    observation
                }
                Ok(None) => Observation::Running,
                Err(_) => Observation::Unknown,
            }
        }
    }

    pub(super) fn request_stop(&mut self) -> io::Result<()> {
        if self.stop_sent {
            return Ok(());
        }
        #[cfg(unix)]
        {
            // Native Harnesses own tools in separate process groups. A turn interrupt
            // may deliberately keep those tools alive; TERM lets the Harness release them.
            // Keep the leader unreaped throughout this window even if the adapter exits
            // first: a remaining Harness may still be draining its own child resources.
            if self.graceful_stop_started.is_none() {
                self.signal_group(rustix::process::Signal::TERM)?;
                self.graceful_stop_started = Some(Instant::now());
                return Ok(());
            }
            if self
                .graceful_stop_started
                .is_some_and(|started| started.elapsed() < Duration::from_secs(1))
            {
                return Ok(());
            }
            self.signal_group(rustix::process::Signal::KILL)?;
        }
        #[cfg(not(unix))]
        {
            if self.observe() == Observation::Running {
                self.child.start_kill()?;
            }
        }
        self.stop_sent = true;
        Ok(())
    }

    #[cfg(unix)]
    fn signal_group(&mut self, signal: rustix::process::Signal) -> io::Result<()> {
        // Validate the still-owned child before any group signal. After reaping, no
        // signal ever uses this number again, even if another process reused it.
        let observation = self.observe();
        if self.reaped.is_some() || observation == Observation::Unknown {
            return Err(io::Error::other("process ownership unavailable"));
        }
        match rustix::process::kill_process_group(self.group, signal) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
            Err(rustix::io::Errno::PERM) if matches!(observation, Observation::Exited { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        // Darwin returns EPERM when only an exited/unreaped leader remains. Only
        // for an observed exit may we proceed to reap and check group absence. A
        // still-present/inaccessible group remains unknown; EPERM is never success.
        Ok(())
    }

    pub(super) fn stopped(&mut self) -> bool {
        if !self.stop_sent {
            return false;
        }
        if self.reaped.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.reaped = Some(Observation::Exited {
                        code: status.code(),
                    })
                }
                _ => return false,
            }
        }
        #[cfg(unix)]
        {
            // Signal 0 is read-only. A reused PGID can only yield conservative false.
            matches!(
                rustix::process::test_kill_process_group(self.group),
                Err(rustix::io::Errno::SRCH)
            )
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Last resort only; never reported as successful cleanup.
        let _ = self.request_stop();
        #[cfg(unix)]
        if !self.stop_sent {
            let _ = self.signal_group(rustix::process::Signal::KILL);
        }
    }
}
