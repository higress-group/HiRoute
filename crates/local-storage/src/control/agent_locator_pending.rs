//! Recovery-member locator metadata, maintained in the journal transaction.
//! It never replaces the authoritative recovery membership or grants replay rights.
use super::{agent_history_cache::AgentHistoryRow, agent_locator_input, *};
use agent_locator_input::FacetAction;

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-locator.pending")
}

fn action(value: Option<FacetAction>) -> Option<&'static str> {
    match value {
        Some(FacetAction::Configure) => Some("configure"),
        Some(FacetAction::Restore) => Some("restore"),
        None => None,
    }
}

fn read_row(db: &Connection, id: &str) -> rusqlite::Result<(i64, bool, AgentHistoryRow)> {
    db.query_row(
        "SELECT rowid,finalization_pending,operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE operation_id=?1",
        [id], |r| Ok((r.get(0)?, r.get(1)?, AgentHistoryRow {
            operation_id:r.get(2)?, workspace_id:r.get(3)?, principal:r.get(4)?, operation_kind:r.get(5)?,
            idempotency_key:r.get(6)?, request_digest:r.get(7)?, accepted_digest:r.get(8)?,
            state:r.get(9)?, generation:r.get(10)?, encoded:r.get(11)?,
        })),
    )
}

fn insert_unresolved(
    db: &Connection,
    id: &str,
    ordinal: i64,
    row: Option<&AgentHistoryRow>,
    finalization: Option<bool>,
) -> PortResult<()> {
    let workspace = row
        .filter(|row| agent_locator_input::has_bound_header(row))
        .map(|row| row.workspace_id.as_str());
    db.execute("INSERT INTO agent_locator_pending(operation_id,ordinal,workspace_id,generation,state,finalization_pending,resident_declaration,classification)
        VALUES(?1,?2,?3,?4,?5,?6,0,'unresolved')",
        params![id,ordinal,workspace,row.map(|r| r.generation),row.map(|r| r.state.as_str()),finalization]).map_err(|_| invalid())?;
    Ok(())
}

/// Called only during the v33 transaction after authoritative recovery coverage
/// is verified. Corrupt scope remains unresolved, never absent.
pub(super) fn initialize_member(db: &Connection, id: &str) -> PortResult<()> {
    let ordinal: i64 = db
        .query_row(
            "SELECT rowid FROM operations WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    let (actual, finalization, row) = match read_row(db, id) {
        Ok(value) => value,
        Err(
            rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::Utf8Error(..),
        ) => return insert_unresolved(db, id, ordinal, None, None),
        Err(_) => {
            return Err(port(
                PortErrorCode::Unavailable,
                "control.agent-locator.pending-row",
            ));
        }
    };
    if actual != ordinal || row.operation_id != id {
        return Err(invalid());
    }
    let kind = row.operation_kind.as_str();
    if !["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"].contains(&kind) {
        if !agent_locator_input::has_bound_header(&row) {
            return insert_unresolved(db, id, ordinal, Some(&row), Some(finalization));
        }
        db.execute("INSERT INTO agent_locator_pending(operation_id,ordinal,workspace_id,generation,accepted_digest,state,finalization_pending,resident_declaration,classification)
            VALUES(?1,?2,?3,?4,?5,?6,?7,0,'other')", params![id,ordinal,row.workspace_id,row.generation,row.accepted_digest,row.state,finalization]).map_err(|_| invalid())?;
        return Ok(());
    }
    match agent_locator_input::classify(db, &row) {
        Ok(Some(input)) => {
            db.execute("INSERT INTO agent_locator_pending(operation_id,ordinal,workspace_id,context_id,generation,accepted_digest,plan_digest,state,finalization_pending,model_action,collaboration_action,management_action,resident_declaration,classification)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'settings')",
                params![id,ordinal,input.workspace_id.as_str(),input.context,input.generation,input.accepted_digest.as_str(),input.plan_bytes_digest.as_str(),row.state,finalization,action(input.model),action(input.collaboration),action(input.management),input.resident_service_declaration]).map_err(|_| invalid())?;
            Ok(())
        }
        Ok(None) => {
            db.execute("INSERT INTO agent_locator_pending(operation_id,ordinal,workspace_id,generation,accepted_digest,state,finalization_pending,resident_declaration,classification)
                VALUES(?1,?2,?3,?4,?5,?6,?7,0,'other')", params![id,ordinal,row.workspace_id,row.generation,row.accepted_digest,row.state,finalization]).map_err(|_| invalid())?;
            Ok(())
        }
        Err(error)
            if matches!(
                error.code,
                PortErrorCode::Corrupt | PortErrorCode::InvalidData
            ) =>
        {
            insert_unresolved(db, id, ordinal, Some(&row), Some(finalization))
        }
        Err(error) => Err(error),
    }
}

/// Reconcile only this producer's authoritative recovery membership. A stale
/// or rebound prior member is not overwritten as an apparently valid record.
pub(super) fn sync_member(db: &Connection, id: &str) -> PortResult<bool> {
    let pending: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_recovery_members WHERE operation_id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    let existing: Option<(i64, Option<String>, Option<String>, String)> = db.query_row(
        "SELECT ordinal,workspace_id,accepted_digest,classification FROM agent_locator_pending WHERE operation_id=?1",
        [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|_| invalid())?;
    if !pending {
        if existing.is_some() {
            db.execute(
                "DELETE FROM agent_locator_pending WHERE operation_id=?1",
                [id],
            )
            .map_err(|_| invalid())?;
        }
        return Ok(true);
    }
    if let Some((ordinal, workspace, accepted, classification)) = existing {
        let (actual, finalization, row) = read_row(db, id).map_err(|_| invalid())?;
        if ordinal != actual
            || (classification != "unresolved"
                && (workspace.as_deref() != Some(row.workspace_id.as_str())
                    || accepted.as_deref() != Some(row.accepted_digest.as_str())))
        {
            return Ok(false);
        }
        db.execute("UPDATE agent_locator_pending SET generation=?2,state=?3,finalization_pending=?4 WHERE operation_id=?1",
            params![id,row.generation,row.state,finalization]).map_err(|_| invalid())?;
    } else {
        initialize_member(db, id)?;
    }
    Ok(true)
}

/// Structural check used before the upgrade publishes completion. No journal or
/// payload is decoded here; unresolved members remain explicit uncertainties.
pub(super) fn validate_all(db: &Connection) -> PortResult<()> {
    let inconsistent: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_locator_pending p LEFT JOIN operations o USING(operation_id)
          LEFT JOIN operation_recovery_members r USING(operation_id)
         WHERE o.operation_id IS NULL OR r.operation_id IS NULL OR o.rowid IS NOT p.ordinal
            OR (p.classification!='unresolved' AND (o.workspace_id IS NOT p.workspace_id
                OR o.generation IS NOT p.generation OR o.accepted_change_digest IS NOT p.accepted_digest
                OR o.state IS NOT p.state OR o.finalization_pending IS NOT p.finalization_pending)))",
        [], |r| r.get(0),
    ).map_err(|_| invalid())?;
    if inconsistent {
        return Err(invalid());
    }
    let mut query = db.prepare("SELECT workspace_id,context_id,accepted_digest,plan_digest,model_action,collaboration_action,management_action,resident_declaration,classification FROM agent_locator_pending").map_err(|_| invalid())?;
    for row in query
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map_err(|_| invalid())?
    {
        let (
            workspace,
            context,
            accepted,
            digest,
            model,
            collaboration,
            management,
            resident,
            class,
        ) = row.map_err(|_| invalid())?;
        match class.as_str() {
            "settings" => {
                WorkspaceId::parse(workspace.as_deref().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?;
                if context
                    .as_ref()
                    .is_none_or(|s| s.is_empty() || s.len() > 256)
                {
                    return Err(invalid());
                }
                CanonicalDigest::parse(accepted.as_deref().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?;
                CanonicalDigest::parse(digest.as_deref().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?;
                if [&model, &collaboration, &management]
                    .into_iter()
                    .flatten()
                    .any(|a| !["configure", "restore"].contains(&a.as_str()))
                {
                    return Err(invalid());
                }
            }
            "other" | "unresolved"
                if context.is_none()
                    && model.is_none()
                    && collaboration.is_none()
                    && management.is_none()
                    && !resident => {}
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

#[derive(Clone)]
enum PendingScope {
    Facet {
        context: String,
        facet: hiroute_domain::AgentSettingsFacet,
    },
    Resident,
}
impl From<super::AgentLocatorScope<'_>> for PendingScope {
    fn from(scope: super::AgentLocatorScope<'_>) -> Self {
        match scope {
            super::AgentLocatorScope::ContextFacet { context_id, facet } => Self::Facet {
                context: context_id.to_owned(),
                facet,
            },
            super::AgentLocatorScope::GrantProvenance { context_id, .. } => Self::Facet {
                context: context_id.to_owned(),
                facet: hiroute_domain::AgentSettingsFacet::Model,
            },
            super::AgentLocatorScope::ResidentService => Self::Resident,
        }
    }
}

pub struct PendingAgentStatus {
    pub operation_id: OperationId,
    pub state: OperationState,
    pub finalization_pending: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct PendingWitness {
    id: String,
    ordinal: i64,
    generation: u64,
    accepted: String,
    state: String,
    finalization: bool,
    plan_digest: String,
}

impl PendingWitness {
    pub(super) fn matches_current(&self, db: &Connection) -> PortResult<bool> {
        let present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_locator_pending p JOIN operation_recovery_members r USING(operation_id)
             JOIN operations o USING(operation_id) WHERE p.operation_id=?1 AND p.ordinal=?2 AND o.rowid=?2
               AND p.generation=?3 AND o.generation=?3 AND p.accepted_digest=?4 AND o.accepted_change_digest=?4
               AND p.state=?5 AND o.state=?5 AND p.finalization_pending=?6 AND o.finalization_pending=?6 AND p.plan_digest=?7)",
            params![self.id,self.ordinal,self.generation,self.accepted,self.state,self.finalization,self.plan_digest], |r| r.get(0),
        ).map_err(|_| invalid())?;
        Ok(present)
    }
}

pub struct PendingAgentCapture {
    frozen: super::agent_frozen_read::FrozenAgentJournal,
    workspace: WorkspaceId,
    scope: PendingScope,
    witness: PendingWitness,
}

impl PendingAgentCapture {
    pub(super) fn sizes(&self) -> super::AgentLocatorSizes {
        self.frozen.sizes()
    }
    pub(super) fn witness(&self) -> PendingWitness {
        self.witness.clone()
    }
    pub fn verify(self) -> PortResult<PendingAgentStatus> {
        let row = self.frozen.row();
        let input = agent_locator_input::classify(&self.frozen, row)?.ok_or_else(invalid)?;
        let journal = self.frozen.verify_pending()?;
        if input.operation_id != journal.operation_id
            || journal.workspace != self.workspace
            || journal.state.as_str() != self.witness.state
            || input.plan_bytes_digest.as_str() != self.witness.plan_digest
            || input.generation != self.witness.generation
            || input.accepted_digest.as_str() != self.witness.accepted
            || journal.plan.spec() != input.plan.spec()
        {
            return Err(invalid());
        }
        let relevant = match &self.scope {
            PendingScope::Resident => input.resident_service_declaration,
            PendingScope::Facet { context, facet } => {
                input.context == *context
                    && match facet {
                        hiroute_domain::AgentSettingsFacet::Model => input.model.is_some(),
                        hiroute_domain::AgentSettingsFacet::Collaboration => {
                            input.collaboration.is_some()
                        }
                        hiroute_domain::AgentSettingsFacet::ManagementSkill => {
                            input.management.is_some()
                        }
                    }
            }
        };
        if !relevant {
            return Err(invalid());
        }
        Ok(PendingAgentStatus {
            operation_id: journal.operation_id,
            state: journal.state,
            finalization_pending: self.witness.finalization,
        })
    }
}

struct PendingRow {
    id: String,
    ordinal: i64,
    workspace: Option<String>,
    context: Option<String>,
    generation: Option<u64>,
    accepted: Option<String>,
    plan_digest: Option<String>,
    state: Option<String>,
    finalization: Option<bool>,
    model: Option<String>,
    collaboration: Option<String>,
    management: Option<String>,
    resident: bool,
    classification: String,
}

fn potential(row: &PendingRow, workspace: &WorkspaceId, scope: &PendingScope) -> PortResult<bool> {
    if row
        .workspace
        .as_deref()
        .is_some_and(|value| value != workspace.as_str())
    {
        return Ok(false);
    }
    match row.classification.as_str() {
        "unresolved" => Ok(true),
        "other" => Ok(false),
        "settings" => {
            if row.workspace.as_deref() != Some(workspace.as_str()) {
                return Err(invalid());
            }
            Ok(match scope {
                PendingScope::Resident => row.resident,
                PendingScope::Facet { context, facet }
                    if row.context.as_deref() == Some(context.as_str()) =>
                {
                    match facet {
                        hiroute_domain::AgentSettingsFacet::Model => row.model.is_some(),
                        hiroute_domain::AgentSettingsFacet::Collaboration => {
                            row.collaboration.is_some()
                        }
                        hiroute_domain::AgentSettingsFacet::ManagementSkill => {
                            row.management.is_some()
                        }
                    }
                }
                PendingScope::Facet { .. } => false,
            })
        }
        _ => Err(invalid()),
    }
}

fn find_witness(
    db: &Connection,
    workspace: &WorkspaceId,
    requested: super::AgentLocatorScope<'_>,
) -> PortResult<Option<PendingWitness>> {
    let scope = PendingScope::from(requested);
    let mut query = db.prepare("SELECT p.operation_id,p.ordinal,p.workspace_id,p.context_id,p.generation,p.accepted_digest,p.plan_digest,p.state,p.finalization_pending,p.model_action,p.collaboration_action,p.management_action,p.resident_declaration,p.classification FROM agent_locator_pending p JOIN operations o USING(operation_id) ORDER BY o.created_at,p.operation_id").map_err(|_| invalid())?;
    let rows = query
        .query_map([], |r| {
            Ok(PendingRow {
                id: r.get(0)?,
                ordinal: r.get(1)?,
                workspace: r.get(2)?,
                context: r.get(3)?,
                generation: r.get(4)?,
                accepted: r.get(5)?,
                plan_digest: r.get(6)?,
                state: r.get(7)?,
                finalization: r.get(8)?,
                model: r.get(9)?,
                collaboration: r.get(10)?,
                management: r.get(11)?,
                resident: r.get(12)?,
                classification: r.get(13)?,
            })
        })
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    drop(query);
    for pending in rows {
        let relevant = potential(&pending, workspace, &scope)?;
        let mut dirty = db
            .prepare("SELECT flags FROM agent_locator_dirty WHERE operation_id=?1")
            .map_err(|_| invalid())?;
        let mut flags = 0u32;
        for flag in dirty
            .query_map([&pending.id], |r| r.get::<_, u32>(0))
            .map_err(|_| invalid())?
        {
            let flag = flag.map_err(|_| invalid())?;
            if ![1, 2, 4, 8].contains(&flag) {
                return Err(invalid());
            }
            flags |= flag;
        }
        // An externally changed operation or Plan may now belong to this scope.
        if flags & 1 != 0
            || (flags != 0
                && (relevant
                    || (flags & 4 != 0
                        && pending
                            .workspace
                            .as_deref()
                            .is_none_or(|w| w == workspace.as_str()))))
        {
            return Err(invalid());
        }
        if !relevant {
            continue;
        }
        if pending.classification == "unresolved" {
            return Err(invalid());
        }
        let (ordinal, finalization, generation, accepted, state): (i64, bool, u64, String, String) = db.query_row(
            "SELECT rowid,finalization_pending,generation,accepted_change_digest,state FROM operations WHERE operation_id=?1",
            [&pending.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).map_err(|_| invalid())?;
        if ordinal != pending.ordinal
            || Some(generation) != pending.generation
            || Some(accepted.as_str()) != pending.accepted.as_deref()
            || Some(state.as_str()) != pending.state.as_deref()
            || Some(finalization) != pending.finalization
        {
            return Err(invalid());
        }
        let witness = PendingWitness {
            id: pending.id,
            ordinal,
            generation,
            accepted,
            state,
            finalization,
            plan_digest: pending.plan_digest.ok_or_else(invalid)?,
        };
        return Ok(Some(witness));
    }
    Ok(None)
}

/// Metadata-only hint for Home. It is never a journal, native, or execution proof.
pub(super) fn summary(
    db: &Connection,
    workspace: &WorkspaceId,
    requested: super::AgentLocatorScope<'_>,
) -> PortResult<Option<bool>> {
    let Some(witness) = find_witness(db, workspace, requested)? else {
        return Ok(None);
    };
    let state: OperationState =
        serde_json::from_value(Value::String(witness.state)).map_err(|_| invalid())?;
    Ok(Some(state == OperationState::NeedsAttention))
}

pub(super) fn capture(
    db: &Connection,
    workspace: &WorkspaceId,
    requested: super::AgentLocatorScope<'_>,
) -> PortResult<Option<PendingAgentCapture>> {
    let Some(witness) = find_witness(db, workspace, requested)? else {
        return Ok(None);
    };
    let (_, _, row) = read_row(db, &witness.id).map_err(|_| invalid())?;
    let frozen = super::agent_frozen_read::FrozenAgentJournal::capture(db, row)?;
    Ok(Some(PendingAgentCapture {
        frozen,
        workspace: workspace.clone(),
        scope: requested.into(),
        witness,
    }))
}
