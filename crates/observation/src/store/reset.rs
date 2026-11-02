//! Observation-only format replacement. The daemon holds its existing startup
//! writer lock; no old rows or body objects are copied or converted.
use super::{LocalObservationStore, upgrade};
use crate::{DigestAuthority, writer::ObservationStoreError as Error};
use hiroute_platform::{
    durable,
    private_fs::{self, DirectoryRemoval, directory_identity},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    parent: [u64; 2],
    source: [u64; 2],
    fresh: [u64; 2],
    token: String,
}

fn journal(root: &Path) -> Result<PathBuf, Error> {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(Error::Corrupt)?;
    Ok(root
        .parent()
        .ok_or(Error::Corrupt)?
        .join(format!(".{name}.format6-reset.json")))
}

fn paths(root: &Path, record: &Record) -> Result<(PathBuf, PathBuf), Error> {
    if record.schema != 1
        || record.token.len() != 32
        || !record.token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Corrupt);
    }
    let parent = root.parent().ok_or(Error::Corrupt)?;
    if directory_identity(parent).map_err(|_| Error::Io)? != record.parent {
        return Err(Error::Corrupt);
    }
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(Error::Corrupt)?;
    Ok((
        parent.join(format!(".{name}.retired-{}", record.token)),
        parent.join(format!(".{name}.fresh-{}", record.token)),
    ))
}

fn read_record(path: &Path) -> Result<Record, Error> {
    let mut input = private_fs::open_private_file(path).map_err(|_| Error::Io)?;
    if input.metadata().map_err(|_| Error::Io)?.len() > 4096 {
        return Err(Error::Corrupt);
    }
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt)
}

fn identity(path: &Path) -> Result<Option<[u64; 2]>, Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => directory_identity(path)
            .map(Some)
            .map_err(|_| Error::Corrupt),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Io),
    }
}

fn verify_fresh(root: &Path, record: &Record) -> Result<(), Error> {
    if identity(root)? != Some(record.fresh) || upgrade::required(root)? {
        return Err(Error::Corrupt);
    }
    let db =
        Connection::open_with_flags(root.join("activity.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::Corrupt)?;
    let generation: String = db
        .query_row(
            "SELECT value FROM observation_meta WHERE key='format_generation'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::Corrupt)?;
    if generation != record.token {
        return Err(Error::Corrupt);
    }
    Ok(())
}

/// Resume using directory identities, never pathname existence alone. A fresh
/// published generation is never reset again and its newer settings win.
fn publish(root: &Path, record: &Record) -> Result<(), Error> {
    let (retired, stage) = paths(root, record)?;
    match (identity(root)?, identity(&retired)?) {
        (Some(live), None) if live == record.source => {
            if identity(&stage)? != Some(record.fresh) {
                return Err(Error::Corrupt);
            }
            durable::rename(root, &retired).map_err(|_| Error::Io)?;
            durable::directory_barrier(root.parent().ok_or(Error::Io)?).map_err(|_| Error::Io)?;
        }
        (None, Some(old)) if old == record.source => {}
        (Some(live), old) if live == record.fresh && old.is_none_or(|old| old == record.source) => {
            return verify_fresh(root, record);
        }
        _ => return Err(Error::Corrupt),
    }
    verify_fresh(&stage, record)?;
    durable::rename(&stage, root).map_err(|_| Error::Io)?;
    durable::directory_barrier(root.parent().ok_or(Error::Io)?).map_err(|_| Error::Io)?;
    verify_fresh(root, record)
}

impl LocalObservationStore {
    /// Production startup only, while the core storage set owns its writer lock.
    pub fn open_for_daemon_startup(
        root: impl AsRef<Path>,
        authority: DigestAuthority,
    ) -> Result<Self, Error> {
        let root = root.as_ref();
        let progress = journal(root)?;
        if progress.try_exists().map_err(|_| Error::Io)? {
            publish(root, &read_record(&progress)?)?;
        } else if upgrade::required(root)? {
            // Only the owned marker and one tiny setting are read from old SQL.
            let db = Connection::open_with_flags(
                root.join("activity.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|_| Error::Corrupt)?;
            let capacity: Option<String> = db
                .query_row(
                    "SELECT value FROM observation_meta WHERE key='max_active_bytes'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| Error::Corrupt)?;
            if let Some(value) = &capacity {
                let limit: Option<u64> = serde_json::from_str(value).map_err(|_| Error::Corrupt)?;
                if limit == Some(0) {
                    return Err(Error::Corrupt);
                }
            }
            drop(db);
            let mut nonce = [0u8; 16];
            getrandom::fill(&mut nonce).map_err(|_| Error::Io)?;
            let token: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
            let parent = root.parent().ok_or(Error::Corrupt)?;
            let name = root
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(Error::Corrupt)?;
            let stage = parent.join(format!(".{name}.fresh-{token}"));
            private_fs::create_directory(&stage).map_err(|_| Error::Io)?;
            let fresh = Self::open(&stage, authority.clone())?;
            {
                let db = fresh.connection.lock();
                db.execute(
                    "UPDATE observation_meta SET value=?1 WHERE key='format_generation'",
                    [&token],
                )
                .map_err(|_| Error::ActivityUnavailable)?;
                if let Some(value) = capacity {
                    db.execute(
                        "INSERT INTO observation_meta VALUES('max_active_bytes',?1)",
                        [value],
                    )
                    .map_err(|_| Error::ActivityUnavailable)?;
                }
                db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                    .map_err(|_| Error::ActivityUnavailable)?;
            }
            drop(fresh);
            durable::directory_barrier(&stage).map_err(|_| Error::Io)?;
            let record = Record {
                schema: 1,
                parent: directory_identity(parent).map_err(|_| Error::Io)?,
                source: directory_identity(root).map_err(|_| Error::Io)?,
                fresh: directory_identity(&stage).map_err(|_| Error::Io)?,
                token,
            };
            durable::write_private_new(
                &progress,
                &serde_json::to_vec(&record).map_err(|_| Error::Corrupt)?,
            )
            .map_err(|_| Error::Io)?;
            durable::directory_barrier(parent).map_err(|_| Error::Io)?;
            publish(root, &record)?;
        }
        Self::open(root, authority)
    }

    pub fn service_ready(&self) {
        self.reset_cleanup.lock().ready = true;
    }

    /// Called by the existing maintenance hook: bounded work with completion-
    /// based cooling, never a new timer or an eager error retry loop.
    pub fn cleanup_retired_format(&self) -> Result<(), Error> {
        let mut state = self.reset_cleanup.lock();
        if !state.ready || state.done || Instant::now() < state.due {
            return Ok(());
        }
        let started = Instant::now();
        let result = (|| {
            let root = self.activity_path.parent().ok_or(Error::Corrupt)?;
            let progress = journal(root)?;
            if !progress.try_exists().map_err(|_| Error::Io)? {
                state.done = true;
                return Ok(());
            }
            let record = read_record(&progress)?;
            verify_fresh(root, &record)?;
            let (retired, _) = paths(root, &record)?;
            if let Some(id) = identity(&retired)? {
                if id != record.source {
                    return Err(Error::Corrupt);
                }
                if state.removal.is_none() {
                    state.removal =
                        Some(DirectoryRemoval::open(&retired, id).map_err(|_| Error::Io)?);
                }
                if !state
                    .removal
                    .as_mut()
                    .ok_or(Error::Corrupt)?
                    .step(128, Duration::from_millis(10))
                    .map_err(|_| Error::Io)?
                    .done
                {
                    return Ok(());
                }
            }
            durable::directory_barrier(root.parent().ok_or(Error::Io)?).map_err(|_| Error::Io)?;
            fs::remove_file(progress).map_err(|_| Error::Io)?;
            durable::directory_barrier(root.parent().ok_or(Error::Io)?).map_err(|_| Error::Io)?;
            state.removal = None;
            state.done = true;
            Ok(())
        })();
        if result.is_err() {
            state.removal = None;
        }
        state.due = Instant::now()
            + started
                .elapsed()
                .saturating_mul(100)
                .max(if result.is_err() {
                    Duration::from_secs(5)
                } else {
                    Duration::from_secs(1)
                });
        result
    }
}

pub(super) struct Cleanup {
    ready: bool,
    done: bool,
    due: Instant,
    removal: Option<DirectoryRemoval>,
}
impl Default for Cleanup {
    fn default() -> Self {
        Self {
            ready: false,
            done: false,
            due: Instant::now(),
            removal: None,
        }
    }
}

#[cfg(test)]
mod tests;
