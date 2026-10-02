//! Value-owned journal checkpoint. It contains no dynamic source or authorization facts.
//! Cold decoding reconstructs and validates the plan before establishing a new checkpoint.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct JournalCheckpoint {
    identity: CanonicalDigest,
    plan: Arc<TransactionPlanV1>,
    plan_json: Arc<str>,
    generation: u64,
    steps: Vec<OperationStepV1>,
    state: OperationState,
    safe_error_code: Option<String>,
}

/// A journal-only projection; it cannot replace the durable immutable plan.
pub struct OperationJournalUpdate<'a> {
    pub expected_generation: u64,
    pub next_generation: u64,
    pub changed_steps: Vec<&'a OperationStepV1>,
    pub plan_json: &'a str,
}

impl OperationV1 {
    fn immutable_identity(&self) -> Result<CanonicalDigest, OperationValidationError> {
        Ok(CanonicalDigest::of(&(
            self.schema_version,
            &self.operation_id,
            &self.workspace_id,
            &self.idempotency,
            &self.request_digest,
            &self.accepted_digest,
            &self.expected_revisions,
        ))?)
    }

    pub(super) fn establish_journal_checkpoint(&mut self) -> Result<(), OperationValidationError> {
        self.journal_checkpoint = Some(Arc::new(JournalCheckpoint {
            identity: self.immutable_identity()?,
            plan: self.plan.clone(),
            plan_json: StoredOperationInputV1::freeze(&self.plan)?
                .canonical_json()?
                .into(),
            generation: self.generation,
            steps: self.steps.clone(),
            state: self.state,
            safe_error_code: self.safe_error_code.clone(),
        }));
        Ok(())
    }

    pub fn journal_update(&self) -> Result<OperationJournalUpdate<'_>, OperationValidationError> {
        let checkpoint = self
            .journal_checkpoint
            .as_ref()
            .ok_or(OperationValidationError::InvalidDurableJournal)?;
        if checkpoint.identity != self.immutable_identity()?
            || (!Arc::ptr_eq(&checkpoint.plan, &self.plan) && checkpoint.plan != self.plan)
            || checkpoint.generation != self.generation
            || self.steps.len() != checkpoint.steps.len()
            || self
                .steps
                .iter()
                .zip(&checkpoint.steps)
                .any(|(step, before)| {
                    step.sequence != before.sequence
                        || step.kind != before.kind
                        || step.deterministic_input_digest != before.deterministic_input_digest
                })
        {
            return Err(OperationValidationError::InvalidDurableJournal);
        }
        Ok(OperationJournalUpdate {
            expected_generation: self.generation,
            next_generation: self
                .generation
                .checked_add(1)
                .ok_or(OperationValidationError::InvalidDurableJournal)?,
            changed_steps: self
                .steps
                .iter()
                .zip(&checkpoint.steps)
                .filter_map(|(step, before)| (step != before).then_some(step))
                .collect(),
            plan_json: &checkpoint.plan_json,
        })
    }

    /// A local consistency check, not authorization or evidence of a durable commit. Consumers
    /// must also compare with storage before using the value instead of a cold read.
    pub fn journal_is_committed(&self) -> Result<bool, OperationValidationError> {
        let update = self.journal_update()?;
        let checkpoint = self
            .journal_checkpoint
            .as_ref()
            .expect("checked checkpoint");
        Ok(update.changed_steps.is_empty()
            && self.state == checkpoint.state
            && self.safe_error_code == checkpoint.safe_error_code)
    }

    /// Storage calls this only after its journal transaction commits. Failed transactions leave
    /// the previous generation and checkpoint intact, so the same value can be retried.
    pub fn acknowledge_journal_commit(&mut self, generation: u64) {
        debug_assert_eq!(self.generation.checked_add(1), Some(generation));
        let checkpoint = self
            .journal_checkpoint
            .as_ref()
            .expect("constructed Operation");
        self.journal_checkpoint = Some(Arc::new(JournalCheckpoint {
            identity: checkpoint.identity.clone(),
            plan: checkpoint.plan.clone(),
            plan_json: checkpoint.plan_json.clone(),
            generation,
            steps: self.steps.clone(),
            state: self.state,
            safe_error_code: self.safe_error_code.clone(),
        }));
        self.generation = generation;
    }
}

// The checkpoint is an in-memory proof, not part of the durable Operation value or its equality.
impl PartialEq for OperationV1 {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.operation_id == other.operation_id
            && self.workspace_id == other.workspace_id
            && self.idempotency == other.idempotency
            && self.request_digest == other.request_digest
            && self.accepted_digest == other.accepted_digest
            && self.expected_revisions == other.expected_revisions
            && self.plan == other.plan
            && self.state == other.state
            && self.generation == other.generation
            && self.steps == other.steps
            && self.safe_error_code == other.safe_error_code
    }
}
