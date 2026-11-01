//! Encoded observation overflow. Temporary writes never acknowledge durable content.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::{ReplayError, ReplayStore, lock, secure_path};

const MAGIC: &[u8; 8] = b"HRCTSP01";
const HEADER_BYTES: usize = 24;
const DIGEST_BYTES: usize = 32;
const IO_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) struct OverflowFactory {
    root: Arc<secure_path::SecureRoot>,
}

impl ReplayStore {
    /// Share only the secure root, not the request's raw streams or their TTL.
    pub(crate) fn overflow_factory(&self) -> OverflowFactory {
        OverflowFactory {
            root: Arc::clone(&self.inner.manager.root),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OverflowStats {
    pub reserved_disk_bytes: usize,
    pub retained_file_bytes: u64,
    pub written_bytes: u64,
    pub read_bytes: u64,
    pub peak_reserved_bytes: usize,
    pub live_segments: usize,
    pub created_segments: usize,
    pub cleanup_failures: usize,
}

pub(crate) struct OverflowQuota {
    limit: usize,
    reserved: AtomicUsize,
    retained: AtomicU64,
    written: AtomicU64,
    read: AtomicU64,
    peak: AtomicUsize,
    live_segments: AtomicUsize,
    created_segments: AtomicUsize,
    cleanup_failures: AtomicUsize,
}

impl OverflowQuota {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            reserved: AtomicUsize::new(0),
            retained: AtomicU64::new(0),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            peak: AtomicUsize::new(0),
            live_segments: AtomicUsize::new(0),
            created_segments: AtomicUsize::new(0),
            cleanup_failures: AtomicUsize::new(0),
        })
    }

    fn reserve(&self, bytes: usize) -> Result<(), ReplayError> {
        let mut live = self.reserved.load(Ordering::Acquire);
        loop {
            let next = live.checked_add(bytes).ok_or(ReplayError::LengthOverflow)?;
            if next > self.limit {
                return Err(ReplayError::StructureLimit);
            }
            match self.reserved.compare_exchange_weak(
                live,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.peak.fetch_max(next, Ordering::Relaxed);
                    return Ok(());
                }
                Err(actual) => live = actual,
            }
        }
    }

    fn released(&self, reserved: usize, retained: u64) {
        self.reserved.fetch_sub(reserved, Ordering::AcqRel);
        self.retained.fetch_sub(retained, Ordering::AcqRel);
    }

    pub(crate) fn snapshot(&self) -> OverflowStats {
        OverflowStats {
            reserved_disk_bytes: self.reserved.load(Ordering::Acquire),
            retained_file_bytes: self.retained.load(Ordering::Acquire),
            written_bytes: self.written.load(Ordering::Relaxed),
            read_bytes: self.read.load(Ordering::Relaxed),
            peak_reserved_bytes: self.peak.load(Ordering::Relaxed),
            live_segments: self.live_segments.load(Ordering::Acquire),
            created_segments: self.created_segments.load(Ordering::Relaxed),
            cleanup_failures: self.cleanup_failures.load(Ordering::Relaxed),
        }
    }
}

struct DirectoryOwner {
    directory: Arc<secure_path::SecureDirectory>,
    quota: Arc<OverflowQuota>,
    failed_reserved: AtomicUsize,
    failed_retained: AtomicU64,
}

impl Drop for DirectoryOwner {
    fn drop(&mut self) {
        // All segment handles have closed before the last directory owner.
        // Do not report disk released when owner cleanup actually failed.
        if secure_path::remove_request_directory(&self.directory).is_ok() {
            self.quota.released(
                self.failed_reserved.load(Ordering::Relaxed),
                self.failed_retained.load(Ordering::Relaxed),
            );
        } else {
            self.quota.cleanup_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct Segment {
    directory: Arc<DirectoryOwner>,
    entry: secure_path::SecureFile,
    file: Mutex<Option<File>>,
    reserved: AtomicUsize,
    retained: AtomicU64,
}

impl Segment {
    fn note_length(&self, length: u64) {
        let previous = self.retained.fetch_max(length, Ordering::AcqRel);
        if length > previous {
            self.directory
                .quota
                .retained
                .fetch_add(length - previous, Ordering::AcqRel);
        }
    }

    fn write_at(&self, offset: u64, bytes: &[u8]) -> Result<(), ReplayError> {
        for (index, part) in bytes.chunks(IO_BYTES).enumerate() {
            let at = offset
                .checked_add((index * IO_BYTES) as u64)
                .ok_or(ReplayError::LengthOverflow)?;
            let mut handle = lock(&self.file);
            let file = handle.as_mut().ok_or(ReplayError::AfterTerminal)?;
            file.seek(SeekFrom::Start(at))?;
            if let Err(error) = file.write_all(part) {
                if let Ok(metadata) = file.metadata() {
                    self.note_length(metadata.len());
                }
                return Err(error.into());
            }
            self.note_length(at + part.len() as u64);
            self.directory
                .quota
                .written
                .fetch_add(part.len() as u64, Ordering::Relaxed);
        }
        Ok(())
    }

    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), ReplayError> {
        for (index, part) in bytes.chunks_mut(IO_BYTES).enumerate() {
            let at = offset
                .checked_add((index * IO_BYTES) as u64)
                .ok_or(ReplayError::LengthOverflow)?;
            let mut handle = lock(&self.file);
            let file = handle.as_mut().ok_or(ReplayError::AfterTerminal)?;
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(part)?;
            self.directory
                .quota
                .read
                .fetch_add(part.len() as u64, Ordering::Relaxed);
        }
        Ok(())
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        let file = self
            .file
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let removed = file.as_ref().is_some_and(|file| {
            secure_path::remove_file(&self.directory.directory, &self.entry, file).is_ok()
        });
        drop(file);
        let reserved = self.reserved.load(Ordering::Relaxed);
        let retained = self.retained.load(Ordering::Relaxed);
        if removed {
            self.directory.quota.released(reserved, retained);
        } else {
            self.directory
                .failed_reserved
                .fetch_add(reserved, Ordering::Relaxed);
            self.directory
                .failed_retained
                .fetch_add(retained, Ordering::Relaxed);
            self.directory
                .quota
                .cleanup_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        self.directory
            .quota
            .live_segments
            .fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub(crate) struct OverflowRange {
    segment: Arc<Segment>,
    offset: u64,
    length: usize,
    sequence: u64,
    digest: [u8; DIGEST_BYTES],
}

impl OverflowRange {
    pub(crate) fn len(&self) -> usize {
        self.length
    }

    /// The caller owns independent drain scratch. This never needs queue credit.
    pub(crate) fn read(&self) -> Result<Vec<u8>, ReplayError> {
        let mut header = [0; HEADER_BYTES];
        self.segment.read_at(self.offset, &mut header)?;
        if header[..8] != *MAGIC
            || u64::from_be_bytes(header[8..16].try_into().unwrap()) != self.sequence
            || u64::from_be_bytes(header[16..24].try_into().unwrap()) != self.length as u64
        {
            return Err(ReplayError::Integrity);
        }
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(self.length)
            .map_err(|_| std::io::Error::other("content drain allocation unavailable"))?;
        payload.resize(self.length, 0);
        let payload_at = self
            .offset
            .checked_add(HEADER_BYTES as u64)
            .ok_or(ReplayError::LengthOverflow)?;
        self.segment.read_at(payload_at, &mut payload)?;
        let mut trailer = [0; DIGEST_BYTES];
        self.segment.read_at(
            payload_at
                .checked_add(self.length as u64)
                .ok_or(ReplayError::LengthOverflow)?,
            &mut trailer,
        )?;
        let mut hash = Sha256::new();
        hash.update(header);
        hash.update(&payload);
        let digest: [u8; DIGEST_BYTES] = hash.finalize().into();
        if digest != self.digest || digest != trailer {
            return Err(ReplayError::Integrity);
        }
        Ok(payload)
    }
}

struct WriterState {
    directory: Option<Arc<DirectoryOwner>>,
    segment: Option<Arc<Segment>>,
    offset: usize,
}

pub(crate) struct OverflowWriter {
    factory: OverflowFactory,
    quota: Arc<OverflowQuota>,
    segment_bytes: usize,
    state: Mutex<WriterState>,
}

impl OverflowFactory {
    pub(crate) fn writer(&self, quota: Arc<OverflowQuota>, segment_bytes: usize) -> OverflowWriter {
        OverflowWriter {
            factory: self.clone(),
            quota,
            segment_bytes: segment_bytes.max(HEADER_BYTES + DIGEST_BYTES),
            state: Mutex::new(WriterState {
                directory: None,
                segment: None,
                offset: 0,
            }),
        }
    }
}

impl OverflowWriter {
    pub(crate) fn append(
        &self,
        sequence: u64,
        payload: &[u8],
    ) -> Result<OverflowRange, ReplayError> {
        let length = HEADER_BYTES
            .checked_add(payload.len())
            .and_then(|bytes| bytes.checked_add(DIGEST_BYTES))
            .ok_or(ReplayError::LengthOverflow)?;
        let (segment, offset) = {
            let mut state = lock(&self.state);
            if state.directory.is_none() {
                state.directory = Some(Arc::new(DirectoryOwner {
                    directory: secure_path::create_request_directory(&self.factory.root)?,
                    quota: Arc::clone(&self.quota),
                    failed_reserved: AtomicUsize::new(0),
                    failed_retained: AtomicU64::new(0),
                }));
            }
            if state.segment.is_none()
                || (state.offset != 0 && state.offset.saturating_add(length) > self.segment_bytes)
            {
                let directory = Arc::clone(state.directory.as_ref().unwrap());
                let (entry, file) = secure_path::create_file(&directory.directory, "content")?;
                self.quota.live_segments.fetch_add(1, Ordering::AcqRel);
                self.quota.created_segments.fetch_add(1, Ordering::Relaxed);
                state.segment = Some(Arc::new(Segment {
                    directory,
                    entry,
                    file: Mutex::new(Some(file)),
                    reserved: AtomicUsize::new(0),
                    retained: AtomicU64::new(0),
                }));
                state.offset = 0;
            }
            let next = state
                .offset
                .checked_add(length)
                .ok_or(ReplayError::LengthOverflow)?;
            self.quota.reserve(length)?;
            let segment = Arc::clone(state.segment.as_ref().unwrap());
            segment.reserved.fetch_add(length, Ordering::Relaxed);
            let offset = state.offset as u64;
            state.offset = next;
            (segment, offset)
        };
        // No sequence, pending, quota, or writer-state lock spans encoded IO.
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..16].copy_from_slice(&sequence.to_be_bytes());
        header[16..24].copy_from_slice(&(payload.len() as u64).to_be_bytes());
        let mut hash = Sha256::new();
        hash.update(header);
        hash.update(payload);
        let digest: [u8; DIGEST_BYTES] = hash.finalize().into();
        segment.write_at(offset, &header)?;
        let payload_at = offset
            .checked_add(HEADER_BYTES as u64)
            .ok_or(ReplayError::LengthOverflow)?;
        segment.write_at(payload_at, payload)?;
        segment.write_at(
            payload_at
                .checked_add(payload.len() as u64)
                .ok_or(ReplayError::LengthOverflow)?,
            &digest,
        )?;
        {
            let mut handle = lock(&segment.file);
            handle.as_mut().ok_or(ReplayError::AfterTerminal)?.flush()?;
        }
        Ok(OverflowRange {
            segment,
            offset,
            length: payload.len(),
            sequence,
            digest,
        })
    }

    /// Release the writer's current segment; ranges keep only their own backing.
    pub(crate) fn seal(&self) {
        let mut state = lock(&self.state);
        state.segment = None;
        state.offset = 0;
    }
}

#[cfg(test)]
#[path = "overflow_tests.rs"]
mod tests;
