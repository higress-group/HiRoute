//! Bounded Unix inherited-fd reader.
//!
//! Opens `/dev/fd/N` without changing the inherited description's file-status
//! flags. macOS `/dev/fd/N` follows dup semantics, so `O_NONBLOCK` on that open
//! does not make a blocking pipe nonblocking. `poll` enforces the deadline.

use std::io::{ErrorKind, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::FileTypeExt;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnixFdReadBounds {
    pub min_fd: u32,
    pub max_bytes: usize,
    pub timeout: Duration,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UnixFdReadError;

pub(crate) fn read_unix_fd_bounded(
    fd: u32,
    bounds: UnixFdReadBounds,
) -> Result<Vec<u8>, UnixFdReadError> {
    if fd < bounds.min_fd {
        return Err(UnixFdReadError);
    }
    let mut file = std::fs::File::open(format!("/dev/fd/{fd}")).map_err(|_| UnixFdReadError)?;
    let file_type = file.metadata().map_err(|_| UnixFdReadError)?.file_type();
    if !file_type.is_file() && !file_type.is_fifo() {
        return Err(UnixFdReadError);
    }

    let deadline = Instant::now() + bounds.timeout;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let now = Instant::now();
        if now >= deadline || bytes.len() > bounds.max_bytes {
            return Err(UnixFdReadError);
        }
        let remaining = deadline.saturating_duration_since(now);
        let timeout = PollTimeout::try_from(remaining).map_err(|_| UnixFdReadError)?;
        let mut fds = [PollFd::new(file.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, timeout) {
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(UnixFdReadError),
            Ok(0) => return Err(UnixFdReadError),
            Ok(_) => {
                let want = bounds
                    .max_bytes
                    .saturating_add(1)
                    .saturating_sub(bytes.len())
                    .min(buf.len());
                if want == 0 {
                    return Err(UnixFdReadError);
                }
                match file.read(&mut buf[..want]) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes.extend_from_slice(&buf[..n]);
                        if bytes.len() > bounds.max_bytes {
                            return Err(UnixFdReadError);
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => continue,
                    Err(_) => return Err(UnixFdReadError),
                }
            }
        }
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod watchdog {
    use super::*;
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    use std::os::fd::{AsFd, AsRawFd};
    use std::process::{Command, Stdio};
    use std::{env, thread};

    pub(crate) const CHILD_FD: &str = "HIROUTE_UNIX_BOUNDED_FD";

    pub(crate) fn child_fd() -> Option<u32> {
        env::var(CHILD_FD).ok()?.parse().ok()
    }

    pub(crate) fn spawn(fd: &(impl AsFd + AsRawFd), kind: &str) -> std::process::Child {
        fcntl(fd, FcntlArg::F_SETFD(FdFlag::empty())).unwrap();
        Command::new(env::current_exe().unwrap())
            .arg(thread::current().name().expect("test thread name"))
            .arg("--exact")
            .arg("--nocapture")
            .env(CHILD_FD, fd.as_raw_fd().to_string())
            .env("HIROUTE_UNIX_BOUNDED_FD_KIND", kind)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    pub(crate) fn wait(mut child: std::process::Child, limit: Duration) {
        let started = Instant::now();
        let status = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status,
                None if started.elapsed() >= limit => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("watchdog: inherited fd reader exceeded {limit:?}");
                }
                None => thread::sleep(Duration::from_millis(20)),
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        assert!(status.success(), "watchdog child {status}; {stderr}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};
    use std::os::fd::AsRawFd;

    const SHORT: UnixFdReadBounds = UnixFdReadBounds {
        min_fd: 0,
        max_bytes: 512,
        timeout: Duration::from_millis(300),
    };

    #[test]
    fn accepts_regular_file_and_closed_pipe() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"hello").unwrap();
        file.rewind().unwrap();
        assert_eq!(
            read_unix_fd_bounded(file.as_raw_fd() as u32, SHORT).unwrap(),
            b"hello"
        );

        let (read, write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write, b"pipe").unwrap();
        drop(write);
        assert_eq!(
            read_unix_fd_bounded(read.as_raw_fd() as u32, SHORT).unwrap(),
            b"pipe"
        );
    }

    #[test]
    fn rejects_min_fd_device_and_oversize() {
        assert_eq!(
            read_unix_fd_bounded(
                0,
                UnixFdReadBounds {
                    min_fd: 3,
                    max_bytes: 512,
                    timeout: Duration::from_secs(2)
                }
            ),
            Err(UnixFdReadError)
        );
        let zero = std::fs::File::open("/dev/zero").unwrap();
        assert_eq!(
            read_unix_fd_bounded(zero.as_raw_fd() as u32, SHORT),
            Err(UnixFdReadError)
        );
        let mut oversize = tempfile::tempfile().unwrap();
        oversize.write_all(&[b'x'; 513]).unwrap();
        oversize.rewind().unwrap();
        assert_eq!(
            read_unix_fd_bounded(oversize.as_raw_fd() as u32, SHORT),
            Err(UnixFdReadError)
        );
    }

    fn timeout_under_watchdog(kind: &str, prefix: &[u8]) {
        if let Some(fd) = watchdog::child_fd() {
            std::process::exit(if read_unix_fd_bounded(fd, SHORT).is_err() {
                0
            } else {
                1
            });
        }
        let (read, write) = nix::unistd::pipe().unwrap();
        if !prefix.is_empty() {
            nix::unistd::write(&write, prefix).unwrap();
        }
        let child = watchdog::spawn(&read, kind);
        watchdog::wait(child, Duration::from_secs(2));
        drop(write);
    }

    #[test]
    fn empty_pipe_times_out_under_watchdog() {
        timeout_under_watchdog("empty", b"");
    }

    #[test]
    fn partial_pipe_without_eof_times_out_under_watchdog() {
        timeout_under_watchdog("partial", b"partial");
    }
}
