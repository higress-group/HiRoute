use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::content_ref::ContentRef;
use hiroute_gateway_core::runtime::body::{
    ChargedBytes, ChargedBytesBuilder, MemoryRole, Reservation, StreamBudget,
};

use super::secure_path;
use super::{ReplayConfig, ReplayError, ReplayStore};

pub(super) struct ReplayStreamBuilder {
    directory: Arc<secure_path::SecureDirectory>,
    ordinal: u64,
    role: MemoryRole,
    budget: StreamBudget,
    config: ReplayConfig,
    memory_counter: Arc<AtomicUsize>,
    memory_gate: Arc<Mutex<()>>,
    force_disk: Arc<AtomicBool>,
    mode: BuilderMode,
    total_len: u64,
    json_escaped_len: u64,
}

enum BuilderMode {
    Memory(MemoryBuffer),
    Disk(ProvisionalDisk),
}

struct MemoryBuffer {
    segments: Vec<ChargedBytes>,
    metadata: Vec<Reservation>,
    pending: Option<ChargedBytesBuilder>,
    retained: usize,
    memory_counter: Arc<AtomicUsize>,
}

struct ProvisionalDisk {
    path: secure_path::SecureFile,
    file: File,
}

pub(super) struct ReplayStream {
    pub(super) reference: ContentRef,
    ranges: Arc<[ContentRef]>,
    role: MemoryRole,
    read_buffer_bytes: usize,
    directory: Arc<secure_path::SecureDirectory>,
    memory_counter: Arc<AtomicUsize>,
    backing: StreamBacking,
}

impl ReplayStream {
    pub(super) fn contains(&self, reference: &ContentRef) -> bool {
        *reference == self.reference || self.ranges.binary_search(reference).is_ok()
    }

    #[cfg(test)]
    pub(super) fn disk_path(&self) -> Option<PathBuf> {
        match &self.backing {
            StreamBacking::Memory { .. } => None,
            StreamBacking::Disk { path, .. } => {
                lock(path).as_ref().map(|path| path.path().to_path_buf())
            }
        }
    }

    #[cfg(test)]
    pub(super) fn truncate_last_byte(&self) -> Result<(), ReplayError> {
        let StreamBacking::Disk { file, .. } = &self.backing else {
            return Err(ReplayError::Integrity);
        };
        let file = lock(file);
        let file = file.as_ref().ok_or(ReplayError::Integrity)?;
        let len = file.metadata()?.len();
        file.set_len(len.checked_sub(1).ok_or(ReplayError::Integrity)?)?;
        file.sync_all()?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> Result<Vec<u8>, ReplayError> {
        let StreamBacking::Disk { file, .. } = &self.backing else {
            return Err(ReplayError::Integrity);
        };
        let mut file = lock(file);
        let file = file.as_mut().ok_or(ReplayError::Integrity)?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    #[cfg(test)]
    pub(super) fn tamper_last_byte(&self) -> Result<(), ReplayError> {
        let StreamBacking::Disk { file, .. } = &self.backing else {
            return Err(ReplayError::Integrity);
        };
        let mut file = lock(file);
        let file = file.as_mut().ok_or(ReplayError::Integrity)?;
        let position = file
            .metadata()?
            .len()
            .checked_sub(1)
            .ok_or(ReplayError::Integrity)?;
        file.seek(SeekFrom::Start(position))?;
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte)?;
        byte[0] ^= 0x80;
        file.seek(SeekFrom::Start(position))?;
        file.write_all(&byte)?;
        file.sync_all()?;
        Ok(())
    }

    pub(super) fn freeze_path(&self) -> Result<(), ReplayError> {
        let StreamBacking::Disk { path, file, .. } = &self.backing else {
            return Ok(());
        };
        let mut path_guard = lock(path);
        let file_guard = lock(file);
        if let (Some(path), Some(file)) = (path_guard.as_ref(), file_guard.as_ref()) {
            secure_path::remove_file(&self.directory, path, file)?;
            path_guard.take();
        }
        Ok(())
    }

    pub(super) fn validate_backing_handle(&self) -> Result<(), ReplayError> {
        let StreamBacking::Disk { file, .. } = &self.backing else {
            return Ok(());
        };
        let file = lock(file);
        secure_path::validate_owner_file_handle(file.as_ref().ok_or(ReplayError::Integrity)?)
    }
}

enum StreamBacking {
    Memory {
        segments: Vec<ChargedBytes>,
        _metadata: Vec<Reservation>,
        retained: usize,
    },
    Disk {
        path: Mutex<Option<secure_path::SecureFile>>,
        file: Mutex<Option<File>>,
    },
}

pub struct ReplayReader {
    stream: Arc<ReplayStream>,
    _store: ReplayStore,
    reference: ContentRef,
    state: ReaderState,
    observed_len: u64,
    verified: bool,
    #[cfg(test)]
    disk_reads: usize,
}

enum ReaderState {
    Memory {
        segment: usize,
        offset: usize,
        remaining: u64,
    },
    Disk {
        position: u64,
        remaining: u64,
        buffer: Vec<u8>,
        buffer_offset: usize,
        buffer_len: usize,
        _buffer_reservation: Reservation,
    },
}

impl ReplayStreamBuilder {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        directory: Arc<secure_path::SecureDirectory>,
        ordinal: u64,
        role: MemoryRole,
        budget: StreamBudget,
        config: ReplayConfig,
        memory_counter: Arc<AtomicUsize>,
        memory_gate: Arc<Mutex<()>>,
        force_disk: Arc<AtomicBool>,
    ) -> Result<Self, ReplayError> {
        let mode = if force_disk.load(Ordering::Acquire) {
            BuilderMode::Disk(ProvisionalDisk::create(&directory)?)
        } else {
            BuilderMode::Memory(MemoryBuffer {
                segments: Vec::new(),
                metadata: Vec::new(),
                pending: None,
                retained: 0,
                memory_counter: Arc::clone(&memory_counter),
            })
        };
        Ok(Self {
            directory,
            ordinal,
            role,
            budget,
            config,
            memory_counter,
            memory_gate,
            force_disk,
            mode,
            total_len: 0,
            json_escaped_len: 0,
        })
    }

    pub(super) fn append(&mut self, bytes: &[u8]) -> Result<(), ReplayError> {
        self.total_len = self
            .total_len
            .checked_add(bytes.len() as u64)
            .ok_or(ReplayError::LengthOverflow)?;
        self.json_escaped_len = self
            .json_escaped_len
            .checked_add(json_escaped_len(bytes)?)
            .ok_or(ReplayError::LengthOverflow)?;
        if matches!(self.mode, BuilderMode::Memory(_)) {
            let gate = Arc::clone(&self.memory_gate);
            let _guard = lock(&gate);
            let memory_can_retain = match &self.mode {
                BuilderMode::Memory(memory) => {
                    let global_remaining = self
                        .config
                        .memory_threshold_bytes
                        .saturating_sub(self.memory_counter.load(Ordering::Acquire));
                    memory
                        .available_capacity()
                        .checked_add(global_remaining)
                        .is_some_and(|capacity| capacity >= bytes.len())
                }
                BuilderMode::Disk(_) => false,
            };
            if self.force_disk.load(Ordering::Acquire) || !memory_can_retain {
                self.force_disk.store(true, Ordering::Release);
                self.migrate_to_disk()?;
            }
            if let BuilderMode::Memory(memory) = &mut self.mode {
                memory.append(
                    bytes,
                    &self.budget,
                    self.role,
                    self.config.record_bytes,
                    self.config.memory_threshold_bytes,
                )?;
                return Ok(());
            }
        }
        let BuilderMode::Disk(disk) = &mut self.mode else {
            return Err(ReplayError::Integrity);
        };
        disk.append(bytes)?;
        Ok(())
    }

    pub(super) fn append_range(&mut self, bytes: &[u8]) -> Result<ContentRef, ReplayError> {
        let mut range = ReplayRangeSink::new(self);
        range
            .write_all(bytes)
            .map_err(|_| ReplayError::Io(std::io::Error::other("replay range write failed")))?;
        range.finish()
    }

    pub(super) fn append_json_range(
        &mut self,
        value: &serde_json::Value,
        canonical: bool,
    ) -> Result<ContentRef, ReplayError> {
        let mut range = ReplayRangeSink::new(self);
        let result = if canonical {
            write_canonical_json(&mut range, value)
        } else {
            serde_json::to_writer(&mut range, value)
                .map_err(|error| std::io::Error::other(error.to_string()))
        };
        if result.is_err() {
            return Err(range.error.take().unwrap_or(ReplayError::InvalidJson));
        }
        range.finish()
    }

    pub(super) fn seal(
        self,
        mut ranges: Vec<ContentRef>,
    ) -> Result<(Arc<ReplayStream>, bool), ReplayError> {
        let reference = ContentRef::new(self.ordinal, 0, self.total_len, self.json_escaped_len);
        if ranges.is_empty() {
            ranges.push(reference.clone());
        }
        ranges.sort();
        let mut previous_end = 0_u64;
        for range in &ranges {
            let end = range
                .byte_offset()
                .checked_add(range.byte_len())
                .ok_or(ReplayError::LengthOverflow)?;
            if range.stream_ordinal() != self.ordinal
                || range.byte_offset() < previous_end
                || end > self.total_len
            {
                return Err(ReplayError::Integrity);
            }
            previous_end = end;
        }
        let (backing, disk_backed) = match self.mode {
            BuilderMode::Memory(memory) => (memory.into_backing(), false),
            BuilderMode::Disk(disk) => (disk.seal(self.total_len)?, true),
        };
        Ok((
            Arc::new(ReplayStream {
                reference,
                ranges: ranges.into(),
                role: self.role,
                read_buffer_bytes: self.config.record_bytes,
                directory: self.directory,
                memory_counter: self.memory_counter,
                backing,
            }),
            disk_backed,
        ))
    }

    fn migrate_to_disk(&mut self) -> Result<(), ReplayError> {
        let mut disk = ProvisionalDisk::create(&self.directory)?;
        let BuilderMode::Memory(mut memory) = std::mem::replace(
            &mut self.mode,
            BuilderMode::Memory(MemoryBuffer {
                segments: Vec::new(),
                metadata: Vec::new(),
                pending: None,
                retained: 0,
                memory_counter: Arc::clone(&self.memory_counter),
            }),
        ) else {
            return Ok(());
        };
        memory.finish_pending();
        for segment in &memory.segments {
            disk.append(segment.bytes())?;
        }
        drop(memory);
        self.mode = BuilderMode::Disk(disk);
        Ok(())
    }
}

impl MemoryBuffer {
    fn append(
        &mut self,
        mut bytes: &[u8],
        budget: &StreamBudget,
        role: MemoryRole,
        segment_bytes: usize,
        threshold: usize,
    ) -> Result<(), ReplayError> {
        while !bytes.is_empty() {
            if self.pending.is_none() {
                let capacity = segment_bytes
                    .min(threshold.saturating_sub(self.memory_counter.load(Ordering::Acquire)));
                if capacity == 0 {
                    return Err(ReplayError::LengthOverflow);
                }
                let metadata = budget.reserve(
                    MemoryRole::ModelIrBacking,
                    std::mem::size_of::<ChargedBytes>() + std::mem::size_of::<Reservation>() + 64,
                )?;
                let pending = ChargedBytesBuilder::new(budget, role, capacity)?;
                self.retained = self
                    .retained
                    .checked_add(capacity)
                    .ok_or(ReplayError::LengthOverflow)?;
                self.memory_counter.fetch_add(capacity, Ordering::AcqRel);
                self.metadata.push(metadata);
                self.pending = Some(pending);
            }
            let pending = self.pending.as_mut().expect("pending segment was created");
            let available = pending.capacity() - pending.len();
            let count = available.min(bytes.len());
            pending.extend_from_slice(&bytes[..count])?;
            bytes = &bytes[count..];
            if pending.len() == pending.capacity() {
                self.finish_pending();
            }
        }
        Ok(())
    }

    fn available_capacity(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |pending| pending.capacity() - pending.len())
    }

    fn finish_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.segments.push(pending.finish());
        }
    }

    fn into_backing(mut self) -> StreamBacking {
        self.finish_pending();
        let retained = self.retained;
        self.retained = 0;
        StreamBacking::Memory {
            segments: std::mem::take(&mut self.segments),
            _metadata: std::mem::take(&mut self.metadata),
            retained,
        }
    }
}

impl Drop for MemoryBuffer {
    fn drop(&mut self) {
        if self.retained != 0 {
            self.memory_counter
                .fetch_sub(self.retained, Ordering::AcqRel);
        }
    }
}

impl ProvisionalDisk {
    fn create(directory: &secure_path::SecureDirectory) -> Result<Self, ReplayError> {
        let (path, file) = secure_path::create_file(directory, "replay")?;
        Ok(Self { path, file })
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), ReplayError> {
        self.file.write_all(bytes)?;
        Ok(())
    }

    fn seal(self, total_len: u64) -> Result<StreamBacking, ReplayError> {
        self.file.sync_all()?;
        if self.file.metadata()?.len() != total_len {
            return Err(ReplayError::Integrity);
        }
        secure_path::validate_owner_file_handle(&self.file)?;
        Ok(StreamBacking::Disk {
            path: Mutex::new(Some(self.path)),
            file: Mutex::new(Some(self.file)),
        })
    }
}

impl ReplayReader {
    pub(super) fn open(
        store: ReplayStore,
        stream: Arc<ReplayStream>,
        reference: ContentRef,
    ) -> Result<Self, ReplayError> {
        if !stream.contains(&reference) {
            return Err(ReplayError::Integrity);
        }
        let state = match &stream.backing {
            StreamBacking::Memory { segments, .. } => {
                let mut remaining_offset = usize::try_from(reference.byte_offset())
                    .map_err(|_| ReplayError::LengthOverflow)?;
                let mut segment = 0;
                while let Some(bytes) = segments.get(segment).map(ChargedBytes::bytes) {
                    if remaining_offset < bytes.len() {
                        break;
                    }
                    remaining_offset -= bytes.len();
                    segment += 1;
                }
                ReaderState::Memory {
                    segment,
                    offset: remaining_offset,
                    remaining: reference.byte_len(),
                }
            }
            StreamBacking::Disk { file, .. } => {
                if lock(file).is_none() {
                    return Err(ReplayError::Integrity);
                }
                let buffer_bytes = stream
                    .read_buffer_bytes
                    .min(usize::try_from(reference.byte_len()).unwrap_or(usize::MAX));
                let buffer_reservation = store.budget().reserve(stream.role, buffer_bytes)?;
                ReaderState::Disk {
                    position: reference.byte_offset(),
                    remaining: reference.byte_len(),
                    buffer: vec![0_u8; buffer_bytes],
                    buffer_offset: 0,
                    buffer_len: 0,
                    _buffer_reservation: buffer_reservation,
                }
            }
        };
        Ok(Self {
            stream,
            _store: store,
            reference,
            state,
            observed_len: 0,
            verified: false,
            #[cfg(test)]
            disk_reads: 0,
        })
    }

    pub fn next_charged(
        &mut self,
        budget: &StreamBudget,
        role: MemoryRole,
        max_bytes: usize,
    ) -> Result<Option<ChargedBytes>, ReplayError> {
        if max_bytes == 0 {
            return Err(ReplayError::InvalidConfig);
        }
        let mut output = ChargedBytesBuilder::new(budget, role, max_bytes)?;
        output.resize_zeroed(max_bytes)?;
        let read = self.read_verified(output.as_mut_slice())?;
        if read == 0 {
            return Ok(None);
        }
        output.truncate(read);
        Ok(Some(output.finish()))
    }

    pub fn verify_terminal(&mut self) -> Result<(), ReplayError> {
        let mut sink = [0_u8; 4096];
        while self.read_verified(&mut sink)? != 0 {}
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn disk_read_count(&self) -> usize {
        self.disk_reads
    }

    fn read_verified(&mut self, output: &mut [u8]) -> Result<usize, ReplayError> {
        if output.is_empty() {
            return Ok(0);
        }
        let read = match &mut self.state {
            ReaderState::Memory {
                segment,
                offset,
                remaining,
            } => {
                if *remaining == 0 {
                    self.verify_length()?;
                    return Ok(0);
                }
                let StreamBacking::Memory { segments, .. } = &self.stream.backing else {
                    return Err(ReplayError::Integrity);
                };
                let Some(source) = segments.get(*segment) else {
                    self.verify_length()?;
                    return Ok(0);
                };
                let available = &source.bytes()[*offset..];
                let count = available
                    .len()
                    .min(output.len())
                    .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
                output[..count].copy_from_slice(&available[..count]);
                *offset += count;
                *remaining -= count as u64;
                if *offset == source.bytes().len() {
                    *segment += 1;
                    *offset = 0;
                }
                count
            }
            ReaderState::Disk { .. } => self.read_disk(output)?,
        };
        self.observed_len = self
            .observed_len
            .checked_add(read as u64)
            .ok_or(ReplayError::LengthOverflow)?;
        let finished = match &self.state {
            ReaderState::Memory { remaining, .. } | ReaderState::Disk { remaining, .. } => {
                *remaining == 0
            }
        };
        if finished {
            self.verify_length()?;
        }
        Ok(read)
    }

    fn read_disk(&mut self, output: &mut [u8]) -> Result<usize, ReplayError> {
        let StreamBacking::Disk { file, .. } = &self.stream.backing else {
            return Err(ReplayError::Integrity);
        };
        let ReaderState::Disk {
            position,
            remaining,
            buffer,
            buffer_offset,
            buffer_len,
            ..
        } = &mut self.state
        else {
            return Err(ReplayError::Integrity);
        };
        if *remaining == 0 {
            return Ok(0);
        }
        if *buffer_offset == *buffer_len {
            let count = buffer
                .len()
                .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
            if count == 0 {
                return Err(ReplayError::Integrity);
            }
            let mut pinned_guard = lock(file);
            let pinned = pinned_guard.as_mut().ok_or(ReplayError::Integrity)?;
            pinned.seek(SeekFrom::Start(*position))?;
            let read = pinned.read(&mut buffer[..count])?;
            if read == 0 {
                return Err(ReplayError::Integrity);
            }
            *position = position
                .checked_add(read as u64)
                .ok_or(ReplayError::LengthOverflow)?;
            *buffer_offset = 0;
            *buffer_len = read;
            #[cfg(test)]
            {
                self.disk_reads += 1;
            }
        }
        let available = &buffer[*buffer_offset..*buffer_len];
        let count = available
            .len()
            .min(output.len())
            .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
        output[..count].copy_from_slice(&available[..count]);
        *buffer_offset += count;
        *remaining -= count as u64;
        Ok(count)
    }

    fn verify_length(&mut self) -> Result<(), ReplayError> {
        if self.verified {
            return Ok(());
        }
        let backing_len_matches = match &self.stream.backing {
            StreamBacking::Disk { file, .. } if self.reference == self.stream.reference => {
                lock(file)
                    .as_ref()
                    .ok_or(ReplayError::Integrity)?
                    .metadata()?
                    .len()
                    == self.reference.byte_len()
            }
            _ => true,
        };
        if self.observed_len != self.reference.byte_len() || !backing_len_matches {
            return Err(ReplayError::Integrity);
        }
        self.verified = true;
        Ok(())
    }
}

impl Read for ReplayReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.read_verified(output)
            .map_err(|error| std::io::Error::other(error.to_string()))
    }
}

impl Drop for ReplayStream {
    fn drop(&mut self) {
        match &mut self.backing {
            StreamBacking::Memory { retained, .. } => {
                self.memory_counter.fetch_sub(*retained, Ordering::AcqRel);
            }
            StreamBacking::Disk { path, file, .. } => {
                let mut path_guard = lock(path);
                let file_guard = lock(file);
                if let (Some(path), Some(file)) = (path_guard.as_ref(), file_guard.as_ref()) {
                    let _ = secure_path::remove_file(&self.directory, path, file);
                    path_guard.take();
                }
                drop(file_guard);
                lock(file).take();
            }
        }
    }
}

fn json_escaped_len(bytes: &[u8]) -> Result<u64, ReplayError> {
    bytes.iter().try_fold(0_u64, |total, byte| {
        let width = match byte {
            b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
            0x00..=0x1f => 6,
            _ => 1,
        };
        total.checked_add(width).ok_or(ReplayError::LengthOverflow)
    })
}

struct ReplayRangeSink<'a> {
    builder: &'a mut ReplayStreamBuilder,
    byte_offset: u64,
    byte_len: u64,
    json_escaped_len: u64,
    error: Option<ReplayError>,
}

impl<'a> ReplayRangeSink<'a> {
    fn new(builder: &'a mut ReplayStreamBuilder) -> Self {
        let byte_offset = builder.total_len;
        Self {
            builder,
            byte_offset,
            byte_len: 0,
            json_escaped_len: 0,
            error: None,
        }
    }

    fn finish(self) -> Result<ContentRef, ReplayError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(ContentRef::new(
            self.builder.ordinal,
            self.byte_offset,
            self.byte_len,
            self.json_escaped_len,
        ))
    }
}

impl Write for ReplayRangeSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.error.is_some() {
            return Err(std::io::Error::other("replay range is poisoned"));
        }
        let byte_len = self
            .byte_len
            .checked_add(bytes.len() as u64)
            .ok_or(ReplayError::LengthOverflow);
        let escaped = json_escaped_len(bytes).and_then(|value| {
            self.json_escaped_len
                .checked_add(value)
                .ok_or(ReplayError::LengthOverflow)
        });
        let result = byte_len.and_then(|byte_len| escaped.map(|escaped| (byte_len, escaped)));
        let (byte_len, escaped) = match result {
            Ok(lengths) => lengths,
            Err(error) => {
                self.error = Some(error);
                return Err(std::io::Error::other("replay range length overflow"));
            }
        };
        if let Err(error) = self.builder.append(bytes) {
            self.error = Some(error);
            return Err(std::io::Error::other("replay range backing write failed"));
        }
        self.byte_len = byte_len;
        self.json_escaped_len = escaped;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn write_canonical_json(
    writer: &mut impl Write,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    match value {
        serde_json::Value::Array(values) => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b",")?;
                }
                write_canonical_json(writer, value)?;
            }
            writer.write_all(b"]")
        }
        serde_json::Value::Object(object) => {
            writer.write_all(b"{")?;
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    writer.write_all(b",")?;
                }
                serde_json::to_writer(&mut *writer, key)
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                writer.write_all(b":")?;
                write_canonical_json(writer, &object[key])?;
            }
            writer.write_all(b"}")
        }
        value => serde_json::to_writer(writer, value)
            .map_err(|error| std::io::Error::other(error.to_string())),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
