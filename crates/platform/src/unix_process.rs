//! Native identity of a macOS process that may outlive its HiRoute parent.

use nix::libc;
use std::{io, mem::MaybeUninit};

/// Start time of a live process. Exited processes, including zombies, have no
/// live identity. Other inspection failures are returned to the owner.
pub fn process_start_time(pid: u32) -> io::Result<Option<(u64, u64)>> {
    let pid = i32::try_from(pid).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the buffer has the exact layout and length required by PROC_PIDTBSDINFO.
    let count = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if count != size {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: the kernel returned the complete BSD process record.
    let info = unsafe { info.assume_init() };
    if info.pbi_status == libc::SZOMB {
        return Ok(None);
    }
    Ok(Some((info.pbi_start_tvsec, info.pbi_start_tvusec)))
}
