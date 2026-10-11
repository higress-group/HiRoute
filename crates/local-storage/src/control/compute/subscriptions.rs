use hiroute_domain::{OperationId, PortError, PortErrorCode, PortResult};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Deserialize;

use super::super::ControlStore;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeSubscriptionValidationStateV1 {
    Staged,
    Verified,
    Retained,
    Released,
}

/// Records Operation B before admission commits. A close/release racing an accepted or recovered
/// save can therefore observe B and must not tear down the resource still owned by A.
pub(super) fn begin_save_handoff_in(
    transaction: &Transaction<'_>,
    operation: &hiroute_domain::OperationV1,
) -> PortResult<()> {
    let Some((candidate, validation)) = save_handoff_binding(operation)? else {
        return Ok(());
    };
    let approval_id = OperationId::parse(&validation.approval_operation.operation_id)
        .map_err(|_| error(PortErrorCode::Corrupt, "subscription.handoff.approval"))?;
    let row = load_in(transaction, approval_id.as_str())?
        .ok_or_else(|| error(PortErrorCode::Conflict, "subscription.handoff.missing"))?;
    validate_handoff_binding(&row, &candidate, &validation)?;
    if row.state != ComputeSubscriptionValidationStateV1::Verified
        || row.save_operation_id.is_some()
    {
        return Err(error(
            PortErrorCode::Conflict,
            "subscription.handoff.unavailable",
        ));
    }
    let changed = transaction
        .execute(
            "UPDATE compute_subscription_validations
             SET save_operation_id=?2, updated_at=unixepoch()
             WHERE operation_id=?1 AND state='verified' AND save_operation_id IS NULL",
            params![approval_id.as_str(), operation.operation_id.as_str()],
        )
        .map_err(|_| unavailable("subscription.handoff.begin"))?;
    if changed != 1 {
        return Err(error(PortErrorCode::Conflict, "subscription.handoff.raced"));
    }
    Ok(())
}

pub(super) fn finish_save_handoff_in(
    transaction: &Transaction<'_>,
    operation: &hiroute_domain::OperationV1,
) -> PortResult<()> {
    let Some((candidate, validation)) = save_handoff_binding(operation)? else {
        return Ok(());
    };
    let approval_id = OperationId::parse(&validation.approval_operation.operation_id)
        .map_err(|_| error(PortErrorCode::Corrupt, "subscription.handoff.approval"))?;
    let row = load_in(transaction, approval_id.as_str())?
        .ok_or_else(|| error(PortErrorCode::Conflict, "subscription.handoff.missing"))?;
    validate_handoff_binding(&row, &candidate, &validation)?;
    if row.save_operation_id.as_deref() != Some(operation.operation_id.as_str()) {
        return Err(error(
            PortErrorCode::Conflict,
            "subscription.handoff.binding",
        ));
    }
    if operation.state == hiroute_domain::OperationState::Succeeded
        && row.state == ComputeSubscriptionValidationStateV1::Verified
    {
        let changed = transaction
            .execute(
                "UPDATE compute_subscription_validations
                 SET state='retained', save_operation_id=?2, updated_at=unixepoch()
                 WHERE operation_id=?1 AND state='verified'",
                params![approval_id.as_str(), operation.operation_id.as_str()],
            )
            .map_err(|_| unavailable("subscription.handoff.update"))?;
        if changed != 1 {
            return Err(error(PortErrorCode::Conflict, "subscription.handoff.raced"));
        }
    } else if operation.state == hiroute_domain::OperationState::RolledBack
        && row.state == ComputeSubscriptionValidationStateV1::Verified
    {
        let changed = transaction
            .execute(
                "UPDATE compute_subscription_validations
                 SET save_operation_id=NULL, updated_at=unixepoch()
                 WHERE operation_id=?1 AND state='verified' AND save_operation_id=?2",
                params![approval_id.as_str(), operation.operation_id.as_str()],
            )
            .map_err(|_| unavailable("subscription.handoff.rollback"))?;
        if changed != 1 {
            return Err(error(PortErrorCode::Conflict, "subscription.handoff.raced"));
        }
    }
    Ok(())
}

fn save_handoff_binding(
    operation: &hiroute_domain::OperationV1,
) -> PortResult<Option<(HandoffCandidateV2, HandoffValidationV2)>> {
    if operation.plan.spec().command_id != "compute.connection.apply" {
        return Ok(None);
    }
    // The legacy pre-Gateway projection flow intentionally shares the command identifier but
    // has a different durable payload. Only the versioned management schema participates in a
    // subscription-validation handoff; other authorized compute writes must pass through.
    if operation
        .plan
        .spec()
        .desired_state
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some("hiroute.compute-management-change/v2")
    {
        return Ok(None);
    }
    let change: HandoffChangeV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| error(PortErrorCode::Corrupt, "subscription.handoff.change"))?;
    let Some(validation) = change.validation else {
        return Ok(None);
    };
    let HandoffSubjectV2::Candidate { candidate } = change.subject else {
        // The sealed management planner has already matched this SavedSource's retained
        // validation and current revisions. Editing it preserves the original receipt's
        // save owner; only a Candidate save initiates a new check-to-save handoff.
        return Ok(None);
    };
    Ok(Some((candidate, validation)))
}

fn validate_handoff_binding(
    row: &ComputeSubscriptionValidationRecordV1,
    candidate: &HandoffCandidateV2,
    validation: &HandoffValidationV2,
) -> PortResult<()> {
    let stored: serde_json::Value = serde_json::from_str(&row.record_json)
        .map_err(|_| error(PortErrorCode::Corrupt, "subscription.handoff.record"))?;
    let stored_validation: HandoffValidationV2 = serde_json::from_value(
        stored
            .get("validation")
            .cloned()
            .ok_or_else(|| error(PortErrorCode::Corrupt, "subscription.handoff.validation"))?,
    )
    .map_err(|_| error(PortErrorCode::Corrupt, "subscription.handoff.validation"))?;
    if row.candidate_ref != candidate.candidate_ref
        || row.candidate_revision != candidate.candidate_revision
        || &stored_validation != validation
        || !matches!(
            row.state,
            ComputeSubscriptionValidationStateV1::Verified
                | ComputeSubscriptionValidationStateV1::Retained
        )
    {
        return Err(error(
            PortErrorCode::Conflict,
            "subscription.handoff.binding",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct HandoffChangeV2 {
    subject: HandoffSubjectV2,
    validation: Option<HandoffValidationV2>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HandoffSubjectV2 {
    Candidate {
        candidate: HandoffCandidateV2,
    },
    SavedSource {
        #[serde(rename = "source_id")]
        _source_id: String,
    },
}

#[derive(Deserialize)]
struct HandoffCandidateV2 {
    candidate_ref: String,
    candidate_revision: u64,
}

#[derive(Deserialize, Eq, PartialEq)]
struct HandoffValidationV2 {
    approval_operation: HandoffOperationV1,
    validation_ref: String,
    validation_revision: u64,
}

#[derive(Deserialize, Eq, PartialEq)]
struct HandoffOperationV1 {
    operation_id: String,
}

impl ComputeSubscriptionValidationStateV1 {
    fn parse(value: &str) -> PortResult<Self> {
        match value {
            "staged" => Ok(Self::Staged),
            "verified" => Ok(Self::Verified),
            "retained" => Ok(Self::Retained),
            "released" => Ok(Self::Released),
            _ => Err(error(PortErrorCode::Corrupt, "subscription.state.decode")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeSubscriptionValidationRecordV1 {
    pub operation_id: String,
    pub candidate_ref: String,
    pub candidate_revision: u64,
    pub record_json: String,
    pub state: ComputeSubscriptionValidationStateV1,
    pub save_operation_id: Option<String>,
}

impl ControlStore {
    pub fn stage_compute_subscription_validation(
        &self,
        operation_id: &OperationId,
        candidate_ref: &str,
        candidate_revision: u64,
        record_json: &str,
    ) -> PortResult<ComputeSubscriptionValidationRecordV1> {
        validate(candidate_ref, candidate_revision, record_json)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| unavailable("subscription.stage.begin"))?;
        let owns_writer: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM writer_claim w JOIN operations o ON o.operation_id=w.operation_id
                    WHERE w.singleton=1 AND w.operation_id=?1
                      AND o.state NOT IN ('succeeded','rolled_back','needs_attention')
                )",
                [operation_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| unavailable("subscription.stage.owner"))?;
        if !owns_writer {
            return Err(error(PortErrorCode::Conflict, "subscription.stage.owner"));
        }
        let existing = load_in(&transaction, operation_id.as_str())?;
        if let Some(existing) = existing {
            if existing.candidate_ref == candidate_ref
                && existing.candidate_revision == candidate_revision
                && existing.record_json == record_json
            {
                transaction
                    .commit()
                    .map_err(|_| unavailable("subscription.stage.commit"))?;
                return Ok(existing);
            }
            return Err(error(
                PortErrorCode::Conflict,
                "subscription.stage.immutable",
            ));
        }
        transaction
            .execute(
                "INSERT INTO compute_subscription_validations(
                    operation_id,candidate_ref,candidate_revision,record_json,state,updated_at
                 ) VALUES(?1,?2,?3,?4,'staged',unixepoch())",
                params![
                    operation_id.as_str(),
                    candidate_ref,
                    candidate_revision,
                    record_json
                ],
            )
            .map_err(|_| unavailable("subscription.stage.insert"))?;
        transaction
            .commit()
            .map_err(|_| unavailable("subscription.stage.commit"))?;
        // `transaction.commit()` consumes the transaction but the surrounding `RefMut` remains
        // live. Release it before the read-back borrows the same RefCell again. Without this,
        // first-time staging commits successfully and then panics, stranding the Operation at its
        // started step; recovery only appeared to work because the idempotent path returns above.
        drop(connection);
        self.compute_subscription_validation(operation_id)?
            .ok_or_else(|| error(PortErrorCode::Corrupt, "subscription.stage.missing"))
    }

    pub fn compute_subscription_validation(
        &self,
        operation_id: &OperationId,
    ) -> PortResult<Option<ComputeSubscriptionValidationRecordV1>> {
        load_in(&self.connection.borrow(), operation_id.as_str())
    }

    /// Returns the newest durable revision for one stable subscription candidate. Callers may
    /// only reuse its checked facts after validating lifecycle state and current discovery
    /// evidence; released and retained records still advance the backend revision.
    pub fn latest_compute_subscription_validation(
        &self,
        candidate_ref: &str,
    ) -> PortResult<Option<ComputeSubscriptionValidationRecordV1>> {
        validate_candidate_ref(candidate_ref)?;
        let operation_id = self
            .connection
            .borrow()
            .query_row(
                "SELECT operation_id
                 FROM compute_subscription_validations
                 WHERE candidate_ref=?1
                 ORDER BY candidate_revision DESC, updated_at DESC, rowid DESC
                 LIMIT 1",
                [candidate_ref],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| unavailable("subscription.latest"))?;
        operation_id
            .map(|operation_id| {
                load_in(&self.connection.borrow(), &operation_id)?
                    .ok_or_else(|| error(PortErrorCode::Corrupt, "subscription.latest.missing"))
            })
            .transpose()
    }

    pub fn activate_compute_subscription_validation(
        &self,
        operation_id: &OperationId,
    ) -> PortResult<ComputeSubscriptionValidationRecordV1> {
        self.update_compute_subscription_validation(operation_id, "verified", None)
    }

    pub fn retain_compute_subscription_validation(
        &self,
        operation_id: &OperationId,
        save_operation_id: &OperationId,
    ) -> PortResult<ComputeSubscriptionValidationRecordV1> {
        self.update_compute_subscription_validation(
            operation_id,
            "retained",
            Some(save_operation_id.as_str()),
        )
    }

    pub fn release_compute_subscription_validation(
        &self,
        operation_id: &OperationId,
    ) -> PortResult<ComputeSubscriptionValidationRecordV1> {
        self.update_compute_subscription_validation(operation_id, "released", None)
    }

    fn update_compute_subscription_validation(
        &self,
        operation_id: &OperationId,
        state: &str,
        save_operation_id: Option<&str>,
    ) -> PortResult<ComputeSubscriptionValidationRecordV1> {
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE compute_subscription_validations
                 SET state=?2, save_operation_id=coalesce(?3,save_operation_id), updated_at=unixepoch()
                 WHERE operation_id=?1 AND state!='released'",
                params![operation_id.as_str(), state, save_operation_id],
            )
            .map_err(|_| unavailable("subscription.state.update"))?;
        if changed != 1 {
            return Err(error(
                PortErrorCode::Conflict,
                "subscription.state.transition",
            ));
        }
        self.compute_subscription_validation(operation_id)?
            .ok_or_else(|| error(PortErrorCode::Corrupt, "subscription.state.missing"))
    }
}

fn load_in(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> PortResult<Option<ComputeSubscriptionValidationRecordV1>> {
    let row = connection
        .query_row(
            "SELECT candidate_ref,candidate_revision,record_json,state,save_operation_id
             FROM compute_subscription_validations WHERE operation_id=?1",
            [operation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| unavailable("subscription.read"))?;
    row.map(
        |(candidate_ref, candidate_revision, record_json, state, save_operation_id)| {
            validate(&candidate_ref, candidate_revision, &record_json)?;
            Ok(ComputeSubscriptionValidationRecordV1 {
                operation_id: operation_id.to_owned(),
                candidate_ref,
                candidate_revision,
                record_json,
                state: ComputeSubscriptionValidationStateV1::parse(&state)?,
                save_operation_id,
            })
        },
    )
    .transpose()
}

fn validate(candidate_ref: &str, candidate_revision: u64, record_json: &str) -> PortResult<()> {
    validate_candidate_ref(candidate_ref)?;
    if candidate_revision == 0
        || record_json.is_empty()
        || record_json.len() > 8 * 1024 * 1024
        || serde_json::from_str::<serde_json::Value>(record_json).is_err()
    {
        return Err(error(
            PortErrorCode::InvalidData,
            "subscription.record.invalid",
        ));
    }
    Ok(())
}

fn validate_candidate_ref(candidate_ref: &str) -> PortResult<()> {
    if candidate_ref.is_empty()
        || candidate_ref.len() > 256
        || candidate_ref.contains("..")
        || candidate_ref.contains("//")
        || !candidate_ref.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b':' | b'-')
        })
    {
        return Err(error(
            PortErrorCode::InvalidData,
            "subscription.record.invalid",
        ));
    }
    Ok(())
}

fn unavailable(context: &'static str) -> PortError {
    error(PortErrorCode::Unavailable, context)
}

fn error(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_tempdir as tempdir;

    #[test]
    fn first_stage_returns_the_committed_record() {
        let directory = tempdir().unwrap();
        // Let ControlStore create the immediate parent with its required owner-only mode. Some
        // platforms do not give tempfile's outer directory that exact mode.
        let database = directory.path().join("store/control.db");
        let backups = directory.path().join("backups");
        let store =
            ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
        let operation_id = OperationId::parse("op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO operations(
                        operation_id,workspace_id,principal,operation_kind,idempotency_key,
                        request_digest,accepted_change_digest,state,generation,operation_json,
                        created_at,updated_at
                     ) VALUES(?1,'personal/default','desktop','ApplySubscriptionCheck','stage',
                              'sha256:request','sha256:accepted','applying_agent_artifacts',1,
                              '{}',1,1)",
                    [operation_id.as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO writer_claim VALUES (1,?1,1)",
                    [operation_id.as_str()],
                )
                .unwrap();
        });

        let record = store
            .stage_compute_subscription_validation(
                &operation_id,
                "candidate/cpa/codex/account",
                1,
                "{}",
            )
            .unwrap();

        assert_eq!(record.operation_id, operation_id.as_str());
        assert_eq!(record.state, ComputeSubscriptionValidationStateV1::Staged);
        assert_eq!(
            store
                .compute_subscription_validation(&operation_id)
                .unwrap(),
            Some(record)
        );
    }

    #[test]
    fn latest_validation_preserves_backend_revision_and_lifecycle_across_reopen() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("control.db");
        let backups = directory.path().join("backups");
        let store =
            ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
        let candidate = "candidate/cpa/codex/account";
        let first = "op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let latest = "op_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        store.with_connection(|connection| {
            for (operation_id, key, revision) in
                [(first, "first", 2_u64), (latest, "latest", 4_u64)]
            {
                connection
                    .execute(
                        "INSERT INTO operations(
                            operation_id,workspace_id,principal,operation_kind,idempotency_key,
                            request_digest,accepted_change_digest,state,generation,operation_json,
                            created_at,updated_at
                         ) VALUES(?1,'personal/default','desktop','ApplySubscriptionCheck',?2,
                                  'sha256:request','sha256:accepted','succeeded',1,'{}',1,1)",
                        params![operation_id, key],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO compute_subscription_validations(
                            operation_id,candidate_ref,candidate_revision,record_json,state,updated_at
                         ) VALUES(?1,?2,?3,'{}','verified',1)",
                        params![operation_id, candidate, revision],
                    )
                    .unwrap();
            }
        });
        drop(store);

        let reopened =
            ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
        let record = reopened
            .latest_compute_subscription_validation(candidate)
            .unwrap()
            .unwrap();
        assert_eq!(record.operation_id, latest);
        assert_eq!(record.candidate_revision, 4);
        assert_eq!(record.state, ComputeSubscriptionValidationStateV1::Verified);
        reopened
            .release_compute_subscription_validation(&OperationId::parse(latest).unwrap())
            .unwrap();
        assert_eq!(
            reopened
                .latest_compute_subscription_validation(candidate)
                .unwrap()
                .unwrap()
                .state,
            ComputeSubscriptionValidationStateV1::Released
        );
        assert!(
            reopened
                .latest_compute_subscription_validation("candidate//invalid")
                .is_err()
        );
    }
}
