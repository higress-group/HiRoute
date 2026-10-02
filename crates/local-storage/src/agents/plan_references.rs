//! Coherent read projection of original Operation model settings and independent grants.
use super::{ControlStore, agent_read::decode_succeeded_agent, port};
use hiroute_domain::{
    AgentCollaborationGrant, AgentPlanId, AgentPlanReference, AgentPlanReferenceKind,
    AgentPlanReferenceReadPort, AgentPlanReferenceSubject, AgentPlanReferences, CanonicalDigest,
    PortErrorCode, PortResult, WorkspaceId,
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
        {
            let mut statement = transaction.prepare("SELECT grant_json,context_id,generation,grant_id FROM agent_collaboration_grants WHERE workspace_id=?1 ORDER BY context_id")
                .map_err(|_| port(PortErrorCode::Unavailable, "agents.references.grants"))?;
            let rows = statement
                .query_map(params![workspace.as_str()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|_| port(PortErrorCode::Unavailable, "agents.references.grant_rows"))?;
            for row in rows {
                let (json, context, generation, grant_id) =
                    row.map_err(|_| port(PortErrorCode::Corrupt, "agents.references.grant_row"))?;
                let grant: AgentCollaborationGrant = serde_json::from_str(&json)
                    .map_err(|_| port(PortErrorCode::Corrupt, "agents.references.grant_decode"))?;
                grant
                    .validate()
                    .map_err(|_| port(PortErrorCode::Corrupt, "agents.references.grant_invalid"))?;
                if &grant.workspace_id != workspace
                    || grant.context_id != context
                    || grant.generation != generation
                    || grant.grant_id != grant_id
                {
                    return Err(port(
                        PortErrorCode::Corrupt,
                        "agents.references.grant_identity",
                    ));
                }
                if grant.enabled && grant.allowed_plan_ids.contains(plan) {
                    references.push(AgentPlanReference {
                        subject: AgentPlanReferenceSubject::CollaborationContext {
                            context_id: grant.context_id,
                        },
                        kind: AgentPlanReferenceKind::CollaborationAllowed,
                        revision: grant.generation,
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
