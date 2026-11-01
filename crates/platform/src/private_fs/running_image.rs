//! Bind a macOS executable handle to the process's actual mapped Mach-O vnode.
use nix::libc;
use std::{fs::File, io, mem::MaybeUninit, os::unix::fs::MetadataExt};

// sys/proc_info.h: proc_regioninfo. The four protection fields, file offset
// and fourteen u32 region-detail fields precede the region address and size.
#[repr(C)]
struct RegionInfo {
    _protection: [u32; 4],
    _offset: u64,
    _details: [u32; 14],
    address: u64,
    size: u64,
}
const _: () = assert!(std::mem::size_of::<RegionInfo>() == 96);

#[repr(C)]
struct RegionWithPath {
    region: RegionInfo,
    vnode: libc::vnode_info_path,
}

/// current_exe() names the original path, which a same-version hot install may
/// have replaced. Only return a held file if the kernel's mapped-image identity
/// matches it; callers must preserve future recovery material on refusal.
pub fn open_running_executable() -> io::Result<File> {
    let file = super::open_regular_file(&std::env::current_exe()?)?;
    // SAFETY: image index zero is the process's main image; no dereference is
    // needed. The address is used only as an input to the native region query.
    let header = unsafe { libc::_dyld_get_image_header(0) };
    if header.is_null() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let address = header as usize as u64;
    let mut region = MaybeUninit::<RegionWithPath>::zeroed();
    let length = i32::try_from(std::mem::size_of::<RegionWithPath>())
        .map_err(|_| io::ErrorKind::InvalidData)?;
    // SAFETY: this fixed repr(C) buffer matches proc_regionwithpathinfo in the
    // SDK. The kernel receives its exact writable size for the current process.
    let count = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            8,
            address,
            region.as_mut_ptr().cast(),
            length,
        )
    };
    if count != length {
        return Err(io::Error::other("running image identity unavailable"));
    }
    // SAFETY: the query returned the complete initialized buffer.
    let region = unsafe { region.assume_init() };
    if address < region.region.address
        || address >= region.region.address.saturating_add(region.region.size)
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mapped = region.vnode.vip_vi.vi_stat;
    let actual = file.metadata()?;
    if actual.dev() as u32 != mapped.vst_dev || actual.ino() != mapped.vst_ino {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}

#[cfg(test)]
mod tests;
