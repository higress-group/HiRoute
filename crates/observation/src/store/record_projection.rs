//! Lossless record normalization against immutable carrier columns. Tags 2/3
//! omit repeated fields, but authenticate the complete reconstructed JSON value.
//! Legacy tags 0/1 remain independently readable without rewriting old rows.
use hiroute_domain::{ConversationContentEnvelopeV1, ObservationStreamV1, canonicalize_json};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::records;
use crate::writer::ObservationStoreError as Error;

#[cfg(test)]
mod tests;

pub(crate) struct ContentColumns {
    pub stream: ObservationStreamV1,
    pub sequence: u64,
    pub event_id: String,
    pub workspace_id: String,
    pub request_id: String,
    pub direction: String,
    pub fork_id: String,
    pub phase: String,
}

impl ContentColumns {
    pub(crate) fn from_envelope(envelope: &ConversationContentEnvelopeV1) -> Self {
        Self {
            stream: envelope.producer.stream.clone(),
            sequence: envelope.sequence,
            event_id: envelope.event_id.to_string(),
            workspace_id: envelope.correlation.workspace_id.to_string(),
            request_id: envelope.correlation.request_id.to_string(),
            direction: envelope.direction.as_str().into(),
            fork_id: envelope.fork_id.clone(),
            phase: envelope.phase.as_str().into(),
        }
    }

    pub(crate) fn shared(&self) -> Value {
        json!({
            "producer": {
                "producer_id": self.stream.producer_id,
                "producer_epoch": self.stream.producer_epoch,
                "stream_id": self.stream.stream_id,
            },
            "sequence": self.sequence,
            "event_id": self.event_id,
            "correlation": {"workspace_id": self.workspace_id, "request_id": self.request_id},
            "direction": self.direction,
            "fork_id": self.fork_id,
            "phase": self.phase,
        })
    }
}

pub(super) fn feedback_fields(channel: &str, stream: &ObservationStreamV1) -> Value {
    json!({
        "identity": {
            "channel": channel,
            "producer_id": stream.producer_id,
            "producer_epoch": stream.producer_epoch,
            "stream_id": stream.stream_id,
        },
    })
}

pub(crate) fn encode(value: Value, shared: &Value) -> Result<Vec<u8>, Error> {
    let value = canonicalize_json(value);
    let original = serde_json::to_vec(&value).map_err(|_| Error::Corrupt)?;
    // Keep the existing unbounded identity representation for large records;
    // normalization must not introduce a new admission or expansion limit.
    if original.len() > records::MAX_RECORD {
        return records::encode(&original);
    }
    let mut residual = value;
    subtract(&mut residual, shared)?;
    let residual = serde_json::to_vec(&residual).map_err(|_| Error::Corrupt)?;
    records::encode_projected(&residual, &original)
}

pub(crate) fn decode(frame: &[u8], shared: &Value) -> Result<Value, Error> {
    match frame.first() {
        Some(0 | 1) => {
            return serde_json::from_slice(&records::decode(frame)?).map_err(|_| Error::Corrupt);
        }
        Some(2 | 3) => {}
        _ => return Err(Error::Corrupt),
    }
    let header = frame.get(..37).ok_or(Error::Corrupt)?;
    let length = u32::from_le_bytes(header[1..5].try_into().map_err(|_| Error::Corrupt)?) as usize;
    if length > records::MAX_RECORD || frame.len() > length + 37 {
        return Err(Error::Corrupt);
    }
    let body = records::payload(frame, header[0] == 3, length)?;
    if body.len() > length {
        return Err(Error::Corrupt);
    }
    let mut value = serde_json::from_slice(&body).map_err(|_| Error::Corrupt)?;
    restore(&mut value, shared)?;
    let value = canonicalize_json(value);
    let original = serde_json::to_vec(&value).map_err(|_| Error::Corrupt)?;
    if original.len() != length || Sha256::digest(&original)[..] != header[5..37] {
        return Err(Error::Corrupt);
    }
    Ok(value)
}

fn subtract(value: &mut Value, shared: &Value) -> Result<(), Error> {
    let object = value.as_object_mut().ok_or(Error::Corrupt)?;
    for (key, expected) in shared.as_object().ok_or(Error::Corrupt)? {
        let actual = object.get_mut(key).ok_or(Error::Corrupt)?;
        if expected.is_object() {
            subtract(actual, expected)?;
        } else if actual == expected {
            object.remove(key);
        } else {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}

fn restore(value: &mut Value, shared: &Value) -> Result<(), Error> {
    let object = value.as_object_mut().ok_or(Error::Corrupt)?;
    for (key, expected) in shared.as_object().ok_or(Error::Corrupt)? {
        if expected.is_object() {
            restore(object.get_mut(key).ok_or(Error::Corrupt)?, expected)?;
        } else if object.insert(key.clone(), expected.clone()).is_some() {
            // Projected fields must not also occur in the residual record.
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
