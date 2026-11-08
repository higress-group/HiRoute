//! Shared binding-reference read used by Preview and by the transactional writer.
use std::collections::BTreeSet;

use super::*;
use crate::control::plan_references::ActivePlanReferences;
use hiroute_domain::ComputeManagementMutationV2;

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
    let active = ActivePlanReferences::read(connection, workspace)?;
    let mut references = BTreeSet::new();
    // Current heads include disabled plans. Old versions only block while held or preparing;
    // immutable historical records alone must not make a connection impossible to clean up.
    for (version, _) in active.versions(None)? {
        let value = serde_json::to_value(&version)
            .map_err(|_| corrupt("compute.management.references.json"))?;
        if references_binding(&value, &bindings) {
            references.insert(version.reference.plan_id.as_str().to_owned());
        }
    }
    for draft in active.drafts(None)? {
        let value = serde_json::to_value(&draft)
            .map_err(|_| corrupt("compute.management.references.json"))?;
        if references_binding(&value, &bindings) {
            references.insert(draft.draft_id);
        }
    }
    // Active publications also cover supported unversioned plans, and the installation window
    // before a newly prepared version receives its product head.
    for (_, publication) in &active.publications {
        for plan in &publication.plans {
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
