//! Bounded discovery for the background collector. Explicit deletion retains
//! its immediate candidate lookup; discovery never grants deletion authority.
use hiroute_domain::ObservationQueryError;
use rusqlite::{Connection, OpenFlags, params};

use super::LocalObservationStore;

const SCAN_ROWS: usize = 256;
const DELETE_ROWS: usize = 32;

// Apply the limit to blob identities, not to the number of orphan matches.
// The existing primary/partial indexes make both reference probes point lookups.
const SCAN_SQL: &str = "SELECT b.workspace_id,b.blob_digest,
       NOT EXISTS(SELECT 1 FROM content_instances_v2 c
         WHERE c.workspace_id=b.workspace_id AND c.content_blob_digest=b.blob_digest
           AND c.state='complete')
       AND NOT EXISTS(SELECT 1 FROM observation_capacity_blobs_v5 p
         WHERE p.workspace=b.workspace_id AND p.digest=b.blob_digest)
     FROM content_blobs_v2 b
     WHERE (b.workspace_id,b.blob_digest)>(?1,?2)
     ORDER BY b.workspace_id,b.blob_digest LIMIT ?3";

/// Rebuildable process-local progress. A restart or completed sweep starts at
/// the beginning, so changes behind the cursor cannot be permanently missed.
#[derive(Default)]
pub(crate) struct BlobGcCursor {
    after: Option<(String, String)>,
    connection: Option<Connection>,
}

impl LocalObservationStore {
    /// Returns (removed, more scan work). Commit progress only after successful
    /// deletion, so a failed pass revisits the same window without losing work.
    pub(crate) fn collect_garbage_incremental(
        &self,
        cursor: &mut BlobGcCursor,
    ) -> Result<(u64, bool), ObservationQueryError> {
        // Keep one reader for this worker, but no transaction between pages.
        // Reopening every 256 rows would repeatedly parse the entire SQL schema.
        if cursor.connection.is_none() {
            cursor.connection = Some(
                Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|_| ObservationQueryError::Unavailable)?,
            );
        }
        let connection = cursor.connection.as_ref().expect("GC reader initialized");
        let (workspace, digest) = cursor
            .after
            .as_ref()
            .map(|(w, d)| (w.as_str(), d.as_str()))
            .unwrap_or(("", ""));
        let mut statement = connection
            .prepare(SCAN_SQL)
            .map_err(|_| ObservationQueryError::Unavailable)?;
        let rows = statement
            .query_map(params![workspace, digest, SCAN_ROWS], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(|_| ObservationQueryError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationQueryError::Corrupt)?;
        drop(statement);

        let mut candidates = Vec::new();
        let mut consumed = 0;
        for (workspace, digest, orphan) in &rows {
            consumed += 1;
            if *orphan {
                candidates.push((workspace.clone(), digest.clone()));
                if candidates.len() == DELETE_ROWS {
                    break;
                }
            }
        }
        let removed = self.collect_unreferenced_candidates(&candidates)?;
        let more = rows.len() == SCAN_ROWS || consumed < rows.len();
        cursor.after = if more {
            rows.get(consumed - 1)
                .map(|(w, d, _)| (w.clone(), d.clone()))
        } else {
            None
        };
        Ok((removed, more))
    }
}

#[cfg(test)]
mod tests;
