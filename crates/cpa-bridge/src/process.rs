use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::artifact::VerifiedCpaBinary;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod attached;

#[derive(Clone, Debug)]
pub struct CpaLaunch {
    pub binary: VerifiedCpaBinary,
    pub config_path: PathBuf,
    pub work_dir: PathBuf,
    pub(crate) management_password: Arc<crate::config::SecretText>,
    pub(crate) proxy_environment: crate::proxy_environment::ProxyEnvironment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpaExit {
    pub code: Option<i32>,
}

pub trait CpaProcessHandle: Send {
    fn pid(&self) -> u32;
    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError>;
    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError>;

    /// Checks whether an attached process still owns the private launch directory.
    fn has_work_dir(&self, _expected: &Path) -> Result<bool, CpaProcessError> {
        Ok(false)
    }
}

pub trait CpaProcessBackend: Send + Sync {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError>;

    /// Holds the OS identity of a recorded child after the former HiRoute owner has died.
    /// Callers authenticate the recorded child before any signal or routing.
    fn attach_authenticated(&self, pid: u32) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError>;

    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError>;
}

pub(crate) fn attach_recovered_process(
    backend: &dyn CpaProcessBackend,
    pid: u32,
) -> Result<Box<dyn CpaProcessHandle>, crate::CpaLifecycleError> {
    backend
        .attach_authenticated(pid)
        .map_err(|error| match error {
            CpaProcessError::UntrustedOrphan => crate::CpaLifecycleError::UntrustedOrphan,
            error => crate::CpaLifecycleError::Process(error),
        })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdCpaProcessBackend;

impl CpaProcessBackend for StdCpaProcessBackend {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        if !launch.config_path.is_absolute() || !launch.work_dir.is_absolute() {
            return Err(CpaProcessError::InvalidLaunch);
        }
        #[cfg(not(windows))]
        let child = Command::new(launch.binary.path())
            .arg("--config")
            .arg(&launch.config_path)
            // v7.2.140: use the embedded catalog and suppress remote model-catalog fetches.
            .arg("--local-model")
            .arg("--local-password-stdin")
            .env_clear()
            .envs(launch.proxy_environment.iter())
            .current_dir(&launch.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(CpaProcessError::Spawn)?;
        #[cfg(windows)]
        let child = spawn_windows(launch)?;
        let mut handle = ChildHandle { child };
        // The inherited pipe carries the instance secret once; closing it completes bootstrap.
        // ChildHandle kills and reaps the child if this write fails.
        let mut input = handle
            .child
            .stdin
            .take()
            .ok_or(CpaProcessError::InvalidLaunch)?;
        input
            .write_all(launch.management_password.expose().as_bytes())
            .map_err(CpaProcessError::Bootstrap)?;
        drop(input);
        Ok(Box::new(handle))
    }

    fn attach_authenticated(&self, pid: u32) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        #[cfg(windows)]
        {
            if !self.pid_is_running(pid)? {
                return Err(CpaProcessError::NotRunning);
            }
            // The first supported Windows daemon owns a no-breakaway resident job. Its
            // death kills every CPA descendant. A live numeric PID from an old probe
            // record supplies neither process nor tree termination authority.
            Err(CpaProcessError::UntrustedOrphan)
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            Ok(Box::new(attached::AttachedHandle::open(pid)?))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = pid;
            Err(CpaProcessError::UnsupportedPlatform)
        }
    }

    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
        platform_pid_is_running(pid)
    }
}

struct ChildHandle {
    #[cfg(not(windows))]
    child: Child,
    #[cfg(windows)]
    child: hiroute_platform::windows::job::Child,
}

impl Drop for ChildHandle {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        let _ = self.child.terminate();
    }
}

impl CpaProcessHandle for ChildHandle {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        #[cfg(not(windows))]
        {
            self.child
                .try_wait()
                .map(|status| {
                    status.map(|status| CpaExit {
                        code: status.code(),
                    })
                })
                .map_err(CpaProcessError::Inspect)
        }
        #[cfg(windows)]
        {
            let code = self.child.exit_code().map_err(CpaProcessError::Inspect)?;
            if code.is_some() && !self.child.empty().map_err(CpaProcessError::Inspect)? {
                // A root exit is not a completed cleanup while it still has descendants.
                self.child.terminate().map_err(CpaProcessError::Terminate)?;
                return Ok(None);
            }
            Ok(code.map(|code| CpaExit { code: Some(code) }))
        }
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        if let Some(exit) = self.try_exit()? {
            return Ok(exit);
        }
        #[cfg(not(windows))]
        self.child.kill().map_err(CpaProcessError::Terminate)?;
        #[cfg(windows)]
        self.child.terminate().map_err(CpaProcessError::Terminate)?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.try_exit()? {
                return Ok(exit);
            }
            if Instant::now() >= deadline {
                return Err(CpaProcessError::ShutdownTimeout);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(windows)]
fn spawn_windows(
    launch: &CpaLaunch,
) -> Result<hiroute_platform::windows::job::Child, CpaProcessError> {
    use hiroute_platform::windows::job::{Child, Launch, Stream};
    use std::ffi::OsString;

    // Only OS bootstrap and this runtime's captured proxy policy enter the child.
    // In particular, ambient credentials and shell hooks never cross env_clear().
    let system_root = std::env::var_os("SystemRoot")
        .filter(|value| PathBuf::from(value).is_absolute())
        .ok_or(CpaProcessError::InvalidLaunch)?;
    let mut environment: Vec<(OsString, OsString)> = launch
        .proxy_environment
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    environment.push(("SystemRoot".into(), system_root.clone()));
    environment.push(("WINDIR".into(), system_root));
    // Go's fallback temp directory is the Windows directory, which a standard user
    // cannot write. Keep temporary OAuth/material work inside this private runtime.
    let temporary = launch.work_dir.join("tmp");
    hiroute_platform::windows::security::ensure_private_directory(&temporary)
        .map_err(CpaProcessError::Spawn)?;
    hiroute_platform::windows::security::open_owner_only(&temporary, true)
        .map_err(CpaProcessError::Spawn)?;
    environment.push(("TEMP".into(), temporary.clone().into_os_string()));
    environment.push(("TMP".into(), temporary.into_os_string()));
    let args = [
        "--config".into(),
        launch.config_path.clone().into_os_string(),
        "--local-model".into(),
        "--local-password-stdin".into(),
    ];
    Child::spawn(Launch {
        program: launch.binary.path(),
        args: &args,
        env: &environment,
        cwd: Some(&launch.work_dir),
        stdin: Stream::Piped,
        stdout: Stream::Null,
        stderr: Stream::Null,
    })
    .map_err(CpaProcessError::Spawn)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn platform_pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    attached::pid_is_running(pid)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn platform_pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return Ok(false);
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) | Err(rustix::io::Errno::PERM) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => Err(CpaProcessError::Platform(error.into())),
    }
}

#[cfg(windows)]
fn platform_pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    if pid == 0 {
        return Ok(false);
    }
    hiroute_platform::windows::job::process_is_running(pid).map_err(CpaProcessError::Platform)
}

#[cfg(not(any(unix, windows)))]
fn platform_pid_is_running(_pid: u32) -> Result<bool, CpaProcessError> {
    Err(CpaProcessError::UnsupportedPlatform)
}

#[derive(Debug, Error)]
pub enum CpaProcessError {
    #[error("CPA launch paths are invalid")]
    InvalidLaunch,
    #[error("CPA process could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("CPA private bootstrap pipe failed: {0}")]
    Bootstrap(std::io::Error),
    #[error("CPA process state could not be inspected: {0}")]
    Inspect(std::io::Error),
    #[error("CPA process could not be terminated: {0}")]
    Terminate(std::io::Error),
    #[error("CPA platform process operation failed: {0}")]
    Platform(std::io::Error),
    #[error("CPA external process termination failed")]
    TerminateExternal,
    #[error("CPA process is not running")]
    NotRunning,
    #[error("CPA orphan lacks an authenticated Windows process-tree capability")]
    UntrustedOrphan,
    #[error("CPA process did not stop before the shutdown deadline")]
    ShutdownTimeout,
    #[error("CPA process platform is not supported by this build")]
    UnsupportedPlatform,
}

#[cfg(all(test, unix))]
#[path = "process_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "process_windows_tests.rs"]
mod windows_tests;
