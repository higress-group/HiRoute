//! Native identity of a macOS process that may outlive its HiRoute parent.

use nix::libc;
use std::mem::MaybeUninit;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::{fs, io};

// XNU's sys/proc_info_private.h describes this fixed proc_pidinfo record.
const PROC_PIDT_BSDINFOWITHUNIQID: i32 = 18;

#[repr(C)]
struct ProcessInfo {
    bsd: libc::proc_bsdinfo,
    executable_uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: u32,
    original_parent_pid_version: u32,
    reserved: [u64; 2],
}

const _: () = assert!(size_of::<ProcessInfo>() == 192);

unsafe extern "C" {
    // Available in libSystem since before HiRoute's macOS 15 minimum.
    fn proc_signal_with_audittoken(audit_token: *mut u32, signal: i32) -> i32;
}

/// An identity accepted by the kernel only while the same PID generation is live.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ProcessIdentity {
    pid: i32,
    pid_version: u32,
}

impl ProcessIdentity {
    pub fn open(pid: i32) -> io::Result<Option<Self>> {
        if pid <= 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(
            process_info::<ProcessInfo>(pid, PROC_PIDT_BSDINFOWITHUNIQID)?
                .filter(|info| info.bsd.pbi_status != libc::SZOMB)
                .map(|info| Self {
                    pid,
                    pid_version: info.pid_version,
                }),
        )
    }

    pub fn is_live(self) -> io::Result<bool> {
        Ok(Self::open(self.pid)? == Some(self))
    }

    pub fn has_work_dir(self, expected: &Path) -> io::Result<bool> {
        let metadata = fs::metadata(expected)?;
        let Some(info) =
            process_info::<libc::proc_vnodepathinfo>(self.pid, libc::PROC_PIDVNODEPATHINFO)?
        else {
            return Ok(false);
        };
        let current = info.pvi_cdir.vip_vi.vi_stat;
        Ok(current.vst_dev == metadata.dev() as u32
            && current.vst_ino == metadata.ino()
            && self.is_live()?)
    }

    pub fn send_signal(self, signal: i32) -> io::Result<()> {
        // XNU checks the audit token's PID and generation together when resolving
        // the target. Zeroed fields provide no delegated identity or privileges.
        let mut token = [0u32; 8];
        token[5] = self.pid as u32;
        token[7] = self.pid_version;
        // SAFETY: The audit token is eight native u32 words. The call borrows it
        // for the syscall and enforces ordinary signal permissions.
        match unsafe { proc_signal_with_audittoken(token.as_mut_ptr(), signal) } {
            0 => Ok(()),
            error => Err(io::Error::from_raw_os_error(error)),
        }
    }
}

fn process_info<T>(pid: i32, flavor: i32) -> io::Result<Option<T>> {
    let mut info = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as i32;
    // SAFETY: each caller selects the native flavor that initializes exactly T.
    let count = unsafe { libc::proc_pidinfo(pid, flavor, 0, info.as_mut_ptr().cast(), size) };
    if count == size {
        // SAFETY: the kernel reported writing the complete native record.
        return Ok(Some(unsafe { info.assume_init() }));
    }
    if count == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        return Err(error);
    }
    Err(io::ErrorKind::InvalidData.into())
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
    fn signal_requires_the_live_pid_generation() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = OwnedChild(
            Command::new("/bin/sleep")
                .current_dir(dir.path())
                .arg("10")
                .spawn()
                .unwrap(),
        );
        let process = ProcessIdentity::open(child.0.id() as i32).unwrap().unwrap();
        assert!(process.has_work_dir(dir.path()).unwrap());
        let different_dir = tempfile::tempdir().unwrap();
        assert!(!process.has_work_dir(different_dir.path()).unwrap());
        let stale = ProcessIdentity {
            pid_version: process.pid_version.wrapping_add(1),
            ..process
        };
        assert_eq!(
            stale.send_signal(libc::SIGTERM).unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(child.0.try_wait().unwrap().is_none());
        process.send_signal(libc::SIGTERM).unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGTERM));
        assert!(!process.is_live().unwrap());
    }
}
