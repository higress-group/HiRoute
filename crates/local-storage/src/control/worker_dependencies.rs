use hiroute_domain::delegation::WorkerHarnessV1;
use hiroute_domain::{
    CanonicalDigest, CompensationOutcome, EffectReconciliation, OperationId, OwnedEffectKind,
    OwnedEffectV1, PortErrorCode, PortResult, WorkerDependencySelectionChangeV1,
    WorkerDependencySelectionRecordV1, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;

use super::{ControlStore, port};

const EFFECT_SCHEMA: &str = "hiroute.worker-dependency-selection-compensation/v1";

impl ControlStore {
    pub fn worker_dependency_selection(
        &self,
        workspace: &WorkspaceId,
        harness: WorkerHarnessV1,
    ) -> PortResult<Option<(WorkerDependencySelectionRecordV1, u64)>> {
        let connection = self.connection.borrow();
        let row = selection_row(&connection, workspace, harness)?;
        row.map(|row| {
            let selection = decode_selection(&row.selection_json, harness)?;
            Ok((selection, row.revision))
        })
        .transpose()
    }

    pub(super) fn worker_dependency_selection_revision_in(
        connection: &Connection,
        workspace: &WorkspaceId,
        harness: WorkerHarnessV1,
    ) -> PortResult<u64> {
        Ok(selection_row(connection, workspace, harness)?
            .map(|row| row.revision)
            .unwrap_or(0))
    }

    pub(super) fn stage_worker_dependency_selection(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        change: &WorkerDependencySelectionChangeV1,
    ) -> PortResult<OwnedEffectV1> {
        if let Some(existing) = self.observe_worker_dependency_selection(operation_id, workspace)? {
            return match existing {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    Ok(effect)
                }
                EffectReconciliation::Missing => Err(port(
                    PortErrorCode::Conflict,
                    "control.worker_dependency.effect.compensated",
                )),
                EffectReconciliation::OwnershipLost(_) => Err(port(
                    PortErrorCode::Conflict,
                    "control.worker_dependency.effect.ownership",
                )),
            };
        }
        let after_revision = change.after_revision().ok_or_else(|| {
            port(
                PortErrorCode::InvalidData,
                "control.worker_dependency.revision_overflow",
            )
        })?;
        let harness = change.after_selection.harness;
        let after_json = serde_json::to_string(&change.after_selection).map_err(|_| {
            port(
                PortErrorCode::InvalidData,
                "control.worker_dependency.encode",
            )
        })?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.effect.begin",
                )
            })?;
        if effect_row(&transaction, operation_id)?.is_some() {
            drop(transaction);
            drop(connection);
            return self
                .observe_worker_dependency_selection(operation_id, workspace)?
                .and_then(|state| match state {
                    EffectReconciliation::Staged(effect)
                    | EffectReconciliation::Applied(effect) => Some(Ok(effect)),
                    EffectReconciliation::Missing | EffectReconciliation::OwnershipLost(_) => None,
                })
                .unwrap_or_else(|| {
                    Err(port(
                        PortErrorCode::Conflict,
                        "control.worker_dependency.effect.race",
                    ))
                });
        }
        let before = selection_row(&transaction, workspace, harness)?;
        let before_revision = before.as_ref().map(|row| row.revision).unwrap_or(0);
        if before_revision != change.before_revision {
            return Err(port(
                PortErrorCode::Conflict,
                "control.worker_dependency.selection_stale",
            ));
        }
        transaction
            .execute(
                "INSERT INTO worker_dependency_selection_effects(
                    operation_id, workspace_id, harness, before_revision, before_json,
                    before_owner_operation_id, after_revision, after_json, activated, compensated
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 0)",
                params![
                    operation_id.as_str(),
                    workspace.as_str(),
                    harness_key(harness),
                    before_revision,
                    before.as_ref().map(|row| row.selection_json.as_str()),
                    before.as_ref().map(|row| row.owner_operation_id.as_str()),
                    after_revision,
                    after_json,
                ],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.effect.write",
                )
            })?;
        transaction.commit().map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.worker_dependency.effect.commit",
            )
        })?;
        selection_effect(
            operation_id,
            workspace,
            harness,
            before.as_ref().map(|row| row.selection_json.as_str()),
            &after_json,
            after_revision,
        )
    }

    pub(super) fn observe_worker_dependency_selection(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<EffectReconciliation>> {
        let connection = self.connection.borrow();
        let Some(record) = effect_row(&connection, operation_id)? else {
            return Ok(None);
        };
        if record.workspace_id != workspace.as_str() {
            return Err(port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.effect.workspace",
            ));
        }
        let harness = parse_harness(&record.harness)?;
        decode_selection(&record.after_json, harness)?;
        if let Some(before) = record.before_json.as_deref() {
            decode_selection(before, harness)?;
        } else if record.before_revision != 0 || record.before_owner_operation_id.is_some() {
            return Err(port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.effect.before",
            ));
        }
        let effect = selection_effect(
            operation_id,
            workspace,
            harness,
            record.before_json.as_deref(),
            &record.after_json,
            record.after_revision,
        )?;
        if record.compensated {
            return Ok(Some(EffectReconciliation::Missing));
        }
        if !record.activated {
            return Ok(Some(EffectReconciliation::Staged(effect)));
        }
        let current = selection_row(&connection, workspace, harness)?;
        if current.as_ref().is_some_and(|current| {
            current.revision == record.after_revision
                && current.selection_json == record.after_json
                && current.owner_operation_id == operation_id.as_str()
        }) {
            Ok(Some(EffectReconciliation::Applied(effect)))
        } else {
            Ok(Some(EffectReconciliation::OwnershipLost(effect)))
        }
    }

    pub(super) fn activate_worker_dependency_selection(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<Option<OwnedEffectV1>> {
        let Some(operation_id) = worker_effect_operation(effect)? else {
            return Ok(None);
        };
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.activate.begin",
                )
            })?;
        let operation_id = OperationId::parse(operation_id).map_err(|_| {
            port(
                PortErrorCode::InvalidData,
                "control.worker_dependency.activate.operation",
            )
        })?;
        let record = effect_row(&transaction, &operation_id)?.ok_or_else(|| {
            port(
                PortErrorCode::NotFound,
                "control.worker_dependency.activate.effect",
            )
        })?;
        if record.compensated {
            return Err(port(
                PortErrorCode::Conflict,
                "control.worker_dependency.activate.compensated",
            ));
        }
        if record.activated {
            return Ok(Some(effect.clone()));
        }
        let workspace = WorkspaceId::parse(record.workspace_id.clone()).map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.activate.workspace",
            )
        })?;
        let harness = parse_harness(&record.harness)?;
        let current = selection_row(&transaction, &workspace, harness)?;
        if !selection_matches_before(current.as_ref(), &record) {
            return Err(port(
                PortErrorCode::Conflict,
                "control.worker_dependency.activate.selection_stale",
            ));
        }
        let after = decode_selection(&record.after_json, harness)?;
        let canonical_after = serde_json::to_string(&after).map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.activate.encode",
            )
        })?;
        if canonical_after != record.after_json {
            return Err(port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.activate.canonical",
            ));
        }
        transaction
            .execute(
                "INSERT INTO worker_dependency_selections(
                    workspace_id, harness, revision, selection_json, owner_operation_id, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
                 ON CONFLICT(workspace_id, harness) DO UPDATE SET
                    revision=excluded.revision,
                    selection_json=excluded.selection_json,
                    owner_operation_id=excluded.owner_operation_id,
                    updated_at=excluded.updated_at",
                params![
                    workspace.as_str(),
                    harness_key(harness),
                    record.after_revision,
                    record.after_json,
                    operation_id.as_str(),
                ],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.activate.write",
                )
            })?;
        transaction
            .execute(
                "UPDATE worker_dependency_selection_effects SET activated=1
                 WHERE operation_id=?1 AND activated=0 AND compensated=0",
                params![operation_id.as_str()],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.activate.mark",
                )
            })?;
        transaction.commit().map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.worker_dependency.activate.commit",
            )
        })?;
        Ok(Some(effect.clone()))
    }

    pub(super) fn compensate_worker_dependency_selection(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<Option<CompensationOutcome>> {
        let Some(operation_id) = worker_effect_operation(effect)? else {
            return Ok(None);
        };
        let operation_id = OperationId::parse(operation_id).map_err(|_| {
            port(
                PortErrorCode::InvalidData,
                "control.worker_dependency.compensate.operation",
            )
        })?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.compensate.begin",
                )
            })?;
        let record = effect_row(&transaction, &operation_id)?.ok_or_else(|| {
            port(
                PortErrorCode::NotFound,
                "control.worker_dependency.compensate.effect",
            )
        })?;
        if record.compensated {
            return Ok(Some(CompensationOutcome::AlreadyCompensated));
        }
        if !record.activated {
            transaction
                .execute(
                    "UPDATE worker_dependency_selection_effects SET compensated=1
                     WHERE operation_id=?1 AND activated=0 AND compensated=0",
                    params![operation_id.as_str()],
                )
                .map_err(|_| {
                    port(
                        PortErrorCode::Unavailable,
                        "control.worker_dependency.compensate.stage",
                    )
                })?;
            transaction.commit().map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.compensate.commit",
                )
            })?;
            return Ok(Some(CompensationOutcome::Compensated));
        }
        let workspace = WorkspaceId::parse(record.workspace_id.clone()).map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.compensate.workspace",
            )
        })?;
        let harness = parse_harness(&record.harness)?;
        let current = selection_row(&transaction, &workspace, harness)?;
        if !current.as_ref().is_some_and(|current| {
            current.revision == record.after_revision
                && current.selection_json == record.after_json
                && current.owner_operation_id == operation_id.as_str()
        }) {
            return Ok(Some(CompensationOutcome::OwnershipLost));
        }
        if let Some(before_json) = record.before_json.as_deref() {
            let before = decode_selection(before_json, harness)?;
            let owner = record.before_owner_operation_id.as_deref().ok_or_else(|| {
                port(
                    PortErrorCode::Corrupt,
                    "control.worker_dependency.compensate.before_owner",
                )
            })?;
            transaction
                .execute(
                    "UPDATE worker_dependency_selections SET
                        revision=?3, selection_json=?4, owner_operation_id=?5, updated_at=unixepoch()
                     WHERE workspace_id=?1 AND harness=?2",
                    params![
                        workspace.as_str(),
                        harness_key(harness),
                        record.before_revision,
                        serde_json::to_string(&before).map_err(|_| port(
                            PortErrorCode::Corrupt,
                            "control.worker_dependency.compensate.encode"
                        ))?,
                        owner,
                    ],
                )
                .map_err(|_| {
                    port(
                        PortErrorCode::Unavailable,
                        "control.worker_dependency.compensate.restore",
                    )
                })?;
        } else {
            transaction
                .execute(
                    "DELETE FROM worker_dependency_selections
                     WHERE workspace_id=?1 AND harness=?2 AND owner_operation_id=?3",
                    params![
                        workspace.as_str(),
                        harness_key(harness),
                        operation_id.as_str()
                    ],
                )
                .map_err(|_| {
                    port(
                        PortErrorCode::Unavailable,
                        "control.worker_dependency.compensate.delete",
                    )
                })?;
        }
        transaction
            .execute(
                "UPDATE worker_dependency_selection_effects SET compensated=1
                 WHERE operation_id=?1 AND activated=1 AND compensated=0",
                params![operation_id.as_str()],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.worker_dependency.compensate.mark",
                )
            })?;
        transaction.commit().map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.worker_dependency.compensate.commit",
            )
        })?;
        Ok(Some(CompensationOutcome::Compensated))
    }
}

#[derive(Clone)]
struct SelectionRow {
    revision: u64,
    selection_json: String,
    owner_operation_id: String,
}

#[derive(Clone)]
struct EffectRow {
    workspace_id: String,
    harness: String,
    before_revision: u64,
    before_json: Option<String>,
    before_owner_operation_id: Option<String>,
    after_revision: u64,
    after_json: String,
    activated: bool,
    compensated: bool,
}

fn selection_row(
    connection: &Connection,
    workspace: &WorkspaceId,
    harness: WorkerHarnessV1,
) -> PortResult<Option<SelectionRow>> {
    connection
        .query_row(
            "SELECT revision, selection_json, owner_operation_id
             FROM worker_dependency_selections WHERE workspace_id=?1 AND harness=?2",
            params![workspace.as_str(), harness_key(harness)],
            |row| {
                Ok(SelectionRow {
                    revision: row.get(0)?,
                    selection_json: row.get(1)?,
                    owner_operation_id: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.worker_dependency.selection.read",
            )
        })
}

fn effect_row(
    connection: &Connection,
    operation_id: &OperationId,
) -> PortResult<Option<EffectRow>> {
    connection
        .query_row(
            "SELECT workspace_id, harness, before_revision, before_json,
                    before_owner_operation_id, after_revision, after_json, activated, compensated
             FROM worker_dependency_selection_effects WHERE operation_id=?1",
            params![operation_id.as_str()],
            |row| {
                Ok(EffectRow {
                    workspace_id: row.get(0)?,
                    harness: row.get(1)?,
                    before_revision: row.get(2)?,
                    before_json: row.get(3)?,
                    before_owner_operation_id: row.get(4)?,
                    after_revision: row.get(5)?,
                    after_json: row.get(6)?,
                    activated: row.get(7)?,
                    compensated: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.worker_dependency.effect.read",
            )
        })
}

fn selection_matches_before(current: Option<&SelectionRow>, effect: &EffectRow) -> bool {
    match (current, effect.before_json.as_deref()) {
        (None, None) => effect.before_revision == 0 && effect.before_owner_operation_id.is_none(),
        (Some(current), Some(before_json)) => {
            current.revision == effect.before_revision
                && current.selection_json == before_json
                && Some(current.owner_operation_id.as_str())
                    == effect.before_owner_operation_id.as_deref()
        }
        _ => false,
    }
}

fn selection_effect(
    operation_id: &OperationId,
    workspace: &WorkspaceId,
    harness: WorkerHarnessV1,
    before_json: Option<&str>,
    after_json: &str,
    after_revision: u64,
) -> PortResult<OwnedEffectV1> {
    let before_fingerprint = before_json.map(|value| CanonicalDigest::of_bytes(value.as_bytes()));
    let after_fingerprint = CanonicalDigest::of_bytes(after_json.as_bytes());
    Ok(OwnedEffectV1 {
        effect_id: format!("worker-dependency-selection:{}", harness_key(harness)),
        kind: OwnedEffectKind::Control,
        target: format!(
            "{}/worker-dependency-selection/{}",
            workspace.as_str(),
            harness_key(harness)
        ),
        before_fingerprint,
        after_fingerprint: Some(after_fingerprint),
        compensation: json!({
            "schema": EFFECT_SCHEMA,
            "operation_id": operation_id.as_str(),
            "after_revision": after_revision,
        })
        .into(),
    })
}

fn worker_effect_operation(effect: &OwnedEffectV1) -> PortResult<Option<String>> {
    if effect
        .compensation
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some(EFFECT_SCHEMA)
    {
        return Ok(None);
    }
    if effect.kind != OwnedEffectKind::Control {
        return Err(port(
            PortErrorCode::InvalidData,
            "control.worker_dependency.effect.kind",
        ));
    }
    effect
        .compensation
        .get("operation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .map(Some)
        .ok_or_else(|| {
            port(
                PortErrorCode::InvalidData,
                "control.worker_dependency.effect.operation",
            )
        })
}

fn decode_selection(
    encoded: &str,
    harness: WorkerHarnessV1,
) -> PortResult<WorkerDependencySelectionRecordV1> {
    let decoded: WorkerDependencySelectionRecordV1 =
        serde_json::from_str(encoded).map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "control.worker_dependency.selection.decode",
            )
        })?;
    let validated = WorkerDependencySelectionRecordV1::new(
        decoded.harness,
        decoded.adapter_path,
        decoded.cli_path,
        decoded.node_path,
    )
    .map_err(|_| {
        port(
            PortErrorCode::Corrupt,
            "control.worker_dependency.selection.invalid",
        )
    })?;
    if validated.harness != harness {
        return Err(port(
            PortErrorCode::Corrupt,
            "control.worker_dependency.selection.harness",
        ));
    }
    Ok(validated)
}

pub(super) const fn harness_key(harness: WorkerHarnessV1) -> &'static str {
    match harness {
        WorkerHarnessV1::CodexCli => "codex_cli",
        WorkerHarnessV1::ClaudeCode => "claude_code",
        WorkerHarnessV1::QoderCli => "qoder_cli",
        WorkerHarnessV1::Pi => "pi",
        WorkerHarnessV1::DeepseekHarness => "deepseek_harness",
    }
}

fn parse_harness(value: &str) -> PortResult<WorkerHarnessV1> {
    match value {
        "codex_cli" => Ok(WorkerHarnessV1::CodexCli),
        "claude_code" => Ok(WorkerHarnessV1::ClaudeCode),
        "qoder_cli" => Ok(WorkerHarnessV1::QoderCli),
        "pi" => Ok(WorkerHarnessV1::Pi),
        "deepseek_harness" => Ok(WorkerHarnessV1::DeepseekHarness),
        _ => Err(port(
            PortErrorCode::Corrupt,
            "control.worker_dependency.harness",
        )),
    }
}

#[cfg(test)]
#[path = "worker_dependencies_tests.rs"]
mod tests;
