use std::collections::BTreeSet;
use std::fs;

use hiroute_domain::{
    CanonicalDigest, DeletionDataClass, ObservationCapabilityV1, ObservationPrincipalV1,
    ObservationQueryError, SessionDeletionOutcomeV1, SessionDeletionPreviewV1,
    SessionDeletionSpecV1, SessionId, TombstoneReason,
};
use rusqlite::{Transaction, params};

use super::value_rollup::{
    archive_session_value, delete_session_tombstones, delete_session_value_rollups,
    session_rollup_count, session_tombstone_count,
};
use super::{LocalObservationStore, increment_store_revision, store_revision};

pub(super) const GARBAGE_CANDIDATES_SQL: &str =
    "SELECT b.workspace_id, b.blob_digest, b.object_path FROM content_blobs_v2 b
                     WHERE NOT EXISTS (
                       SELECT 1 FROM content_instances_v2 c
                       WHERE c.workspace_id=b.workspace_id
                         AND c.content_blob_digest=b.blob_digest AND c.state='complete'
                     ) ORDER BY b.rowid LIMIT ?1";

impl LocalObservationStore {
    pub(super) fn preview_session_deletion_base(
        &self,
        principal: &ObservationPrincipalV1,
        spec: &SessionDeletionSpecV1,
    ) -> Result<SessionDeletionPreviewV1, ObservationQueryError> {
        authorize_retention(principal, spec)?;
        let connection = self.connection.lock();
        require_session(&connection, &spec.workspace_id, &spec.session_id)?;
        let revision = store_revision(&connection).map_err(|_| ObservationQueryError::Corrupt)?;
        let counts = deletion_counts(&connection, spec)?;
        let rollup_contributions = session_rollup_count(&connection, spec)?;
        let tombstones = session_tombstone_count(&connection, spec)?;
        #[derive(serde::Serialize)]
        struct DigestInput<'a> {
            schema: &'static str,
            spec: &'a SessionDeletionSpecV1,
            store_revision: u64,
        }
        let change_digest = CanonicalDigest::of(&DigestInput {
            schema: hiroute_domain::OBSERVATION_RETENTION_SCHEMA_V1,
            spec,
            store_revision: revision,
        })
        .map_err(|_| ObservationQueryError::Corrupt)?;
        Ok(SessionDeletionPreviewV1 {
            spec: spec.clone(),
            store_revision: revision,
            change_digest,
            content_instances: counts.0,
            turns: counts.1,
            requests: counts.2,
            receipts: counts.3,
            value_entries: counts.4,
            rollup_contributions,
            tombstones,
        })
    }

    pub fn apply_session_deletion(
        &self,
        principal: &ObservationPrincipalV1,
        spec: &SessionDeletionSpecV1,
        expected_revision: u64,
        accepted_digest: &CanonicalDigest,
        now_ms: i64,
    ) -> Result<SessionDeletionOutcomeV1, ObservationQueryError> {
        // Legacy clients cannot confirm the additional task-content scope.
        self.preview_session_deletion(principal, spec)?;
        self.apply_session_deletion_with_managed(
            principal,
            spec,
            expected_revision,
            accepted_digest,
            now_ms,
            None,
        )
    }

    pub(super) fn apply_session_deletion_with_managed(
        &self,
        principal: &ObservationPrincipalV1,
        spec: &SessionDeletionSpecV1,
        expected_revision: u64,
        accepted_digest: &CanonicalDigest,
        now_ms: i64,
        managed: Option<&hiroute_domain::SessionDeletionPreviewV2>,
    ) -> Result<SessionDeletionOutcomeV1, ObservationQueryError> {
        authorize_retention(principal, spec)?;
        let reviewed = self.preview_session_deletion_base(principal, spec)?;
        if reviewed.store_revision != expected_revision {
            return Err(ObservationQueryError::RevisionConflict);
        }
        if &reviewed.change_digest != accepted_digest {
            return Err(ObservationQueryError::StalePreview);
        }
        let mut connection = self.connection.lock();
        let current = store_revision(&connection).map_err(|_| ObservationQueryError::Corrupt)?;
        if current != expected_revision {
            return Err(ObservationQueryError::RevisionConflict);
        }
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        if let Some(preview) = managed {
            for scope in &preview.managed_scopes {
                crate::managed_text::apply_managed_delete(
                    &transaction,
                    &crate::managed_text::ManagedTextDeletePreview {
                        scope: crate::managed_text::ManagedTextScope {
                            workspace_id: scope.workspace_id.clone(),
                            task_id: scope.task_id.clone(),
                            run_id: scope.run_id.clone(),
                        },
                        through_ms: preview.through_ms,
                        visibility_generation: scope.visibility_generation,
                        reference_count: scope.reference_count,
                    },
                )
                .map_err(|_| ObservationQueryError::StalePreview)?;
            }
        }
        crate::query_v2::invalidate_visibility(&transaction)
            .map_err(|_| ObservationQueryError::Unavailable)?;
        let deletion_through = managed.map(|p| p.through_ms).unwrap_or(now_ms);
        transaction.execute("INSERT INTO observation_session_barriers_v2(workspace_id,session_id,through_ms) VALUES(?1,?2,?3) ON CONFLICT(workspace_id,session_id) DO UPDATE SET through_ms=MAX(through_ms,excluded.through_ms)",params![spec.workspace_id.as_str(),spec.session_id.as_str(),deletion_through]).map_err(|_|ObservationQueryError::Unavailable)?;
        transaction.execute("INSERT OR IGNORE INTO observation_request_tombstones_v2(workspace_id,request_id,deleted_ms) SELECT workspace_id,request_id,?3 FROM logical_requests WHERE workspace_id=?1 AND session_id=?2",params![spec.workspace_id.as_str(),spec.session_id.as_str(),now_ms]).map_err(|_|ObservationQueryError::Unavailable)?;
        delete_content_rows(&transaction, spec)?;
        if spec.delete_rollups {
            delete_session_value_rollups(&transaction, spec)?;
            delete_session_tombstones(&transaction, spec)?;
        }
        if spec.data_class == DeletionDataClass::FactsAndContent {
            if !spec.delete_rollups {
                archive_session_value(&transaction, spec)?;
            }
            delete_detail_rows(&transaction, spec)?;
        }
        let tombstone_reason = if !spec.delete_rollups {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO observation_tombstones
                     (workspace_id, session_id, reason, delete_scope, deleted_at_ms)
                     VALUES (?1, ?2, 'user_deleted', ?3, ?4)",
                    params![
                        spec.workspace_id.as_str(),
                        spec.session_id.as_str(),
                        deletion_scope(spec),
                        now_ms,
                    ],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
            Some(TombstoneReason::UserDeleted)
        } else {
            None
        };
        if spec.data_class == DeletionDataClass::FactsAndContent && spec.delete_rollups {
            transaction
                .execute(
                    "DELETE FROM sessions WHERE workspace_id=?1 AND session_id=?2",
                    params![spec.workspace_id.as_str(), spec.session_id.as_str()],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
        } else {
            transaction
                .execute(
                    "UPDATE sessions SET content_completeness='deleted',
                       tombstone_reason=CASE
                         WHEN NOT ?4 THEN 'user_deleted' ELSE NULL END,
                       facts_completeness=CASE WHEN ?3='facts_and_content' THEN 'unknown'
                                               ELSE facts_completeness END,
                       agent_id=CASE WHEN ?3='facts_and_content' THEN '' ELSE agent_id END,
                       correlation=CASE WHEN ?3='facts_and_content' THEN 'unproven'
                                        ELSE correlation END,
                       updated_at_ms=MAX(updated_at_ms, ?5)
                     WHERE workspace_id=?1 AND session_id=?2",
                    params![
                        spec.workspace_id.as_str(),
                        spec.session_id.as_str(),
                        deletion_scope(spec),
                        spec.delete_rollups,
                        now_ms,
                    ],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
        }
        increment_store_revision(&transaction).map_err(|_| ObservationQueryError::Unavailable)?;
        if let Some(preview) = managed {
            let logical = SessionDeletionOutcomeV1 {
                new_store_revision: store_revision(&transaction)
                    .map_err(|_| ObservationQueryError::Unavailable)?,
                tombstone_reason,
                garbage_collected_blobs: 0,
            };
            let body =
                serde_json::to_string(&logical).map_err(|_| ObservationQueryError::Corrupt)?;
            transaction
                .execute(
                    "INSERT INTO observation_delete_jobs_v2(digest,outcome_json) VALUES(?1,?2)",
                    params![preview.change_digest.as_str(), body],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
        }

        transaction
            .commit()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        drop(connection);
        let garbage_collected_blobs = if managed.is_some() {
            // The durable job already records logical erasure. Physical cleanup
            // is bounded and retryable, and its pending state is returned by V2.
            self.collect_garbage_batch(32).unwrap_or(0)
        } else {
            self.collect_garbage()?
        };
        let new_store_revision = {
            let connection = self.connection.lock();
            store_revision(&connection).map_err(|_| ObservationQueryError::Corrupt)?
        };
        Ok(SessionDeletionOutcomeV1 {
            new_store_revision,
            tombstone_reason,
            garbage_collected_blobs,
        })
    }

    pub fn run_retention(&self, now_ms: i64) -> Result<u64, ObservationQueryError> {
        let cutoff = now_ms
            .checked_sub(self.retention.detail_retention_ms)
            .ok_or(ObservationQueryError::InvalidQuery)?;
        let mut connection = self.connection.lock();
        let sessions = {
            let mut statement = connection
                .prepare(
                    "SELECT workspace_id, session_id FROM sessions
                     WHERE updated_at_ms <= ?1 AND tombstone_reason IS NULL",
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
            statement
                .query_map([cutoff], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| ObservationQueryError::Unavailable)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ObservationQueryError::Corrupt)?
        };
        if sessions.is_empty() {
            drop(connection);
            let _ = self.collect_garbage()?;
            return Ok(0);
        }
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        crate::query_v2::invalidate_visibility(&transaction)
            .map_err(|_| ObservationQueryError::Unavailable)?;
        for (workspace, session) in &sessions {
            let workspace = hiroute_domain::WorkspaceId::parse(workspace.clone())
                .map_err(|_| ObservationQueryError::Corrupt)?;
            let session =
                SessionId::parse(session.clone()).map_err(|_| ObservationQueryError::Corrupt)?;
            let spec = SessionDeletionSpecV1 {
                workspace_id: workspace.clone(),
                session_id: session.clone(),
                data_class: DeletionDataClass::FactsAndContent,
                delete_rollups: false,
            };
            transaction.execute("INSERT INTO observation_session_barriers_v2(workspace_id,session_id,through_ms) VALUES(?1,?2,?3) ON CONFLICT(workspace_id,session_id) DO UPDATE SET through_ms=MAX(through_ms,excluded.through_ms)",params![workspace.as_str(),session.as_str(),cutoff]).map_err(|_|ObservationQueryError::Unavailable)?;
            transaction.execute("INSERT OR IGNORE INTO observation_request_tombstones_v2(workspace_id,request_id,deleted_ms) SELECT workspace_id,request_id,?3 FROM logical_requests WHERE workspace_id=?1 AND session_id=?2",params![workspace.as_str(),session.as_str(),now_ms]).map_err(|_|ObservationQueryError::Unavailable)?;
            delete_content_rows(&transaction, &spec)?;
            archive_session_value(&transaction, &spec)?;
            delete_detail_rows(&transaction, &spec)?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO observation_tombstones
                     (workspace_id, session_id, reason, delete_scope, deleted_at_ms)
                     VALUES (?1, ?2, 'retention_expired', 'details_and_content', ?3)",
                    params![workspace.as_str(), session.as_str(), now_ms],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
            transaction
                .execute(
                    "UPDATE sessions SET agent_id='', correlation='unproven',
                       facts_completeness='unknown', content_completeness='expired',
                       tombstone_reason='retention_expired'
                     WHERE workspace_id=?1 AND session_id=?2",
                    params![workspace.as_str(), session.as_str()],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
        }
        increment_store_revision(&transaction).map_err(|_| ObservationQueryError::Unavailable)?;
        transaction
            .commit()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        drop(connection);
        let _ = self.collect_garbage()?;
        Ok(sessions.len() as u64)
    }

    pub fn collect_garbage(&self) -> Result<u64, ObservationQueryError> {
        let removed = self.collect_garbage_batch(200)?;
        self.remove_orphan_blob_files()?;
        self.remove_orphan_staging_entries()?;
        Ok(removed)
    }

    pub fn collect_garbage_batch(&self, limit: usize) -> Result<u64, ObservationQueryError> {
        if limit == 0 || limit > 200 {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let mut connection = self.connection.lock();
        let unreferenced = {
            let mut statement = connection
                .prepare(GARBAGE_CANDIDATES_SQL)
                .map_err(|_| ObservationQueryError::Unavailable)?;
            statement
                .query_map([limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|_| ObservationQueryError::Unavailable)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ObservationQueryError::Corrupt)?
        };
        let transaction = connection
            .transaction()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        for (workspace, digest, path) in &unreferenced {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ObservationQueryError::Unavailable),
            }
            transaction
                .execute(
                    "DELETE FROM content_blobs_v2 WHERE workspace_id=?1 AND blob_digest=?2",
                    params![workspace, digest],
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
        }
        if !unreferenced.is_empty() {
            increment_store_revision(&transaction)
                .map_err(|_| ObservationQueryError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| ObservationQueryError::Unavailable)?;
        Ok(unreferenced.len() as u64)
    }

    fn remove_orphan_blob_files(&self) -> Result<(), ObservationQueryError> {
        let connection = self.connection.lock();
        let referenced = {
            let mut statement = connection
                .prepare("SELECT object_path FROM content_blobs_v2")
                .map_err(|_| ObservationQueryError::Unavailable)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|_| ObservationQueryError::Unavailable)?
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|_| ObservationQueryError::Corrupt)?
        };
        for entry in
            fs::read_dir(&self.blob_root).map_err(|_| ObservationQueryError::Unavailable)?
        {
            let path = entry
                .map_err(|_| ObservationQueryError::Unavailable)?
                .path();
            if path.is_file() && !referenced.contains(&path.to_string_lossy().into_owned()) {
                fs::remove_file(path).map_err(|_| ObservationQueryError::Unavailable)?;
            }
        }
        Ok(())
    }

    fn remove_orphan_staging_entries(&self) -> Result<(), ObservationQueryError> {
        let connection = self.connection.lock();
        let active = {
            let mut statement = connection
                .prepare(
                    "SELECT workspace_id, content_id FROM content_instances_v2
                     WHERE state='installing'",
                )
                .map_err(|_| ObservationQueryError::Unavailable)?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| ObservationQueryError::Unavailable)?
                .map(|row| {
                    row.map(|(workspace_id, content_id)| {
                        super::portable_hash(&format!("{workspace_id}\0{content_id}"))
                    })
                })
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|_| ObservationQueryError::Corrupt)?
        };
        for entry in
            fs::read_dir(&self.staging_root).map_err(|_| ObservationQueryError::Unavailable)?
        {
            let path = entry
                .map_err(|_| ObservationQueryError::Unavailable)?
                .path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(ObservationQueryError::Corrupt)?;
            if !active.contains(name) {
                if path.is_dir() {
                    fs::remove_dir_all(path).map_err(|_| ObservationQueryError::Unavailable)?;
                } else {
                    fs::remove_file(path).map_err(|_| ObservationQueryError::Unavailable)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn authorize_retention(
    principal: &ObservationPrincipalV1,
    spec: &SessionDeletionSpecV1,
) -> Result<(), ObservationQueryError> {
    if principal.workspace_id == spec.workspace_id
        && principal.allows(ObservationCapabilityV1::ManageRetention)
    {
        Ok(())
    } else {
        Err(ObservationQueryError::Unauthorized)
    }
}

fn require_session(
    connection: &rusqlite::Connection,
    workspace: &hiroute_domain::WorkspaceId,
    session: &SessionId,
) -> Result<(), ObservationQueryError> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND session_id=?2)",
            params![workspace.as_str(), session.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    if exists {
        Ok(())
    } else {
        Err(ObservationQueryError::NotFound)
    }
}

fn deletion_counts(
    connection: &rusqlite::Connection,
    spec: &SessionDeletionSpecV1,
) -> Result<(u64, u64, u64, u64, u64), ObservationQueryError> {
    let sql = [
        "SELECT COUNT(*) FROM content_instances_v2 c WHERE c.workspace_id=?1 AND c.request_id IN
         (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
        "SELECT COUNT(*) FROM turns WHERE workspace_id=?1 AND session_id=?2",
        "SELECT COUNT(*) FROM logical_requests WHERE workspace_id=?1 AND session_id=?2",
        "SELECT COUNT(*) FROM routing_receipts WHERE workspace_id=?1 AND session_id=?2",
        "SELECT COUNT(*) FROM value_ledger_entries WHERE workspace_id=?1 AND session_id=?2",
    ];
    let mut counts = [0_u64; 5];
    let selected = match spec.data_class {
        DeletionDataClass::ContentOnly => 1,
        DeletionDataClass::FactsAndContent => sql.len(),
    };
    for (index, query) in sql.into_iter().take(selected).enumerate() {
        let value: i64 = connection
            .query_row(
                query,
                params![spec.workspace_id.as_str(), spec.session_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| ObservationQueryError::Unavailable)?;
        counts[index] = value
            .try_into()
            .map_err(|_| ObservationQueryError::Corrupt)?;
    }
    Ok((counts[0], counts[1], counts[2], counts[3], counts[4]))
}

fn delete_content_rows(
    transaction: &Transaction<'_>,
    spec: &SessionDeletionSpecV1,
) -> Result<(), ObservationQueryError> {
    let workspace = spec.workspace_id.as_str();
    let session = spec.session_id.as_str();
    transaction
        .execute(
            "DELETE FROM observation_sensitive_payloads_v2 WHERE workspace_id=?1 AND session_id=?2",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    // Cover legacy inline wire carriers as well. Preserve immutable digests,
    // sequence and identities; a tombstone is not a synthesized wire envelope.
    transaction.execute("UPDATE execution_fact_events SET envelope_json=json_object('managed_sensitive_ref','fact:'||envelope_digest) WHERE workspace_id=?1 AND session_id=?2",params![workspace,session])
        .map_err(|_|ObservationQueryError::Unavailable)?;
    transaction.execute("UPDATE routing_receipts SET body_json=json_object('managed_sensitive_ref','receipt:'||body_digest) WHERE workspace_id=?1 AND session_id=?2",params![workspace,session])
        .map_err(|_|ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM transcript_roots_v2 WHERE workspace_id=?1 AND conversation_id=?2",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM conversation_content_events_v2 WHERE workspace_id=?1 AND request_id IN
         (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM content_chunks_v2 WHERE workspace_id=?1 AND content_id IN
         (SELECT content_id FROM content_instances_v2 WHERE workspace_id=?1 AND request_id IN
          (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2))",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM request_content_refs_v2 WHERE workspace_id=?1 AND request_id IN
         (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM content_instances_v2 WHERE workspace_id=?1 AND request_id IN
         (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM content_message_instances_v2 WHERE workspace_id=?1 AND request_id IN
         (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
            params![workspace, session],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction.execute(
        "DELETE FROM conversation_content_streams_v2 WHERE workspace_id=?1 AND conversation_id=?2",
        params![workspace, session],
    ).map_err(|_| ObservationQueryError::Unavailable)?;
    // Rebuildable text is still sensitive content. Erase orphaned index text
    // inside the same logical deletion transaction, including unpublished builds.
    transaction.execute(
        "DELETE FROM observation_text_blocks_v2 WHERE workspace=?1 AND NOT EXISTS(
           SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=?1 AND c.content_blob_digest=observation_text_blocks_v2.digest AND c.state='complete')",
        [workspace],
    ).map_err(|_|ObservationQueryError::Unavailable)?;
    transaction.execute(
        "DELETE FROM observation_text_index_v2 WHERE workspace=?1 AND NOT EXISTS(
           SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=?1 AND c.content_blob_digest=observation_text_index_v2.digest AND c.state='complete')",
        [workspace],
    ).map_err(|_|ObservationQueryError::Unavailable)?;
    Ok(())
}

fn delete_detail_rows(
    transaction: &Transaction<'_>,
    spec: &SessionDeletionSpecV1,
) -> Result<(), ObservationQueryError> {
    transaction
        .execute(
            "DELETE FROM plan_quality_segments WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "UPDATE observation_meta SET value=CAST(value AS INTEGER)+1
             WHERE key='plan_quality_generation'",
            [],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    crate::valuation::archive_session(
        transaction,
        spec.workspace_id.as_str(),
        spec.session_id.as_str(),
        !spec.delete_rollups,
    )
    .map_err(|_| ObservationQueryError::Unavailable)?;
    for table in [
        "observation_run_links",
        "observation_link_events",
        "observation_attempt_models_v2",
        "observation_safe_facts_v2",
    ] {
        transaction
            .execute(
                &format!(
                    "DELETE FROM {table} WHERE workspace_id=?1 AND request_id IN
                (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)"
                ),
                params![spec.workspace_id.as_str(), spec.session_id.as_str()],
            )
            .map_err(|_| ObservationQueryError::Unavailable)?;
    }
    transaction
        .execute(
            "DELETE FROM execution_fact_events WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM observation_gaps WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM routing_receipts WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM value_ledger_entries WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM attempts WHERE workspace_id=?1 AND request_id IN
             (SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM logical_requests WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    transaction
        .execute(
            "DELETE FROM turns WHERE workspace_id=?1 AND session_id=?2",
            params![spec.workspace_id.as_str(), spec.session_id.as_str()],
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    Ok(())
}

fn deletion_scope(spec: &SessionDeletionSpecV1) -> &'static str {
    match spec.data_class {
        DeletionDataClass::ContentOnly => "content_only",
        DeletionDataClass::FactsAndContent => "facts_and_content",
    }
}
