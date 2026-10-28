//! An operation-bound, non-executable snapshot for strict verification outside
//! the shared storage lock. It contains only referenced payloads from one owner.
use super::{
    agent_history_cache::AgentHistoryRow,
    agent_read::{AgentReadSource, IndexedStep},
    *,
};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};

type Object<'a> = BTreeMap<String, &'a RawValue>;

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-read.capture")
}

fn object(value: &str) -> PortResult<Object<'_>> {
    serde_json::from_str(value).map_err(|_| invalid())
}

fn collect_step(raw: &RawValue, digests: &mut BTreeSet<CanonicalDigest>) -> PortResult<()> {
    let step = object(raw.get())?;
    let Some(effects) = step.get("effects") else {
        return Ok(());
    };
    let effects: Vec<&RawValue> = serde_json::from_str(effects.get()).map_err(|_| invalid())?;
    for effect in effects {
        let effect = object(effect.get())?;
        if let Some(compensation) = effect.get("compensation")
            && let Some(digest) = payloads::referenced_digest(compensation)?
        {
            digests.insert(digest);
        }
    }
    Ok(())
}

fn references(
    row: &AgentHistoryRow,
    steps: &[IndexedStep],
) -> PortResult<BTreeSet<CanonicalDigest>> {
    let envelope = object(&row.encoded)?;
    let mut digests = BTreeSet::new();
    if let Some(digest) = payloads::referenced_digest(envelope.get("plan").ok_or_else(invalid)?)? {
        digests.insert(digest);
    }
    let journal_steps: Vec<&RawValue> =
        serde_json::from_str(envelope.get("steps").ok_or_else(invalid)?.get())
            .map_err(|_| invalid())?;
    if journal_steps.len() != 6 || steps.len() != 6 {
        return Err(invalid());
    }
    for step in journal_steps {
        collect_step(step, &mut digests)?;
    }
    for indexed in steps {
        let record = object(&indexed.encoded)?;
        collect_step(record.get("step").ok_or_else(invalid)?, &mut digests)?;
    }
    Ok(digests)
}

pub(super) struct FrozenAgentJournal {
    row: AgentHistoryRow,
    steps: Vec<IndexedStep>,
    payloads: BTreeMap<CanonicalDigest, String>,
}

impl FrozenAgentJournal {
    /// The caller must hold one owned Control read transaction. This copies the
    /// actual row and only its own referenced Plan/effect payloads, not history.
    pub(super) fn capture(db: &Connection, row: AgentHistoryRow) -> PortResult<Self> {
        let steps = db.indexed_steps(&row.operation_id)?;
        let digests = references(&row, &steps)?;
        let mut payloads = BTreeMap::new();
        for digest in digests {
            let value: String = db.query_row(
                "SELECT payload_json FROM operation_payloads WHERE operation_id=?1 AND digest=?2",
                params![row.operation_id, digest.as_str()], |r| r.get(0),
            ).map_err(|_| invalid())?;
            payloads.insert(digest, value);
        }
        Ok(Self {
            row,
            steps,
            payloads,
        })
    }

    pub(super) fn row(&self) -> &AgentHistoryRow {
        &self.row
    }

    pub(super) fn sizes(&self) -> super::AgentLocatorSizes {
        super::AgentLocatorSizes {
            operations: 1,
            operation_bytes: self
                .steps
                .iter()
                .fold(self.row.encoded.len(), |total, step| {
                    total.saturating_add(step.encoded.len())
                }),
            payloads: self.payloads.len(),
            payload_bytes: self
                .payloads
                .values()
                .fold(0usize, |total, value| total.saturating_add(value.len())),
        }
    }

    #[cfg(test)]
    pub(super) fn payload_count(&self) -> usize {
        self.payloads.len()
    }

    pub(super) fn verify(&self) -> PortResult<SucceededAgentOperationV1> {
        super::agent_read::decode_from_source(self, &self.row.encoded, Some(&self.row))
    }

    /// Recovery validation yields only non-executable status facts. The temporary
    /// reconstructed Operation is dropped here and never enters the bundle.
    pub(super) fn verify_pending(&self) -> PortResult<PendingJournal> {
        let encoded = payloads::expand(self, &self.row.encoded)?;
        let durable: DurableOperation = serde_json::from_str(&encoded).map_err(|_| invalid())?;
        if !self.row.matches(&durable) {
            return Err(invalid());
        }
        let plan =
            hiroute_domain::AgentPlanReadV1::from_stored(&durable.plan).map_err(|_| invalid())?;
        if plan.operation_kind().map_err(|_| invalid())? != durable.idempotency.operation_kind {
            return Err(invalid());
        }
        let raw: Value = serde_json::from_str(&encoded).map_err(|_| invalid())?;
        if serde_json::to_value(&durable.steps).map_err(|_| invalid())? != raw["steps"] {
            return Err(invalid());
        }
        super::agent_read::validate_indexed_steps(self, durable.operation_id.as_str(), &raw)?;
        let operation = super::decode_operation(&Default::default(), &encoded)?;
        Ok(PendingJournal {
            operation_id: operation.operation_id,
            workspace: operation.workspace_id,
            state: operation.state,
            plan,
        })
    }
}

pub(super) struct PendingJournal {
    pub(super) operation_id: OperationId,
    pub(super) workspace: WorkspaceId,
    pub(super) state: OperationState,
    pub(super) plan: hiroute_domain::AgentPlanReadV1,
}

impl payloads::PayloadRead for FrozenAgentJournal {
    fn payload(&self, owner: &str, digest: &CanonicalDigest) -> PortResult<String> {
        if owner != self.row.operation_id {
            return Err(invalid());
        }
        self.payloads.get(digest).cloned().ok_or_else(invalid)
    }
}

impl AgentReadSource for FrozenAgentJournal {
    fn identity(&self, durable: &DurableOperation) -> PortResult<bool> {
        Ok(self.row.matches(durable))
    }
    fn indexed_steps(&self, operation_id: &str) -> PortResult<Vec<IndexedStep>> {
        if operation_id != self.row.operation_id {
            return Err(invalid());
        }
        Ok(self.steps.clone())
    }
}
