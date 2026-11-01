//! Shared semantic JSON bytes for history fingerprints and optional Content.
use std::io::{Read, Write};

use serde_json::Value;

use crate::content_ref::{ContentValueExt, JsonValueExt};
use crate::replay::{ReplayError, ReplayReader, ReplayStore};

const QUANTUM: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsonPumpStatus {
    More,
    Yield,
    Done,
}

pub(crate) struct JsonPump {
    pub written: usize,
    pub status: JsonPumpStatus,
}

pub(crate) struct JsonFuel {
    bytes: usize,
    steps: usize,
}

impl Default for JsonFuel {
    fn default() -> Self {
        Self {
            bytes: QUANTUM,
            steps: 256,
        }
    }
}

enum Frame<'a> {
    Value(&'a Value),
    Array {
        values: std::slice::Iter<'a, Value>,
        first: bool,
    },
    ObjectCollect {
        values: serde_json::map::Iter<'a>,
        entries: Vec<(&'a str, &'a Value)>,
    },
    ObjectEmit {
        entries: Vec<(&'a str, &'a Value)>,
        index: usize,
    },
    Literal {
        bytes: &'static [u8],
        offset: usize,
    },
    Scalar {
        bytes: Vec<u8>,
        offset: usize,
    },
    String(Quoted<'a>),
    Raw(ReplayReader),
}

enum StringSource<'a> {
    Inline {
        bytes: &'a [u8],
        offset: usize,
    },
    Replay {
        reader: ReplayReader,
        buffer: Vec<u8>,
        used: usize,
        offset: usize,
    },
}

enum SourceByte {
    Byte(u8),
    End,
    Yield,
}

impl StringSource<'_> {
    fn copy_plain(&mut self, output: &mut [u8], fuel: &mut JsonFuel) -> usize {
        let (bytes, offset) = match self {
            Self::Inline { bytes, offset } => (*bytes, offset),
            Self::Replay {
                buffer,
                used,
                offset,
                ..
            } => (&buffer[..*used], offset),
        };
        let available = &bytes[*offset..];
        let length = available.len().min(output.len()).min(fuel.bytes);
        let count = available[..length]
            .iter()
            .position(|byte| *byte < 0x20 || *byte == b'"' || *byte == b'\\')
            .unwrap_or(length);
        output[..count].copy_from_slice(&available[..count]);
        *offset += count;
        fuel.bytes -= count;
        count
    }

    fn next(&mut self, fuel: &mut JsonFuel) -> Result<SourceByte, ReplayError> {
        match self {
            Self::Inline { bytes, offset } => {
                let Some(&byte) = bytes.get(*offset) else {
                    return Ok(SourceByte::End);
                };
                *offset += 1;
                Ok(SourceByte::Byte(byte))
            }
            Self::Replay {
                reader,
                buffer,
                used,
                offset,
            } => {
                if *offset == *used {
                    if fuel.bytes == 0 {
                        return Ok(SourceByte::Yield);
                    }
                    if buffer.is_empty() {
                        buffer.resize(QUANTUM, 0);
                    }
                    *used = reader.read(&mut buffer[..QUANTUM.min(fuel.bytes)])?;
                    *offset = 0;
                    fuel.bytes -= *used;
                    if *used == 0 {
                        reader.verify_terminal()?;
                        return Ok(SourceByte::End);
                    }
                }
                let byte = buffer[*offset];
                *offset += 1;
                Ok(SourceByte::Byte(byte))
            }
        }
    }
}

struct Quoted<'a> {
    source: StringSource<'a>,
    phase: u8,
    pending: [u8; 6],
    used: usize,
    length: usize,
}

impl<'a> Quoted<'a> {
    fn inline(value: &'a str) -> Self {
        Self::new(StringSource::Inline {
            bytes: value.as_bytes(),
            offset: 0,
        })
    }

    fn new(source: StringSource<'a>) -> Self {
        Self {
            source,
            phase: 0,
            pending: [0; 6],
            used: 0,
            length: 0,
        }
    }

    fn pump(
        &mut self,
        output: &mut [u8],
        fuel: &mut JsonFuel,
    ) -> Result<(usize, bool), ReplayError> {
        let mut written = 0;
        while written < output.len() && fuel.bytes != 0 {
            if self.used < self.length {
                let count = (self.length - self.used)
                    .min(output.len() - written)
                    .min(fuel.bytes);
                output[written..written + count]
                    .copy_from_slice(&self.pending[self.used..self.used + count]);
                self.used += count;
                written += count;
                fuel.bytes -= count;
                continue;
            }
            self.used = 0;
            self.length = 0;
            if self.phase == 0 {
                self.pending[0] = b'"';
                self.length = 1;
                self.phase = 1;
            } else if self.phase == 2 {
                self.pending[0] = b'"';
                self.length = 1;
                self.phase = 3;
            } else if self.phase == 3 {
                return Ok((written, true));
            } else {
                // Keep ordinary UTF-8 spans contiguous; only actual escapes
                // need the small pending buffer. Avoid a byte-at-a-time copy
                // in the shared history/capture critical path.
                let plain = self.source.copy_plain(&mut output[written..], fuel);
                if plain != 0 {
                    written += plain;
                    continue;
                }
                let byte = match self.source.next(fuel)? {
                    SourceByte::End => {
                        self.phase = 2;
                        continue;
                    }
                    SourceByte::Yield => return Ok((written, false)),
                    SourceByte::Byte(byte) => byte,
                };
                let short = match byte {
                    b'"' => Some(b'"'),
                    b'\\' => Some(b'\\'),
                    b'\x08' => Some(b'b'),
                    b'\x0c' => Some(b'f'),
                    b'\n' => Some(b'n'),
                    b'\r' => Some(b'r'),
                    b'\t' => Some(b't'),
                    _ => None,
                };
                if let Some(escaped) = short {
                    self.pending[..2].copy_from_slice(&[b'\\', escaped]);
                    self.length = 2;
                } else if byte < 0x20 {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    self.pending = [
                        b'\\',
                        b'u',
                        b'0',
                        b'0',
                        HEX[(byte >> 4) as usize],
                        HEX[(byte & 15) as usize],
                    ];
                    self.length = 6;
                } else {
                    self.pending[0] = byte;
                    self.length = 1;
                }
            }
        }
        Ok((written, self.phase == 3 && self.used == self.length))
    }
}

/// Borrows an immutable tree owned by the surrounding pinned capture future.
/// Stack/object indices contain references, never copies of keys or values.
pub(crate) struct CanonicalJsonReplayCursor<'a> {
    replay: &'a ReplayStore,
    stack: Vec<Frame<'a>>,
    failed: bool,
}

impl<'a> CanonicalJsonReplayCursor<'a> {
    pub(crate) fn new(value: &'a Value, replay: &'a ReplayStore) -> Self {
        Self {
            replay,
            stack: vec![Frame::Value(value)],
            failed: false,
        }
    }

    pub(crate) fn frame_bytes() -> usize {
        std::mem::size_of::<Frame<'static>>()
    }

    pub(crate) fn pump(
        &mut self,
        output: &mut [u8],
        fuel: &mut JsonFuel,
    ) -> Result<JsonPump, ReplayError> {
        if self.failed {
            return Err(ReplayError::Integrity);
        }
        if output.is_empty() {
            return Err(ReplayError::InvalidConfig);
        }
        let result = self.pump_inner(output, fuel);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn pump_inner(
        &mut self,
        output: &mut [u8],
        fuel: &mut JsonFuel,
    ) -> Result<JsonPump, ReplayError> {
        let mut written = 0;
        while written < output.len() && !self.stack.is_empty() && fuel.bytes != 0 && fuel.steps != 0
        {
            fuel.steps -= 1;
            let frame = self.stack.pop().unwrap();
            match frame {
                Frame::Value(value) => {
                    if let Some(reference) = value.content_ref() {
                        if !self.replay.is_canonical_json(&reference) {
                            return Err(ReplayError::Integrity);
                        }
                        self.stack.push(Frame::Raw(self.replay.reader(&reference)?));
                        continue;
                    }
                    match value {
                        Value::Null => self.literal(b"null"),
                        Value::Bool(true) => self.literal(b"true"),
                        Value::Bool(false) => self.literal(b"false"),
                        Value::Number(_) => self.stack.push(Frame::Scalar {
                            bytes: serde_json::to_vec(value)
                                .map_err(|_| ReplayError::InvalidJson)?,
                            offset: 0,
                        }),
                        Value::String(value) => {
                            let quoted = if let Some(reference) = value.content_ref() {
                                Quoted::new(StringSource::Replay {
                                    reader: self.replay.reader(&reference)?,
                                    buffer: Vec::new(),
                                    used: 0,
                                    offset: 0,
                                })
                            } else {
                                Quoted::inline(value)
                            };
                            self.stack.push(Frame::String(quoted));
                        }
                        Value::Array(values) => {
                            self.stack.push(Frame::Array {
                                values: values.iter(),
                                first: true,
                            });
                            self.literal(b"[");
                        }
                        Value::Object(values) => self.stack.push(Frame::ObjectCollect {
                            values: values.iter(),
                            entries: Vec::new(),
                        }),
                    }
                }
                Frame::Array { mut values, first } => {
                    if let Some(value) = values.next() {
                        self.stack.push(Frame::Array {
                            values,
                            first: false,
                        });
                        self.stack.push(Frame::Value(value));
                        if !first {
                            self.literal(b",");
                        }
                    } else {
                        self.literal(b"]");
                    }
                }
                Frame::ObjectCollect {
                    mut values,
                    mut entries,
                } => {
                    let mut complete = false;
                    while fuel.steps != 0 {
                        fuel.steps -= 1;
                        if let Some((key, value)) = values.next() {
                            entries.push((key.as_str(), value));
                        } else {
                            complete = true;
                            break;
                        }
                    }
                    if complete {
                        // Preserve the existing canonical owner's lexicographic
                        // order with its mature standard-library sort. Async
                        // snapshots already bound this reference index; no new
                        // general sorting implementation is introduced.
                        entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
                        self.stack.push(Frame::ObjectEmit { entries, index: 0 });
                        self.literal(b"{");
                    } else {
                        self.stack.push(Frame::ObjectCollect { values, entries });
                    }
                }
                Frame::ObjectEmit { entries, index } => {
                    if let Some(&(key, value)) = entries.get(index) {
                        self.stack.push(Frame::ObjectEmit {
                            entries,
                            index: index + 1,
                        });
                        self.stack.push(Frame::Value(value));
                        self.literal(b":");
                        self.stack.push(Frame::String(Quoted::inline(key)));
                        if index != 0 {
                            self.literal(b",");
                        }
                    } else {
                        self.literal(b"}");
                    }
                }
                Frame::Literal { bytes, offset } => {
                    let count = (bytes.len() - offset)
                        .min(output.len() - written)
                        .min(fuel.bytes);
                    output[written..written + count]
                        .copy_from_slice(&bytes[offset..offset + count]);
                    written += count;
                    fuel.bytes -= count;
                    if offset + count < bytes.len() {
                        self.stack.push(Frame::Literal {
                            bytes,
                            offset: offset + count,
                        });
                    }
                }
                Frame::Scalar { bytes, offset } => {
                    let count = (bytes.len() - offset)
                        .min(output.len() - written)
                        .min(fuel.bytes);
                    output[written..written + count]
                        .copy_from_slice(&bytes[offset..offset + count]);
                    written += count;
                    fuel.bytes -= count;
                    if offset + count < bytes.len() {
                        self.stack.push(Frame::Scalar {
                            bytes,
                            offset: offset + count,
                        });
                    }
                }
                Frame::String(mut quoted) => {
                    let (count, done) = quoted.pump(&mut output[written..], fuel)?;
                    written += count;
                    if !done {
                        self.stack.push(Frame::String(quoted));
                    }
                }
                Frame::Raw(mut reader) => {
                    let end = output.len().min(written + fuel.bytes);
                    let count = reader.read(&mut output[written..end])?;
                    if count == 0 {
                        reader.verify_terminal()?;
                    } else {
                        written += count;
                        fuel.bytes -= count;
                        self.stack.push(Frame::Raw(reader));
                    }
                }
            }
        }
        Ok(JsonPump {
            written,
            status: if self.stack.is_empty() {
                JsonPumpStatus::Done
            } else if fuel.bytes == 0 || fuel.steps == 0 {
                JsonPumpStatus::Yield
            } else {
                JsonPumpStatus::More
            },
        })
    }

    fn literal(&mut self, bytes: &'static [u8]) {
        self.stack.push(Frame::Literal { bytes, offset: 0 });
    }
}

pub(crate) fn write_canonical_json_with_replay(
    writer: &mut impl Write,
    value: &Value,
    replay: &ReplayStore,
) -> Option<()> {
    let mut cursor = CanonicalJsonReplayCursor::new(value, replay);
    let mut buffer = [0; QUANTUM];
    loop {
        let result = cursor.pump(&mut buffer, &mut JsonFuel::default()).ok()?;
        writer.write_all(&buffer[..result.written]).ok()?;
        if result.status == JsonPumpStatus::Done {
            return Some(());
        }
    }
}

#[cfg(test)]
#[path = "replay_json_tests.rs"]
mod tests;
