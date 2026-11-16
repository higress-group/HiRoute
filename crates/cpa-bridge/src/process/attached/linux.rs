use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{AtFlags, Mode, OFlags, open, openat, stat, statat};
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::path::Path;

pub(super) struct NativeProcess {
    proc_dir: OwnedFd,
    pidfd: Option<OwnedFd>,
}

impl NativeProcess {
    pub(super) fn open(pid: i32) -> io::Result<Option<Self>> {
        let pid = Pid::from_raw(pid).ok_or(io::ErrorKind::InvalidInput)?;
        let pidfd = match pidfd_open(pid, PidfdFlags::empty()) {
            Ok(fd) => Some(fd),
            Err(Errno::NOSYS | Errno::PERM | Errno::ACCESS) => None,
            Err(Errno::SRCH) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let proc_dir = match open_proc_dir(pid) {
            Ok(directory) => directory,
            Err(Errno::NOENT | Errno::SRCH) => {
                if let Some(fd) = &pidfd
                    && pidfd_is_live(fd)?
                {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let process = Self { proc_dir, pidfd };
        Ok(process.is_live()?.then_some(process))
    }

    pub(super) fn is_live(&self) -> io::Result<bool> {
        match &self.pidfd {
            Some(fd) => pidfd_is_live(fd),
            None => proc_dir_is_live(&self.proc_dir),
        }
    }

    pub(super) fn has_work_dir(&self, expected: &Path) -> io::Result<bool> {
        let expected = stat(expected)?;
        let current = match statat(&self.proc_dir, "cwd", AtFlags::empty()) {
            Ok(current) => current,
            Err(Errno::NOENT | Errno::SRCH) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        Ok(current.st_dev == expected.st_dev && current.st_ino == expected.st_ino)
    }

    pub(super) fn send_signal(&self, signal: Signal) -> io::Result<()> {
        // Linux 5.1 and later also accept the held procfs directory. Its inode
        // retains the original kernel PID identity even after numeric PID reuse.
        pidfd_send_signal(self.pidfd.as_ref().unwrap_or(&self.proc_dir), signal).map_err(Into::into)
    }
}

fn pidfd_is_live(fd: &OwnedFd) -> io::Result<bool> {
    let mut descriptors = [PollFd::new(fd, PollFlags::IN)];
    poll(&mut descriptors, Some(&Timespec::default()))
        .map(|count| count == 0)
        .map_err(Into::into)
}

fn open_proc_dir(pid: Pid) -> rustix::io::Result<OwnedFd> {
    open(
        format!("/proc/{}", pid.as_raw_nonzero()),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
}

fn proc_dir_is_live(directory: &OwnedFd) -> io::Result<bool> {
    let stat = match openat(
        directory,
        "stat",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT | Errno::SRCH) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut content = Vec::new();
    match File::from(stat).read_to_end(&mut content) {
        Ok(_) => content
            .rsplit(|byte| *byte == b')')
            .next()
            .and_then(|fields| fields.strip_prefix(b" "))
            .and_then(|fields| fields.first())
            .map(|state| !matches!(*state, b'Z' | b'X'))
            .ok_or_else(|| io::ErrorKind::InvalidData.into()),
        Err(error) if error.raw_os_error() == Some(Errno::SRCH.raw_os_error()) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::{Child, Command};

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn held_proc_directory_binds_legacy_signal_and_exit() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = OwnedChild(
            Command::new("/bin/sleep")
                .current_dir(directory.path())
                .arg("10")
                .spawn()
                .unwrap(),
        );
        let pid = Pid::from_raw(child.0.id() as i32).unwrap();
        let process = NativeProcess {
            proc_dir: open_proc_dir(pid).unwrap(),
            pidfd: None,
        };
        assert!(process.is_live().unwrap());
        assert!(process.has_work_dir(directory.path()).unwrap());
        assert!(!process.has_work_dir(Path::new("/")).unwrap());
        process.send_signal(Signal::TERM).unwrap();
        assert_eq!(
            child.0.wait().unwrap().signal(),
            Some(Signal::TERM.as_raw())
        );
        assert!(!process.is_live().unwrap());
        assert_eq!(
            process
                .send_signal(Signal::KILL)
                .unwrap_err()
                .raw_os_error(),
            Some(Errno::SRCH.raw_os_error())
        );
    }
}
