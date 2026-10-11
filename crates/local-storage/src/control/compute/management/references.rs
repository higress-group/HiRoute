//! Shared binding-reference read used by Preview and by the transactional writer.
use std::collections::BTreeSet;

use super::*;
use hiroute_domain::{ComputeManagementMutationV2, PlanDraftV1, PlanVersionV1};

pub(super) fn validate_removal(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutationV2,
) -> PortResult<()> {
    let removed: Vec<String> = mutation
        .expected()
        .into_iter()
        .flat_map(|source| &source.models)
        .filter(|model| {
            !mutation.desired().is_some_and(|source| {
                source
                    .models
                    .iter()
                    .any(|kept| kept.binding_id == model.binding_id)
            })
        })
        .map(|model| model.binding_id.clone())
        .collect();
    if !read(connection, workspace, &removed)?.is_empty() {
        return Err(conflict("compute.management.models_in_use"));
    }
    Ok(())
}

pub(super) fn read(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    bindings: &[String],
) -> PortResult<Vec<String>> {
    if bindings.is_empty() {
        return Ok(Vec::new());
    }
    let bindings: BTreeSet<&str> = bindings.iter().map(String::as_str).collect();
    let has_versions: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM plan_versions WHERE workspace_id=?1)",
            [workspace.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| port("compute.management.references.versions"))?;
    let recovery: Option<bool> = connection
        .query_row(
            "SELECT ready FROM plan_version_recovery WHERE workspace_id=?1",
            [workspace.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.management.references.recovery"))?;
    if recovery == Some(false) || (has_versions && recovery != Some(true)) {
        return Err(conflict("compute.management.references.recovery_required"));
    }
    let mut references = BTreeSet::new();
    // Current heads include disabled plans. Old versions only block while held or preparing;
    // immutable historical records alone must not make a connection impossible to clean up.
    let mut statement = connection.prepare("SELECT v.version_json FROM plan_versions v WHERE v.workspace_id=?1 AND (v.state='prepared' OR EXISTS(SELECT 1 FROM plan_heads h WHERE h.workspace_id=v.workspace_id AND h.plan_id=v.plan_id AND json_extract(h.head_json,'$.status') != 'deleted' AND json_extract(h.head_json,'$.reference.content_revision')=v.content_revision) OR EXISTS(SELECT 1 FROM plan_version_holds h WHERE h.workspace_id=v.workspace_id AND h.plan_id=v.plan_id AND h.content_revision=v.content_revision))")
        .map_err(|_| port("compute.management.references.prepare"))?;
    let rows = statement
        .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
        .map_err(|_| port("compute.management.references.query"))?;
    for row in rows {
        let json = row.map_err(|_| port("compute.management.references.row"))?;
        let version: PlanVersionV1 = decode(&json)?;
        version
            .validate()
            .map_err(|_| corrupt("compute.management.references.version"))?;
        let value: Value = serde_json::from_str(&json)
            .map_err(|_| corrupt("compute.management.references.json"))?;
        if references_binding(&value, &bindings) {
            references.insert(version.reference.plan_id.as_str().to_owned());
        }
    }
    let mut statement = connection
        .prepare("SELECT draft_json FROM plan_drafts WHERE workspace_id=?1")
        .map_err(|_| port("compute.management.references.drafts"))?;
    for row in statement
        .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
        .map_err(|_| port("compute.management.references.drafts"))?
    {
        let json = row.map_err(|_| port("compute.management.references.draft"))?;
        let draft: PlanDraftV1 = decode(&json)?;
        draft
            .validate()
            .map_err(|_| corrupt("compute.management.references.draft"))?;
        let value: Value = serde_json::from_str(&json)
            .map_err(|_| corrupt("compute.management.references.json"))?;
        if references_binding(&value, &bindings) {
            references.insert(draft.draft_id);
        }
    }
    // Active publications also cover supported unversioned plans, and the installation window
    // before a newly prepared version receives its product head.
    let mut statement = connection.prepare("SELECT publication_bytes FROM gateway_publications WHERE workspace_id=?1 AND state IN ('active','prepared','lkg')")
        .map_err(|_| port("compute.management.references.publications"))?;
    for row in statement
        .query_map([workspace.as_str()], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|_| port("compute.management.references.publications"))?
    {
        let bytes = row.map_err(|_| port("compute.management.references.publication"))?;
        let publication = hiroute_domain::GatewayPublicationV1::decode_persisted(&bytes)
            .map_err(|_| corrupt("compute.management.references.publication"))?;
        for plan in publication.plans {
            let value = serde_json::to_value(&plan)
                .map_err(|_| corrupt("compute.management.references.publication.plan"))?;
            if references_binding(&value, &bindings) {
                references.insert(plan.agent_plan_id().as_str().to_owned());
            }
        }
    }
    Ok(references.into_iter().collect())
}

fn references_binding(value: &Value, bindings: &BTreeSet<&str>) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("binding_id")
                .and_then(Value::as_str)
                .is_some_and(|id| bindings.contains(id))
                || object
                    .get("quality_anchor_binding_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| bindings.contains(id))
                || object
                    .get("automatic_reasoning")
                    .and_then(Value::as_object)
                    .is_some_and(|map| map.keys().any(|id| bindings.contains(id.as_str())))
                || object
                    .values()
                    .any(|value| references_binding(value, bindings))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| references_binding(value, bindings)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_free_pool_configuration_keys_remain_references_without_materialized_candidates() {
        let binding = "binding/temporarily-unavailable";
        let config = serde_json::json!({"free_pool":{"automatic_reasoning":{binding:{"kind":"toggle","enabled":true}}}});
        assert!(references_binding(&config, &BTreeSet::from([binding])));
        assert!(!references_binding(
            &config,
            &BTreeSet::from(["binding/unrelated"])
        ));
    }
}
