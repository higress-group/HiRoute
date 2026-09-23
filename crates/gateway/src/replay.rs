//! Request-owned replay backing.
//!
//! A request has one owner and any number of independent sequential readers.
//! Small streams retain capacity-charged segments. Once the request crosses the
//! threshold, new streams and the crossing stream use a single owner-only
//! plaintext file. Paths never enter `ContentRef`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hiroute_gateway_core::runtime::body::{BodyError, MemoryRole, Reservation, StreamBudget};
use thiserror::Error;

use crate::content_ref::ContentRef;

mod secure_path;
mod stream;
#[cfg(windows)]
// Win32 token, ACL, and security-descriptor APIs are FFI-only. Keep the
// exception scoped to this module; the crate denies unsafe code everywhere
// else.
#[allow(unsafe_code)]
mod windows_acl;

pub use stream::ReplayReader;
use stream::{ReplayStream, ReplayStreamBuilder};

const DEFAULT_MEMORY_THRESHOLD: usize = 10 * 1024 * 1024;
const DEFAULT_RECORD_BYTES: usize = 16 * 1024;
const DEFAULT_ORPHAN_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const STREAM_METADATA_BYTES: usize = 1024;
const RANGE_METADATA_BYTES: usize = 256;

#[derive(Clone, Debug)]
pub struct ReplayConfig {
    pub root: PathBuf,
    pub memory_threshold_bytes: usize,
    pub record_bytes: usize,
    pub orphan_ttl: Duration,
}

impl ReplayConfig {
    pub fn production_default() -> Self {
        // System temp directories may be aliases (for example /tmp on Linux or /var
        // on macOS). Resolve that base, while leaving the replay leaf itself subject
        // to SecureRoot's owner, permission, and no-symlink checks.
        let temporary = std::env::temp_dir();
        let temporary = std::fs::canonicalize(&temporary).unwrap_or(temporary);
        Self {
            root: temporary.join("hiroute-replay"),
            memory_threshold_bytes: DEFAULT_MEMORY_THRESHOLD,
            record_bytes: DEFAULT_RECORD_BYTES,
            orphan_ttl: DEFAULT_ORPHAN_TTL,
        }
    }

    pub fn from_environment() -> Result<Self, ReplayError> {
        let mut config = Self::production_default();
        if let Some(root) = std::env::var_os("HIROUTE_REPLAY_ROOT") {
            config.root = PathBuf::from(root);
        }
        if let Some(value) = parse_environment("HIROUTE_REPLAY_MEMORY_THRESHOLD")? {
            config.memory_threshold_bytes = value;
        }
        if let Some(value) = parse_environment("HIROUTE_REPLAY_RECORD_BYTES")? {
            config.record_bytes = value;
        }
        if let Some(value) = parse_environment("HIROUTE_REPLAY_ORPHAN_TTL_MS")? {
            config.orphan_ttl = Duration::from_millis(value);
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ReplayError> {
        if !self.root.is_absolute()
            || self.memory_threshold_bytes == 0
            || self.record_bytes == 0
            || self.record_bytes > self.memory_threshold_bytes
            || self.orphan_ttl.is_zero()
        {
            return Err(ReplayError::InvalidConfig);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ReplayManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    root: Arc<secure_path::SecureRoot>,
    config: ReplayConfig,
}

#[derive(Clone)]
pub struct ReplayStore {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    manager: Arc<ManagerInner>,
    directory: Arc<secure_path::SecureDirectory>,
    budget: StreamBudget,
    streams: Mutex<BTreeMap<u64, Arc<ReplayStream>>>,
    metadata: Mutex<Vec<Reservation>>,
    next_stream: AtomicU64,
    memory_live: Arc<AtomicUsize>,
    memory_gate: Arc<Mutex<()>>,
    force_disk: Arc<AtomicBool>,
    prevalidated: AtomicBool,
    terminal: AtomicBool,
}

pub struct ReplayWriter {
    store: ReplayStore,
    builder: Option<ReplayStreamBuilder>,
}

pub struct ReplayContentWriter {
    store: ReplayStore,
    builder: Option<ReplayStreamBuilder>,
    ranges: Vec<ContentRef>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplaySnapshot {
    pub memory_retained_bytes: usize,
    pub disk_backed: bool,
    pub live_streams: usize,
    pub terminal: bool,
}

impl ReplayManager {
    pub fn open(config: ReplayConfig) -> Result<Self, ReplayError> {
        config.validate()?;
        let root = secure_path::prepare_root(&config.root, config.orphan_ttl)?;
        Ok(Self {
            inner: Arc::new(ManagerInner { root, config }),
        })
    }

    pub fn from_environment() -> Result<Self, ReplayError> {
        Self::open(ReplayConfig::from_environment()?)
    }

    pub fn begin_request(&self, budget: StreamBudget) -> Result<ReplayStore, ReplayError> {
        let base_metadata = budget.reserve(
            MemoryRole::ModelIrBacking,
            std::mem::size_of::<StoreInner>() + 512,
        )?;
        let directory = secure_path::create_request_directory(&self.inner.root)?;
        Ok(ReplayStore {
            inner: Arc::new(StoreInner {
                manager: Arc::clone(&self.inner),
                directory,
                budget,
                streams: Mutex::new(BTreeMap::new()),
                metadata: Mutex::new(vec![base_metadata]),
                next_stream: AtomicU64::new(0),
                memory_live: Arc::new(AtomicUsize::new(0)),
                memory_gate: Arc::new(Mutex::new(())),
                force_disk: Arc::new(AtomicBool::new(false)),
                prevalidated: AtomicBool::new(false),
                terminal: AtomicBool::new(false),
            }),
        })
    }

    pub fn root(&self) -> &Path {
        self.inner.root.path()
    }
}

impl ReplayStore {
    pub fn begin_raw(&self) -> Result<ReplayWriter, ReplayError> {
        self.begin_stream(MemoryRole::RawRequest)
    }

    pub fn store_content(&self, bytes: &[u8]) -> Result<ContentRef, ReplayError> {
        let mut writer = self.begin_content_pool()?;
        let reference = writer.append(bytes)?;
        writer.seal()?;
        Ok(reference)
    }

    pub fn begin_content_pool(&self) -> Result<ReplayContentWriter, ReplayError> {
        Ok(ReplayContentWriter {
            store: self.clone(),
            builder: Some(self.new_builder(MemoryRole::ModelIrBacking)?),
            ranges: Vec::new(),
        })
    }

    pub fn reader(&self, reference: &ContentRef) -> Result<ReplayReader, ReplayError> {
        let stream = lock(&self.inner.streams)
            .get(&reference.stream_ordinal())
            .cloned()
            .ok_or(ReplayError::UnknownContentRef)?;
        if !stream.contains(reference) {
            return Err(ReplayError::Integrity);
        }
        ReplayReader::open(self.clone(), stream, reference.clone())
    }

    pub(crate) fn has_reference(&self, reference: &ContentRef) -> bool {
        lock(&self.inner.streams)
            .get(&reference.stream_ordinal())
            .is_some_and(|stream| stream.contains(reference))
    }

    /// Validates every live backing through the exact file handle retained at
    /// seal time, then removes its pathname where the platform permits it.
    /// No stream may be added after this boundary; later calls only validate reference membership.
    pub fn prevalidate(&self, references: &[ContentRef]) -> Result<(), ReplayError> {
        if self.inner.prevalidated.load(Ordering::Acquire) {
            return references.iter().try_for_each(|reference| {
                let streams = lock(&self.inner.streams);
                match streams.get(&reference.stream_ordinal()) {
                    Some(stream) if stream.contains(reference) => Ok(()),
                    Some(_) => Err(ReplayError::Integrity),
                    None => Err(ReplayError::UnknownContentRef),
                }
            });
        }
        let streams = lock(&self.inner.streams)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for reference in references {
            match streams
                .iter()
                .find(|stream| stream.reference.stream_ordinal() == reference.stream_ordinal())
            {
                Some(stream) if stream.contains(reference) => {}
                Some(_) => return Err(ReplayError::Integrity),
                None => return Err(ReplayError::UnknownContentRef),
            }
        }
        for stream in &streams {
            stream.validate_backing_handle()?;
        }
        for stream in &streams {
            stream.freeze_path()?;
        }
        self.inner.prevalidated.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn is_prevalidated(&self) -> bool {
        self.inner.prevalidated.load(Ordering::Acquire)
    }

    pub fn release_stream(&self, reference: &ContentRef) -> Result<(), ReplayError> {
        let mut streams = lock(&self.inner.streams);
        match streams.get(&reference.stream_ordinal()) {
            Some(stream) if stream.reference != *reference => Err(ReplayError::Integrity),
            Some(_) => {
                streams.remove(&reference.stream_ordinal());
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub fn mark_terminal(&self) {
        self.inner.terminal.store(true, Ordering::Release);
    }

    pub fn snapshot(&self) -> ReplaySnapshot {
        ReplaySnapshot {
            memory_retained_bytes: self.inner.memory_live.load(Ordering::Acquire),
            disk_backed: self.inner.force_disk.load(Ordering::Acquire),
            live_streams: lock(&self.inner.streams).len(),
            terminal: self.inner.terminal.load(Ordering::Acquire),
        }
    }

    pub(crate) fn budget(&self) -> &StreamBudget {
        &self.inner.budget
    }

    pub(crate) fn charge_metadata(&self, bytes: usize) -> Result<(), ReplayError> {
        let reservation = self
            .inner
            .budget
            .reserve(MemoryRole::ModelIrBacking, bytes)?;
        lock(&self.inner.metadata).push(reservation);
        Ok(())
    }

    fn begin_stream(&self, role: MemoryRole) -> Result<ReplayWriter, ReplayError> {
        Ok(ReplayWriter {
            store: self.clone(),
            builder: Some(self.new_builder(role)?),
        })
    }

    fn new_builder(&self, role: MemoryRole) -> Result<ReplayStreamBuilder, ReplayError> {
        if self.inner.terminal.load(Ordering::Acquire)
            || self.inner.prevalidated.load(Ordering::Acquire)
        {
            return Err(ReplayError::AfterTerminal);
        }
        // Covers the bounded stream map entry, request-local path/name,
        // synchronization state and fixed backing metadata before allocating
        // any of them. Range-dependent metadata is charged separately.
        self.charge_metadata(STREAM_METADATA_BYTES)?;
        let ordinal = self
            .inner
            .next_stream
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .ok_or(ReplayError::LengthOverflow)?;
        ReplayStreamBuilder::new(
            Arc::clone(&self.inner.directory),
            ordinal,
            role,
            self.inner.budget.clone(),
            self.inner.manager.config.clone(),
            Arc::clone(&self.inner.memory_live),
            Arc::clone(&self.inner.memory_gate),
            Arc::clone(&self.inner.force_disk),
        )
    }

    #[cfg(test)]
    fn stream_path(&self, reference: &ContentRef) -> Option<PathBuf> {
        let streams = lock(&self.inner.streams);
        let stream = streams.get(&reference.stream_ordinal())?;
        stream.disk_path()
    }

    #[cfg(test)]
    fn truncate_stream_last_byte(&self, reference: &ContentRef) -> Result<(), ReplayError> {
        let streams = lock(&self.inner.streams);
        streams
            .get(&reference.stream_ordinal())
            .ok_or(ReplayError::UnknownContentRef)?
            .truncate_last_byte()
    }

    #[cfg(test)]
    fn stream_backing_bytes(&self, reference: &ContentRef) -> Result<Vec<u8>, ReplayError> {
        let streams = lock(&self.inner.streams);
        streams
            .get(&reference.stream_ordinal())
            .ok_or(ReplayError::UnknownContentRef)?
            .backing_bytes()
    }

    #[cfg(test)]
    fn tamper_stream_last_byte(&self, reference: &ContentRef) -> Result<(), ReplayError> {
        let streams = lock(&self.inner.streams);
        streams
            .get(&reference.stream_ordinal())
            .ok_or(ReplayError::UnknownContentRef)?
            .tamper_last_byte()
    }

    #[cfg(test)]
    fn request_directory(&self) -> &Path {
        self.inner.directory.path()
    }
}

impl ReplayWriter {
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), ReplayError> {
        let result = self
            .builder
            .as_mut()
            .ok_or(ReplayError::AfterTerminal)?
            .append(bytes);
        if result.is_err() {
            self.builder.take();
        }
        result
    }

    pub fn seal(mut self) -> Result<ContentRef, ReplayError> {
        let builder = self.builder.take().ok_or(ReplayError::AfterTerminal)?;
        let (stream, disk_backed) = builder.seal(Vec::new())?;
        let reference = stream.reference.clone();
        let mut streams = lock(&self.store.inner.streams);
        if streams.insert(reference.stream_ordinal(), stream).is_some() {
            return Err(ReplayError::Integrity);
        }
        if disk_backed {
            self.store.inner.force_disk.store(true, Ordering::Release);
        }
        Ok(reference)
    }
}

impl ReplayContentWriter {
    pub fn append(&mut self, bytes: &[u8]) -> Result<ContentRef, ReplayError> {
        self.reserve_range()?;
        let reference = self
            .builder
            .as_mut()
            .ok_or(ReplayError::AfterTerminal)?
            .append_range(bytes)?;
        self.ranges.push(reference.clone());
        Ok(reference)
    }

    pub fn append_json(
        &mut self,
        value: &serde_json::Value,
        canonical: bool,
    ) -> Result<ContentRef, ReplayError> {
        self.reserve_range()?;
        let reference = self
            .builder
            .as_mut()
            .ok_or(ReplayError::AfterTerminal)?
            .append_json_range(value, canonical)?;
        self.ranges.push(reference.clone());
        Ok(reference)
    }

    fn reserve_range(&self) -> Result<(), ReplayError> {
        self.store.charge_metadata(RANGE_METADATA_BYTES)
    }

    pub fn seal(mut self) -> Result<(), ReplayError> {
        let builder = self.builder.take().ok_or(ReplayError::AfterTerminal)?;
        if self.ranges.is_empty() {
            return Err(ReplayError::Integrity);
        }
        let (stream, disk_backed) = builder.seal(std::mem::take(&mut self.ranges))?;
        let ordinal = stream.reference.stream_ordinal();
        let mut streams = lock(&self.store.inner.streams);
        if streams.insert(ordinal, stream).is_some() {
            return Err(ReplayError::Integrity);
        }
        if disk_backed {
            self.store.inner.force_disk.store(true, Ordering::Release);
        }
        Ok(())
    }
}

impl Drop for ReplayWriter {
    fn drop(&mut self) {
        // Close a provisional Windows handle before the store's last-owner
        // cleanup runs; owner-only files are opened with sharing disabled.
        self.builder.take();
    }
}

impl Drop for ReplayContentWriter {
    fn drop(&mut self) {
        self.builder.take();
    }
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        lock(&self.streams).clear();
        let _ = secure_path::remove_request_directory(&self.directory);
    }
}

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("replay configuration is invalid")]
    InvalidConfig,
    #[error("OS cryptographic randomness is unavailable")]
    RandomUnavailable,
    #[error("replay content integrity validation failed")]
    Integrity,
    #[error("replay path is unsafe")]
    UnsafePath,
    #[error("replay path escaped its owner root")]
    PathEscape,
    #[error("replay permissions are not owner-only")]
    UnsafePermissions,
    #[error("replay secure-name allocation was exhausted")]
    NameExhausted,
    #[error("replay length overflow")]
    LengthOverflow,
    #[error("replay content is not valid UTF-8")]
    InvalidUtf8,
    #[error("replay request structure exceeds its bounded metadata contract")]
    StructureLimit,
    #[error("model IR JSON cannot be serialized for replay")]
    InvalidJson,
    #[error("unknown replay content reference")]
    UnknownContentRef,
    #[error("replay owner is already terminal")]
    AfterTerminal,
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn parse_environment<T: std::str::FromStr>(name: &str) -> Result<Option<T>, ReplayError> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(None);
    };
    value
        .into_string()
        .map_err(|_| ReplayError::InvalidConfig)?
        .parse()
        .map(Some)
        .map_err(|_| ReplayError::InvalidConfig)
}

#[cfg(test)]
#[path = "replay/tests.rs"]
mod tests;
