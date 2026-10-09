//! Coherent model-reference projection of the original successful settings Operations.
use super::{ControlStore, agent_read::decode_succeeded_agent, port};
use hiroute_domain::{
    AgentFacetIntent, AgentModelDefaultSelectionV2, AgentModelSelectionV2, AgentPlanId,
    AgentPlanReference, AgentPlanReferenceKind, AgentPlanReferenceReadPort,
    AgentPlanReferenceSubject, AgentPlanReferences, AgentSettingsSpecV2, CanonicalDigest,
    OperationStepKind, PortErrorCode, PortResult, SettingsServiceCompletionV1, WorkspaceId,
};
use rusqlite::params;
use std::collections::BTreeSet;

impl AgentPlanReferenceReadPort for ControlStore {
    fn agent_plan_references(
        &self,
        workspace: &WorkspaceId,
        plan: &AgentPlanId,
    ) -> PortResult<AgentPlanReferences> {
        AgentPlanId::parse(plan.as_str())
            .map_err(|_| port(PortErrorCode::InvalidData, "agents.references.plan"))?;
        let mut connection = self.connection.borrow_mut();
        // Deferred read transaction keeps both sources on one SQLite snapshot. No writes,
        // gate acquisition, filesystem, secrets or runtime calls inside this transaction.
        let transaction = connection
            .transaction()
            .map_err(|_| port(PortErrorCode::Unavailable, "agents.references.snapshot"))?;
        let mut references = Vec::new();
        let mut seen = BTreeSet::new();
        {
            let mut statement = transaction.prepare(
                "SELECT state,operation_json FROM operations WHERE workspace_id=?1 AND operation_kind IN ('ApplyAgentConnectionChange','ApplyAgentConnectionRestore') ORDER BY rowid DESC"
            ).map_err(|_| port(PortErrorCode::Unavailable, "agents.references.operations"))?;
            let rows = statement
                .query_map(params![workspace.as_str()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| port(PortErrorCode::Unavailable, "agents.references.rows"))?;
            for row in rows {
                let (state, encoded) =
                    row.map_err(|_| port(PortErrorCode::Corrupt, "agents.references.row"))?;
                match state.as_str() {
                    "rolled_back" => continue,
                    "succeeded" => {}
                    _ => {
                        return Err(port(
                            PortErrorCode::Unavailable,
                            "agents.references.recovery_required",
                        ));
                    }
                }
                let operation = decode_succeeded_agent(&transaction, &encoded)?;
                if &operation.workspace_id != workspace {
                    return Err(port(PortErrorCode::Corrupt, "agents.references.workspace"));
                }
                if operation.plan.spec().command_id == "agents.settings.apply" {
                    let settings: AgentSettingsSpecV2 =
                        serde_json::from_value(operation.plan.spec().desired_state.clone())
                            .map_err(|_| {
                                port(PortErrorCode::Corrupt, "agents.references.settings")
                            })?;
                    // A collaboration-only edit does not supersede the model facet. A restore
                    // does supersede it, even when another facet is configured in the same edit.
                    if matches!(settings.model, AgentFacetIntent::Keep)
                        || !seen.insert(format!("agent-connection/{}", settings.context_id))
                    {
                        continue;
                    }
                    if let AgentFacetIntent::Configure {
                        settings: selection,
                    } = settings.model
                    {
                        let revision = operation
                            .step(OperationStepKind::Activate)
                            .terminal_result
                            .as_deref()
                            .and_then(SettingsServiceCompletionV1::parse)
                            .filter(|receipt| receipt.publication_revision > 0)
                            .ok_or_else(|| {
                                port(PortErrorCode::Corrupt, "agents.references.receipt")
                            })?
                            .publication_revision;
                        append_settings_references(
                            &mut references,
                            plan,
                            settings.context_id,
                            &selection,
                            revision,
                        );
                    }
                    continue;
                }
                let connection_id = operation
                    .plan
                    .spec()
                    .resource_id
                    .as_ref()
                    .ok_or_else(|| port(PortErrorCode::Corrupt, "agents.references.subject"))?;
                if !seen.insert(connection_id.clone()) {
                    continue;
                }
                if operation.idempotency.operation_kind == "ApplyAgentConnectionRestore" {
                    continue;
                }
                let active = operation
                    .plan
                    .agent_connection_projection()
                    .map_err(|_| port(PortErrorCode::Corrupt, "agents.references.projection"))?
                    .ok_or_else(|| port(PortErrorCode::Corrupt, "agents.references.missing"))?;
                let subject = AgentPlanReferenceSubject::ModelProfile {
                    connection_id: active.connection_id,
                    agent_id: active.connection.agent_id,
                    profile_id: active.connection.profile_id,
                };
                if &active.connection.grant.default_agent_plan_id == plan {
                    references.push(AgentPlanReference {
                        subject: subject.clone(),
                        kind: AgentPlanReferenceKind::DefaultModel,
                        revision: active.connection.revision,
                    });
                }
                if active
                    .connection
                    .grant
                    .allowed_agent_plan_ids
                    .contains(plan)
                {
                    references.push(AgentPlanReference {
                        subject,
                        kind: AgentPlanReferenceKind::ModelAllowed,
                        revision: active.connection.revision,
                    });
                }
            }
        }
        references.sort();
        let facts_digest = CanonicalDigest::of(&(workspace, plan, &references))
            .map_err(|_| port(PortErrorCode::Corrupt, "agents.references.digest"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "agents.references.snapshot_end"))?;
        Ok(AgentPlanReferences {
            workspace_id: workspace.clone(),
            plan_id: plan.clone(),
            references,
            facts_digest,
        })
    }
}

fn append_settings_references(
    references: &mut Vec<AgentPlanReference>,
    plan: &AgentPlanId,
    context_id: String,
    selection: &AgentModelSelectionV2,
    revision: u64,
) {
    if !selection.allowed_plan_ids().contains(plan) {
        return;
    }
    let subject = AgentPlanReferenceSubject::ModelContext { context_id };
    // Claude's named presets are native model selections; each must remain usable. Additional
    // model providers have no HiRoute-owned default selection and contribute only allowed refs.
    if matches!(selection, AgentModelSelectionV2::ClaudeLauncher { .. })
        || matches!(selection, AgentModelSelectionV2::CodexDefault {
            default_selection: AgentModelDefaultSelectionV2::Plan { plan_id }, ..
        } if plan_id == plan)
    {
        references.push(AgentPlanReference {
            subject: subject.clone(),
            kind: AgentPlanReferenceKind::DefaultModel,
            revision,
        });
    }
    references.push(AgentPlanReference {
        subject,
        kind: AgentPlanReferenceKind::ModelAllowed,
        revision,
    });
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_model_references_distinguish_defaults_from_additional_routes() {
        let plan = AgentPlanId::parse("plan/review").unwrap();
        let cases = [
            (
                json!({"mode":"codex_default", "native_model_mode":"hiroute_only",
                    "fixed_models":[], "allowed_plan_ids":["plan/review"],
                    "default_selection":{"kind":"plan", "plan_id":"plan/review"}}),
                true,
            ),
            (
                json!({"mode":"codex_default", "native_model_mode":"preserve_available",
                    "fixed_models":[], "allowed_plan_ids":["plan/review"],
                    "default_selection":{"kind":"preserve_native"}}),
                false,
            ),
            (
                json!({"mode":"claude_launcher", "surfaces":["claude_cli"], "fixed_models":[],
                    "preset_mappings":{"opus":{"kind":"plan", "plan_id":"plan/review"},
                        "sonnet":{"kind":"preserve_native"}, "haiku":{"kind":"preserve_native"}}}),
                true,
            ),
            (
                json!({"mode":"qoder_additional", "allowed_plan_ids":["plan/review"]}),
                false,
            ),
            (
                json!({"mode":"pi_additional", "allowed_plan_ids":["plan/review"]}),
                false,
            ),
            (
                json!({"mode":"dsh_additional", "allowed_plan_ids":["plan/review"]}),
                false,
            ),
        ];
        for (value, is_default) in cases {
            let selection: AgentModelSelectionV2 = serde_json::from_value(value).unwrap();
            selection.validate().unwrap();
            let mut references = Vec::new();
            append_settings_references(&mut references, &plan, "context/one".into(), &selection, 9);
            let expected = if is_default {
                vec![
                    AgentPlanReferenceKind::DefaultModel,
                    AgentPlanReferenceKind::ModelAllowed,
                ]
            } else {
                vec![AgentPlanReferenceKind::ModelAllowed]
            };
            assert_eq!(
                references.iter().map(|r| r.kind).collect::<Vec<_>>(),
                expected
            );
            assert!(references.iter().all(|r| r.revision == 9
                && r.subject
                    == AgentPlanReferenceSubject::ModelContext {
                        context_id: "context/one".into()
                    }));
            references.clear();
            append_settings_references(
                &mut references,
                &AgentPlanId::parse("plan/other").unwrap(),
                "context/one".into(),
                &selection,
                9,
            );
            assert!(references.is_empty());
        }
    }
}
