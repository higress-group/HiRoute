//! One-time structural validation before publishing the new locator upgrade.
//! This inspects locator metadata, never historical operation payloads.
use super::{agent_locator, *};
use hiroute_domain::AgentAccessGrantRefV1;

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-locator.complete")
}

struct Member {
    ordinal: i64,
    operation: Option<String>,
    workspace: Option<String>,
    context: Option<String>,
    generation: Option<u64>,
    accepted: Option<String>,
    plan_digest: Option<String>,
    model: Option<String>,
    collaboration: Option<String>,
    management: Option<String>,
    resident: bool,
    grant_json: Option<String>,
    grant_unresolved: bool,
    classification: String,
}

impl Member {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            ordinal: row.get(0)?,
            operation: row.get(1)?,
            workspace: row.get(2)?,
            context: row.get(3)?,
            generation: row.get(4)?,
            accepted: row.get(5)?,
            plan_digest: row.get(6)?,
            model: row.get(7)?,
            collaboration: row.get(8)?,
            management: row.get(9)?,
            resident: row.get(10)?,
            grant_json: row.get(11)?,
            grant_unresolved: row.get(12)?,
            classification: row.get(13)?,
        })
    }

    fn validate(&self) -> PortResult<()> {
        if self.classification != "settings" {
            if !["other", "unresolved"].contains(&self.classification.as_str())
                || self.context.is_some()
                || self.model.is_some()
                || self.collaboration.is_some()
                || self.management.is_some()
                || self.resident
                || self.grant_json.is_some()
                || self.grant_unresolved
            {
                return Err(invalid());
            }
            return Ok(());
        }
        let operation = self.operation.as_deref().ok_or_else(invalid)?;
        let workspace = self.workspace.as_deref().ok_or_else(invalid)?;
        let context = self.context.as_deref().ok_or_else(invalid)?;
        OperationId::parse(operation).map_err(|_| invalid())?;
        WorkspaceId::parse(workspace).map_err(|_| invalid())?;
        CanonicalDigest::parse(self.accepted.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        CanonicalDigest::parse(self.plan_digest.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if self.generation.is_none()
            || context.is_empty()
            || context.len() > 256
            || (self.grant_unresolved && self.grant_json.is_some())
            || [&self.model, &self.collaboration, &self.management]
                .into_iter()
                .flatten()
                .any(|action| !["configure", "restore"].contains(&action.as_str()))
        {
            return Err(invalid());
        }
        if let Some(json) = &self.grant_json {
            let grant: AgentAccessGrantRefV1 = serde_json::from_str(json).map_err(|_| invalid())?;
            grant.validate().map_err(|_| invalid())?;
            if grant.owner_scope() != workspace
                || grant.connection_id() != format!("agent-connection/{context}")
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn grant_key(&self) -> PortResult<Option<String>> {
        self.grant_json
            .as_ref()
            .map(|json| {
                let reference: AgentAccessGrantRefV1 =
                    serde_json::from_str(json).map_err(|_| invalid())?;
                CanonicalDigest::of(&reference)
                    .map(|digest| digest.to_string())
                    .map_err(|_| invalid())
            })
            .transpose()
    }
}

const COLUMNS: &str = "ordinal,operation_id,workspace_id,context_id,generation,accepted_digest,plan_digest,model_action,collaboration_action,management_action,resident_declaration,grant_ref_json,grant_unresolved,classification";

fn head_at_least(
    db: &Connection,
    member: &Member,
    scope: &str,
    context: &str,
    key: &str,
) -> PortResult<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_locator_heads WHERE workspace_id=?1 AND scope=?2 AND context_id=?3 AND grant_key=?4 AND ordinal>=?5)",
        params![member.workspace.as_deref(),scope,context,key,member.ordinal], |r| r.get(0),
    ).map_err(|_| invalid())
}

fn validate_all(db: &Connection) -> PortResult<()> {
    agent_locator::validate_coverage(db)?;
    super::agent_locator_pending::validate_all(db)?;
    let dirty: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_locator_dirty)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if dirty {
        return Err(invalid());
    }
    let mut query = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM agent_locator_members NOT INDEXED ORDER BY ordinal"
        ))
        .map_err(|_| invalid())?;
    let members = query
        .query_map([], Member::read)
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    drop(query);
    for member in &members {
        member.validate()?;
        if member.classification != "settings" {
            continue;
        }
        let context = member.context.as_deref().ok_or_else(invalid)?;
        for (scope, action) in [
            ("model", &member.model),
            ("collaboration", &member.collaboration),
            ("management", &member.management),
        ] {
            if action.is_some() && !head_at_least(db, member, scope, context, "")? {
                return Err(invalid());
            }
        }
        if member.resident && !head_at_least(db, member, "resident", "", "")? {
            return Err(invalid());
        }
        if let Some(key) = member.grant_key()?
            && !head_at_least(db, member, "grant", context, &key)?
        {
            return Err(invalid());
        }
    }
    let mut query = db.prepare("SELECT workspace_id,scope,context_id,grant_key,ordinal,action FROM agent_locator_heads NOT INDEXED").map_err(|_| invalid())?;
    let heads = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    for (workspace, scope, context, key, ordinal, action) in heads {
        let member = members
            .binary_search_by_key(&ordinal, |m| m.ordinal)
            .ok()
            .and_then(|index| members.get(index))
            .ok_or_else(invalid)?;
        if member.classification != "settings"
            || member.workspace.as_deref() != Some(workspace.as_str())
        {
            return Err(invalid());
        }
        let valid = match scope.as_str() {
            "model" => {
                member.context.as_deref() == Some(context.as_str())
                    && key.is_empty()
                    && member.model.as_deref() == Some(action.as_str())
            }
            "collaboration" => {
                member.context.as_deref() == Some(context.as_str())
                    && key.is_empty()
                    && member.collaboration.as_deref() == Some(action.as_str())
            }
            "management" => {
                member.context.as_deref() == Some(context.as_str())
                    && key.is_empty()
                    && member.management.as_deref() == Some(action.as_str())
            }
            "resident" => {
                context.is_empty() && key.is_empty() && action == "declaration" && member.resident
            }
            "grant" => {
                member.context.as_deref() == Some(context.as_str())
                    && action == "grant"
                    && member.grant_key()?.as_deref() == Some(key.as_str())
            }
            _ => false,
        };
        if !valid {
            return Err(invalid());
        }
    }
    Ok(())
}

impl ControlStore {
    pub(crate) fn validate_agent_locator(&self, full: bool) -> PortResult<()> {
        let db = self.connection.borrow();
        if full {
            validate_all(&db)
        } else {
            agent_locator::validate_coverage(&db)
        }
    }
}
