//! Incremental removal of a caller-owned, identity-bound retired directory.
//! The caller supplies retirement authority; this module never selects paths.
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, ReadDir},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemovalProgress {
    pub removed: usize,
    pub done: bool,
}

struct Frame {
    held: File,
    identity: [u64; 2],
    name: OsString,
    path: PathBuf,
    entries: ReadDir,
}

/// Keeps directory iterators between batches, so large flat directories are
/// walked once rather than rescanned from their first entry on every tick.
pub struct DirectoryRemoval {
    parent: File,
    root_identity: [u64; 2],
    mount: u64,
    frames: Vec<Frame>,
}

fn unsafe_entry() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

pub fn directory_identity(path: &Path) -> io::Result<[u64; 2]> {
    native::identity(&native::open_directory(path)?)
}

impl DirectoryRemoval {
    pub fn open(path: &Path, expected: [u64; 2]) -> io::Result<Self> {
        let parent = native::open_directory(path.parent().ok_or_else(unsafe_entry)?)?;
        let name = path.file_name().ok_or_else(unsafe_entry)?.to_owned();
        let held = native::open_child(&parent, &name, true)?;
        if native::identity(&held)? != expected {
            return Err(unsafe_entry());
        }
        let mount = native::mount(&held)?;
        Ok(Self {
            parent,
            root_identity: expected,
            mount,
            frames: vec![Frame {
                identity: expected,
                held,
                name,
                path: path.to_owned(),
                entries: fs::read_dir(path)?,
            }],
        })
    }

    pub fn step(&mut self, limit: usize, budget: Duration) -> io::Result<RemovalProgress> {
        if limit == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        // A renamed/replaced root or descendant loses removal authority. Name
        // enumeration alone never chooses the object that is actually removed.
        for (index, frame) in self.frames.iter().enumerate() {
            let parent = if index == 0 {
                &self.parent
            } else {
                &self.frames[index - 1].held
            };
            if native::identity(&native::open_child(parent, &frame.name, true)?)? != frame.identity
            {
                return Err(unsafe_entry());
            }
        }
        let started = Instant::now();
        let mut removed = 0;
        while !self.frames.is_empty() {
            if removed == limit || (removed > 0 && started.elapsed() >= budget) {
                break;
            }
            let index = self.frames.len() - 1;
            let entry = self.frames[index].entries.next().transpose()?;
            if let Some(entry) = entry {
                let name = entry.file_name();
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    native::remove_link(&self.frames[index].held, &name)?;
                    removed += 1;
                    continue;
                }
                if !kind.is_dir() && !kind.is_file() {
                    return Err(unsafe_entry());
                }
                let held = native::open_child(&self.frames[index].held, &name, kind.is_dir())?;
                let identity = native::identity(&held)?;
                if identity[0] != self.root_identity[0] {
                    return Err(unsafe_entry());
                }
                if kind.is_dir() {
                    if self.frames.len() >= MAX_DEPTH || native::mount(&held)? != self.mount {
                        return Err(unsafe_entry());
                    }
                    let path = self.frames[index].path.join(&name);
                    self.frames.push(Frame {
                        held,
                        identity,
                        name,
                        entries: fs::read_dir(&path)?,
                        path,
                    });
                } else {
                    native::remove(&self.frames[index].held, &name, &held, false)?;
                    removed += 1;
                }
            } else {
                let frame = &self.frames[index];
                let parent = if index == 0 {
                    &self.parent
                } else {
                    &self.frames[index - 1].held
                };
                native::remove(parent, &frame.name, &frame.held, true)?;
                self.frames.pop();
                removed += 1;
            }
        }
        Ok(RemovalProgress {
            removed,
            done: self.frames.is_empty(),
        })
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    use nix::{
        fcntl::{OFlag, open, openat},
        sys::stat::Mode,
        unistd::{UnlinkatFlags, unlinkat},
    };
    #[cfg(target_os = "linux")]
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    pub(super) fn open_directory(path: &Path) -> io::Result<File> {
        let fd = open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        Ok(File::from(fd))
    }
    pub(super) fn open_child(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
        let mut flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
        if directory {
            flags |= OFlag::O_DIRECTORY;
        }
        Ok(File::from(openat(parent, name, flags, Mode::empty())?))
    }
    pub(super) fn identity(file: &File) -> io::Result<[u64; 2]> {
        let meta = file.metadata()?;
        Ok([meta.dev(), meta.ino()])
    }
    pub(super) fn mount(file: &File) -> io::Result<u64> {
        #[cfg(target_os = "linux")]
        {
            // st_dev cannot distinguish a same-filesystem bind mount. Check
            // directory handles only; content files are never read for cleanup.
            let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))?;
            info.lines()
                .find_map(|line| line.strip_prefix("mnt_id:")?.trim().parse().ok())
                .ok_or_else(unsafe_entry)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(file.metadata()?.dev())
        }
    }
    pub(super) fn remove(
        parent: &File,
        name: &OsStr,
        held: &File,
        directory: bool,
    ) -> io::Result<()> {
        let current = open_child(parent, name, directory)?;
        if identity(&current)? != identity(held)? {
            return Err(unsafe_entry());
        }
        unlinkat(
            parent,
            name,
            if directory {
                UnlinkatFlags::RemoveDir
            } else {
                UnlinkatFlags::NoRemoveDir
            },
        )?;
        Ok(())
    }
    pub(super) fn remove_link(parent: &File, name: &OsStr) -> io::Result<()> {
        unlinkat(parent, name, UnlinkatFlags::NoRemoveDir)?;
        Ok(())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::windows::security;
    pub(super) fn open_directory(path: &Path) -> io::Result<File> {
        security::open_write_through(path, true)
    }
    pub(super) fn open_child(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
        security::open_shared_delete_at(parent, name, directory)
    }
    pub(super) fn identity(file: &File) -> io::Result<[u64; 2]> {
        security::file_identity(file).map(|(volume, index)| [u64::from(volume), index])
    }
    pub(super) fn mount(file: &File) -> io::Result<u64> {
        identity(file).map(|id| id[0])
    }
    pub(super) fn remove(
        _parent: &File,
        _name: &OsStr,
        held: &File,
        _directory: bool,
    ) -> io::Result<()> {
        security::delete_owned_by_handle(held)
    }
    pub(super) fn remove_link(_parent: &File, _name: &OsStr) -> io::Result<()> {
        Err(unsafe_entry())
    }
}

#[cfg(test)]
mod tests;
