use crate::{LocalObservationStore, content::ContentDigestAccumulator};
use hiroute_domain::{ContentBlobDigest, ObservationQueryError as Error};
use rusqlite::{OptionalExtension, params};
use std::{
    fs::File,
    io::Read,
    time::{Duration, Instant},
};

const BLOCK: usize = 64 * 1024;
const OVERLAP: usize = 1024;
const SCAN_ROWS: usize = 256;
const NEXT_SOURCE_SQL: &str = "SELECT b.rowid,b.workspace_id,b.blob_digest,b.object_path,b.byte_count,b.media_type FROM content_blobs_v2 b
             WHERE b.rowid>?1 AND b.rowid<=?2 AND b.state='complete' AND (b.media_type LIKE 'text/%' OR b.media_type='application/json' OR b.media_type LIKE 'application/vnd.hiroute.%')
               AND NOT EXISTS(SELECT 1 FROM observation_text_index_v2 i WHERE i.workspace=b.workspace_id AND i.digest=b.blob_digest)
               AND EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=b.workspace_id AND c.content_blob_digest=b.blob_digest AND c.state='complete')
             ORDER BY b.rowid LIMIT 1";

struct Building {
    workspace: String,
    digest: String,
    expected_bytes: u64,
    file: File,
    accumulator: ContentDigestAccumulator,
    consumed: u64,
    ordinal: u64,
    utf8_tail: Vec<u8>,
    overlap: String,
}

pub struct TextIndexBuilder {
    _owner: crate::maintenance::LocalWorkerGuard,
    building: Option<Building>,
    scan_after: i64,
}
impl TextIndexBuilder {
    pub fn new(store: &LocalObservationStore) -> Result<Self, Error> {
        let owner = crate::maintenance::LocalWorkerGuard::acquire(&store.index_running)
            .ok_or(Error::Unavailable)?;
        // Rebuild interrupted unpublished sources. Readers never depended on
        // those blocks, so restart cannot invalidate an already published hit.
        let mut connection = store.connection.lock();
        let transaction = connection.transaction().map_err(|_| Error::Unavailable)?;
        transaction.execute_batch(
            "DELETE FROM observation_text_blocks_v2 WHERE EXISTS(
                SELECT 1 FROM observation_text_index_v2 i WHERE i.workspace=observation_text_blocks_v2.workspace
                    AND i.digest=observation_text_blocks_v2.digest AND i.state='building');
             DELETE FROM observation_text_index_v2 WHERE state='building';"
        ).map_err(|_| Error::Unavailable)?;
        transaction.commit().map_err(|_| Error::Unavailable)?;
        Ok(Self {
            building: None,
            scan_after: 0,
            _owner: owner,
        })
    }

    /// One production maintenance owner, at most 8 MiB input and 500 ms between
    /// block operations per cycle. Ordinary filesystem IO is not hard realtime.
    pub fn cycle(&mut self, store: &LocalObservationStore) -> Result<usize, Error> {
        let started = Instant::now();
        let mut bytes = 0;
        let mut completed = 0;
        while bytes < 8 * 1024 * 1024 && started.elapsed() < Duration::from_millis(500) {
            if self.building.is_none() {
                let (building, exhausted) = next(store, &mut self.scan_after)?;
                self.building = building;
                if self.building.is_none() {
                    if exhausted {
                        break;
                    }
                    continue;
                }
            }
            let job = self.building.as_mut().ok_or(Error::Unavailable)?;
            let mut buffer = vec![0u8; BLOCK];
            let count = match job.file.read(&mut buffer) {
                Ok(count) => count,
                Err(_) => {
                    let job = self.building.take().ok_or(Error::Unavailable)?;
                    finish(store, job, false)?;
                    return Err(Error::Unavailable);
                }
            };
            buffer.truncate(count);
            bytes += count;
            job.accumulator.update(&buffer);
            let mut text_bytes = std::mem::take(&mut job.utf8_tail);
            text_bytes.extend_from_slice(&buffer);
            let valid = match std::str::from_utf8(&text_bytes) {
                Ok(_) => text_bytes.len(),
                Err(error) if error.error_len().is_none() && count != 0 => error.valid_up_to(),
                Err(_) => {
                    let job = self.building.take().ok_or(Error::Unavailable)?;
                    finish(store, job, false)?;
                    completed += 1;
                    continue;
                }
            };
            job.utf8_tail = text_bytes[valid..].to_vec();
            let new_text = std::str::from_utf8(&text_bytes[..valid]).map_err(|_| Error::Corrupt)?;
            if !new_text.is_empty() {
                let original_start = job.consumed.saturating_sub(job.overlap.len() as u64);
                let text = format!("{}{new_text}", job.overlap);
                let (folded, offsets) = super::fold(&text);
                let connection = store.connection.lock();
                connection.execute(
                    "INSERT INTO observation_text_blocks_v2(workspace,digest,ordinal,original_start,primary_start,folded,offsets,original)
                     SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM observation_text_index_v2 WHERE workspace=?1 AND digest=?2 AND state='building')",
                    params![job.workspace,job.digest,job.ordinal,original_start,job.consumed,folded,offsets,text],
                ).map_err(|_| Error::Unavailable)?;
                job.consumed += valid as u64;
                job.ordinal += 1;
                let mut overlap_start = text.len().saturating_sub(OVERLAP);
                while !text.is_char_boundary(overlap_start) {
                    overlap_start += 1;
                }
                job.overlap = text[overlap_start..].into();
            }
            if count == 0 {
                let job = self.building.take().ok_or(Error::Unavailable)?;
                finish(store, job, true)?;
                completed += 1;
                if completed == 200 {
                    break;
                }
            }
        }
        Ok(completed)
    }
}

fn next(store: &LocalObservationStore, after: &mut i64) -> Result<(Option<Building>, bool), Error> {
    let source = {
        let connection = store.connection.lock();
        // Bound the rows visited while holding the ingestion lock, including
        // already indexed and temporarily unreferenced blobs. Wrap on exhaustion
        // so deletion/reappearance and late completion cannot be skipped forever.
        let through: Option<i64> = connection.query_row(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM content_blobs_v2 WHERE rowid>?1 ORDER BY rowid LIMIT ?2)",
            params![*after, SCAN_ROWS], |row| row.get(0),
        ).map_err(|_| Error::Unavailable)?;
        let Some(through) = through else {
            *after = 0;
            return Ok((None, true));
        };
        let source: Option<(i64, String, String, String, u64, String)> = connection
            .query_row(NEXT_SOURCE_SQL, params![*after, through], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .optional()
            .map_err(|_| Error::Unavailable)?;
        *after = source.as_ref().map_or(through, |row| row.0);
        source
    };
    let Some((_, workspace, digest, path, expected_bytes, media)) = source else {
        return Ok((None, false));
    };
    let file = File::open(path);
    store.connection.lock().execute(
        "INSERT OR IGNORE INTO observation_text_index_v2(workspace,digest,state) VALUES(?1,?2,?3)",
        params![workspace,digest,if file.is_ok() {"building"} else {"failed"}],
    ).map_err(|_| Error::Unavailable)?;
    let file = file.map_err(|_| Error::Unavailable)?;
    Ok((
        Some(Building {
            workspace,
            digest,
            expected_bytes,
            file,
            accumulator: store.authority.content_accumulator(&media),
            consumed: 0,
            ordinal: 0,
            utf8_tail: Vec::new(),
            overlap: String::new(),
        }),
        false,
    ))
}

fn finish(store: &LocalObservationStore, job: Building, readable: bool) -> Result<(), Error> {
    let (actual, bytes) = job.accumulator.finish();
    let valid = readable
        && bytes == job.expected_bytes
        && ContentBlobDigest::parse(job.digest.clone()).ok().as_ref() == Some(&actual);
    let mut connection = store.connection.lock();
    let transaction = connection.transaction().map_err(|_| Error::Unavailable)?;
    let live: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM content_instances_v2 WHERE workspace_id=?1 AND content_blob_digest=?2 AND state='complete')",
        params![job.workspace,job.digest], |row|row.get(0),
    ).map_err(|_| Error::Unavailable)?;
    if !valid || !live {
        transaction
            .execute(
                "DELETE FROM observation_text_blocks_v2 WHERE workspace=?1 AND digest=?2",
                params![job.workspace, job.digest],
            )
            .map_err(|_| Error::Unavailable)?;
    }
    transaction
        .execute(
            "UPDATE observation_text_index_v2 SET state=?3,published=(SELECT COALESCE(MAX(published),0)+1 FROM observation_text_index_v2) WHERE workspace=?1 AND digest=?2",
            params![
                job.workspace,
                job.digest,
                if valid && live { "ready" } else { "failed" }
            ],
        )
        .map_err(|_| Error::Unavailable)?;
    transaction.commit().map_err(|_| Error::Unavailable)
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
