//! Stream a literal envelope without hiding its proven ingress-backed leaves.
use std::collections::HashSet;
use std::io::{self, Read, Write};

use serde_json::Value;

use super::{ContentRef, JSON_MARKER_KEY, data_uri_payload_ref};
use crate::replay::{ReplayContentWriter, ReplayError, ReplayStore};

pub(super) fn append_literal_json(
    value: &Value,
    generated: &HashSet<usize>,
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
) -> Result<ContentRef, ReplayError> {
    if pool.is_none() {
        *pool = Some(replay.begin_content_pool()?);
    }
    pool.as_mut()
        .ok_or(ReplayError::Integrity)?
        .append_canonical_json_with(|writer| write_value(writer, value, generated, replay))
}

fn write_value(
    writer: &mut dyn Write,
    value: &Value,
    generated: &HashSet<usize>,
    replay: &ReplayStore,
) -> io::Result<()> {
    if let Value::String(text) = value
        && generated.contains(&(text.as_ptr() as usize))
    {
        if let Some(reference) = ContentRef::from_wire_marker(text) {
            return write_reference_string(writer, "", &reference, replay);
        }
        if let Some(reference) = data_uri_payload_ref(text) {
            let (_, payload) = text.split_once(";base64,").ok_or_else(invalid_proof)?;
            let prefix = &text[..text.len() - payload.len()];
            return write_reference_string(writer, prefix, &reference, replay);
        }
        return Err(invalid_proof());
    }
    if let Value::Object(fields) = value
        && fields.len() == 1
        && let Some(Value::String(marker)) = fields.get(JSON_MARKER_KEY)
        && generated.contains(&(marker.as_ptr() as usize))
    {
        let reference = ContentRef::from_wire_marker(marker).ok_or_else(invalid_proof)?;
        // Only an exact canonical JSON range may be copied into a newly
        // canonical envelope. Native raw arguments remain quoted strings here.
        if !replay.is_canonical_json(&reference) {
            return Err(invalid_proof());
        }
        let mut reader = replay.reader(&reference).map_err(io::Error::other)?;
        io::copy(&mut reader, writer)?;
        return reader.verify_terminal().map_err(io::Error::other);
    }
    match value {
        Value::Array(values) => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b",")?;
                }
                write_value(writer, value, generated, replay)?;
            }
            writer.write_all(b"]")
        }
        Value::Object(values) => {
            writer.write_all(b"{")?;
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    writer.write_all(b",")?;
                }
                serde_json::to_writer(&mut *writer, key).map_err(io::Error::other)?;
                writer.write_all(b":")?;
                write_value(writer, &values[key], generated, replay)?;
            }
            writer.write_all(b"}")
        }
        // A client locator or reserved envelope without generated-node proof
        // is ordinary JSON, even if the same range exists in this request.
        value => serde_json::to_writer(writer, value).map_err(io::Error::other),
    }
}

fn invalid_proof() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid generated JSON reference",
    )
}

fn write_reference_string(
    writer: &mut dyn Write,
    prefix: &str,
    reference: &ContentRef,
    replay: &ReplayStore,
) -> io::Result<()> {
    writer.write_all(b"\"")?;
    write_fragment(writer, prefix.as_bytes())?;
    let mut reader = replay.reader(reference).map_err(io::Error::other)?;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        write_fragment(writer, &buffer[..read])?;
    }
    reader.verify_terminal().map_err(io::Error::other)?;
    writer.write_all(b"\"")
}

fn write_fragment(writer: &mut dyn Write, mut bytes: &[u8]) -> io::Result<()> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    while let Some(index) = bytes
        .iter()
        .position(|byte| matches!(byte, b'"' | b'\\' | 0..=0x1f))
    {
        writer.write_all(&bytes[..index])?;
        let byte = bytes[index];
        match byte {
            b'"' => writer.write_all(b"\\\"")?,
            b'\\' => writer.write_all(b"\\\\")?,
            b'\x08' => writer.write_all(b"\\b")?,
            b'\x0c' => writer.write_all(b"\\f")?,
            b'\n' => writer.write_all(b"\\n")?,
            b'\r' => writer.write_all(b"\\r")?,
            b'\t' => writer.write_all(b"\\t")?,
            value => writer.write_all(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[(value >> 4) as usize],
                HEX[(value & 15) as usize],
            ])?,
        }
        bytes = &bytes[index + 1..];
    }
    writer.write_all(bytes)
}
