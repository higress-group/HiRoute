//! Stream identity is shared only inside protocol evidence. Frozen outcomes and
//! content metadata remain independently committed, bounded records.
use crate::store::cached_sql;
use crate::writer::ObservationStoreError as Error;
use hiroute_domain::ObservationStreamV1;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, io::Read};

pub(crate) const MAX_RECORD: usize = 4 * 1024 * 1024;

pub(crate) fn encode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    encode_payload(bytes, bytes, 0)
}

pub(super) fn encode_projected(bytes: &[u8], original: &[u8]) -> Result<Vec<u8>, Error> {
    encode_payload(bytes, original, 2)
}

fn encode_payload(bytes: &[u8], original: &[u8], tag_offset: u8) -> Result<Vec<u8>, Error> {
    let length = u32::try_from(original.len()).map_err(|_| Error::Corrupt)?;
    // Large pre-existing replies retain their original identity representation.
    // Only the bounded compressed form may expand during decoding.
    let compressed = if bytes.len() <= MAX_RECORD {
        zstd::bulk::compress(bytes, 1).map_err(|_| Error::Corrupt)?
    } else {
        Vec::new()
    };
    let (tag, payload) = if !compressed.is_empty() && compressed.len() < bytes.len() {
        (1, compressed.as_slice())
    } else {
        (0, bytes)
    };
    let mut result = Vec::with_capacity(payload.len() + 37);
    result.push(tag + tag_offset);
    result.extend_from_slice(&length.to_le_bytes());
    result.extend_from_slice(&Sha256::digest(original));
    result.extend_from_slice(payload);
    Ok(result)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Cow<'_, [u8]>, Error> {
    let header = bytes.get(..37).ok_or(Error::Corrupt)?;
    let length = u32::from_le_bytes(header[1..5].try_into().map_err(|_| Error::Corrupt)?) as usize;
    let compressed = match header[0] {
        0 => false,
        1 => true,
        _ => return Err(Error::Corrupt),
    };
    let result = payload(bytes, compressed, length)?;
    if result.len() != length || Sha256::digest(&result)[..] != header[5..37] {
        return Err(Error::Corrupt);
    }
    Ok(result)
}

/// Decode bytes only; the caller must authenticate its complete logical record.
pub(super) fn payload(
    bytes: &[u8],
    compressed: bool,
    length: usize,
) -> Result<Cow<'_, [u8]>, Error> {
    let body = bytes.get(37..).ok_or(Error::Corrupt)?;
    if compressed {
        if length > MAX_RECORD || bytes.len() > MAX_RECORD + 37 {
            return Err(Error::Corrupt);
        }
        let mut decoder = zstd::stream::read::Decoder::new(body).map_err(|_| Error::Corrupt)?;
        decoder.window_log_max(22).map_err(|_| Error::Corrupt)?;
        let mut output = Vec::with_capacity(length.min(64 * 1024));
        decoder
            .take(length as u64 + 1)
            .read_to_end(&mut output)
            .map_err(|_| Error::Corrupt)?;
        Ok(Cow::Owned(output))
    } else {
        Ok(Cow::Borrowed(body))
    }
}

pub(crate) fn stream_ref(
    db: &Connection,
    channel: &str,
    stream: &ObservationStreamV1,
) -> Result<i64, Error> {
    intern_stream(
        db,
        channel,
        stream.producer_id.as_str(),
        stream.producer_epoch.as_str(),
        stream.stream_id.as_str(),
    )
}

pub(crate) fn intern_stream(
    db: &Connection,
    channel: &str,
    producer: &str,
    epoch: &str,
    stream: &str,
) -> Result<i64, Error> {
    let args = params![channel, producer, epoch, stream];
    let query = "SELECT stream_ref FROM observation_streams_v5 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4";
    if let Some(id) = cached_sql::query_row(db, query, args, |r| r.get(0))
        .optional()
        .map_err(|_| Error::ActivityUnavailable)?
    {
        return Ok(id);
    }
    cached_sql::execute(db, "INSERT INTO observation_streams_v5(channel,producer_id,producer_epoch,stream_id) VALUES(?1,?2,?3,?4)", args).map_err(|_| Error::ActivityUnavailable)?;
    Ok(db.last_insert_rowid())
}

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observation_streams_v5(
 stream_ref INTEGER PRIMARY KEY, channel TEXT NOT NULL, producer_id TEXT NOT NULL,
 producer_epoch TEXT NOT NULL, stream_id TEXT NOT NULL, references_count INTEGER NOT NULL DEFAULT 0,
 UNIQUE(channel,producer_id,producer_epoch,stream_id));
CREATE TABLE IF NOT EXISTS observation_events(
 stream_ref INTEGER NOT NULL REFERENCES observation_streams_v5, sequence INTEGER NOT NULL,
 event_id TEXT NOT NULL,payload_digest TEXT NOT NULL,
 PRIMARY KEY(stream_ref,sequence),UNIQUE(stream_ref,event_id));
CREATE TABLE IF NOT EXISTS conversation_content_events_v2(
 stream_ref INTEGER NOT NULL REFERENCES observation_streams_v5,sequence INTEGER NOT NULL,event_id TEXT NOT NULL,
 workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,direction TEXT NOT NULL,fork_id TEXT NOT NULL,
 phase TEXT NOT NULL,metadata_json BLOB NOT NULL,canonical_chunk_object_ref TEXT,
 canonical_chunk_byte_offset INTEGER,canonical_chunk_byte_count INTEGER,envelope_digest TEXT NOT NULL,
 PRIMARY KEY(stream_ref,sequence),UNIQUE(stream_ref,event_id));
CREATE INDEX IF NOT EXISTS content_events_v2_request ON conversation_content_events_v2(workspace_id,request_id,direction,fork_id,sequence);
CREATE TABLE IF NOT EXISTS observation_feedback_v2(
 feedback_id INTEGER PRIMARY KEY AUTOINCREMENT,stream_ref INTEGER NOT NULL REFERENCES observation_streams_v5,
 rejected_or_acked_sequence INTEGER NOT NULL,event_id TEXT NOT NULL,envelope_digest TEXT NOT NULL,
 feedback_kind TEXT NOT NULL,feedback_json BLOB NOT NULL,created_at_unix_nanos INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS observation_feedback_v2_lookup ON observation_feedback_v2(stream_ref,rejected_or_acked_sequence,event_id,envelope_digest,feedback_id DESC);
"#;

/// Select the same request-owned batch using its existing index order.
pub(super) fn retirement_order(table: &str) -> Option<&'static str> {
    // Match the existing request indices, including their rowid tie-breaker.
    // Feedback and carrier deletion must select exactly the same batch.
    match table {
        "conversation_content_events_v2" => Some("e.direction,e.fork_id,e.sequence,e.rowid"),
        "execution_fact_events" => Some("e.sequence,e.rowid"),
        _ => None,
    }
}

/// Once the request barrier is committed, its accepted feedback is obsolete.
/// Keep event-ID/digest conflicts and stream checkpoints as compact replay proof.
/// Feedback without an accepted request carrier has no proven retirement scope.
pub(super) fn retire_request_feedback(
    db: &Connection,
    workspace: &str,
    request: &str,
    table: &str,
    bounded: bool,
) -> rusqlite::Result<usize> {
    let source = match table {
        "conversation_content_events_v2" => {
            "SELECT e.stream_ref,e.sequence,e.event_id,e.envelope_digest FROM conversation_content_events_v2 e WHERE e.workspace_id=?1 AND e.request_id=?2"
        }
        "execution_fact_events" => {
            "SELECT s.stream_ref,e.sequence,e.event_id,e.envelope_digest FROM execution_fact_events e JOIN observation_streams_v5 s ON s.channel='execution_fact' AND s.producer_id=e.producer_id AND s.producer_epoch=e.producer_epoch AND s.stream_id=e.stream_id WHERE e.workspace_id=?1 AND e.request_id=?2"
        }
        _ => return Ok(0),
    };
    let order = retirement_order(table).ok_or(rusqlite::Error::InvalidQuery)?;
    let retired: bool = cached_sql::query_row(
        db,
        "SELECT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 WHERE workspace_id=?1 AND request_id=?2)",
        params![workspace, request],
        |r| r.get(0),
    )?;
    if !retired {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let limit = if bounded { " LIMIT 128" } else { "" };
    db.execute(&format!("WITH batch AS MATERIALIZED ({source} ORDER BY {order}{limit}) DELETE FROM observation_feedback_v2 WHERE feedback_id IN (SELECT f.feedback_id FROM batch b JOIN observation_feedback_v2 f ON f.stream_ref=b.stream_ref AND f.rejected_or_acked_sequence=b.sequence AND f.event_id=b.event_id AND f.envelope_digest=b.envelope_digest{limit})"), params![workspace,request])
}

pub(super) fn retire_session_feedback(
    db: &Connection,
    workspace: &str,
    session: &str,
) -> rusqlite::Result<()> {
    let requests: Vec<String> = db
        .prepare("SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2")?
        .query_map(params![workspace, session], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for request in requests {
        for table in ["conversation_content_events_v2", "execution_fact_events"] {
            retire_request_feedback(db, workspace, &request, table, false)?;
        }
    }
    Ok(())
}

pub(crate) fn install_reference_triggers(db: &Connection) -> rusqlite::Result<()> {
    for table in [
        "observation_events",
        "conversation_content_events_v2",
        "observation_feedback_v2",
    ] {
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS {table}_stream_insert AFTER INSERT ON {table} BEGIN
          UPDATE observation_streams_v5 SET references_count=references_count+1 WHERE stream_ref=NEW.stream_ref; END;
          CREATE TRIGGER IF NOT EXISTS {table}_stream_delete AFTER DELETE ON {table} BEGIN
          UPDATE observation_streams_v5 SET references_count=references_count-1 WHERE stream_ref=OLD.stream_ref;
          DELETE FROM observation_streams_v5 WHERE stream_ref=OLD.stream_ref AND references_count=0; END;"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_identity_compression_and_corruption() {
        for bytes in [
            b"".as_slice(),
            b"tiny",
            b"abcdefghabcdefghabcdefghabcdefghabcdefghabcdefgh",
        ] {
            assert_eq!(decode(&encode(bytes).unwrap()).unwrap().as_ref(), bytes);
        }
        let mut frame = encode(&vec![b'x'; 100000]).unwrap();
        assert_eq!(frame[0], 1);
        frame[1..5].copy_from_slice(&((MAX_RECORD + 1) as u32).to_le_bytes());
        assert!(decode(&frame).is_err());
        assert!(decode(&[99, 0, 0, 0, 0]).is_err());
        assert!(decode(&[0, 5, 0, 0, 0, 1]).is_err());
        let mut frame = encode(&vec![b'x'; 100000]).unwrap();
        frame.truncate(frame.len() / 2);
        assert!(decode(&frame).is_err());
        let large = vec![0; MAX_RECORD + 1];
        let frame = encode(&large).unwrap();
        assert_eq!(frame[0], 0);
        assert_eq!(decode(&frame).unwrap().as_ref(), large.as_slice());
    }
}
