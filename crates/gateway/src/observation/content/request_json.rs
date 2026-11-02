//! The same borrowed semantic cursor drives history fingerprints and Content.
//! Each pass validates backing; Content keeps only one admitted chunk in RAM.
use std::io::{self, Write};

use super::*;
use crate::context_hold::{CanonicalJsonReplayCursor, JsonFuel, JsonPumpStatus};

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

impl RequestObservation {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn append_replay_json_run(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        value: &Value,
    ) -> Result<(), ()> {
        if self.content_capture_failed() {
            return Err(());
        }
        let _drain = super::super::PartDrain::new(self, false);
        let mut measured = DigestWriter::new(&self.inner.key);
        {
            let mut cursor = CanonicalJsonReplayCursor::new(value, replay);
            let mut bytes = vec![0; 16 * 1024];
            loop {
                let step = cursor
                    .pump(&mut bytes, &mut JsonFuel::default())
                    .map_err(|_| ())?;
                measured.write_all(&bytes[..step.written]).map_err(|_| ())?;
                self.content_checkpoint(step.written, step.status == JsonPumpStatus::Yield)
                    .await;
                if step.status == JsonPumpStatus::Done {
                    break;
                }
            }
        }
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
        let mut cursor = CanonicalJsonReplayCursor::new(value, replay);
        let mut bytes = vec![0; CONTENT_CHUNK_BYTES];
        let mut used = 0;
        let mut emitted = false;
        let mut digest = DigestWriter::new(&self.inner.key);
        loop {
            let step = cursor
                .pump(&mut bytes[used..], &mut JsonFuel::default())
                .map_err(|_| ())?;
            digest
                .write_all(&bytes[used..used + step.written])
                .map_err(|_| ())?;
            used += step.written;
            if used == bytes.len()
                || (step.status == JsonPumpStatus::Done && (used != 0 || !emitted))
            {
                self.append_content_chunk_run(ContentChunk {
                    direction: "request_input",
                    attempt_id: None,
                    message_ordinal,
                    part_ordinal,
                    role,
                    kind,
                    media_type: "application/json",
                    bytes: &bytes[..used],
                    public_ref: Some(public_ref.clone()),
                    whole_digest: &whole_digest,
                    message_instance_id: &message_id,
                    content_id: &public_ref.content_id,
                    starts_content: !emitted,
                    transport_frame_id: None,
                    downstream_delivery: None,
                })
                .await;
                emitted = true;
                used = 0;
            }
            self.content_checkpoint(step.written, step.status == JsonPumpStatus::Yield)
                .await;
            if step.status == JsonPumpStatus::Done {
                break;
            }
        }
        if digest.bytes != byte_count || digest.finish() != whole_digest {
            return Err(());
        }
        Ok(())
    }
}
