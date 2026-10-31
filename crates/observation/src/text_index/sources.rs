//! A coalesced lower bound for sources that may newly need indexing.
//! Notifications commit with their source, including writes on peer connections.
use crate::{LocalObservationStore, writer::ObservationStoreError};
use hiroute_domain::ObservationQueryError as Error;
use rusqlite::{Connection, OptionalExtension};

const DIRTY_FROM: &str = "text_index_dirty_from_v1";

pub(super) fn migrate(db: &Connection) -> Result<(), ObservationStoreError> {
    // The index is derived and restart still makes one bounded full pass. Do
    // not enumerate historical sources while opening an existing store.
    for (name, event, guard, workspace, digest) in [
        (
            "blob_insert",
            "INSERT ON content_blobs_v2",
            "1",
            "NEW.workspace_id",
            "NEW.blob_digest",
        ),
        (
            "blob_update",
            "UPDATE OF state,media_type ON content_blobs_v2",
            "1",
            "NEW.workspace_id",
            "NEW.blob_digest",
        ),
        (
            "instance_insert",
            "INSERT ON content_instances_v2",
            "NEW.state='complete'",
            "NEW.workspace_id",
            "NEW.content_blob_digest",
        ),
        (
            "instance_update",
            "UPDATE OF state,workspace_id,content_blob_digest ON content_instances_v2",
            "NEW.state='complete' AND (OLD.state IS NOT NEW.state OR OLD.workspace_id IS NOT NEW.workspace_id OR OLD.content_blob_digest IS NOT NEW.content_blob_digest)",
            "NEW.workspace_id",
            "NEW.content_blob_digest",
        ),
        (
            "index_delete",
            "DELETE ON observation_text_index_v2",
            "1",
            "OLD.workspace",
            "OLD.digest",
        ),
    ] {
        db.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS observation_text_dirty_{name}
             AFTER {event} WHEN {guard} BEGIN
               INSERT INTO observation_meta(key,value)
               SELECT '{DIRTY_FROM}',CAST(b.rowid AS TEXT) FROM content_blobs_v2 b
               WHERE b.workspace_id={workspace} AND b.blob_digest={digest}
                 AND b.state='complete'
                 AND (b.media_type LIKE 'text/%' OR b.media_type='application/json'
                      OR b.media_type LIKE 'application/vnd.hiroute.%')
                 AND NOT EXISTS(SELECT 1 FROM observation_text_index_v2 i
                     WHERE i.workspace=b.workspace_id AND i.digest=b.blob_digest)
                 AND EXISTS(SELECT 1 FROM content_instances_v2 c
                     WHERE c.workspace_id=b.workspace_id AND c.content_blob_digest=b.blob_digest
                       AND c.state='complete')
               ON CONFLICT(key) DO UPDATE SET value=CAST(MIN(
                   CAST(observation_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT);
             END;"
        )).map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    Ok(())
}

pub(super) fn take_dirty(store: &LocalObservationStore) -> Result<Option<i64>, Error> {
    let mut db = store.connection.lock();
    let present = db
        .query_row(
            "SELECT 1 FROM observation_meta WHERE key=?1",
            [DIRTY_FROM],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    if present.is_none() {
        return Ok(None);
    }
    // A peer can lower the watermark after the cheap presence check. Consume
    // the actual value atomically, not a stale value read before the transaction.
    let tx = db.transaction().map_err(|_| Error::Unavailable)?;
    let value: Option<String> = tx
        .query_row(
            "DELETE FROM observation_meta WHERE key=?1 RETURNING value",
            [DIRTY_FROM],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    let from = value
        .map(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(Error::Corrupt)
        })
        .transpose()?;
    tx.commit().map_err(|_| Error::Unavailable)?;
    Ok(from)
}
