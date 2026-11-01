//! Streaming content-object installation and durable append replay verification.

use crate::store::cached_sql;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};

use hiroute_domain::{
    CanonicalDigest, ContentId, ConversationContentEnvelopeV1, ConversationContentPhaseV2,
    ObservationAckV2, ObservationBlobAcknowledgementV1, ObservationDigestSubjectV1,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::store::LocalObservationStore;

use super::lifecycle::ContentProjectionError;

/// Only the latest committed operation can be retried. Its immutable digest
/// was checked by the commit head; an append also checks the surviving bytes.
/// A small commit ACK makes no claim about historical body availability.
pub(crate) fn verify_content_replay(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    _digest: &CanonicalDigest,
    _acknowledgement: &ObservationAckV2,
) -> Result<(), ContentProjectionError> {
    if envelope.phase != ConversationContentPhaseV2::Append {
        return Ok(());
    }
    if super::capture::skip_context(transaction, envelope)? {
        return Ok(());
    }
    let content_id = envelope
        .content_id
        .as_ref()
        .ok_or(ContentProjectionError::ContentCorrupt)?;
    let (state, digest): (String, String) = cached_sql::query_row(transaction,
        "SELECT state,content_blob_digest FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2",
        params![envelope.correlation.workspace_id.as_str(),content_id.as_str()],
        |row| Ok((row.get(0)?,row.get(1)?)))
        .map_err(|_| ContentProjectionError::ContentCorrupt)?;
    if state == "complete" {
        return verify_acknowledged_blob(
            store,
            transaction,
            envelope,
            &ObservationBlobAcknowledgementV1 {
                content_id: content_id.to_string(),
                digest,
            },
        );
    }
    if state != "installing" {
        return Err(ContentProjectionError::ContentCorrupt);
    }
    let ordinal = envelope
        .chunk_ordinal
        .ok_or(ContentProjectionError::ContentCorrupt)?;
    let bytes = decode_base64(
        envelope
            .canonical_bytes_base64
            .as_deref()
            .ok_or(ContentProjectionError::ContentCorrupt)?,
    )?;
    let (path,count,event): (String,i64,String) = cached_sql::query_row(transaction,
        "SELECT object_path,byte_count,event_id FROM content_chunks_v2 WHERE workspace_id=?1 AND content_id=?2 AND chunk_ordinal=?3",
        params![envelope.correlation.workspace_id.as_str(),content_id.as_str(),i64::from(ordinal)],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
        .map_err(|_| ContentProjectionError::ContentCorrupt)?;
    if count != i64_from_u64(bytes.len() as u64)? || event != envelope.event_id.as_str() {
        return Err(ContentProjectionError::ContentCorrupt);
    }
    let mut input = File::open(
        store
            .object_path(&path)
            .map_err(|_| ContentProjectionError::ContentCorrupt)?,
    )
    .map_err(|_| ContentProjectionError::ContentStorage)?;
    // Active chunk files contain exactly one append; no permanent offset ledger.
    let mut buffer = [0_u8; 16 * 1024];
    let mut offset = 0;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| ContentProjectionError::ContentStorage)?;
        if read == 0 {
            break;
        }
        if bytes.get(offset..offset + read) != Some(&buffer[..read]) {
            return Err(ContentProjectionError::ContentCorrupt);
        }
        offset += read;
    }
    if offset != bytes.len() {
        return Err(ContentProjectionError::ContentCorrupt);
    }
    Ok(())
}

fn verify_acknowledged_blob(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    acknowledgement: &ObservationBlobAcknowledgementV1,
) -> Result<(), ContentProjectionError> {
    let (digest, path, byte_count, media): (String, String, i64, String) = cached_sql::query_row(
        transaction,
        "SELECT c.content_blob_digest, b.object_path, b.byte_count, b.media_type
             FROM content_instances_v2 c JOIN content_blobs_v2 b
               ON b.workspace_id=c.workspace_id AND b.blob_digest=c.content_blob_digest
             WHERE c.workspace_id=?1 AND c.request_id=?2 AND c.direction=?3 AND c.fork_id=?4
               AND c.content_id=?5 AND c.state='complete' AND b.state='complete'",
        params![
            envelope.correlation.workspace_id.as_str(),
            envelope.correlation.request_id.as_str(),
            envelope.direction.as_str(),
            envelope.fork_id,
            acknowledgement.content_id,
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .map_err(|_| ContentProjectionError::ContentCorrupt)?;
    if digest != acknowledgement.digest || byte_count < 0 {
        return Err(ContentProjectionError::ContentCorrupt);
    }
    let mut input = File::open(
        store
            .object_path(&path)
            .map_err(|_| ContentProjectionError::ContentCorrupt)?,
    )
    .map_err(|_| ContentProjectionError::ContentStorage)?;
    let mut accumulator = store.authority.content_accumulator(&media);
    let mut observed_count = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| ContentProjectionError::ContentStorage)?;
        if read == 0 {
            break;
        }
        observed_count = observed_count
            .checked_add(read as u64)
            .ok_or(ContentProjectionError::ContentCorrupt)?;
        accumulator.update(&buffer[..read]);
    }
    let (observed_digest, digested_count) = accumulator.finish();
    if observed_count != digested_count
        || observed_count != byte_count as u64
        || observed_digest.as_str() != digest
    {
        return Err(ContentProjectionError::ContentCorrupt);
    }
    Ok(())
}

pub(crate) fn finalize_content(
    store: &LocalObservationStore,
    transaction: &Transaction<'_>,
    envelope: &ConversationContentEnvelopeV1,
    content_id: &str,
    durability: &mut super::durability::ContentDurability,
) -> Result<ObservationBlobAcknowledgementV1, ContentProjectionError> {
    #[cfg(test)]
    let _finalizing = crate::store::ingest_profile::span("blob_finalize");
    let (media, expected_digest, expected_count, state): (String, String, Option<i64>, String) =
        cached_sql::query_row(
            transaction,
            "SELECT canonical_media_type, content_blob_digest, expected_byte_count, state
         FROM content_instances_v2 WHERE workspace_id=?1 AND content_id=?2",
            params![envelope.correlation.workspace_id.as_str(), content_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if state == "complete" {
        return Ok(ObservationBlobAcknowledgementV1 {
            content_id: content_id.into(),
            digest: expected_digest,
        });
    }
    let mut statement = transaction
        .prepare_cached(
            "SELECT object_path FROM content_chunks_v2 WHERE workspace_id=?1 AND content_id=?2
         ORDER BY chunk_ordinal",
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    let paths = statement
        .query_map(
            params![envelope.correlation.workspace_id.as_str(), content_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| ContentProjectionError::ActivityStorage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ContentProjectionError::ActivityStorage)?;
    if paths.is_empty() {
        return Err(ContentProjectionError::MissingBlob(vec![
            ObservationBlobAcknowledgementV1 {
                content_id: content_id.into(),
                digest: expected_digest,
            },
        ]));
    }
    let directory = store.staging_directory(
        &envelope.correlation.workspace_id,
        &ContentId::parse(content_id.to_owned())
            .map_err(|_| ContentProjectionError::ContentCorrupt)?,
    );
    let install_path = directory.join("install.tmp");
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&install_path)
        .map_err(|_| ContentProjectionError::ContentStorage)?;
    let mut accumulator = store.authority.content_accumulator(&media);
    let mut buffer = [0_u8; 16 * 1024];
    for path in paths {
        let mut input = File::open(
            store
                .object_path(&path)
                .map_err(|_| ContentProjectionError::ContentCorrupt)?,
        )
        .map_err(|_| ContentProjectionError::ContentStorage)?;
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|_| ContentProjectionError::ContentStorage)?;
            if read == 0 {
                break;
            }
            accumulator.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|_| ContentProjectionError::ContentStorage)?;
        }
    }
    store.note_object_path(&install_path);
    let (actual_digest, byte_count) = accumulator.finish();
    if actual_digest.as_str() != expected_digest {
        return Err(ContentProjectionError::DigestMismatch {
            subject: ObservationDigestSubjectV1::ContentBlob,
            subject_id: content_id.into(),
            expected: expected_digest,
            rejected: actual_digest.to_string(),
        });
    }
    if expected_count.is_some_and(|expected| expected != byte_count as i64) {
        return Err(ContentProjectionError::DigestMismatch {
            subject: ObservationDigestSubjectV1::ContentBlob,
            subject_id: content_id.into(),
            expected: expected_count.unwrap_or_default().to_string(),
            rejected: byte_count.to_string(),
        });
    }
    let blob_path = store.blob_path(actual_digest.as_str());
    if blob_path.exists() {
        if blob_path
            .metadata()
            .map_err(|_| ContentProjectionError::ContentStorage)?
            .len()
            != byte_count
            || !files_equal(&blob_path, &install_path)?
        {
            return Err(ContentProjectionError::ContentCorrupt);
        }
        drop(output);
        fs::remove_file(&install_path).map_err(|_| ContentProjectionError::ContentStorage)?;
        // An unacknowledged earlier transaction may have installed this name
        // before its durability barrier failed. Sync the retained target,
        // rather than an equal but discarded temporary inode.
        durability.retain(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&blob_path)
                .map_err(|_| ContentProjectionError::ContentStorage)?,
        );
    } else {
        fs::rename(&install_path, &blob_path)
            .map_err(|_| ContentProjectionError::ContentStorage)?;
        durability.retain(output);
    }
    store.note_object_path(&install_path);
    store.note_object_path(&blob_path);
    cached_sql::execute(transaction, "INSERT INTO content_blobs_v2
         (workspace_id, blob_digest, object_path, byte_count, media_type, state, created_at_unix_nanos)
         VALUES (?1, ?2, ?3, ?4, ?5, 'complete', ?6)
         ON CONFLICT(workspace_id, blob_digest) DO NOTHING",
        params![envelope.correlation.workspace_id.as_str(), actual_digest.as_str(), store.object_locator(&blob_path).map_err(|_|ContentProjectionError::ContentCorrupt)?,
            i64_from_u64(byte_count)?, media, i64_from_u64(envelope.occurred_at_unix_nanos)?],
    ).map_err(|_| ContentProjectionError::ActivityStorage)?;
    cached_sql::execute(
        transaction,
        "UPDATE content_instances_v2 SET state='complete', accumulated_byte_count=?3
         WHERE workspace_id=?1 AND content_id=?2",
        params![
            envelope.correlation.workspace_id.as_str(),
            content_id,
            i64_from_u64(byte_count)?
        ],
    )
    .map_err(|_| ContentProjectionError::ActivityStorage)?;
    cached_sql::execute(
        transaction,
        "DELETE FROM content_chunks_v2 WHERE workspace_id=?1 AND content_id=?2",
        params![envelope.correlation.workspace_id.as_str(), content_id,],
    )
    .map_err(|_| ContentProjectionError::ActivityStorage)?;
    Ok(ObservationBlobAcknowledgementV1 {
        content_id: content_id.into(),
        digest: actual_digest.to_string(),
    })
}

fn i64_from_u64(value: u64) -> Result<i64, ContentProjectionError> {
    value
        .try_into()
        .map_err(|_| ContentProjectionError::Invalid("integer"))
}

fn files_equal(
    left: &std::path::Path,
    right: &std::path::Path,
) -> Result<bool, ContentProjectionError> {
    let mut left = File::open(left).map_err(|_| ContentProjectionError::ContentStorage)?;
    let mut right = File::open(right).map_err(|_| ContentProjectionError::ContentStorage)?;
    let mut a = [0_u8; 16 * 1024];
    let mut b = [0_u8; 16 * 1024];
    loop {
        let ar = left
            .read(&mut a)
            .map_err(|_| ContentProjectionError::ContentStorage)?;
        let br = right
            .read(&mut b)
            .map_err(|_| ContentProjectionError::ContentStorage)?;
        if ar != br || a[..ar] != b[..br] {
            return Ok(false);
        }
        if ar == 0 {
            return Ok(true);
        }
    }
}

pub(crate) fn decode_base64(value: &str) -> Result<Vec<u8>, ContentProjectionError> {
    if !value.len().is_multiple_of(4) {
        return Err(ContentProjectionError::Invalid("canonical_bytes_base64"));
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (group_index, group) in value.as_bytes().chunks_exact(4).enumerate() {
        let last = group_index + 1 == value.len() / 4;
        let a = base64_value(group[0])?;
        let b = base64_value(group[1])?;
        let c_pad = group[2] == b'=';
        let d_pad = group[3] == b'=';
        if c_pad && !d_pad || (c_pad || d_pad) && !last {
            return Err(ContentProjectionError::Invalid("canonical_bytes_base64"));
        }
        let c = if c_pad { 0 } else { base64_value(group[2])? };
        let d = if d_pad { 0 } else { base64_value(group[3])? };
        output.push((a << 2) | (b >> 4));
        if !c_pad {
            output.push((b << 4) | (c >> 2));
        }
        if !d_pad {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}
fn base64_value(byte: u8) -> Result<u8, ContentProjectionError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(ContentProjectionError::Invalid("canonical_bytes_base64")),
    }
}
