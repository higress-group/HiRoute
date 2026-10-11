//! Opt-in Unix Debug-only forensic capture. Never feeds the product diagnostic log.
//! A session owns a private directory and an exclusive capture lock. Failed/partial
//! capture is not replay evidence; it never changes the business result.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_gateway_core::runtime::attempt::{AttemptError, AttemptRequestBodyReader};
use hiroute_gateway_core::runtime::body::ChargedBytes;
use serde::{Deserialize, Serialize};

use crate::server::core_runtime::{adapters, profiles::CandidateProtocolProfile};

mod replay;
pub use replay::replay_capture;
#[cfg(test)]
mod tests;

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_TOTAL: u64 = 32 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 4;
const MAX_RECORDS: u64 = 65536;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    source_sha: String,
    binary_sha256: String,
    client: String,
    expires_at: u64,
    delete_after: u64,
}

#[derive(Serialize, Deserialize)]
struct Context {
    session: Session,
    profile: CandidateProtocolProfile,
    chat_tools: Option<adapters::ChatToolProjection>,
    request_bytes: u64,
    streaming: bool,
    correlation: serde_json::Value,
    // Values of transport headers have no role in the response decoder.
    transport_header_values_redacted: bool,
}

#[derive(Clone)]
pub(crate) struct Capture(Arc<Mutex<Writer>>);

pub(crate) struct PendingCapture(Weak<Mutex<Writer>>);

#[derive(Serialize)]
pub(crate) struct CaptureCorrelation {
    pub(crate) request_token: Option<hiroute_diagnostics::identity::CorrelationToken>,
    pub(crate) request_id: String,
    pub(crate) attempt_index: u32,
    pub(crate) attempt_token: Option<hiroute_diagnostics::identity::CorrelationToken>,
}

impl PendingCapture {
    pub(crate) fn promoted(self, correlation: CaptureCorrelation) {
        if let Some(writer) = self.0.upgrade() {
            Capture(writer).correlate(&correlation, true);
        }
    }
}

struct Writer {
    file: File,
    path: PathBuf,
    _lock: File,
    root: PathBuf,
    written: u64,
    records: u64,
    limit: u64,
    expires_at: u64,
    active: bool,
    attempt_correlated: bool,
    failed: bool,
    failed_seal_offset: Option<u64>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

fn read_private(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::other("unsafe or oversized capture file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("capture file grew"));
    }
    Ok(bytes)
}

impl Capture {
    pub(crate) fn begin(
        profile: &CandidateProtocolProfile,
        chat_tools: Option<&adapters::ChatToolProjection>,
        request_bytes: usize,
        streaming: bool,
    ) -> Option<Self> {
        let root = PathBuf::from(std::env::var_os("HIROUTE_PRIVATE_STREAM_CAPTURE")?);
        Self::open(&root, profile, chat_tools, request_bytes, streaming).ok()
    }

    pub(crate) fn register(
        &self,
        request: &crate::server::core_runtime::observation::RequestObservation,
    ) {
        request.register_private_capture(PendingCapture(Arc::downgrade(&self.0)));
    }

    fn open(
        root: &Path,
        profile: &CandidateProtocolProfile,
        chat_tools: Option<&adapters::ChatToolProjection>,
        request_bytes: usize,
        streaming: bool,
    ) -> io::Result<Self> {
        // Canonical spelling rejects symlinks in every component, not just the leaf.
        let metadata = fs::symlink_metadata(root)?;
        if !root.is_absolute() || fs::canonicalize(root)? != root || !metadata.is_dir() {
            return Err(io::Error::other(
                "capture requires a private canonical directory",
            ));
        }
        let session: Session =
            serde_json::from_slice(&read_private(&root.join("session.json"), 4096)?)?;
        let time = now();
        if session.expires_at <= time
            || session.expires_at > time + 3600
            || session.delete_after <= session.expires_at
            || session.delete_after > time + 86400
            || !hex(&session.source_sha, 40)
            || !hex(&session.binary_sha256, 64)
            || session.client.is_empty()
            || session.client.len() > 128
            || root.join("stopped").exists()
        {
            return Err(io::Error::other(
                "invalid, expired or stopped capture session",
            ));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("active.lock"))?;
        let metadata = lock.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("unsafe capture lock"));
        }
        // Keep the inode stable. Kernel ownership disappears on crash/kill and
        // cleanup acquires this same lock before removing an expired session.
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        if root.join("stopped").exists() || now() >= session.expires_at {
            return Err(io::Error::other("capture session stopped or expired"));
        }
        Self::open_locked(
            root,
            lock,
            session,
            profile,
            chat_tools,
            request_bytes,
            streaming,
        )
    }

    fn open_locked(
        root: &Path,
        lock: File,
        session: Session,
        profile: &CandidateProtocolProfile,
        chat_tools: Option<&adapters::ChatToolProjection>,
        request_bytes: usize,
        streaming: bool,
    ) -> io::Result<Self> {
        let mut total = 0u64;
        let mut attempts = 0;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() {
                return Err(io::Error::other("unexpected capture directory entry"));
            }
            total = total.saturating_add(metadata.len());
            if entry.path().extension().is_some_and(|e| e == "capture") {
                attempts += 1;
            }
        }
        if attempts >= MAX_ATTEMPTS || total >= MAX_TOTAL || request_bytes as u64 >= MAX_FILE {
            return Err(io::Error::other("capture budget exhausted"));
        }
        let mut profile = profile.clone();
        if let crate::server::core_runtime::profiles::CriticalFact::Exact(headers) =
            &mut profile.connector.headers
        {
            for (_, value) in &mut headers.required_headers {
                *value = "redacted".into();
            }
        }
        let context = Context {
            session,
            profile,
            chat_tools: chat_tools.cloned(),
            request_bytes: request_bytes as u64,
            streaming,
            correlation: correlation(),
            transport_header_values_redacted: true,
        };
        let path = root.join(format!("attempt-{}.capture", attempts + 1));
        let file = private_file(&path)?;
        let mut writer = Writer {
            file,
            path,
            _lock: lock,
            root: root.to_owned(),
            written: 0,
            records: 0,
            limit: MAX_FILE.min(MAX_TOTAL - total),
            expires_at: context.session.expires_at,
            active: true,
            attempt_correlated: false,
            failed: false,
            failed_seal_offset: None,
        };
        writer.write(0, &serde_json::to_vec(&context)?)?;
        Ok(Self(Arc::new(Mutex::new(writer))))
    }

    fn record(&self, kind: u8, bytes: &[u8]) {
        let mut writer = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if writer.active && writer.write(kind, bytes).is_err() {
            writer.active = false;
        }
    }

    pub(crate) fn response(&self, event: &hiroute_gateway_core::runtime::attempt::PrecommitEvent) {
        use hiroute_gateway_core::runtime::attempt::PrecommitEvent;
        self.record_attempt_correlation();
        match event {
            PrecommitEvent::ResponseHead(head) => {
                self.record(3, &head.status().as_u16().to_le_bytes());
            }
            PrecommitEvent::Body(bytes) => self.record(4, bytes.bytes()),
            PrecommitEvent::EndStream => self.record(5, &[]),
            PrecommitEvent::SseEvent { .. } => {}
        }
    }

    pub(crate) fn record_attempt_correlation(&self) {
        // Snapshot observation before taking the writer lock. Formal promotion
        // supplies its own validated snapshot and never calls back into state.
        if let Some(request) = crate::server::core_runtime::observation::active_request() {
            self.correlate(&request.private_capture_correlation(), false);
        }
    }

    fn correlate(&self, correlation: &CaptureCorrelation, allow_sealed: bool) {
        let mut writer = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if writer.attempt_correlated || correlation.attempt_token.is_none() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(correlation) else {
            return;
        };
        if writer.active {
            writer.attempt_correlated = true;
            if writer.write(7, &bytes).is_err() {
                writer.active = false;
            }
        } else if allow_sealed && let Some(offset) = writer.failed_seal_offset.take() {
            // Never overwrite the sample after publishing `stopped`: its owner
            // may terminate us at any instruction. Publish a complete replacement.
            writer.attempt_correlated = true;
            let _ = writer.correlate_sealed(offset, &bytes);
        }
    }

    pub(crate) fn failed(&self) {
        let mut writer = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if writer.failed {
            return;
        }
        writer.failed = true;
        if writer.active && writer.write(6, &[]).is_err() {
            writer.active = false;
        }
        // The supervisor can terminate the process immediately after `stopped`.
        // Seal and flush this sample before publishing that signal, even while
        // the request reader or response driver still owns another Capture.
        writer.failed_seal_offset = writer.seal();
        let _ = private_file(&writer.root.join("stopped"));
    }

    pub(crate) fn wrap(
        &self,
        body: Box<dyn AttemptRequestBodyReader>,
    ) -> Box<dyn AttemptRequestBodyReader> {
        Box::new(CapturedBody {
            body,
            capture: self.clone(),
            ended: false,
        })
    }
}

fn correlation() -> serde_json::Value {
    crate::server::core_runtime::observation::active_request()
        .and_then(|r| serde_json::to_value(r.private_capture_correlation()).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|c| c.is_ascii_hexdigit())
}

impl Writer {
    fn seal(&mut self) -> Option<u64> {
        // A missing seal is never replayable. Partial requests remain rejected
        // by the independent request-completeness check during replay.
        let seal_offset = self.written;
        let sealed = self.active && self.write(8, &[]).is_ok();
        self.active = false;
        let flushed = self.file.sync_all().is_ok();
        if !flushed && sealed {
            self.invalidate_seal(seal_offset);
        }
        if !flushed || !sealed {
            return None;
        }
        Some(seal_offset)
    }

    fn invalidate_seal(&self, offset: u64) {
        // A partial write or flush failure must not advertise sealed evidence.
        if self.file.set_len(offset).is_err() {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn correlate_sealed(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.check_correlation_tail(offset, bytes.len(), true)?;
        let replacement = CorrelationReplacement {
            file: private_file(&self.path.with_extension("correlation"))?,
            path: self.path.with_extension("correlation"),
        };
        let mut file = &replacement.file;
        let source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)?;
        if io::copy(&mut source.take(offset), &mut file)? != offset {
            return Err(io::Error::other("incomplete capture replacement"));
        }
        file.write_all(&[7])?;
        #[cfg(test)]
        tests::correlation_checkpoint(&self.root, "partial");
        file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        file.write_all(bytes)?;
        file.write_all(&[8])?;
        file.write_all(&0u64.to_le_bytes())?;
        file.sync_all()?;
        // Recheck original identity and the now-present temporary copy's total.
        self.check_correlation_tail(offset, bytes.len(), false)?;
        let next_file = file.try_clone()?;
        #[cfg(test)]
        tests::correlation_checkpoint(&self.root, "ready");
        fs::rename(&replacement.path, &self.path)?;
        self.file = next_file;
        self.written = offset + 18 + bytes.len() as u64;
        self.records += 1;
        #[cfg(test)]
        tests::correlation_checkpoint(&self.root, "published");
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    fn check_correlation_tail(
        &self,
        offset: u64,
        bytes: usize,
        reserve_copy: bool,
    ) -> io::Result<()> {
        let root = fs::symlink_metadata(&self.root)?;
        let file = self.file.metadata()?;
        let path = fs::symlink_metadata(&self.path)?;
        let new_size = offset.saturating_add(18).saturating_add(bytes as u64);
        let mut total = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let metadata = fs::symlink_metadata(entry?.path())?;
            if !metadata.is_file() {
                return Err(io::Error::other("unsafe capture directory entry"));
            }
            total = total.saturating_add(metadata.len());
        }
        if !root.is_dir()
            || fs::canonicalize(&self.root)? != self.root
            || !file.is_file()
            || !path.is_file()
            || path.ino() != file.ino()
            || path.dev() != file.dev()
            || file.len() != self.written
            || self.written != offset.saturating_add(9)
            || now() >= self.expires_at
            || new_size > self.limit
            || total.saturating_add(if reserve_copy { new_size } else { 0 }) > MAX_TOTAL
            || self.records.saturating_add(1) > MAX_RECORDS
        {
            return Err(io::Error::other(
                "unsafe or over-limit capture correlation tail",
            ));
        }
        Ok(())
    }

    // Binary records: kind:u8, length:u64 LE, exact bytes. File order is read order;
    // offsets are cumulative per direction. These are NOT TCP/TLS packet boundaries.
    fn write(&mut self, kind: u8, bytes: &[u8]) -> io::Result<()> {
        let size = 9u64.saturating_add(bytes.len() as u64);
        if now() >= self.expires_at
            || self.written.saturating_add(size) > self.limit
            || self.records >= MAX_RECORDS
        {
            self.active = false;
            return Err(io::Error::other("capture bound reached"));
        }
        self.file.write_all(&[kind])?;
        self.file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        self.file.write_all(bytes)?;
        self.written += size;
        self.records += 1;
        Ok(())
    }
}
// A killed process can leave a bounded private temporary file. The existing
// session retention owner removes it together with the rest of the session.
struct CorrelationReplacement {
    file: File,
    path: PathBuf,
}
impl Drop for CorrelationReplacement {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.seal();
    }
}

struct CapturedBody {
    body: Box<dyn AttemptRequestBodyReader>,
    capture: Capture,
    ended: bool,
}
impl std::fmt::Debug for CapturedBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateCapturedBody")
    }
}
impl AttemptRequestBodyReader for CapturedBody {
    fn visible_bytes(&self) -> usize {
        self.body.visible_bytes()
    }
    fn max_chunk_bytes(&self) -> usize {
        self.body.max_chunk_bytes()
    }
    fn high_water_bytes(&self) -> usize {
        self.body.high_water_bytes()
    }
    fn next_chunk(&mut self) -> Result<Option<ChargedBytes>, AttemptError> {
        let chunk = self.body.next_chunk()?;
        if let Some(bytes) = &chunk {
            self.capture.record(1, bytes.bytes());
        } else if !self.ended {
            self.capture.record(2, &[]);
            self.ended = true;
        }
        Ok(chunk)
    }
    fn release(&mut self) {
        self.body.release();
    }
}
