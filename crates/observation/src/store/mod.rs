//! Physical `activity.db` plus an external content-object directory.
pub(crate) mod agent_attribution;
mod backfill;
pub(crate) mod blob_gc;
pub(crate) mod capacity;
mod extensions;
mod unified_retention;

pub(crate) mod fact_log;
#[cfg(test)]
mod field_replay_tests;
#[cfg(test)]
mod field_tests;
mod gap;
mod ingest;
mod protocol;
pub(crate) mod records;
mod request_retention;
mod retention;
mod safe_facts;
mod schema;
pub(crate) mod sensitive;
#[cfg(test)]
pub(crate) mod tests;
mod upgrade;
mod value_rollup;

use std::fs;
use std::path::{Path, PathBuf};

use hiroute_domain::{ContentId, RetentionPolicyV1, WorkspaceId};
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::DigestAuthority;
use crate::writer::ObservationStoreError;

#[cfg(test)]
type SessionReadTestHook = Box<dyn FnOnce(&LocalObservationStore) + Send>;

pub struct LocalObservationStore {
    pub(crate) timeline_cache: Mutex<crate::query_v2::routing_timeline::cache::TimelineCache>,
    pub(crate) maintenance_running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) index_running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) maintenance_errors: std::sync::atomic::AtomicU64,
    pub(crate) query_workers: std::sync::atomic::AtomicUsize,
    pub(crate) connection: Mutex<Connection>,
    pub(crate) activity_path: PathBuf,
    pub(crate) staging_root: PathBuf,
    pub(crate) blob_root: PathBuf,
    pub(crate) authority: DigestAuthority,
    pub(crate) retention: RetentionPolicyV1,
    pub(crate) capacity: Mutex<capacity::State>,
    #[cfg(test)]
    pub(crate) session_read_test_hook: Mutex<Option<SessionReadTestHook>>,
}

impl LocalObservationStore {
    pub fn legacy_history_requires_seal(
        root: impl AsRef<Path>,
    ) -> Result<bool, ObservationStoreError> {
        upgrade::required(root.as_ref())
    }

    /// Called only after the startup coordinator has published the complete
    /// sealed source. Initialize privately, so a crash cannot leave a partly
    /// created format-4/5 database at the authoritative live name.
    pub fn open_fresh_after_seal(
        root: impl AsRef<Path>,
        authority: DigestAuthority,
    ) -> Result<Self, ObservationStoreError> {
        let root = root.as_ref();
        if root.exists() {
            return Self::open(root, authority);
        }
        let parent = root.parent().ok_or(ObservationStoreError::Io)?;
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).map_err(|_| ObservationStoreError::Io)?;
        let stage = parent.join(format!(
            ".observation-fresh-{:016x}",
            u64::from_le_bytes(nonce)
        ));
        hiroute_platform::private_fs::create_directory(&stage)
            .map_err(|_| ObservationStoreError::Io)?;
        let store = Self::open(&stage, authority.clone())?;
        store
            .connection
            .lock()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        drop(store);
        hiroute_platform::durable::directory_barrier(&stage)
            .map_err(|_| ObservationStoreError::Io)?;
        if root.exists() {
            return Err(ObservationStoreError::Corrupt);
        }
        hiroute_platform::durable::rename(&stage, root).map_err(|_| ObservationStoreError::Io)?;
        hiroute_platform::durable::directory_barrier(parent)
            .map_err(|_| ObservationStoreError::Io)?;
        Self::open(root, authority)
    }

    pub fn open(
        root: impl AsRef<Path>,
        authority: DigestAuthority,
    ) -> Result<Self, ObservationStoreError> {
        let root = root.as_ref();
        upgrade::prepare(root)?;
        let content_root = root.join("conversation-content");
        let staging_root = content_root.join("staging");
        let blob_root = content_root.join("blobs");
        fs::create_dir_all(&staging_root).map_err(|_| ObservationStoreError::Io)?;
        fs::create_dir_all(&blob_root).map_err(|_| ObservationStoreError::Io)?;
        let activity_path = root.join("activity.db");
        let fresh = !activity_path.exists();
        let mut connection = Connection::open(&activity_path)
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if fresh {
            connection
                .execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")
                .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        }
        schema::migrate(&mut connection)?;
        extensions::migrate(&mut connection)?;
        // Optional, rebuildable analysis must not make otherwise supported core
        // storage unreadable when its own table-local format is unavailable.
        let _ = crate::analysis::migrate(&mut connection);
        connection
            .execute_batch(capacity::SCHEMA)
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        Ok(Self {
            timeline_cache: Default::default(),
            maintenance_running: Default::default(),
            index_running: Default::default(),
            maintenance_errors: std::sync::atomic::AtomicU64::new(0),
            query_workers: std::sync::atomic::AtomicUsize::new(0),
            connection: Mutex::new(connection),
            activity_path,
            staging_root,
            blob_root,
            authority,
            retention: RetentionPolicyV1::default(),
            capacity: Mutex::default(),
            #[cfg(test)]
            session_read_test_hook: Mutex::default(),
        })
    }

    pub fn retention_policy(&self) -> &RetentionPolicyV1 {
        &self.retention
    }

    pub(crate) fn staging_directory(
        &self,
        workspace_id: &WorkspaceId,
        content_id: &ContentId,
    ) -> PathBuf {
        self.staging_root.join(portable_hash(&format!(
            "{}\0{}",
            workspace_id.as_str(),
            content_id.as_str()
        )))
    }

    pub(crate) fn object_locator(&self, path: &Path) -> Result<String, ObservationStoreError> {
        let root = self
            .activity_path
            .parent()
            .ok_or(ObservationStoreError::Corrupt)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ObservationStoreError::Corrupt)?;
        if !relative.starts_with("conversation-content") {
            return Err(ObservationStoreError::Corrupt);
        }
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }
    pub(crate) fn object_path(&self, stored: &str) -> Result<PathBuf, ObservationStoreError> {
        let path = Path::new(stored);
        let root = self
            .activity_path
            .parent()
            .ok_or(ObservationStoreError::Corrupt)?;
        // Legacy locators are admitted only inside this owned root. Conversion
        // rewrites them; no path outside the active generation is a dependency.
        let relative = if path.is_absolute() {
            path.strip_prefix(root)
                .map_err(|_| ObservationStoreError::Corrupt)?
        } else {
            path
        };
        if !relative.starts_with("conversation-content")
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(ObservationStoreError::Corrupt);
        }
        Ok(root.join(relative))
    }

    pub(crate) fn blob_path(&self, digest: &str) -> PathBuf {
        self.blob_root.join(portable_hash(digest))
    }
}

fn portable_hash(value: &str) -> String {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

pub(crate) fn store_revision(connection: &Connection) -> Result<u64, ObservationStoreError> {
    let value: String = connection
        .query_row(
            "SELECT value FROM observation_meta WHERE key='store_revision'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ObservationStoreError::Corrupt)?;
    value.parse().map_err(|_| ObservationStoreError::Corrupt)
}

pub(crate) fn increment_store_revision(
    connection: &Connection,
) -> Result<u64, ObservationStoreError> {
    let next = store_revision(connection)?
        .checked_add(1)
        .ok_or(ObservationStoreError::Corrupt)?;
    connection
        .execute(
            "UPDATE observation_meta SET value=?1 WHERE key='store_revision'",
            [next.to_string()],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|n| i64::try_from(n.as_millis()).ok())
        .unwrap_or(0);
    crate::analysis::sync(connection, 200, now_ms)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(next)
}
