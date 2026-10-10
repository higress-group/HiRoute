//! A receipt of the child and socket identities observed at trusted startup.
//! Only examined while holding desktop.lock. It never authorizes an external peer.
use nix::{
    errno::Errno,
    sys::{
        signal::kill,
        socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket},
    },
    unistd::Pid,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    fn of(meta: &Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    pid: u32,
    directory: Identity,
    sockets: Vec<Identity>,
    /// The gateway address this owned run served; read-only evidence for recovering the exact
    /// stable address when its record is lost. Absent in receipts from older runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway_address: Option<String>,
}

pub(crate) fn validate_ancestors(path: &Path) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|_| "PRIVATE_PATH_INVALID")?;
    for ancestor in path.ancestors() {
        let meta = match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("PRIVATE_PATH_INVALID".into()),
        };
        if meta.file_type().is_symlink()
            && ![("/tmp", "/private/tmp"), ("/var", "/private/var")]
                .iter()
                .any(|(a, t)| {
                    ancestor == Path::new(a)
                        && std::fs::canonicalize(ancestor).ok().as_deref() == Some(Path::new(t))
                })
        {
            return Err("PRIVATE_PATH_INVALID".into());
        }
    }
    Ok(())
}
pub(crate) fn validate_lock(file: &File) -> Result<(), String> {
    let m = file.metadata().map_err(|_| "RESIDENT_LOCK_UNAVAILABLE")?;
    if !m.is_file() || m.nlink() != 1 {
        return Err("RESIDENT_LOCK_INVALID".into());
    }
    Ok(())
}
fn paths(runtime: &Path) -> [PathBuf; 2] {
    [
        runtime.join("hiroute/control.sock"),
        runtime.join("hiroute/agent-grant-v1.sock"),
    ]
}
fn snapshot(
    runtime: &Path,
    pid: u32,
    gateway_address: Option<&std::net::SocketAddr>,
) -> Result<Receipt, String> {
    let dir = runtime.join("hiroute");
    validate_ancestors(&dir)?;
    let meta = std::fs::symlink_metadata(&dir).map_err(|_| "RESIDENT_ENDPOINT_INVALID")?;
    if !meta.is_dir() {
        return Err("RESIDENT_ENDPOINT_INVALID".into());
    }
    let mut sockets = Vec::new();
    for path in paths(runtime) {
        let m = std::fs::symlink_metadata(path).map_err(|_| "RESIDENT_ENDPOINT_INVALID")?;
        if !m.file_type().is_socket() {
            return Err("RESIDENT_ENDPOINT_INVALID".into());
        }
        sockets.push(Identity::of(&m));
    }
    Ok(Receipt {
        version: 1,
        pid,
        directory: Identity::of(&meta),
        sockets,
        gateway_address: gateway_address.map(|address| address.to_string()),
    })
}
fn same_owned_nodes(old: &Receipt, current: &Receipt) -> bool {
    // macOS can assign a different st_dev to the same persisted APFS files after reboot.
    // Compare the inodes at every known path, while requiring each snapshot to describe
    // one volume. snapshot() already checked type, owner and private permissions.
    old.directory.inode == current.directory.inode
        && old.sockets.len() == current.sockets.len()
        && old
            .sockets
            .iter()
            .all(|socket| socket.device == old.directory.device)
        && current
            .sockets
            .iter()
            .all(|socket| socket.device == current.directory.device)
        && old
            .sockets
            .iter()
            .zip(&current.sockets)
            .all(|(before, now)| before.inode == now.inode)
}
pub(crate) fn record(
    lock: &mut File,
    runtime: &Path,
    pid: u32,
    gateway_address: Option<&std::net::SocketAddr>,
) -> Result<(), String> {
    let receipt = snapshot(runtime, pid, gateway_address)?;
    let bytes = serde_json::to_vec(&receipt).map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE")?;
    lock.seek(SeekFrom::Start(0))
        .and_then(|_| lock.set_len(0))
        .and_then(|_| lock.write_all(&bytes))
        .and_then(|_| lock.sync_all())
        .map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE".into())
}
/// The address the last owned run served, as recorded in the desktop.lock receipt. A missing or
/// older receipt carries no evidence; it never authorizes choosing a different address.
pub(crate) fn recorded_gateway_address(
    lock: &mut File,
) -> Result<Option<std::net::SocketAddr>, String> {
    lock.seek(SeekFrom::Start(0))
        .map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE".to_string())?;
    let mut bytes = Vec::new();
    Read::by_ref(lock)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE".to_string())?;
    let receipt: Receipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) if bytes.len() <= 4096 => receipt,
        _ => return Ok(None),
    };
    if receipt.version != 1 {
        return Ok(None);
    }
    Ok(receipt
        .gateway_address
        .as_deref()
        .and_then(|address| address.parse().ok()))
}
pub(crate) fn recoverable(lock: &mut File, runtime: &Path) -> Result<bool, String> {
    lock.seek(SeekFrom::Start(0))
        .map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE")?;
    let mut bytes = Vec::new();
    Read::by_ref(lock)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESIDENT_RECEIPT_UNAVAILABLE")?;
    // No receipt, interrupted write or old format: preserve the unknown endpoint.
    let old: Receipt = match serde_json::from_slice(&bytes) {
        Ok(r) if bytes.len() <= 4096 => r,
        _ => return Ok(false),
    };
    if old.version != 1 || old.pid == 0 || old.pid > i32::MAX as u32 {
        return Ok(false);
    }
    let current = match snapshot(runtime, old.pid, None) {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    if !same_owned_nodes(&old, &current) {
        return Ok(false);
    }
    // A reused PID or permission error is conservative: never signal or reclaim it.
    if kill(Pid::from_raw(old.pid as i32), None) != Err(Errno::ESRCH) {
        return Ok(false);
    }
    for path in paths(runtime) {
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .map_err(|_| "RESIDENT_ENDPOINT_UNAVAILABLE")?;
        nix::fcntl::fcntl(
            &fd,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )
        .map_err(|_| "RESIDENT_ENDPOINT_UNAVAILABLE")?;
        nix::fcntl::fcntl(
            &fd,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .map_err(|_| "RESIDENT_ENDPOINT_UNAVAILABLE")?;
        let address = UnixAddr::new(&path).map_err(|_| "RESIDENT_ENDPOINT_INVALID")?;
        if connect(fd.as_raw_fd(), &address) != Err(Errno::ECONNREFUSED) {
            return Ok(false);
        }
    }
    // The daemon performs its own validated stale-socket cleanup and storage locking.
    // Desktop does not unlink endpoints, and a live listener winning the race blocks startup.
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    };
    fn fixture() -> (tempfile::TempDir, File, Vec<UnixListener>, u32) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("hiroute")).unwrap();
        std::fs::set_permissions(
            root.path().join("hiroute"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let listeners = paths(root.path())
            .into_iter()
            .map(|path| {
                let l = UnixListener::bind(&path).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
                l
            })
            .collect();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.path().join("lock"))
            .unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        (root, lock, listeners, pid)
    }
    #[test]
    fn only_dead_recorded_unreplaced_endpoints_are_recoverable() {
        let (root, mut lock, listeners, pid) = fixture();
        assert!(
            !recoverable(&mut lock, root.path()).unwrap(),
            "unknown listener"
        );
        record(&mut lock, root.path(), pid, None).unwrap();
        assert!(
            !recoverable(&mut lock, root.path()).unwrap(),
            "live listener despite dead recorded PID"
        );
        drop(listeners);
        assert!(recoverable(&mut lock, root.path()).unwrap());
        record(&mut lock, root.path(), std::process::id(), None).unwrap();
        assert!(
            !recoverable(&mut lock, root.path()).unwrap(),
            "living/reused PID"
        );
    }
    #[test]
    fn a_reboot_device_number_change_does_not_orphan_owned_dead_sockets() {
        let (root, mut lock, listeners, pid) = fixture();
        record(&mut lock, root.path(), pid, None).unwrap();
        lock.seek(SeekFrom::Start(0)).unwrap();
        let mut receipt: Receipt = serde_json::from_reader(&lock).unwrap();
        receipt.directory.device ^= 1;
        for socket in &mut receipt.sockets {
            socket.device ^= 1;
        }
        lock.seek(SeekFrom::Start(0)).unwrap();
        lock.set_len(0).unwrap();
        serde_json::to_writer(&mut lock, &receipt).unwrap();
        drop(listeners);

        assert!(recoverable(&mut lock, root.path()).unwrap());

        let endpoint = paths(root.path())[0].clone();
        std::fs::remove_file(&endpoint).unwrap();
        let replacement = UnixListener::bind(&endpoint).unwrap();
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(replacement);
        assert!(!recoverable(&mut lock, root.path()).unwrap());
    }
    #[test]
    fn replaced_or_symlinked_endpoint_is_never_reclaimed_but_modes_are_allowed() {
        let (root, mut lock, listeners, pid) = fixture();
        record(&mut lock, root.path(), pid, None).unwrap();
        drop(listeners);
        let endpoint = paths(root.path())[0].clone();
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(recoverable(&mut lock, root.path()).unwrap());
        assert_eq!(
            std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
            0o666
        );
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        let original = root.path().join("original.sock");
        std::fs::rename(&endpoint, &original).unwrap();
        let replacement = UnixListener::bind(&endpoint).unwrap();
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(replacement);
        assert!(!recoverable(&mut lock, root.path()).unwrap());
        std::fs::remove_file(&endpoint).unwrap();
        symlink(&original, &endpoint).unwrap();
        assert!(!recoverable(&mut lock, root.path()).unwrap());
        assert!(
            std::fs::symlink_metadata(endpoint)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(original.exists());
    }
}
