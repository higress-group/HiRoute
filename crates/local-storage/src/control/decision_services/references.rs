use hiroute_domain::{CanonicalDigest, PlanReferenceBlockerV1, PlanReferenceProtectionV1};

use super::*;
use crate::control::active_plan_references::ActivePlanReferences;

impl ControlStore {
    pub fn decision_removal_preview(
        &self,
        workspace: &WorkspaceId,
        id: &str,
        expected_revision: u64,
    ) -> PortResult<(CanonicalDigest, Vec<PlanReferenceBlockerV1>)> {
        let connection = self.connection.borrow();
        let transaction = connection.unchecked_transaction().map_err(|_| storage())?;
        let current: Option<(u64, String)> = transaction.query_row(
            "SELECT revision,service_json FROM decision_services WHERE workspace_id=?1 AND service_id=?2 ORDER BY revision DESC LIMIT 1",
            params![workspace.as_str(), id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|_| storage())?;
        let Some((revision, encoded)) =
            current.filter(|(revision, _)| *revision == expected_revision)
        else {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "decision_service.revision_conflict",
            ));
        };
        let record = decode_record(id, revision, &encoded)?;
        let digest = CanonicalDigest::of(&record).map_err(|_| invalid())?;
        Ok((digest, read(&transaction, workspace, id)?))
    }
}

pub(super) fn read(
    connection: &Connection,
    workspace: &WorkspaceId,
    id: &str,
) -> PortResult<Vec<PlanReferenceBlockerV1>> {
    let unknown: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM decision_plan_references WHERE workspace_id=?1 AND service_id=?2 AND (owner_kind NOT IN ('version','draft') OR (owner_kind='draft' AND owner_revision!=0) OR (owner_kind='version' AND owner_revision<=0)))",
        params![workspace.as_str(), id], |row| row.get(0),
    ).map_err(|_| storage())?;
    if unknown {
        return Err(invalid());
    }
    let active = ActivePlanReferences::read(connection, workspace)?;
    let mut blockers = Vec::new();
    active.versions(Some(id), |version, reasons| {
        for protection in reasons {
            blockers.push(PlanReferenceBlockerV1 {
                plan_id: Some(version.reference.plan_id.as_str().to_owned()),
                draft_id: None,
                revision: version.reference.content_revision,
                protection,
            });
        }
        Ok(())
    })?;
    active.drafts(Some(id), |draft| {
        blockers.push(PlanReferenceBlockerV1 {
            plan_id: draft.plan_id.map(|plan| plan.as_str().to_owned()),
            draft_id: Some(draft.draft_id),
            revision: draft.revision,
            protection: PlanReferenceProtectionV1::Draft,
        });
        Ok(())
    })?;
    for (protection, publication) in &active.publications {
        for plan in &publication.plans {
            let value = serde_json::to_value(plan).map_err(|_| invalid())?;
            if references_service(&value, id) {
                blockers.push(PlanReferenceBlockerV1 {
                    plan_id: Some(plan.agent_plan_id().as_str().to_owned()),
                    draft_id: None,
                    revision: plan.body.agent_plan_revision,
                    protection: *protection,
                });
            }
        }
    }
    Ok(blockers)
}

fn references_service(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(object) => {
            (object
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "decision_service" | "decision_service_v2" | "decision_service_v3"
                    )
                })
                && object
                    .get("service")
                    .and_then(|service| service.get("id"))
                    .and_then(Value::as_str)
                    == Some(id))
                || object.values().any(|value| references_service(value, id))
        }
        Value::Array(values) => values.iter().any(|value| references_service(value, id)),
        _ => false,
    }
}
