use std::fmt;
use std::io::Read;
use std::mem::size_of;

use hiroute_gateway_core::runtime::attempt::{AttemptError, AttemptRequestBodyReader};
use hiroute_gateway_core::runtime::body::{
    ChargedBytes, ChargedBytesBuilder, MemoryRole, Reservation, StreamBudget,
};

use crate::replay::{ReplayReader, ReplayStore};

use super::template::{PreparedNativeTemplate, PreparedReplayTemplate, ReplacementEncoding};

pub fn sequential_attempt_body(
    template: PreparedNativeTemplate,
    replay: ReplayStore,
    budget: &StreamBudget,
    write_quantum: usize,
) -> Result<Box<dyn AttemptRequestBodyReader>, AttemptError> {
    let retained_bytes = template.retained_bytes();
    let template = PreparedReplayTemplate {
        bytes: template.bytes,
        replacements: template.replacements,
        wire_len: template.wire_len,
    };
    sequential_replay_body_with_retained_bytes(
        template,
        replay,
        budget,
        write_quantum,
        retained_bytes,
    )
}

pub(crate) fn sequential_replay_body(
    template: PreparedReplayTemplate,
    replay: ReplayStore,
    budget: &StreamBudget,
    write_quantum: usize,
) -> Result<Box<dyn AttemptRequestBodyReader>, AttemptError> {
    let retained_bytes = template.retained_bytes();
    sequential_replay_body_with_retained_bytes(
        template,
        replay,
        budget,
        write_quantum,
        retained_bytes,
    )
}

fn sequential_replay_body_with_retained_bytes(
    template: PreparedReplayTemplate,
    replay: ReplayStore,
    budget: &StreamBudget,
    write_quantum: usize,
    retained_bytes: Option<usize>,
) -> Result<Box<dyn AttemptRequestBodyReader>, AttemptError> {
    if write_quantum == 0 {
        return Err(AttemptError::ZeroWriteQuantum);
    }
    if !replay.is_prevalidated() {
        return Err(AttemptError::Transport(
            "attempt replay was not prevalidated".into(),
        ));
    }
    let content_buffer_bytes = write_quantum.min(8 * 1024);
    let metadata_bytes = retained_bytes
        .and_then(|bytes| bytes.checked_add(content_buffer_bytes))
        .and_then(|bytes| bytes.checked_add(size_of::<SequentialNativeBody>()))
        .ok_or_else(|| AttemptError::Transport("wire template overflow".into()))?;
    let metadata = budget.reserve(MemoryRole::AttemptWire, metadata_bytes)?;
    Ok(Box::new(SequentialNativeBody {
        template,
        replay: Some(replay),
        budget: budget.clone(),
        write_quantum,
        metadata: Some(metadata),
        template_offset: 0,
        replacement_index: 0,
        content_reader: None,
        content_buffer: vec![0_u8; content_buffer_bytes],
        content_offset: 0,
        content_len: 0,
        escape: [0_u8; 6],
        escape_offset: 0,
        escape_len: 0,
        emitted: 0,
        verified_complete: false,
        released: false,
    }))
}

struct SequentialNativeBody {
    template: PreparedReplayTemplate,
    replay: Option<ReplayStore>,
    budget: StreamBudget,
    write_quantum: usize,
    metadata: Option<Reservation>,
    template_offset: usize,
    replacement_index: usize,
    content_reader: Option<ReplayReader>,
    content_buffer: Vec<u8>,
    content_offset: usize,
    content_len: usize,
    escape: [u8; 6],
    escape_offset: usize,
    escape_len: usize,
    emitted: usize,
    verified_complete: bool,
    released: bool,
}

impl fmt::Debug for SequentialNativeBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SequentialNativeBody")
            .field("wire_len", &self.template.wire_len)
            .field("write_quantum", &self.write_quantum)
            .field("emitted", &self.emitted)
            .field("released", &self.released)
            .finish()
    }
}

impl AttemptRequestBodyReader for SequentialNativeBody {
    fn visible_bytes(&self) -> usize {
        self.template.wire_len
    }

    fn max_chunk_bytes(&self) -> usize {
        self.write_quantum
    }

    fn next_chunk(&mut self) -> Result<Option<ChargedBytes>, AttemptError> {
        if self.released {
            return Ok(None);
        }
        if self.emitted == self.template.wire_len {
            self.verify_complete()?;
            return Ok(None);
        }
        let capacity = self
            .write_quantum
            .min(self.template.wire_len.saturating_sub(self.emitted));
        let mut output = ChargedBytesBuilder::new(&self.budget, MemoryRole::AttemptWire, capacity)?;
        while self.emitted < self.template.wire_len && output.len() < capacity {
            if self.escape_offset < self.escape_len {
                let end = self
                    .escape_len
                    .min(self.escape_offset + capacity - output.len());
                output.extend_from_slice(&self.escape[self.escape_offset..end])?;
                self.emitted += end - self.escape_offset;
                self.escape_offset = end;
                continue;
            }
            if self.content_reader.is_some() {
                if self.fill_content_buffer()? {
                    let encoding = self
                        .template
                        .replacements
                        .get(self.replacement_index)
                        .ok_or_else(|| {
                            AttemptError::Transport("missing template replacement".into())
                        })?
                        .encoding;
                    let available = &self.content_buffer[self.content_offset
                        ..self
                            .content_len
                            .min(self.content_offset + capacity - output.len())];
                    let count = if encoding == ReplacementEncoding::RawJson {
                        available.len()
                    } else {
                        available
                            .iter()
                            .position(|byte| matches!(byte, b'"' | b'\\' | 0..=0x1f))
                            .unwrap_or(available.len())
                    };
                    if count > 0 {
                        output.extend_from_slice(&available[..count])?;
                        self.content_offset += count;
                        self.emitted += count;
                    } else {
                        let byte = self.content_buffer[self.content_offset];
                        self.content_offset += 1;
                        self.prepare_output(byte);
                    }
                    continue;
                } else {
                    self.content_reader.take();
                    let replacement = &self.template.replacements[self.replacement_index];
                    self.template_offset = replacement.end;
                    self.replacement_index += 1;
                    continue;
                }
            }
            let next_start = self
                .template
                .replacements
                .get(self.replacement_index)
                .map_or(self.template.bytes.len(), |replacement| replacement.start);
            if self.template_offset < next_start {
                let remaining_capacity = capacity - output.len();
                let end = next_start.min(self.template_offset + remaining_capacity);
                output.extend_from_slice(&self.template.bytes[self.template_offset..end])?;
                let written = end - self.template_offset;
                self.template_offset = end;
                self.emitted += written;
                continue;
            }
            if let Some(replacement) = self.template.replacements.get(self.replacement_index) {
                let replay = self
                    .replay
                    .as_ref()
                    .ok_or_else(|| AttemptError::Transport("replay owner released".into()))?;
                self.content_reader = Some(
                    replay
                        .reader(&replacement.content)
                        .map_err(replay_attempt_error)?,
                );
                self.content_offset = 0;
                self.content_len = 0;
                continue;
            }
            if self.template_offset != self.template.bytes.len() {
                return Err(AttemptError::Transport(
                    "wire template cursor is inconsistent".into(),
                ));
            }
            break;
        }
        if output.is_empty() {
            return Err(AttemptError::Transport(
                "sequential encoder ended before declared length".into(),
            ));
        }
        Ok(Some(output.finish()))
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        self.content_reader.take();
        self.replay.take();
        self.content_buffer.clear();
        self.content_buffer.shrink_to_fit();
        self.template.release_storage();
        self.metadata.take();
        self.released = true;
    }
}

impl SequentialNativeBody {
    fn verify_complete(&mut self) -> Result<(), AttemptError> {
        if self.verified_complete {
            return Ok(());
        }
        if self.escape_offset != self.escape_len || self.content_offset != self.content_len {
            return Err(AttemptError::Transport(
                "sequential encoder exceeded its declared length".into(),
            ));
        }
        if let Some(reader) = self.content_reader.as_mut() {
            reader.verify_terminal().map_err(replay_attempt_error)?;
            self.content_reader.take();
            let replacement = self
                .template
                .replacements
                .get(self.replacement_index)
                .ok_or_else(|| AttemptError::Transport("missing template replacement".into()))?;
            self.template_offset = replacement.end;
            self.replacement_index += 1;
        }
        if self.replacement_index != self.template.replacements.len()
            || self.template_offset != self.template.bytes.len()
        {
            return Err(AttemptError::Transport(
                "sequential encoder ended before template verification".into(),
            ));
        }
        self.verified_complete = true;
        Ok(())
    }

    fn fill_content_buffer(&mut self) -> Result<bool, AttemptError> {
        if self.content_offset == self.content_len {
            let reader = self
                .content_reader
                .as_mut()
                .ok_or_else(|| AttemptError::Transport("missing ContentRef reader".into()))?;
            self.content_len = reader
                .read(&mut self.content_buffer)
                .map_err(|_| AttemptError::SequentialBodyContractViolation)?;
            self.content_offset = 0;
        }
        Ok(self.content_len != 0)
    }

    fn prepare_output(&mut self, byte: u8) {
        self.escape_offset = 0;
        let escape_len = match byte {
            b'"' => {
                self.escape[..2].copy_from_slice(br#"\""#);
                2
            }
            b'\\' => {
                self.escape[..2].copy_from_slice(br#"\\"#);
                2
            }
            b'\x08' => self.short_escape(b'b'),
            b'\x0c' => self.short_escape(b'f'),
            b'\n' => self.short_escape(b'n'),
            b'\r' => self.short_escape(b'r'),
            b'\t' => self.short_escape(b't'),
            0x00..=0x1f => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                self.escape[..4].copy_from_slice(b"\\u00");
                self.escape[4] = HEX[(byte >> 4) as usize];
                self.escape[5] = HEX[(byte & 0x0f) as usize];
                6
            }
            _ => {
                self.escape[0] = byte;
                1
            }
        };
        self.escape_len = escape_len;
    }

    fn short_escape(&mut self, value: u8) -> usize {
        self.escape[0] = b'\\';
        self.escape[1] = value;
        2
    }
}

impl Drop for SequentialNativeBody {
    fn drop(&mut self) {
        self.release();
    }
}

fn replay_attempt_error(_error: crate::replay::ReplayError) -> AttemptError {
    AttemptError::SequentialBodyContractViolation
}
