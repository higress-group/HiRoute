//! Rebuildable successful-journal facts, never native status or execution authority.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::Arc;

// Account for decoded allocations as well as both persisted copies. Admission is
// stable within a scan: a history larger than the budget cannot churn its cache.
const MAX_ENTRIES: usize = 128;
const MAX_ACCOUNTED_BYTES: usize = 512 * 1024 * 1024;

#[derive(Default)]
pub(super) struct AgentHistoryCache {
    entries: BTreeMap<String, Entry>,
    bytes: usize,
    store_identity: Option<(String, i64)>,
    #[cfg(test)]
    pub(super) decodes: usize,
    #[cfg(test)]
    pub(super) budget_override: Option<(usize, usize)>,
}
struct Entry {
    fingerprint: [u8; 32],
    bytes: usize,
    facts: Arc<SucceededAgentOperationV1>,
}

/// These are the columns of the selected row, not another matching row found by
/// an EXISTS query. Raw substitution between two valid operations must fail.
pub(super) struct AgentHistoryRow {
    operation_id: String,
    workspace_id: String,
    principal: String,
    operation_kind: String,
    idempotency_key: String,
    request_digest: String,
    accepted_digest: String,
    state: String,
    generation: u64,
    encoded: String,
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
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
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
    fn fingerprint(&self, connection: &Connection) -> PortResult<([u8; 32], usize)> {
        let mut hash = Sha256::new();
        // Internal, process-local hash domain; no serialized/public schema.
        hash.update(b"verified-agent-history-cache\0");
        let mut bytes = 0usize;
        for field in self.fields() {
            frame(&mut hash, field.as_bytes());
            bytes = bytes.saturating_add(field.len());
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
                bytes = bytes.saturating_add(field.len());
            }
            count += 1;
        }
        hash.update(count.to_be_bytes());
        // Invalid counts are rejected even before a cache lookup.
        if count != 6 {
            return Err(corrupt());
        }
        Ok((
            hash.finalize().into(),
            bytes.saturating_mul(8).saturating_add(64 * 1024),
        ))
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
    fn remove(&mut self, id: &str) {
        if let Some(old) = self.entries.remove(id) {
            self.bytes -= old.bytes;
        }
    }
    fn read(
        &mut self,
        connection: &Connection,
        row: &AgentHistoryRow,
    ) -> PortResult<Arc<SucceededAgentOperationV1>> {
        let (fingerprint, bytes) = row.fingerprint(connection)?;
        if let Some(entry) = self.entries.get(&row.operation_id)
            && entry.fingerprint == fingerprint
        {
            return Ok(Arc::clone(&entry.facts));
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
        #[cfg(not(test))]
        let (max_entries, max_bytes) = (MAX_ENTRIES, MAX_ACCOUNTED_BYTES);
        #[cfg(test)]
        let (max_entries, max_bytes) = self
            .budget_override
            .unwrap_or((MAX_ENTRIES, MAX_ACCOUNTED_BYTES));
        if self.entries.len() < max_entries && bytes <= max_bytes.saturating_sub(self.bytes) {
            self.entries.insert(
                row.operation_id.clone(),
                Entry {
                    fingerprint,
                    bytes,
                    facts: Arc::clone(&facts),
                },
            );
            self.bytes += bytes;
        }
        Ok(facts)
    }
}

impl ControlStore {
    /// Exact current membership and raw dependency closure are always read in one
    /// SQLite snapshot. Only immutable facts are shared; callers still read native
    /// files, grants, pending operations and publication state on every request.
    /// Execution/recovery continue to use their uncached strict journal readers.
    pub fn verified_agent_history_for_kinds(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
    ) -> PortResult<Vec<Arc<SucceededAgentOperationV1>>> {
        self.read_verified_agent_history(workspace, kinds, true)
    }

    /// Discovery still visits every accepted operation through its own strict
    /// immutable-input reader. Missing provenance cannot turn a path hint into
    /// authority, nor make an otherwise readable journal block startup.
    pub(super) fn verified_agent_history_for_discovery(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
    ) -> PortResult<Vec<Arc<SucceededAgentOperationV1>>> {
        self.read_verified_agent_history(workspace, kinds, false)
    }

    fn read_verified_agent_history(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
        require_complete_provenance: bool,
    ) -> PortResult<Vec<Arc<SucceededAgentOperationV1>>> {
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
            "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE workspace_id=? AND operation_kind IN ({placeholders}) AND state='succeeded' ORDER BY rowid DESC"
        )).map_err(|_| unavailable())?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(
                    std::iter::once(workspace.as_str()).chain(kinds.iter().copied()),
                ),
                AgentHistoryRow::read,
            )
            .map_err(|_| unavailable())?;
        let mut cache = self.agent_history_cache.borrow_mut();
        let store: String = connection
            .query_row(
                "SELECT store_uuid FROM storage_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| unavailable())?;
        if store.is_empty() {
            return Err(corrupt());
        }
        // This is only an additional invalidator for external writes. The
        // transaction and complete raw fingerprint, not data_version, prove a hit.
        let data_version = connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(|_| unavailable())?;
        let identity = (store, data_version);
        if cache.store_identity.as_ref() != Some(&identity) {
            cache.entries.clear();
            cache.bytes = 0;
            cache.store_identity = Some(identity);
        }
        let mut facts = Vec::new();
        let mut members = std::collections::BTreeSet::new();
        for row in rows {
            let row = row.map_err(|_| corrupt())?;
            members.insert(row.operation_id.clone());
            match cache.read(&connection, &row) {
                Ok(fact) => facts.push(fact),
                Err(error)
                    if !require_complete_provenance && error.code == PortErrorCode::Corrupt =>
                {
                    // No unverified fact is returned or cached as a proof.
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
                entry.facts.workspace_id == *workspace
                    && kinds.contains(&entry.facts.idempotency.operation_kind.as_str())
                    && !members.contains(*id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in obsolete {
            cache.remove(&id);
        }
        drop(statement);
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|_| unavailable())?;
        }
        Ok(facts)
    }
}
