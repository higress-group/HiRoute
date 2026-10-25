use hiroute_domain::{
    CanonicalDigest, GatewayPublicationRevision, PortError, PortErrorCode, PortResult,
    PreparePublicationOutcome, PublicationRecordV1, PublicationRepositoryPort, WorkspaceId,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::ControlStore;

impl ControlStore {
    /// Removes only the exact still-prepared record owned by a compensating Operation.
    pub fn discard_prepared_publication(
        &self,
        workspace: &WorkspaceId,
        publication_revision: GatewayPublicationRevision,
        digest: &CanonicalDigest,
    ) -> PortResult<()> {
        let changed = self
            .connection
            .borrow()
            .execute(
                "DELETE FROM gateway_publications
                 WHERE workspace_id = ?1 AND publication_revision = ?2
                   AND digest = ?3 AND state = 'prepared'",
                params![
                    workspace.as_str(),
                    publication_revision.get(),
                    digest.as_str()
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.discard.delete"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err(port(
                PortErrorCode::Conflict,
                "publication.discard.ownership",
            ))
        }
    }
}

impl PublicationRepositoryPort for ControlStore {
    fn prepare_publication(
        &self,
        record: &PublicationRecordV1,
        expected_active_revision: Option<GatewayPublicationRevision>,
    ) -> PortResult<PreparePublicationOutcome> {
        let publication = record
            .verify()
            .map_err(|_| port(PortErrorCode::InvalidData, "publication.prepare.verify"))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.begin"))?;

        let existing: Option<(String, Vec<u8>, String)> = transaction
            .query_row(
                "SELECT digest, publication_bytes, state FROM gateway_publications
                 WHERE workspace_id = ?1 AND publication_revision = ?2",
                params![
                    record.workspace_id.as_str(),
                    record.publication_revision.get()
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.existing"))?;
        if let Some((digest, bytes, state)) = existing {
            if digest == record.digest.as_str()
                && bytes.as_slice() == record.bytes.as_slice()
                && matches!(state.as_str(), "prepared" | "active")
            {
                transaction
                    .commit()
                    .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.commit"))?;
                return Ok(PreparePublicationOutcome::ExistingSame);
            }
            return Err(port(
                PortErrorCode::Conflict,
                "publication.prepare.revision",
            ));
        }

        let active: Option<u64> = transaction
            .query_row(
                "SELECT active_revision FROM publication_heads WHERE workspace_id = ?1",
                params![record.workspace_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.head"))?
            .flatten();
        if active != expected_active_revision.map(GatewayPublicationRevision::get)
            || active.is_some_and(|active| active >= record.publication_revision.get())
        {
            return Err(port(PortErrorCode::Conflict, "publication.prepare.cas"));
        }
        let active_row: Option<(u64, String, Vec<u8>)> = transaction
            .query_row(
                "SELECT publication_revision, digest, publication_bytes
                 FROM gateway_publications
                 WHERE workspace_id = ?1 AND state = 'active'",
                params![record.workspace_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.active"))?;
        if active_row.as_ref().map(|row| row.0) != active {
            return Err(port(
                PortErrorCode::Corrupt,
                "publication.prepare.head-mismatch",
            ));
        }
        if let Some((revision, digest, bytes)) = active_row {
            let active_record = PublicationRecordV1::from_parts(
                record.workspace_id.clone(),
                GatewayPublicationRevision::new(revision)
                    .map_err(|_| port(PortErrorCode::Corrupt, "publication.prepare.revision"))?,
                CanonicalDigest::parse(digest)
                    .map_err(|_| port(PortErrorCode::Corrupt, "publication.prepare.digest"))?,
                bytes,
            )
            .map_err(|_| port(PortErrorCode::Corrupt, "publication.prepare.active-verify"))?;
            let active_publication = active_record
                .verify()
                .map_err(|_| port(PortErrorCode::Corrupt, "publication.prepare.active-verify"))?;
            publication
                .validate_transition_from(&active_publication)
                .map_err(|_| port(PortErrorCode::Conflict, "publication.prepare.transition"))?;
        }
        let prepared_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM gateway_publications
                    WHERE workspace_id = ?1 AND state = 'prepared'
                 )",
                params![record.workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.pending"))?;
        if prepared_exists {
            return Err(port(PortErrorCode::Conflict, "publication.prepare.pending"));
        }
        transaction
            .execute(
                "INSERT INTO gateway_publications(
                    workspace_id, publication_revision, digest, publication_bytes,
                    state, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 'prepared', unixepoch(), unixepoch())",
                params![
                    record.workspace_id.as_str(),
                    record.publication_revision.get(),
                    record.digest.as_str(),
                    record.bytes.as_slice(),
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.insert"))?;
        super::current_dependencies::record_publication_proofs(&transaction, &publication)?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.prepare.commit"))?;
        Ok(PreparePublicationOutcome::Created)
    }

    fn prepared_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        load_by_state(self, workspace, "prepared")
    }

    fn active_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        load_by_state(self, workspace, "active")
    }

    fn last_known_good_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        load_by_state(self, workspace, "lkg")
    }

    fn mark_publication_active(
        &self,
        workspace: &WorkspaceId,
        publication_revision: GatewayPublicationRevision,
        digest: &CanonicalDigest,
    ) -> PortResult<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.begin"))?;
        let row: Option<(String, String)> = transaction
            .query_row(
                "SELECT digest, state FROM gateway_publications
                 WHERE workspace_id = ?1 AND publication_revision = ?2",
                params![workspace.as_str(), publication_revision.get()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.read"))?;
        let Some((stored_digest, state)) = row else {
            return Err(port(
                PortErrorCode::NotFound,
                "publication.activate.missing",
            ));
        };
        if stored_digest != digest.as_str() || !matches!(state.as_str(), "prepared" | "active") {
            return Err(port(
                PortErrorCode::Conflict,
                "publication.activate.identity",
            ));
        }
        if state == "active" {
            transaction
                .commit()
                .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.commit"))?;
            return Ok(());
        }
        let prior_active: Option<u64> = transaction
            .query_row(
                "SELECT active_revision FROM publication_heads WHERE workspace_id = ?1",
                params![workspace.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.head"))?
            .flatten();
        transaction
            .execute(
                "UPDATE gateway_publications SET state = 'historical', updated_at = unixepoch()
                 WHERE workspace_id = ?1 AND state = 'lkg'",
                params![workspace.as_str()],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.old-lkg"))?;
        if let Some(prior_active) = prior_active {
            let changed = transaction
                .execute(
                    "UPDATE gateway_publications SET state = 'lkg', updated_at = unixepoch()
                     WHERE workspace_id = ?1 AND publication_revision = ?2 AND state = 'active'",
                    params![workspace.as_str(), prior_active],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.lkg"))?;
            if changed != 1 {
                return Err(port(
                    PortErrorCode::Corrupt,
                    "publication.activate.head-mismatch",
                ));
            }
        }
        let changed = transaction
            .execute(
                "UPDATE gateway_publications SET state = 'active', updated_at = unixepoch()
                 WHERE workspace_id = ?1 AND publication_revision = ?2 AND state = 'prepared'",
                params![workspace.as_str(), publication_revision.get()],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.swap"))?;
        if changed != 1 {
            return Err(port(PortErrorCode::Conflict, "publication.activate.swap"));
        }
        transaction
            .execute(
                "INSERT INTO publication_heads(
                    workspace_id, active_revision, lkg_revision, generation, updated_at
                 ) VALUES (?1, ?2, ?3, 1, unixepoch())
                 ON CONFLICT(workspace_id) DO UPDATE SET
                    active_revision = excluded.active_revision,
                    lkg_revision = excluded.lkg_revision,
                    generation = publication_heads.generation + 1,
                    updated_at = unixepoch()",
                params![workspace.as_str(), publication_revision.get(), prior_active],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "publication.activate.head-write",
                )
            })?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "publication.activate.commit"))?;
        Ok(())
    }
}

fn load_by_state(
    store: &ControlStore,
    workspace: &WorkspaceId,
    state: &'static str,
) -> PortResult<Option<PublicationRecordV1>> {
    let row: Option<(u64, String, Vec<u8>)> = store
        .connection
        .borrow()
        .query_row(
            "SELECT publication_revision, digest, publication_bytes
             FROM gateway_publications WHERE workspace_id = ?1 AND state = ?2",
            params![workspace.as_str(), state],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|_| port(PortErrorCode::Unavailable, "publication.read"))?;
    // Do not cache current membership or trust a digest label alone. The single
    // SELECT above binds state/revision/digest/bytes in a fresh SQLite snapshot.
    // Exact unchanged bytes can retain their value-owned authentication proof.
    let mut cache = store.active_publication_cache.borrow_mut();
    if state == "active" {
        if let (Some((revision, digest, bytes)), Some(cached)) = (&row, cache.as_ref())
            && cached.workspace_id == *workspace
            && cached.publication_revision.get() == *revision
            && cached.digest.as_str() == digest
            && cached.bytes.as_slice() == bytes.as_slice()
        {
            return Ok(Some(cached.clone()));
        }
        *cache = None;
    }
    let record = row
        .map(|(revision, digest, bytes)| {
            PublicationRecordV1::from_parts(
                workspace.clone(),
                GatewayPublicationRevision::new(revision)
                    .map_err(|_| port(PortErrorCode::Corrupt, "publication.revision"))?,
                CanonicalDigest::parse(digest)
                    .map_err(|_| port(PortErrorCode::Corrupt, "publication.digest"))?,
                bytes,
            )
            .map_err(|_| port(PortErrorCode::Corrupt, "publication.verify"))
        })
        .transpose()?;
    // Account conservatively for decoded allocations and derived projections.
    // Oversized records retain the same strict uncached reader, without eviction
    // churn or an unbounded per-workspace map. This is not an RSS guarantee.
    if state == "active"
        && let Some(record) = &record
        && record
            .bytes
            .len()
            .saturating_mul(8)
            .saturating_add(64 * 1024)
            <= 64 * 1024 * 1024
    {
        *cache = Some(record.clone());
    }
    Ok(record)
}

fn port(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[cfg(test)]
mod routing_publication_storage_tests {
    use std::fs;

    use crate::test_tempdir as tempdir;
    use hiroute_domain::{AliasRegistryV1, GatewayPublicationV1};

    use super::*;

    fn owner_only(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    fn record(workspace: &WorkspaceId, revision: u64) -> PublicationRecordV1 {
        let publication = GatewayPublicationV1::new(
            workspace.clone(),
            GatewayPublicationRevision::new(revision).unwrap(),
            AliasRegistryV1::default(),
            Vec::new(),
        )
        .unwrap();
        PublicationRecordV1::from_publication(workspace.clone(), &publication).unwrap()
    }

    #[test]
    fn routing_publication_store_keeps_prepared_active_and_lkg_revisions_distinct() {
        let directory = tempdir().unwrap();
        owner_only(directory.path());
        let store = ControlStore::open(
            &crate::test_storage_authority(),
            directory.path().join("control.db"),
            directory.path().join("backups"),
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        let first = record(&workspace, 1);
        assert_eq!(
            store.prepare_publication(&first, None).unwrap(),
            PreparePublicationOutcome::Created
        );
        assert_eq!(
            store.prepared_publication(&workspace).unwrap(),
            Some(first.clone())
        );
        store
            .mark_publication_active(&workspace, first.publication_revision, &first.digest)
            .unwrap();
        assert_eq!(
            store.active_publication(&workspace).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            store.prepare_publication(&first, None).unwrap(),
            PreparePublicationOutcome::ExistingSame
        );
        store
            .mark_publication_active(&workspace, first.publication_revision, &first.digest)
            .unwrap();

        let second = record(&workspace, 2);
        store
            .prepare_publication(&second, Some(first.publication_revision))
            .unwrap();
        store
            .mark_publication_active(&workspace, second.publication_revision, &second.digest)
            .unwrap();
        assert_eq!(store.active_publication(&workspace).unwrap(), Some(second));
        assert_eq!(
            store.last_known_good_publication(&workspace).unwrap(),
            Some(first)
        );
    }

    #[test]
    fn routing_publication_store_rejects_stale_active_revision_before_insert() {
        let directory = tempdir().unwrap();
        owner_only(directory.path());
        let store = ControlStore::open(
            &crate::test_storage_authority(),
            directory.path().join("control.db"),
            directory.path().join("backups"),
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        let first = record(&workspace, 1);
        store.prepare_publication(&first, None).unwrap();
        store
            .mark_publication_active(&workspace, first.publication_revision, &first.digest)
            .unwrap();
        let second = record(&workspace, 2);
        let error = store.prepare_publication(&second, None).unwrap_err();
        assert_eq!(error.code, PortErrorCode::Conflict);
        assert!(store.prepared_publication(&workspace).unwrap().is_none());
    }

    #[test]
    fn active_publication_shares_only_a_fresh_exact_row_and_rejects_warmed_tampering() {
        let directory = tempdir().unwrap();
        owner_only(directory.path());
        let store = ControlStore::open(
            &crate::test_storage_authority(),
            directory.path().join("control.db"),
            directory.path().join("backups"),
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        let first = record(&workspace, 1);
        store.prepare_publication(&first, None).unwrap();
        store
            .mark_publication_active(&workspace, first.publication_revision, &first.digest)
            .unwrap();
        let cold = store.active_publication(&workspace).unwrap().unwrap();
        for _ in 0..20 {
            let warm = store.active_publication(&workspace).unwrap().unwrap();
            assert!(std::sync::Arc::ptr_eq(&cold.bytes, &warm.bytes));
            assert_eq!(warm, first);
        }
        for sql in [
            "UPDATE gateway_publications SET publication_revision=99 WHERE state='active'",
            "UPDATE gateway_publications SET digest='sha256:0000000000000000000000000000000000000000000000000000000000000000' WHERE state='active'",
        ] {
            store.connection.borrow().execute_batch("BEGIN").unwrap();
            store.connection.borrow().execute_batch(sql).unwrap();
            assert_eq!(
                store.active_publication(&workspace).unwrap_err().code,
                PortErrorCode::Corrupt
            );
            store.connection.borrow().execute_batch("ROLLBACK").unwrap();
            assert_eq!(
                store.active_publication(&workspace).unwrap(),
                Some(first.clone())
            );
        }
        let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
        let mut wrong = first.bytes.as_ref().clone();
        wrong.push(b' ');
        external.execute_batch("BEGIN").unwrap();
        external
            .execute(
                "UPDATE gateway_publications SET publication_bytes=?1 WHERE state='active'",
                [&wrong],
            )
            .unwrap();
        assert_eq!(
            store.active_publication(&workspace).unwrap(),
            Some(first.clone())
        );
        external.execute_batch("COMMIT").unwrap();
        assert_eq!(
            store.active_publication(&workspace).unwrap_err().code,
            PortErrorCode::Corrupt
        );
        external
            .execute(
                "UPDATE gateway_publications SET publication_bytes=?1 WHERE state='active'",
                [first.bytes.as_slice()],
            )
            .unwrap();
        assert_eq!(
            store.active_publication(&workspace).unwrap(),
            Some(first.clone())
        );
        let second = record(&workspace, 2);
        store
            .prepare_publication(&second, Some(first.publication_revision))
            .unwrap();
        store
            .mark_publication_active(&workspace, second.publication_revision, &second.digest)
            .unwrap();
        assert_eq!(
            store.active_publication(&workspace).unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            store.last_known_good_publication(&workspace).unwrap(),
            Some(first)
        );
        external
            .execute("DELETE FROM gateway_publications WHERE state='active'", [])
            .unwrap();
        assert!(store.active_publication(&workspace).unwrap().is_none());
    }
}
