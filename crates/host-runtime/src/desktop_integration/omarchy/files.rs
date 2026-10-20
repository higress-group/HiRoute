use super::{Context, types::*};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn no_aliases(path: &Path) -> Result<(), Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Error::InvalidRecord);
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() => return Err(Error::Conflict(ancestor.into())),
            Ok(m) => {
                if m.is_dir()
                    && (m.uid() != 0 && m.uid() != nix::unistd::geteuid().as_raw()
                        || m.mode() & 0o022 != 0 && !(m.uid() == 0 && m.mode() & 0o1000 != 0))
                {
                    return Err(Error::Conflict(ancestor.into()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub(super) fn directory(path: &Path) -> Result<(), Error> {
    no_aliases(path)?;
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        directory(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(Error::Conflict(path.into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(path)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
pub(super) fn read(
    path: &Path,
    mode: u32,
    limit: usize,
) -> Result<Option<(Vec<u8>, fs::Metadata)>, Error> {
    no_aliases(path)?;
    let mut f = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = f.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o7777 != mode
        || (mode == 0o600 && metadata.nlink() != 1)
    {
        return Err(Error::Conflict(path.into()));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::InvalidRecord);
    }
    Ok(Some((bytes, metadata)))
}
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    directory(path.parent().ok_or(Error::InvalidRecord)?)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::InvalidRecord);
    }
    let _ = read(path, 0o600, 16 * 1024 * 1024)?;
    hiroute_platform::durable::write_private_atomic(path, bytes)?;
    Ok(())
}
pub(super) fn stage(path: &Path, bytes: &[u8]) -> Result<(PathBuf, u64, u64), Error> {
    let parent = path.parent().ok_or(Error::InvalidRecord)?;
    directory(parent)?;
    let suffix = nonce()?;
    let stage = parent.join(format!(".hiroute-omarchy-{suffix}"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&stage)?;
    let written = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&stage);
        return Err(error.into());
    }
    let m = file.metadata()?;
    Ok((stage, m.dev(), m.ino()))
}
pub(super) fn nonce() -> Result<String, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| std::io::Error::other("integration entropy"))?;
    Ok(bytes.iter().map(|n| format!("{n:02x}")).collect())
}
pub(super) fn same_written(path: &Path, change: &Change) -> Result<bool, Error> {
    match read(path, 0o644, 512 * 1024) {
        Ok(Some((bytes, m))) => Ok(m.dev() == change.device
            && m.ino() == change.inode
            && hash(&bytes) == change.after.sha256),
        Ok(None) => Ok(false),
        Err(Error::Conflict(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
pub(super) fn remove_stage(change: &Change) -> Result<(), Error> {
    if !change.stage.exists() {
        return Ok(());
    }
    if !same_written(&change.stage, change)? {
        return Err(Error::RecoveryRequired);
    }
    fs::remove_file(&change.stage)?;
    Ok(())
}
pub(super) fn matches(path: &Path, record: &OwnedItem) -> Result<bool, Error> {
    match read(path, 0o644, 512 * 1024) {
        Ok(Some((bytes, _))) => Ok(hash(&bytes) == record.sha256),
        Ok(None) | Err(Error::Conflict(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
pub(super) fn apply(change: &Change) -> Result<(), Error> {
    no_aliases(&change.after.path)?;
    if !same_written(&change.stage, change)? {
        return Err(Error::RecoveryRequired);
    }
    match &change.before {
        None => fs::hard_link(&change.stage, &change.after.path)?,
        Some(before) => {
            if !matches(&change.after.path, before)? {
                return Err(Error::Conflict(change.after.path.clone()));
            }
            fs::rename(&change.stage, &change.after.path)?;
        }
    }
    hiroute_platform::durable::directory_barrier(
        change.after.path.parent().ok_or(Error::InvalidRecord)?,
    )?;
    Ok(())
}
pub(super) fn rollback(context: &Context, transaction: &Transaction) -> Result<(), Error> {
    for change in transaction.changes.iter().rev() {
        if same_written(&change.after.path, change)? {
            if let Some(bytes) = &change.before_bytes {
                let (replacement, _, _) = stage(&change.after.path, bytes)?;
                if !same_written(&change.after.path, change)? {
                    let _ = fs::remove_file(replacement);
                    return Err(Error::RecoveryRequired);
                }
                fs::rename(replacement, &change.after.path)?;
            } else {
                fs::remove_file(&change.after.path)?;
            }
        } else {
            let unchanged = match &change.before {
                Some(before) => matches(&change.after.path, before)?,
                None => !change.after.path.try_exists()?,
            };
            if !unchanged {
                return Err(Error::RecoveryRequired);
            }
        }
        remove_stage(change)?;
    }
    match &transaction.before {
        Some(before) => write_private(
            &context.manifest_path(),
            &serde_json::to_vec(before).map_err(|_| Error::InvalidRecord)?,
        )?,
        None => {
            if context.manifest_path().exists() {
                fs::remove_file(context.manifest_path())?;
            }
        }
    }
    Ok(())
}
pub(super) struct StateLock(File);

impl Drop for StateLock {
    fn drop(&mut self) {
        // Closing alone does not release flock while a concurrently spawned
        // child still holds the same open-file description before exec.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub(super) fn lock(context: &Context) -> Result<StateLock, Error> {
    lock_until(context, Duration::from_secs(2))
}

fn lock_until(context: &Context, timeout: Duration) -> Result<StateLock, Error> {
    directory(&context.state)?;
    let path = context.state.join("omarchy.lock");
    let _ = read(&path, 0o600, 16)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let started = Instant::now();
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(StateLock(file)),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(Error::Busy);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;

    #[test]
    fn omarchy_state_lock_releases_even_while_an_inherited_description_is_open() {
        let (_temp, context, _) = fixture();
        let held = lock(&context).unwrap();
        // dup shares the open-file description exactly as fork does, without
        // running arbitrary code in a forked multithreaded test process.
        let inherited = held.0.try_clone().unwrap();
        drop(held);
        let next = lock_until(&context, Duration::ZERO).unwrap();
        drop(inherited);
        // Closing the old description must not release the new owner's lock.
        assert!(matches!(
            lock_until(&context, Duration::ZERO),
            Err(Error::Busy)
        ));
        drop(next);
        assert!(lock_until(&context, Duration::ZERO).is_ok());
    }

    #[test]
    fn omarchy_state_lock_contention_is_bounded_and_does_not_steal_ownership() {
        let (_temp, context, _) = fixture();
        let held = lock(&context).unwrap();
        let started = Instant::now();
        assert!(matches!(
            lock_until(&context, Duration::from_millis(30)),
            Err(Error::Busy)
        ));
        assert!(started.elapsed() >= Duration::from_millis(30));
        drop(held);
        assert!(lock_until(&context, Duration::ZERO).is_ok());
    }
}
