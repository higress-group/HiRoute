use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::artifact::VerifiedCpaBinary;

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
}

pub trait CpaProcessBackend: Send + Sync {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError>;

    /// Attaches to a previously authenticated orphaned CPA child after the former HiRoute owner
    /// has died. Callers must authenticate the recorded loopback endpoint before invoking this.
    fn attach_authenticated(&self, pid: u32) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError>;

    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdCpaProcessBackend;

impl CpaProcessBackend for StdCpaProcessBackend {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        if !launch.config_path.is_absolute() || !launch.work_dir.is_absolute() {
            return Err(CpaProcessError::InvalidLaunch);
        }
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
        if pid == 0 || !self.pid_is_running(pid)? {
            return Err(CpaProcessError::NotRunning);
        }
        Ok(Box::new(AttachedHandle { pid }))
    }

    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
        platform_pid_is_running(pid)
    }
}

struct ChildHandle {
    child: Child,
}

impl Drop for ChildHandle {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl CpaProcessHandle for ChildHandle {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        self.child
            .try_wait()
            .map(|status| {
                status.map(|status| CpaExit {
                    code: status.code(),
                })
            })
            .map_err(CpaProcessError::Inspect)
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        if let Some(exit) = self.try_exit()? {
            return Ok(exit);
        }
        self.child.kill().map_err(CpaProcessError::Terminate)?;
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

struct AttachedHandle {
    pid: u32,
}

impl CpaProcessHandle for AttachedHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        if platform_pid_is_running(self.pid)? {
            Ok(None)
        } else {
            Ok(Some(CpaExit { code: None }))
        }
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        platform_terminate_pid(self.pid, timeout)?;
        Ok(CpaExit { code: None })
    }
}

#[cfg(unix)]
fn platform_pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    if pid == 0 {
        return Ok(false);
    }
    let status = Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(CpaProcessError::Platform)?;
    Ok(status.success())
}

#[cfg(unix)]
fn platform_terminate_pid(pid: u32, timeout: Duration) -> Result<(), CpaProcessError> {
    let status = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(CpaProcessError::Platform)?;
    if !status.success() && platform_pid_is_running(pid)? {
        return Err(CpaProcessError::TerminateExternal);
    }
    let deadline = Instant::now() + timeout;
    while platform_pid_is_running(pid)? {
        if Instant::now() >= deadline {
            let forced = Command::new("/bin/kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(CpaProcessError::Platform)?;
            if !forced.success() && platform_pid_is_running(pid)? {
                return Err(CpaProcessError::ShutdownTimeout);
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(windows)]
fn platform_pid_is_running(pid: u32) -> Result<bool, CpaProcessError> {
    if pid == 0 {
        return Ok(false);
    }
    let filter = format!("PID eq {pid}");
    let output = Command::new("tasklist.exe")
        .args(["/FI", filter.as_str(), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .output()
        .map_err(CpaProcessError::Platform)?;
    if !output.status.success() {
        return Err(CpaProcessError::Platform(std::io::Error::other(
            "tasklist failed",
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .any(|line| line.contains(&format!("\"{pid}\""))))
}

#[cfg(windows)]
fn platform_terminate_pid(pid: u32, timeout: Duration) -> Result<(), CpaProcessError> {
    let pid_arg = pid.to_string();
    let status = Command::new("taskkill.exe")
        .args(["/PID", pid_arg.as_str(), "/T"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(CpaProcessError::Platform)?;
    if !status.success() && platform_pid_is_running(pid)? {
        return Err(CpaProcessError::TerminateExternal);
    }
    let deadline = Instant::now() + timeout;
    while platform_pid_is_running(pid)? {
        if Instant::now() >= deadline {
            let forced = Command::new("taskkill.exe")
                .args(["/PID", pid_arg.as_str(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(CpaProcessError::Platform)?;
            if !forced.success() && platform_pid_is_running(pid)? {
                return Err(CpaProcessError::ShutdownTimeout);
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn platform_pid_is_running(_pid: u32) -> Result<bool, CpaProcessError> {
    Err(CpaProcessError::UnsupportedPlatform)
}

#[cfg(not(any(unix, windows)))]
fn platform_terminate_pid(_pid: u32, _timeout: Duration) -> Result<(), CpaProcessError> {
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
    #[error("CPA process did not stop before the shutdown deadline")]
    ShutdownTimeout,
    #[error("CPA process platform is not supported by this build")]
    UnsupportedPlatform,
}

#[cfg(all(test, unix))]
#[path = "process_tests.rs"]
mod tests;
