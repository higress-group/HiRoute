//! Per-connection physical epochs for bounded cross-store reads. A stamp is not
//! business authority and never contains key bytes, credentials or file paths.
use crate::LocalStorageError;
use hiroute_domain::{PortError, PortErrorCode, PortResult};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

fn unavailable() -> PortError {
    PortError::new(PortErrorCode::Unavailable, "storage.physical-read-stamp")
}
fn changed() -> PortError {
    PortError::new(PortErrorCode::Conflict, "storage.physical-read-stamp")
}

#[cfg(unix)]
fn identity(path: &Path) -> std::io::Result<[u64; 2]> {
    use std::os::unix::fs::MetadataExt;
    // POSIX record locks are process-associated: closing another descriptor
    // for this database can release SQLite's locks and leave its cache stale.
    // Check the path without opening or closing an additional descriptor.
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    Ok([metadata.dev(), metadata.ino()])
}

#[cfg(windows)]
fn identity(path: &Path) -> std::io::Result<[u64; 2]> {
    let file = hiroute_platform::windows::security::open_identity(path, false)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    let (volume, index) = hiroute_platform::windows::security::file_identity(&file)?;
    Ok([u64::from(volume), index])
}

#[cfg(not(any(unix, windows)))]
fn identity(_path: &Path) -> std::io::Result<[u64; 2]> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub(crate) struct DatabaseBinding {
    path: PathBuf,
    file: [u64; 2],
    connection: u64,
}

impl DatabaseBinding {
    /// Reopen only the already admitted file. Never create or migrate a replacement.
    pub(crate) fn open_peer(&self) -> Result<Connection, LocalStorageError> {
        if identity(&self.path)? != self.file {
            return Err(LocalStorageError::InvalidData);
        }
        let connection = crate::migrations::open_existing_for_snapshot(&self.path)?;
        if identity(&self.path)? != self.file {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(connection)
    }

    pub(crate) fn new(path: &Path) -> Result<Self, LocalStorageError> {
        let connection = NEXT_CONNECTION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| LocalStorageError::InvalidData)?;
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(Self {
            file: identity(&path)?,
            path,
            connection,
        })
    }

    pub(crate) fn capture(
        &self,
        db: &Connection,
        binding: impl FnOnce(&Connection) -> PortResult<Vec<String>>,
    ) -> PortResult<StorePhysicalStamp> {
        if !db.is_autocommit() {
            return Err(changed());
        }
        if identity(&self.path).map_err(|_| unavailable())? != self.file {
            return Err(changed());
        }
        let storage_binding = binding(db)?;
        let result = StorePhysicalStamp {
            connection: self.connection,
            file: self.file,
            storage_binding,
            data_version: db
                .query_row("PRAGMA data_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
            local_changes: db.total_changes(),
            main_schema: db
                .query_row("PRAGMA main.schema_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
            temp_schema: db
                .query_row("PRAGMA temp.schema_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
        };
        if !db.is_autocommit() || identity(&self.path).map_err(|_| unavailable())? != self.file {
            return Err(changed());
        }
        Ok(result)
    }
}

fn unchanged_in_owned_read(db: &Connection, stamp: &StorePhysicalStamp) -> PortResult<bool> {
    let main: i64 = db
        .query_row("PRAGMA main.schema_version", [], |r| r.get(0))
        .map_err(|_| unavailable())?;
    let temp: i64 = db
        .query_row("PRAGMA temp.schema_version", [], |r| r.get(0))
        .map_err(|_| unavailable())?;
    Ok(!db.is_autocommit()
        && db.total_changes() == stamp.local_changes
        && main == stamp.main_schema
        && temp == stamp.temp_schema)
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct StorePhysicalStamp {
    connection: u64,
    file: [u64; 2],
    storage_binding: Vec<String>,
    data_version: i64,
    local_changes: u64,
    main_schema: i64,
    temp_schema: i64,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AgentReadStamp {
    control: StorePhysicalStamp,
    secrets: Option<StorePhysicalStamp>,
}

impl crate::LocalStorageSet {
    fn agent_read_stamp(&self, include_secrets: bool) -> PortResult<AgentReadStamp> {
        let control = self
            .control
            .connection
            .try_borrow()
            .map_err(|_| changed())?;
        let control_stamp = self.control.physical_binding.capture(&control, |db| {
            let uuid: String = db
                .query_row(
                    "SELECT store_uuid FROM storage_meta WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| unavailable())?;
            if uuid.is_empty() {
                return Err(changed());
            }
            Ok(vec![uuid])
        })?;
        let secrets = if include_secrets {
            Some(self.secrets.physical_read_stamp()?)
        } else {
            None
        };
        Ok(AgentReadStamp {
            control: control_stamp,
            secrets,
        })
    }

    fn with_owned_agent_reads<T>(
        &self,
        baseline: &AgentReadStamp,
        read: impl FnOnce(&crate::ControlStore, Option<&crate::LocalSecretStore>) -> PortResult<T>,
    ) -> PortResult<T> {
        let control = self
            .control
            .connection
            .try_borrow()
            .map_err(|_| changed())?;
        let control_tx = control.unchecked_transaction().map_err(|_| unavailable())?;
        let secret_db = if baseline.secrets.is_some() {
            Some(
                self.secrets
                    .connection
                    .try_borrow()
                    .map_err(|_| changed())?,
            )
        } else {
            None
        };
        let secret_tx = match &secret_db {
            Some(db) => Some(db.unchecked_transaction().map_err(|_| unavailable())?),
            None => None,
        };
        let result = read(
            &self.control,
            baseline.secrets.as_ref().map(|_| &self.secrets),
        )?;
        if !unchanged_in_owned_read(&control, &baseline.control)?
            || match (&secret_db, &baseline.secrets) {
                (Some(db), Some(stamp)) => !unchanged_in_owned_read(db, stamp)?,
                (None, None) => false,
                _ => true,
            }
        {
            // Dropping both transactions rolls back any accidental local mutation.
            return Err(changed());
        }
        if let Some(tx) = secret_tx {
            tx.commit().map_err(|_| unavailable())?;
        }
        control_tx.commit().map_err(|_| unavailable())?;
        Ok(result)
    }

    /// Capture independent owned SQLite snapshots within an unchanged physical
    /// epoch vector. The caller holds its LocalStorageSet lock for this method.
    pub fn capture_agent_read<T>(
        &self,
        include_secrets: bool,
        capture: impl FnOnce(&crate::ControlStore, Option<&crate::LocalSecretStore>) -> PortResult<T>,
    ) -> PortResult<(T, AgentReadStamp)> {
        let before = self.agent_read_stamp(include_secrets)?;
        let result = self.with_owned_agent_reads(&before, capture)?;
        let after = self.agent_read_stamp(include_secrets)?;
        if before != after {
            return Err(changed());
        }
        Ok((result, after))
    }

    /// Recheck business dependencies between E2/E3 after lock-free validation.
    /// The caller must compare the exact dependencies actually consumed.
    pub fn fence_agent_read<T>(
        &self,
        captured: &AgentReadStamp,
        recheck: impl FnOnce(&crate::ControlStore, Option<&crate::LocalSecretStore>) -> PortResult<T>,
    ) -> PortResult<T> {
        let before = self.agent_read_stamp(captured.secrets.is_some())?;
        if &before != captured {
            return Err(changed());
        }
        let result = self.with_owned_agent_reads(&before, recheck)?;
        let after = self.agent_read_stamp(captured.secrets.is_some())?;
        if before != after || &after != captured {
            return Err(changed());
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "physical_read_stamp_tests.rs"]
mod tests;
