//! Rebuildable successful-journal facts, never native status or execution authority.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::Arc;

// Account for the retained decoded projection, not discarded input. Bulk scans
// never evict the working set; an actual current match can replace an older entry.
const MAX_ENTRIES: usize = 128;
const MAX_ACCOUNTED_BYTES: usize = 512 * 1024 * 1024;

#[derive(Default)]
pub(super) struct AgentHistoryCache {
    entries: BTreeMap<String, Entry>,
    bytes: usize,
    store_identity: Option<String>,
    read_epoch: Option<ReadEpoch>,
    use_sequence: u64,
    #[cfg(test)]
    pub(super) decodes: usize,
    #[cfg(test)]
    pub(super) fingerprints: usize,
    #[cfg(test)]
    pub(super) budget_override: Option<(usize, usize)>,
}
struct Entry {
    fingerprint: [u8; 32],
    bytes: usize,
    facts: Arc<SucceededAgentOperationV1>,
    verified_epoch: Option<ReadEpoch>,
    last_used: u64,
}

struct VerifiedFact {
    facts: Arc<SucceededAgentOperationV1>,
    fingerprint: [u8; 32],
    bytes: usize,
    verified_epoch: Option<ReadEpoch>,
}

/// A connection-local proof of unchanged database bytes, never a business revision.
/// External commits, local DML and DDL invalidate this epoch, without discarding
/// decoded facts whose exact bytes can be revalidated. Only our own read
/// transaction can mint it: caller transactions may roll
/// back previously observed writes without changing SQLite's total-change counter.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ReadEpoch {
    data_version: i64,
    local_changes: u64,
    main_schema: i64,
    temp_schema: i64,
}
impl ReadEpoch {
    fn capture(connection: &Connection) -> PortResult<Self> {
        Ok(Self {
            data_version: connection
                .query_row("PRAGMA data_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
            local_changes: connection.total_changes(),
            main_schema: connection
                .query_row("PRAGMA main.schema_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
            temp_schema: connection
                .query_row("PRAGMA temp.schema_version", [], |r| r.get(0))
                .map_err(|_| unavailable())?,
        })
    }
}

/// These are the columns of the selected row, not another matching row found by
/// an EXISTS query. Raw substitution between two valid operations must fail.
#[derive(Clone)]
pub(super) struct AgentHistoryRow {
    pub(super) operation_id: String,
    pub(super) workspace_id: String,
    pub(super) principal: String,
    pub(super) operation_kind: String,
    pub(super) idempotency_key: String,
    pub(super) request_digest: String,
    pub(super) accepted_digest: String,
    pub(super) state: String,
    pub(super) generation: u64,
    pub(super) encoded: String,
}
impl AgentHistoryRow {
    pub(super) fn matches(&self, durable: &DurableOperation) -> bool {
        self.operation_id == durable.operation_id.as_str()
            && self.workspace_id == durable.workspace_id.as_str()
            && self.principal == durable.idempotency.principal
            && self.operation_kind == durable.idempotency.operation_kind
            && self.idempotency_key == durable.idempotency.key
            && self.request_digest == durable.request_digest.as_str()
            && self.accepted_digest == durable.accepted_digest.as_str()
            && self.state == durable.state.as_str()
            && self.generation == durable.generation
    }
    fn fields(&self) -> [&str; 9] {
        [
            &self.operation_id,
            &self.workspace_id,
            &self.principal,
            &self.operation_kind,
            &self.idempotency_key,
            &self.request_digest,
            &self.accepted_digest,
            &self.state,
            &self.encoded,
        ]
    }
    pub(super) fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            operation_id: row.get(0)?,
            workspace_id: row.get(1)?,
            principal: row.get(2)?,
            operation_kind: row.get(3)?,
            idempotency_key: row.get(4)?,
            request_digest: row.get(5)?,
            accepted_digest: row.get(6)?,
            state: row.get(7)?,
            generation: row.get(8)?,
            encoded: row.get(9)?,
        })
    }
    fn fingerprint(&self, connection: &Connection) -> PortResult<[u8; 32]> {
        let mut hash = Sha256::new();
        // Internal, process-local hash domain; no serialized/public schema.
        hash.update(b"verified-agent-history-cache\0");
        for field in self.fields() {
            frame(&mut hash, field.as_bytes());
        }
        hash.update(self.generation.to_be_bytes());
        let mut statement = connection.prepare(
            "SELECT step_no,step_kind,state,step_json FROM operation_steps WHERE operation_id=?1 ORDER BY step_no"
        ).map_err(|_| unavailable())?;
        let rows = statement
            .query_map([&self.operation_id], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| unavailable())?;
        let mut count = 0u64;
        for row in rows {
            let (number, kind, state, encoded) = row.map_err(|_| corrupt())?;
            hash.update(number.to_be_bytes());
            for field in [&kind, &state, &encoded] {
                frame(&mut hash, field.as_bytes());
            }
            count += 1;
        }
        let mut payloads = connection.prepare("SELECT digest,payload_json FROM operation_payloads WHERE operation_id=?1 ORDER BY digest")
            .map_err(|_| unavailable())?;
        for row in payloads
            .query_map([&self.operation_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|_| unavailable())?
        {
            let (digest, payload) = row.map_err(|_| corrupt())?;
            frame(&mut hash, digest.as_bytes());
            frame(&mut hash, payload.as_bytes());
        }
        hash.update(count.to_be_bytes());
        // Invalid counts are rejected even before a cache lookup.
        if count != 6 {
            return Err(corrupt());
        }
        Ok(hash.finalize().into())
    }
}
fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
fn unavailable() -> PortError {
    port(PortErrorCode::Unavailable, "control.agent-history.snapshot")
}
fn corrupt() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-read.journal")
}

impl AgentHistoryCache {
    fn bind_store(&mut self, connection: &Connection, owned_read: bool) -> PortResult<()> {
        let store: String = connection
            .query_row(
                "SELECT store_uuid FROM storage_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| unavailable())?;
        if store.is_empty() {
            return Err(corrupt());
        }
        if self.store_identity.as_ref() != Some(&store) {
            self.entries.clear();
            self.bytes = 0;
            self.store_identity = Some(store);
        }
        self.read_epoch = if owned_read {
            Some(ReadEpoch::capture(connection)?)
        } else {
            None
        };
        Ok(())
    }
    fn remove(&mut self, id: &str) {
        if let Some(old) = self.entries.remove(id) {
            self.bytes -= old.bytes;
        }
    }
    fn read(&mut self, connection: &Connection, row: &AgentHistoryRow) -> PortResult<VerifiedFact> {
        // A selection callback can use another store API on this connection.
        // Once it writes, no later fact in this scan may mint a read-only proof,
        // even if the transaction eventually rolls back after a callback error.
        if let Some(epoch) = self.read_epoch
            && ReadEpoch::capture(connection)? != epoch
        {
            self.read_epoch = None;
        }
        if let Some(epoch) = self.read_epoch
            && let Some(entry) = self.entries.get(&row.operation_id)
            && entry.verified_epoch == Some(epoch)
        {
            return Ok(VerifiedFact {
                facts: Arc::clone(&entry.facts),
                fingerprint: entry.fingerprint,
                bytes: entry.bytes,
                verified_epoch: entry.verified_epoch,
            });
        }
        #[cfg(test)]
        {
            self.fingerprints += 1;
        }
        let fingerprint = row.fingerprint(connection)?;
        if let Some(entry) = self.entries.get_mut(&row.operation_id)
            && entry.fingerprint == fingerprint
        {
            entry.verified_epoch = self.read_epoch;
            return Ok(VerifiedFact {
                facts: Arc::clone(&entry.facts),
                fingerprint,
                bytes: entry.bytes,
                verified_epoch: entry.verified_epoch,
            });
        }
        self.remove(&row.operation_id);
        #[cfg(test)]
        {
            self.decodes += 1;
        }
        let facts = Arc::new(super::agent_read::decode_selected_agent(
            connection,
            &row.encoded,
            Some(row),
        )?);
        let fact = VerifiedFact {
            bytes: facts.accounted_bytes(),
            facts,
            fingerprint,
            verified_epoch: self.read_epoch,
        };
        self.retain(&fact, false);
        Ok(fact)
    }

    fn retain(&mut self, fact: &VerifiedFact, preferred: bool) {
        let id = fact.facts.operation_id.as_str();
        if let Some(entry) = self.entries.get_mut(id) {
            if preferred {
                self.use_sequence = self.use_sequence.saturating_add(1);
                entry.last_used = self.use_sequence;
            }
            return;
        }
        #[cfg(not(test))]
        let (max_entries, max_bytes) = (MAX_ENTRIES, MAX_ACCOUNTED_BYTES);
        #[cfg(test)]
        let (max_entries, max_bytes) = self
            .budget_override
            .unwrap_or((MAX_ENTRIES, MAX_ACCOUNTED_BYTES));
        if max_entries == 0 || fact.bytes > max_bytes {
            return;
        }
        if preferred {
            while self.entries.len() >= max_entries
                || fact.bytes > max_bytes.saturating_sub(self.bytes)
            {
                let oldest = self
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.last_used)
                    .map(|(id, _)| id.clone())
                    .expect("occupied bounded cache");
                self.remove(&oldest);
            }
        }
        if self.entries.len() < max_entries && fact.bytes <= max_bytes.saturating_sub(self.bytes) {
            self.use_sequence = self.use_sequence.saturating_add(1);
            self.entries.insert(
                id.to_owned(),
                Entry {
                    fingerprint: fact.fingerprint,
                    bytes: fact.bytes,
                    facts: Arc::clone(&fact.facts),
                    verified_epoch: fact.verified_epoch,
                    last_used: if preferred { self.use_sequence } else { 0 },
                },
            );
            self.bytes += fact.bytes;
        }
    }
}

impl ControlStore {
    pub fn agent_attribution_for_owners(
        &self,
        workspace: &WorkspaceId,
        owners: &[String],
    ) -> PortResult<(Vec<SucceededAgentOperationV1>, u64)> {
        let connection = self.connection.borrow();
        let transaction = if connection.is_autocommit() {
            Some(
                connection
                    .unchecked_transaction()
                    .map_err(|_| unavailable())?,
            )
        } else {
            None
        };
        let mut cache = self.agent_history_cache.borrow_mut();
        cache.bind_store(&connection, transaction.is_some())?;
        let mut facts = Vec::new();
        let mut rejected = 0u64;
        let mut statement = connection.prepare("SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json
            FROM operations WHERE operation_id=?1 AND workspace_id=?2 AND state='succeeded'
            AND operation_kind='ApplyAgentConnectionChange'")
            .map_err(|_| unavailable())?;
        for id in owners.iter().collect::<std::collections::BTreeSet<_>>() {
            let row = statement
                .query_row(params![id, workspace.as_str()], AgentHistoryRow::read)
                .optional();
            match row {
                Ok(Some(row)) => match cache.read(&connection, &row) {
                    Ok(fact) => {
                        cache.retain(&fact, true);
                        facts.push((*fact.facts).clone());
                    }
                    Err(error)
                        if matches!(
                            error.code,
                            PortErrorCode::Corrupt | PortErrorCode::InvalidData
                        ) =>
                    {
                        rejected = rejected.saturating_add(1)
                    }
                    Err(error) => return Err(error),
                },
                Ok(None) => rejected = rejected.saturating_add(1),
                Err(
                    rusqlite::Error::FromSqlConversionFailure(..)
                    | rusqlite::Error::IntegralValueOutOfRange(..)
                    | rusqlite::Error::InvalidColumnType(..)
                    | rusqlite::Error::Utf8Error(..),
                ) => rejected = rejected.saturating_add(1),
                Err(_) => return Err(unavailable()),
            }
        }
        drop(statement);
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|_| unavailable())?;
        }
        Ok((facts, rejected))
    }

    /// Current membership and dependency validation belong to one SQLite snapshot;
    /// unchanged snapshot proofs avoid repeated raw-closure hashing. Only immutable
    /// facts are shared; callers still read native
    /// files, grants, pending operations and publication state on every request.
    /// Execution/recovery continue to use their uncached strict journal readers.
    pub fn verified_agent_history_for_kinds(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
    ) -> PortResult<Vec<Arc<SucceededAgentOperationV1>>> {
        self.read_verified_agent_history(workspace, kinds, true)
            .map(|(facts, _)| facts)
    }

    pub(super) fn read_verified_agent_history(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
        require_complete_provenance: bool,
    ) -> PortResult<(Vec<Arc<SucceededAgentOperationV1>>, u64)> {
        self.scan_verified_agent_history(
            workspace,
            kinds,
            require_complete_provenance,
            true,
            |_| Ok(false),
        )
        .map(|(facts, rejected, _)| (facts, rejected))
    }

    /// Resolve the newest relevant completed fact. Older operations after that
    /// fact cannot change the answer and are not dependencies of this current read.
    /// Every visited row and its exact payload/journal closure is authenticated in
    /// one SQLite snapshot. History/restore-list callers retain the full reader.
    pub fn find_verified_agent_operation(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
        matches: impl FnMut(&SucceededAgentOperationV1) -> PortResult<bool>,
    ) -> PortResult<Option<Arc<SucceededAgentOperationV1>>> {
        self.scan_verified_agent_history(workspace, kinds, true, false, matches)
            .map(|(mut facts, _, matched)| if matched { facts.pop() } else { None })
    }

    fn scan_verified_agent_history(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
        require_complete_provenance: bool,
        collect_history: bool,
        mut stop: impl FnMut(&SucceededAgentOperationV1) -> PortResult<bool>,
    ) -> PortResult<(Vec<Arc<SucceededAgentOperationV1>>, u64, bool)> {
        super::agent_read::validate_kinds(kinds)?;
        let connection = self.connection.borrow();
        // An existing caller transaction already supplies the snapshot. Never
        // issue a nested BEGIN, nor use before/after data_version as a substitute.
        let transaction = if connection.is_autocommit() {
            Some(
                connection
                    .unchecked_transaction()
                    .map_err(|_| unavailable())?,
            )
        } else {
            None
        };
        let placeholders = vec!["?"; kinds.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT rowid FROM operations WHERE workspace_id=? AND operation_kind IN ({placeholders}) AND state='succeeded' ORDER BY rowid DESC"
        )).map_err(|_| unavailable())?;
        // Sort only row identities. Sorting multi-megabyte historical JSON spills
        // the whole history to temporary files before the first fact is returned.
        let mut selected = connection.prepare("SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE rowid=?1")
            .map_err(|_| unavailable())?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(
                    std::iter::once(workspace.as_str()).chain(kinds.iter().copied()),
                ),
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| unavailable())?;
        let mut cache = self.agent_history_cache.borrow_mut();
        cache.bind_store(&connection, transaction.is_some())?;
        let mut facts = Vec::new();
        let mut rejected = 0_u64;
        let mut members = std::collections::BTreeSet::new();
        let mut matched = false;
        for row in rows {
            let row = row.and_then(|id| selected.query_row([id], AgentHistoryRow::read));
            let row = match row {
                Ok(row) => row,
                Err(error)
                    if !require_complete_provenance
                        && matches!(
                            error,
                            rusqlite::Error::FromSqlConversionFailure(..)
                                | rusqlite::Error::IntegralValueOutOfRange(..)
                                | rusqlite::Error::InvalidColumnType(..)
                                | rusqlite::Error::Utf8Error(..)
                        ) =>
                {
                    // Malformed historical row fields cannot become a proof.
                    // Query/transport failures still fail the whole read below.
                    rejected = rejected.saturating_add(1);
                    continue;
                }
                Err(_) => return Err(corrupt()),
            };
            members.insert(row.operation_id.clone());
            match cache.read(&connection, &row) {
                Ok(fact) => {
                    matched = stop(&fact.facts)?;
                    if matched {
                        cache.retain(&fact, true);
                    }
                    if collect_history || matched {
                        facts.push(fact.facts);
                    }
                    if matched {
                        break;
                    }
                }
                Err(error)
                    if !require_complete_provenance && error.code == PortErrorCode::Corrupt =>
                {
                    // No unverified fact is returned or cached as a proof.
                    rejected = rejected.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
        // Retire entries deleted or moved out of this query, without displacing
        // valid members during repeated sequential scans of an oversized history.
        let obsolete: Vec<_> = cache
            .entries
            .iter()
            .filter(|(id, entry)| {
                !matched
                    && entry.facts.workspace_id == *workspace
                    && kinds.contains(&entry.facts.idempotency.operation_kind.as_str())
                    && !members.contains(*id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in obsolete {
            cache.remove(&id);
        }
        drop(statement);
        drop(selected);
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|_| unavailable())?;
        }
        Ok((facts, rejected, matched))
    }
}
