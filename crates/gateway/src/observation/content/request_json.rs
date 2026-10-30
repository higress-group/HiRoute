//! Request JSON is serialized through the same Replay-aware semantic owner as
//! history comparison. No request-local locator is retained as observed content.
use std::io::{self, Write};

use super::*;
use crate::context_hold::write_canonical_json_with_replay;

struct DigestWriter {
    mac: WorkspaceHmac,
    bytes: u64,
}

impl DigestWriter {
    fn new(key: &[u8; 32]) -> Self {
        let mut mac = WorkspaceHmac::new(key, b"conversation-content-blob");
        mac.update(CANONICALIZATION_VERSION.as_bytes());
        mac.update(b"application/json");
        Self { mac, bytes: 0 }
    }

    fn finish(self) -> String {
        format!("blob-{}", self.mac.finish().trim_start_matches("sha256:"))
    }
}

impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("content size overflow"))?;
        self.mac.update_stream(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ChunkWriter<F> {
    bytes: [u8; CONTENT_CHUNK_BYTES],
    used: usize,
    emitted: bool,
    digest: DigestWriter,
    emit: F,
}

impl<F: FnMut(&[u8], bool)> ChunkWriter<F> {
    fn emit_pending(&mut self) {
        if self.used > 0 || !self.emitted {
            (self.emit)(&self.bytes[..self.used], !self.emitted);
            self.emitted = true;
            self.used = 0;
        }
    }
}

impl<F: FnMut(&[u8], bool)> Write for ChunkWriter<F> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let length = self.digest.write(bytes)?;
        while !bytes.is_empty() {
            let take = bytes.len().min(self.bytes.len() - self.used);
            self.bytes[self.used..self.used + take].copy_from_slice(&bytes[..take]);
            self.used += take;
            bytes = &bytes[take..];
            if self.used == self.bytes.len() {
                self.emit_pending();
            }
        }
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl RequestObservation {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_replay_json(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        value: &Value,
    ) -> Result<(), ()> {
        // Measure and authenticate before publishing a content reference. Both
        // passes verify referenced backing and retain only one bounded chunk.
        let mut measured = DigestWriter::new(&self.inner.key);
        write_canonical_json_with_replay(&mut measured, value, replay).ok_or(())?;
        let byte_count = measured.bytes;
        let whole_digest = measured.finish();
        let message_id = self.message_instance_id("request_input", message_ordinal, role);
        let public_ref = ContentRefV1 {
            content_id: self.content_id(&message_id, part_ordinal, &whole_digest),
            digest: whole_digest.clone(),
            byte_count,
            media_type: "application/json".into(),
        };
        emit_content_ref(self, "request_input", public_ref.clone());
        let mut output = ChunkWriter {
            bytes: [0; CONTENT_CHUNK_BYTES],
            used: 0,
            emitted: false,
            digest: DigestWriter::new(&self.inner.key),
            emit: |bytes: &[u8], starts_content| {
                self.append_content_chunk(ContentChunk {
                    direction: "request_input",
                    attempt: None,
                    message_ordinal,
                    part_ordinal,
                    role,
                    kind,
                    media_type: "application/json",
                    bytes,
                    public_ref: Some(public_ref.clone()),
                    whole_digest: &whole_digest,
                    message_instance_id: &message_id,
                    content_id: &public_ref.content_id,
                    starts_content,
                    transport_frame_id: None,
                    downstream_delivery: None,
                });
            },
        };
        write_canonical_json_with_replay(&mut output, value, replay).ok_or(())?;
        output.emit_pending();
        if output.digest.bytes != byte_count || output.digest.finish() != whole_digest {
            return Err(());
        }
        Ok(())
    }
}
