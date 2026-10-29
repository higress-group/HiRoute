//! Plan-owned records in control.db. Management writes are invoked only by the existing
//! protected Operation path; these methods never publish, mint authority, or start a Worker.
use hiroute_domain::{
    AgentPlanId, PlanDraftV1, PlanExecutionRef, PlanHeadV1, PlanVersionError, PlanVersionV1,
    WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};

use super::ControlStore;
mod draft_operation;
mod holds;
pub(super) use draft_operation::finish_draft_operation;
mod publication;
pub(super) use publication::{finish_plan_operation, legacy_plan_finalization_required};
#[cfg(test)]
mod tests;

impl ControlStore {
    /// Published immutable content, including versions retained by an admitted Worker.
    /// This read never changes retention or publication authority.
    pub fn published_plan_versions(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<PlanVersionV1>, PlanVersionError> {
        let connection = self.connection.borrow();
        let mut statement=connection.prepare("SELECT version_json FROM plan_versions WHERE workspace_id=?1 AND state='published' ORDER BY plan_id,content_revision LIMIT 10001").map_err(storage)?;
        let rows = statement
            .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
            .map_err(storage)?;
        let mut versions = Vec::new();
        for row in rows {
            if versions.len() == 10000 {
                return Err(PlanVersionError::Invalid);
            }
            let version: PlanVersionV1 = decode(&row.map_err(storage)?)?;
            version.validate()?;
            if &version.reference.workspace_id != workspace {
                return Err(PlanVersionError::Invalid);
            }
            versions.push(version);
        }
        Ok(versions)
    }
    pub fn plan_drafts(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<PlanDraftV1>, PlanVersionError> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare("SELECT draft_json FROM plan_drafts WHERE workspace_id=?1 ORDER BY draft_id")
            .map_err(storage)?;
        let rows = statement
            .query_map([workspace.as_str()], |r| r.get::<_, String>(0))
            .map_err(storage)?;
        let mut drafts = Vec::new();
        for row in rows {
            let draft: PlanDraftV1 = decode(&row.map_err(storage)?)?;
            draft.validate()?;
            if &draft.workspace_id != workspace {
                return Err(PlanVersionError::Invalid);
            }
            drafts.push(draft);
        }
        Ok(drafts)
    }

    pub fn plan_draft(
        &self,
        workspace: &WorkspaceId,
        id: &str,
    ) -> Result<Option<PlanDraftV1>, PlanVersionError> {
        let value = self
            .connection
            .borrow()
            .query_row(
                "SELECT draft_json FROM plan_drafts WHERE workspace_id=?1 AND draft_id=?2",
                params![workspace.as_str(), id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|s| decode::<PlanDraftV1>(&s))
            .transpose()?;
        if let Some(value) = &value {
            value.validate()?;
            if &value.workspace_id != workspace || value.draft_id != id {
                return Err(PlanVersionError::Invalid);
            }
        }
        Ok(value)
    }

    pub fn save_plan_draft(
        &self,
        draft: &PlanDraftV1,
        expected_revision: Option<u64>,
    ) -> Result<(), PlanVersionError> {
        draft.validate()?;
        if expected_revision.unwrap_or(0).checked_add(1) != Some(draft.revision) {
            return Err(PlanVersionError::Conflict);
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let actual: Option<u64> = tx
            .query_row(
                "SELECT revision FROM plan_drafts WHERE workspace_id=?1 AND draft_id=?2",
                params![draft.workspace_id.as_str(), draft.draft_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if actual != expected_revision {
            return Err(PlanVersionError::Conflict);
        }
        tx.execute("INSERT INTO plan_drafts(workspace_id,draft_id,revision,draft_json) VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,draft_id) DO UPDATE SET revision=excluded.revision,draft_json=excluded.draft_json", params![draft.workspace_id.as_str(), draft.draft_id, draft.revision, encode(draft)?]).map_err(storage)?;
        tx.commit().map_err(storage)
    }

    pub fn discard_plan_draft(
        &self,
        workspace: &WorkspaceId,
        id: &str,
        expected_revision: u64,
    ) -> Result<(), PlanVersionError> {
        let changed = self
            .connection
            .borrow()
            .execute(
                "DELETE FROM plan_drafts WHERE workspace_id=?1 AND draft_id=?2 AND revision=?3",
                params![workspace.as_str(), id, expected_revision],
            )
            .map_err(storage)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(PlanVersionError::Conflict)
        }
    }

    pub fn plan_head(
        &self,
        workspace: &WorkspaceId,
        id: &AgentPlanId,
    ) -> Result<Option<PlanHeadV1>, PlanVersionError> {
        head_in(&self.connection.borrow(), workspace, id)
    }

    pub fn plan_heads(&self, workspace: &WorkspaceId) -> Result<Vec<PlanHeadV1>, PlanVersionError> {
        let connection = self.connection.borrow();
        let mut stmt = connection
            .prepare("SELECT head_json FROM plan_heads WHERE workspace_id=?1 ORDER BY plan_id")
            .map_err(storage)?;
        let rows = stmt
            .query_map([workspace.as_str()], |r| r.get::<_, String>(0))
            .map_err(storage)?;
        let mut result = Vec::new();
        for row in rows {
            let head: PlanHeadV1 = decode(&row.map_err(storage)?)?;
            head.validate()?;
            if &head.reference.workspace_id != workspace {
                return Err(PlanVersionError::Invalid);
            }
            result.push(head);
        }
        Ok(result)
    }

    pub fn lookup_exact_plan_version(
        &self,
        reference: &PlanExecutionRef,
    ) -> Result<PlanVersionV1, PlanVersionError> {
        lookup_in(&self.connection.borrow(), reference)
    }
}

fn lookup_in(
    connection: &Connection,
    reference: &PlanExecutionRef,
) -> Result<PlanVersionV1, PlanVersionError> {
    reference.validate()?;
    let row: Option<(String,String,String)> = connection.query_row("SELECT content_digest,version_json,state FROM plan_versions WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3", params![reference.workspace_id.as_str(), reference.plan_id.as_str(), reference.content_revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
    let (digest, json, state) = row.ok_or(PlanVersionError::Unavailable)?;
    if digest != reference.content_digest.as_str() {
        return Err(PlanVersionError::Conflict);
    }
    if state != "published" {
        return Err(PlanVersionError::Unavailable);
    }
    let version: PlanVersionV1 = decode(&json)?;
    version.validate()?;
    if &version.reference != reference {
        return Err(PlanVersionError::Invalid);
    }
    Ok(version)
}

fn head_in(
    connection: &Connection,
    workspace: &WorkspaceId,
    id: &AgentPlanId,
) -> Result<Option<PlanHeadV1>, PlanVersionError> {
    let value = connection
        .query_row(
            "SELECT head_json FROM plan_heads WHERE workspace_id=?1 AND plan_id=?2",
            params![workspace.as_str(), id.as_str()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(storage)?
        .map(|s| decode::<PlanHeadV1>(&s))
        .transpose()?;
    if let Some(head) = &value {
        head.validate()?;
        if &head.reference.workspace_id != workspace || &head.reference.plan_id != id {
            return Err(PlanVersionError::Invalid);
        }
    }
    Ok(value)
}

fn encode(value: &impl Serialize) -> Result<String, PlanVersionError> {
    serde_json::to_string(value).map_err(|_| PlanVersionError::Invalid)
}
fn decode<T: DeserializeOwned>(value: &str) -> Result<T, PlanVersionError> {
    serde_json::from_str(value).map_err(|_| PlanVersionError::Invalid)
}
fn storage(_: rusqlite::Error) -> PlanVersionError {
    PlanVersionError::StorageUnavailable
}
