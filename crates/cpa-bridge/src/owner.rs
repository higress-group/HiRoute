use std::fs::{self, OpenOptions};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{
    CpaConfigError, ensure_private_dir, private_atomic_write, validate_private_file,
};

const OWNER_SCHEMA: &str = "hiroute.cpa-owner/v1";
const MAX_OWNER_BYTES: u64 = 8 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerRecord {
    schema: String,
    pub(crate) owner_pid: u32,
    pub(crate) owner_nonce: String,
    pub(crate) cpa_pid: u32,
    pub(crate) address: SocketAddr,
    pub(crate) binary_version: String,
    pub(crate) binary_sha256_hex: String,
    /// Recovery-only omission: authenticate and replace pre-policy orphan processes.
    #[serde(default)]
    pub(crate) proxy_environment_sha256: Option<String>,
}

impl OwnerRecord {
    pub(crate) fn claim(
        owner_pid: u32,
        owner_nonce: String,
        version: String,
        digest: String,
    ) -> Result<Self, OwnerError> {
        let value = Self {
            schema: OWNER_SCHEMA.to_owned(),
            owner_pid,
            owner_nonce,
            cpa_pid: 0,
            address: "127.0.0.1:1".parse().expect("literal address"),
            binary_version: version,
            binary_sha256_hex: digest,
            proxy_environment_sha256: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn running(mut self, cpa_pid: u32, address: SocketAddr) -> Result<Self, OwnerError> {
        self.cpa_pid = cpa_pid;
        self.address = address;
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn transfer(mut self, owner_pid: u32, nonce: String) -> Result<Self, OwnerError> {
        self.owner_pid = owner_pid;
        self.owner_nonce = nonce;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), OwnerError> {
        if self.schema != OWNER_SCHEMA
            || self.owner_pid == 0
            || self.owner_nonce.len() < 32
            || self.owner_nonce.len() > 128
            || !self.address.ip().is_loopback()
            || self.binary_version.is_empty()
            || self.binary_version.len() > 64
            || self.binary_sha256_hex.len() != 64
            || self
                .proxy_environment_sha256
                .as_ref()
                .is_some_and(|digest| {
                    digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || !self
                .binary_sha256_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(OwnerError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct OwnerLease {
    lock_dir: PathBuf,
    nonce: String,
}

pub(crate) enum OwnerClaim {
    Acquired(OwnerLease),
    Existing(OwnerRecord),
}

impl OwnerLease {
    pub(crate) fn acquire(
        lock_dir: &Path,
        initial: &OwnerRecord,
        nonce: String,
    ) -> Result<OwnerClaim, OwnerError> {
        let parent = lock_dir.parent().ok_or(OwnerError::InvalidPath)?;
        initial.validate()?;
        if initial.owner_nonce != nonce {
            return Err(OwnerError::NonceMismatch);
        }
        let _reclaim_guard = lock_reclaim(parent)?;
        match fs::create_dir(lock_dir) {
            Ok(()) => {
                ensure_private_dir(lock_dir)?;
                if let Err(error) = write_record(lock_dir, initial) {
                    let _ = recover_incomplete_lock_dir(lock_dir);
                    return Err(error);
                }
                Ok(OwnerClaim::Acquired(Self {
                    lock_dir: lock_dir.to_owned(),
                    nonce,
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !lock_dir.join("owner.json").exists() {
                    recover_incomplete_lock_dir(lock_dir)?;
                    fs::create_dir(lock_dir).map_err(OwnerError::Io)?;
                    ensure_private_dir(lock_dir)?;
                    write_record(lock_dir, initial)?;
                    return Ok(OwnerClaim::Acquired(Self {
                        lock_dir: lock_dir.to_owned(),
                        nonce,
                    }));
                }
                Ok(OwnerClaim::Existing(read_record(lock_dir)?))
            }
            Err(error) => Err(OwnerError::Io(error)),
        }
    }

    pub(crate) fn reclaim_stale(
        lock_dir: &Path,
        expected: &OwnerRecord,
        replacement: &OwnerRecord,
        nonce: String,
    ) -> Result<OwnerLease, OwnerError> {
        let parent = lock_dir.parent().ok_or(OwnerError::InvalidPath)?;
        expected.validate()?;
        replacement.validate()?;
        if replacement.owner_nonce != nonce {
            return Err(OwnerError::NonceMismatch);
        }
        let _reclaim_guard = lock_reclaim(parent)?;
        if read_record(lock_dir)? != *expected {
            return Err(OwnerError::StaleRecordChanged);
        }
        write_record(lock_dir, replacement)?;
        Ok(Self {
            lock_dir: lock_dir.to_owned(),
            nonce,
        })
    }

    pub(crate) fn write(&self, record: &OwnerRecord) -> Result<(), OwnerError> {
        if record.owner_nonce != self.nonce {
            return Err(OwnerError::NonceMismatch);
        }
        write_record(&self.lock_dir, record)
    }

    pub(crate) fn restore_stale(&self, record: &OwnerRecord) -> Result<(), OwnerError> {
        let parent = self.lock_dir.parent().ok_or(OwnerError::InvalidPath)?;
        let _reclaim_guard = lock_reclaim(parent)?;
        let current = read_record(&self.lock_dir)?;
        if current.owner_nonce != self.nonce {
            return Err(OwnerError::NonceMismatch);
        }
        write_record(&self.lock_dir, record)
    }

    pub(crate) fn release(&self) -> Result<(), OwnerError> {
        self.abandon()
    }

    /// Removes a lock created by this nonce after a partial initialization failure. If an owner
    /// record exists, it must still prove the same nonce before anything is removed.
    pub(crate) fn abandon(&self) -> Result<(), OwnerError> {
        let parent = self.lock_dir.parent().ok_or(OwnerError::InvalidPath)?;
        let _reclaim_guard = lock_reclaim(parent)?;
        let record_path = self.lock_dir.join("owner.json");
        if record_path.exists() {
            let record = read_record(&self.lock_dir)?;
            if record.owner_nonce != self.nonce {
                return Err(OwnerError::NonceMismatch);
            }
            fs::remove_file(record_path).map_err(OwnerError::Io)?;
        }
        fs::remove_dir(&self.lock_dir).map_err(OwnerError::Io)
    }
}

fn recover_incomplete_lock_dir(lock_dir: &Path) -> Result<(), OwnerError> {
    ensure_private_dir(lock_dir)?;
    let entries = fs::read_dir(lock_dir)
        .map_err(OwnerError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(OwnerError::Io)?;
    if entries.len() > 4 {
        return Err(OwnerError::InvalidRecord);
    }
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| OwnerError::InvalidRecord)?;
        let file_type = entry.file_type().map_err(OwnerError::Io)?;
        if !file_type.is_file() || !name.starts_with(".write-") || !name.ends_with(".tmp") {
            return Err(OwnerError::InvalidRecord);
        }
        validate_private_file(&entry.path())?;
        fs::remove_file(entry.path()).map_err(OwnerError::Io)?;
    }
    fs::remove_dir(lock_dir).map_err(OwnerError::Io)
}

fn write_record(lock_dir: &Path, record: &OwnerRecord) -> Result<(), OwnerError> {
    record.validate()?;
    let bytes = serde_json::to_vec(record).map_err(OwnerError::Json)?;
    private_atomic_write(&lock_dir.join("owner.json"), &bytes)?;
    Ok(())
}

fn open_reclaim_file(parent: &Path) -> Result<fs::File, OwnerError> {
    let path = parent.join(".owner-reclaim.lock");
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    set_private_create_mode(&mut create);
    let file = match create.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(OwnerError::Io)?,
        Err(error) => return Err(OwnerError::Io(error)),
    };
    validate_private_file(&path)?;
    Ok(file)
}

fn lock_reclaim(parent: &Path) -> Result<fs::File, OwnerError> {
    let file = open_reclaim_file(parent)?;
    file.try_lock_exclusive()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::WouldBlock => OwnerError::ReclaimContended,
            _ => OwnerError::Io(error),
        })?;
    Ok(file)
}

#[cfg(unix)]
fn set_private_create_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_create_mode(_options: &mut OpenOptions) {}

pub(crate) fn read_record(lock_dir: &Path) -> Result<OwnerRecord, OwnerError> {
    let path = lock_dir.join("owner.json");
    validate_private_file(&path)?;
    let file = fs::File::open(path).map_err(OwnerError::Io)?;
    let mut bytes = Vec::new();
    file.take(MAX_OWNER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(OwnerError::Io)?;
    if bytes.len() as u64 > MAX_OWNER_BYTES {
        return Err(OwnerError::InvalidRecord);
    }
    let record: OwnerRecord = serde_json::from_slice(&bytes).map_err(OwnerError::Json)?;
    record.validate()?;
    Ok(record)
}

#[derive(Debug, Error)]
pub(crate) enum OwnerError {
    #[error("CPA owner path is invalid")]
    InvalidPath,
    #[error("CPA owner record is invalid")]
    InvalidRecord,
    #[error("CPA owner nonce does not match")]
    NonceMismatch,
    #[error("CPA stale-owner reclaim is already in progress")]
    ReclaimContended,
    #[error("CPA stale-owner record changed during reclaim")]
    StaleRecordChanged,
    #[error("CPA owner state I/O failed: {0}")]
    Io(std::io::Error),
    #[error("CPA owner JSON is invalid: {0}")]
    Json(serde_json::Error),
    #[error(transparent)]
    Config(#[from] CpaConfigError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_reclaim_installs_one_exact_replacement_and_rejects_replay() {
        // Parallel process tests may fork while this test holds a file lock. A child
        // retains the same open-file description until exec, even with CLOEXEC. Exercise
        // exact close/reclaim ordering in one isolated test process instead.
        const CASE: &str =
            "owner::tests::stale_reclaim_installs_one_exact_replacement_and_rejects_replay";
        const CHILD: &str = "HIROUTE_ISOLATED_LOCK_TEST";
        if std::env::var(CHILD).as_deref() != Ok(CASE) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CASE])
                .env(CHILD, CASE)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success()
                    && stdout
                        .lines()
                        .any(|line| line == format!("test {CASE} ... ok")),
                "isolated lock test must execute its exact case: {stdout} {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let lock_dir = root.path().join("owner.lock");
        let old_nonce = "a".repeat(43);
        let OwnerClaim::Acquired(old_lease) = OwnerLease::acquire(
            &lock_dir,
            &OwnerRecord::claim(100, old_nonce.clone(), "7.2.140".into(), "a".repeat(64)).unwrap(),
            old_nonce.clone(),
        )
        .unwrap() else {
            panic!("fresh lock was not acquired")
        };
        let old = OwnerRecord::claim(100, old_nonce, "7.2.140".into(), "a".repeat(64))
            .unwrap()
            .running(200, "127.0.0.1:18080".parse().unwrap())
            .unwrap();
        old_lease.write(&old).unwrap();

        let new_nonce = "b".repeat(43);
        let replacement = old.clone().transfer(101, new_nonce.clone()).unwrap();
        let lease =
            OwnerLease::reclaim_stale(&lock_dir, &old, &replacement, new_nonce.clone()).unwrap();
        assert_eq!(read_record(&lock_dir).unwrap(), replacement);

        let replay_nonce = "c".repeat(43);
        let replay = old.clone().transfer(102, replay_nonce.clone()).unwrap();
        // Reproduce the observed busy result under a controlled extra descriptor.
        // Contention must not overwrite the replacement or authorize the stale owner.
        let held_reclaim = lock_reclaim(root.path()).unwrap();
        assert!(matches!(
            OwnerLease::reclaim_stale(&lock_dir, &old, &replay, replay_nonce.clone()),
            Err(OwnerError::ReclaimContended)
        ));
        assert_eq!(read_record(&lock_dir).unwrap(), replacement);
        drop(held_reclaim);
        let error = OwnerLease::reclaim_stale(&lock_dir, &old, &replay, replay_nonce)
            .expect_err("the old owner record must not replace the current owner");
        assert!(matches!(error, OwnerError::StaleRecordChanged), "{error:?}");
        lease.restore_stale(&old).unwrap();
        assert_eq!(read_record(&lock_dir).unwrap(), old);
    }
}
