use std::fs::{self, OpenOptions};
use std::io::{Read, Write};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::*;
use crate::LocalObservationStore;

impl LocalObservationStore {
    /// Register an event before appending chunks. Stable event retries return the
    /// same reference; changing its timestamp or purpose is a conflict.
    pub fn managed_text_put(
        &self,
        input: &ManagedTextInput,
        now_ms: i64,
    ) -> Result<ManagedTextRef, ManagedTextError> {
        let scope = input.scope.key()?;
        if !valid_id(&input.source_event_id)
            || input.original_created_at_ms < 0
            || input.original_created_at_ms > now_ms
            || input.source_revision > i64::MAX as u64
        {
            return Err(ManagedTextError::Invalid);
        }
        let deadline = input
            .original_created_at_ms
            .checked_add(RETENTION_MS)
            .ok_or(ManagedTextError::Invalid)?;
        let identity = serde_json::to_vec(&(&scope, &input.source_event_id, input.source_revision))
            .map_err(|_| ManagedTextError::Invalid)?;
        let id = format!(
            "text-{}",
            self.authority
                .content_blob_digest("managed-text-id/v1", &identity)
        );
        let metadata = serde_json::to_vec(&(
            &identity,
            input.purpose,
            input.original_created_at_ms,
            input
                .import_origin
                .as_ref()
                .map(|origin| (&origin.opaque_id, origin.visibility_generation)),
        ))
        .map_err(|_| ManagedTextError::Invalid)?;
        let input_digest = digest(&metadata);
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO managed_text_scopes(scope) VALUES(?1)",
            [&scope],
        )?;
        if let Some(existing) = load(&transaction, &input.scope, &id, now_ms)? {
            let original: String = transaction.query_row(
                "SELECT input_digest FROM managed_text_refs WHERE id=?1",
                [&id],
                |row| row.get(0),
            )?;
            if original != input_digest {
                return Err(ManagedTextError::Conflict);
            }
            return Ok(existing);
        }
        if now_ms >= deadline {
            return Err(ManagedTextError::Unavailable);
        }
        let (_, cutoff) = barrier(&transaction, &scope)?;
        if input.original_created_at_ms <= cutoff {
            return Err(ManagedTextError::Unavailable);
        }
        if let Some(origin) = &input.import_origin {
            let original = checked(&transaction, &input.scope, origin, now_ms)?;
            if original.state != ManagedTextState::Complete
                || original.original_retention_deadline_ms != deadline
            {
                return Err(ManagedTextError::Unavailable);
            }
        }
        transaction.execute(
            "INSERT INTO managed_text_refs(id,scope,input_digest,created_ms,deadline_ms,state)
             VALUES(?1,?2,?3,?4,?5,'pending')",
            params![
                id,
                scope,
                input_digest,
                input.original_created_at_ms,
                deadline
            ],
        )?;
        let result =
            load(&transaction, &input.scope, &id, now_ms)?.ok_or(ManagedTextError::Storage)?;
        crate::store::increment_store_revision(&transaction)
            .map_err(|_| ManagedTextError::Storage)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Ordinals are contiguous and immutable. Each call writes at most 64 KiB;
    /// replaying an identical chunk is safe after a caller or process crash.
    pub fn managed_text_append(
        &self,
        scope: &ManagedTextScope,
        reference: &ManagedTextRef,
        ordinal: u64,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<(), ManagedTextError> {
        if bytes.is_empty() || bytes.len() > CHUNK_BYTES || ordinal > i64::MAX as u64 {
            return Err(ManagedTextError::Invalid);
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let current = checked(&transaction, scope, reference, now_ms)?;
        let chunk_digest = digest(bytes);
        let previous: Option<(String, bool)> = transaction
            .query_row(
                "SELECT digest,ready FROM managed_text_chunks WHERE ref_id=?1 AND ordinal=?2",
                params![current.opaque_id, ordinal],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((previous, ready)) = previous {
            if previous != chunk_digest {
                return Err(ManagedTextError::Conflict);
            }
            if ready {
                return Ok(());
            }
        }
        if current.state != ManagedTextState::Pending {
            return Err(ManagedTextError::Conflict);
        }
        let count: u64 = transaction.query_row(
            "SELECT chunk_count FROM managed_text_refs WHERE id=?1",
            [&current.opaque_id],
            |row| row.get(0),
        )?;
        if ordinal != count {
            return Err(ManagedTextError::Conflict);
        }
        // Journal the object before creating it. Crash leftovers therefore remain
        // enumerable by ordinary expiry/deletion GC, including incomplete writes.
        transaction.execute(
            "INSERT OR IGNORE INTO managed_text_chunks(ref_id,ordinal,digest,size,ready) VALUES(?1,?2,?3,?4,0)",
            params![current.opaque_id, ordinal, chunk_digest, bytes.len() as u64],
        )?;
        transaction.commit()?;
        let path = self.managed_chunk_path(&current.opaque_id, ordinal);
        // Only an unpublished, journaled chunk can reach this point. The writer
        // guard serializes retries and deletion; readers select ready chunks only.
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|_| ManagedTextError::Storage)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| ManagedTextError::Storage)?;
        // Windows refuses to open a directory handle for fsync (access denied),
        // while NTFS journals directory metadata itself. The chunk file above is
        // still fsynced, so only the directory barrier is Unix-specific.
        #[cfg(unix)]
        fs::File::open(&self.blob_root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| ManagedTextError::Storage)?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE managed_text_chunks SET ready=1 WHERE ref_id=?1 AND ordinal=?2",
            params![current.opaque_id, ordinal],
        )?;
        transaction.execute(
            "UPDATE managed_text_refs SET chunk_count=chunk_count+1 WHERE id=?1",
            [&current.opaque_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn managed_text_finish(
        &self,
        scope: &ManagedTextScope,
        reference: &ManagedTextRef,
        chunk_count: u64,
        now_ms: i64,
    ) -> Result<ManagedTextRef, ManagedTextError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let current = checked(&transaction, scope, reference, now_ms)?;
        let count: u64 = transaction.query_row(
            "SELECT chunk_count FROM managed_text_refs WHERE id=?1",
            [&current.opaque_id],
            |row| row.get(0),
        )?;
        let unfinished: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM managed_text_chunks WHERE ref_id=?1 AND ready=0)",
            [&current.opaque_id],
            |row| row.get(0),
        )?;
        if chunk_count != count || unfinished {
            return Err(ManagedTextError::Conflict);
        }
        transaction.execute(
            "UPDATE managed_text_refs SET state='complete' WHERE id=?1",
            [&current.opaque_id],
        )?;
        let result = load(&transaction, scope, &current.opaque_id, now_ms)?
            .ok_or(ManagedTextError::Storage)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Status always comes from persisted visibility, never the caller's cached
    /// reference. The authenticated application must check content permission.
    pub fn managed_text_resolve(
        &self,
        scope: &ManagedTextScope,
        reference: &ManagedTextRef,
        now_ms: i64,
    ) -> Result<ManagedTextRef, ManagedTextError> {
        if scope != &reference.scope {
            return Err(ManagedTextError::ScopeMismatch);
        }
        let connection = self.managed_reader()?;
        load(&connection, scope, &reference.opaque_id, now_ms)?.ok_or(ManagedTextError::Unavailable)
    }

    /// Bounded external-file I/O without the writer mutex. A second visibility
    /// check linearizes delivery: a deletion committed before it wins.
    pub fn managed_text_read(
        &self,
        scope: &ManagedTextScope,
        reference: &ManagedTextRef,
        first_chunk: u64,
        max_chunks: usize,
        now_ms: i64,
    ) -> Result<ManagedTextPage, ManagedTextError> {
        if max_chunks == 0 || max_chunks > PAGE_BYTES / CHUNK_BYTES || first_chunk > i64::MAX as u64
        {
            return Err(ManagedTextError::Invalid);
        }
        let connection = self.managed_reader()?;
        let current = checked(&connection, scope, reference, now_ms)?;
        let chunks = {
            let mut statement = connection.prepare(
                "SELECT ordinal,digest,size FROM managed_text_chunks WHERE ref_id=?1 AND ready=1 AND ordinal>=?2
                 ORDER BY ordinal LIMIT ?3",
            )?;
            statement
                .query_map(
                    params![current.opaque_id, first_chunk, max_chunks + 1],
                    |row| {
                        Ok((
                            row.get::<_, u64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, usize>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut bytes = Vec::new();
        for (ordinal, expected, size) in chunks.iter().take(max_chunks) {
            let chunk = read_chunk(
                &self.managed_chunk_path(&current.opaque_id, *ordinal),
                *size,
            )?;
            if digest(&chunk) != *expected {
                return Err(ManagedTextError::Storage);
            }
            bytes.extend_from_slice(&chunk);
        }
        checked(&connection, scope, &current, now_ms)?;
        Ok(ManagedTextPage {
            reference: current,
            bytes,
            next_chunk: chunks.get(max_chunks).map(|chunk| chunk.0),
        })
    }

    fn managed_reader(&self) -> Result<Connection, ManagedTextError> {
        Ok(Connection::open_with_flags(
            &self.activity_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?)
    }

    pub(super) fn managed_chunk_path(&self, id: &str, ordinal: u64) -> std::path::PathBuf {
        self.blob_root
            .join(format!("managed-{}-{ordinal}", digest(id.as_bytes())))
    }
}

pub(super) fn barrier(
    connection: &Connection,
    scope: &str,
) -> Result<(u64, i64), ManagedTextError> {
    Ok(connection.query_row(
        "SELECT generation,deleted_through_ms FROM managed_text_scopes WHERE scope=?1",
        [scope],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

fn load(
    connection: &Connection,
    scope: &ManagedTextScope,
    id: &str,
    now_ms: i64,
) -> Result<Option<ManagedTextRef>, ManagedTextError> {
    if !valid_id(id) || now_ms < 0 {
        return Err(ManagedTextError::Invalid);
    }
    let key = scope.key()?;
    let row: Option<(String, i64, u64)> = connection
        .query_row(
            "SELECT r.state,r.deadline_ms,s.generation FROM managed_text_refs r
         JOIN managed_text_scopes s ON s.scope=r.scope WHERE r.id=?1 AND r.scope=?2",
            params![id, key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(state, deadline, generation)| {
        let state = match state.as_str() {
            _ if now_ms >= deadline => ManagedTextState::Expired,
            "deleted" => ManagedTextState::Deleted,
            "pending" => ManagedTextState::Pending,
            "complete" => ManagedTextState::Complete,
            _ => return Err(ManagedTextError::Storage),
        };
        Ok(ManagedTextRef {
            opaque_id: id.into(),
            scope: scope.clone(),
            visibility_generation: generation,
            original_retention_deadline_ms: deadline,
            state,
        })
    })
    .transpose()
}

fn checked(
    connection: &Connection,
    scope: &ManagedTextScope,
    reference: &ManagedTextRef,
    now_ms: i64,
) -> Result<ManagedTextRef, ManagedTextError> {
    if scope != &reference.scope {
        return Err(ManagedTextError::ScopeMismatch);
    }
    let current = load(connection, scope, &reference.opaque_id, now_ms)?
        .ok_or(ManagedTextError::Unavailable)?;
    if matches!(
        current.state,
        ManagedTextState::Deleted | ManagedTextState::Expired | ManagedTextState::Missing
    ) {
        return Err(ManagedTextError::Unavailable);
    }
    if current.visibility_generation != reference.visibility_generation {
        return Err(ManagedTextError::Stale);
    }
    Ok(current)
}

fn read_chunk(path: &std::path::Path, size: usize) -> Result<Vec<u8>, ManagedTextError> {
    if size > CHUNK_BYTES {
        return Err(ManagedTextError::Storage);
    }
    let file = fs::File::open(path).map_err(|_| ManagedTextError::Storage)?;
    let mut bytes = Vec::with_capacity(size);
    file.take((CHUNK_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ManagedTextError::Storage)?;
    if bytes.len() != size {
        return Err(ManagedTextError::Storage);
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
