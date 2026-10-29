//! Typed current discovery. The selected journal is still strictly authenticated.
use super::{
    agent_frozen_read::FrozenAgentJournal, agent_history_cache::AgentHistoryRow, agent_locator,
    agent_locator_input, *,
};
use hiroute_domain::{AgentAccessGrantRefV1, AgentSettingsFacet};
use std::sync::Arc;

/// Distinct keys cannot be substituted for one another. None confers authority
/// over current native files, grants, publication, recovery or later writes.
#[derive(Clone, Copy)]
pub enum AgentLocatorScope<'a> {
    ContextFacet {
        context_id: &'a str,
        facet: AgentSettingsFacet,
    },
    GrantProvenance {
        context_id: &'a str,
        grant: &'a AgentAccessGrantRefV1,
    },
    ResidentService,
}

pub enum AgentLocatorResult {
    Found(Arc<SucceededAgentOperationV1>),
    Pending(super::agent_locator_pending::PendingAgentStatus),
    AbsentWithCompleteCoverage,
}

/// A locator-only Home hint. Recorded never means native or grant verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentLocatorSummary {
    Absent,
    RecordedConfigure,
    RecordedRestore,
    Pending,
    NeedsAttention,
}

/// A non-authoritative raw closure captured under an owned Control read snapshot.
/// Verify it outside the shared store lock and perform the separate final fence.
pub enum CapturedAgentLocator {
    Found(AgentLocatorCapture),
    Pending(
        super::agent_locator_pending::PendingAgentCapture,
        AgentLocatorWitness,
    ),
    AbsentWithCompleteCoverage(AgentLocatorWitness),
}

/// Counts and byte lengths only; does not expose the captured data.
#[derive(Clone, Copy, Default)]
pub struct AgentLocatorSizes {
    pub operations: usize,
    pub operation_bytes: usize,
    pub payloads: usize,
    pub payload_bytes: usize,
}

#[derive(Clone)]
pub struct AgentLocatorWitness {
    workspace: WorkspaceId,
    scope: OwnedScope,
    head: Option<HeadWitness>,
    pending: Option<super::agent_locator_pending::PendingWitness>,
}

#[derive(Clone, Eq, PartialEq)]
struct HeadWitness {
    ordinal: i64,
    operation: String,
    generation: u64,
    accepted: String,
    plan_digest: String,
    action: String,
}

impl CapturedAgentLocator {
    pub fn sizes(&self) -> AgentLocatorSizes {
        match self {
            Self::Found(capture) => capture.frozen.sizes(),
            Self::Pending(capture, _) => capture.sizes(),
            Self::AbsentWithCompleteCoverage(_) => AgentLocatorSizes::default(),
        }
    }
    pub fn witness(&self) -> PortResult<AgentLocatorWitness> {
        match self {
            Self::AbsentWithCompleteCoverage(witness) | Self::Pending(_, witness) => {
                Ok(witness.clone())
            }
            Self::Found(capture) => capture.witness(),
        }
    }
}

impl AgentLocatorWitness {
    /// Compare the exact head and selected SQL identity during the final owned
    /// read. This does not replace the physical stamp or full frozen proof.
    pub fn matches_current(&self, control: &ControlStore) -> PortResult<bool> {
        let db = control.connection.borrow();
        if db.is_autocommit() {
            return Err(port(PortErrorCode::Conflict, "control.agent-locator.fence"));
        }
        agent_locator::validate_coverage(&db)?;
        if let Some(pending) = &self.pending {
            return pending.matches_current(&db);
        }
        let key = Key::new(self.scope.borrow())?;
        let current: Option<HeadWitness> = db.query_row(
            "SELECT h.ordinal,m.operation_id,m.generation,m.accepted_digest,m.plan_digest,h.action
             FROM agent_locator_heads h JOIN agent_locator_members m ON m.ordinal=h.ordinal
             JOIN operations o ON o.rowid=m.ordinal AND o.operation_id=m.operation_id
             WHERE h.workspace_id=?1 AND h.scope=?2 AND h.context_id=?3 AND h.grant_key=?4
               AND o.workspace_id=m.workspace_id AND o.state='succeeded'
               AND o.generation=m.generation AND o.accepted_change_digest=m.accepted_digest",
            params![self.workspace.as_str(),key.scope,key.context,key.hash],
            |r| Ok(HeadWitness { ordinal:r.get(0)?, operation:r.get(1)?, generation:r.get(2)?, accepted:r.get(3)?, plan_digest:r.get(4)?, action:r.get(5)? }),
        ).optional().map_err(|_| invalid())?;
        if current.is_none() {
            let present: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_locator_heads WHERE workspace_id=?1 AND scope=?2 AND context_id=?3 AND grant_key=?4)",
                params![self.workspace.as_str(),key.scope,key.context,key.hash], |r| r.get(0),
            ).map_err(|_| invalid())?;
            if present {
                return Err(invalid());
            }
        }
        Ok(current == self.head)
    }
}

#[derive(Clone)]
enum OwnedScope {
    ContextFacet {
        context_id: String,
        facet: AgentSettingsFacet,
    },
    GrantProvenance {
        context_id: String,
        grant: AgentAccessGrantRefV1,
    },
    ResidentService,
}
impl OwnedScope {
    fn borrow(&self) -> AgentLocatorScope<'_> {
        match self {
            Self::ContextFacet { context_id, facet } => AgentLocatorScope::ContextFacet {
                context_id,
                facet: *facet,
            },
            Self::GrantProvenance { context_id, grant } => {
                AgentLocatorScope::GrantProvenance { context_id, grant }
            }
            Self::ResidentService => AgentLocatorScope::ResidentService,
        }
    }
}
impl From<AgentLocatorScope<'_>> for OwnedScope {
    fn from(scope: AgentLocatorScope<'_>) -> Self {
        match scope {
            AgentLocatorScope::ContextFacet { context_id, facet } => Self::ContextFacet {
                context_id: context_id.to_owned(),
                facet,
            },
            AgentLocatorScope::GrantProvenance { context_id, grant } => Self::GrantProvenance {
                context_id: context_id.to_owned(),
                grant: grant.clone(),
            },
            AgentLocatorScope::ResidentService => Self::ResidentService,
        }
    }
}

pub struct AgentLocatorCapture {
    frozen: FrozenAgentJournal,
    member: Member,
    workspace: WorkspaceId,
    scope: OwnedScope,
    head_action: String,
}

impl AgentLocatorCapture {
    fn witness(&self) -> PortResult<AgentLocatorWitness> {
        Ok(AgentLocatorWitness {
            workspace: self.workspace.clone(),
            scope: self.scope.clone(),
            head: Some(HeadWitness {
                ordinal: self.member.ordinal,
                operation: self.member.id.clone().ok_or_else(invalid)?,
                generation: self.member.generation.ok_or_else(invalid)?,
                accepted: self.member.accepted.clone().ok_or_else(invalid)?,
                plan_digest: self.member.plan_digest.clone().ok_or_else(invalid)?,
                action: self.head_action.clone(),
            }),
            pending: None,
        })
    }

    /// Complete the same strict selected-journal validation over the captured
    /// bytes. The caller must still fence current business/native dependencies.
    pub fn verify(self) -> PortResult<Arc<SucceededAgentOperationV1>> {
        let key = Key::new(self.scope.borrow())?;
        let row = self.frozen.row();
        let input = agent_locator_input::classify(&self.frozen, row)?.ok_or_else(invalid)?;
        let operation = self.frozen.verify()?;
        if self.member.generation != Some(row.generation)
            || self.member.accepted.as_deref() != Some(row.accepted_digest.as_str())
            || self.member.plan_digest.as_deref() != Some(input.plan_bytes_digest.as_str())
            || input.workspace_id != self.workspace
            || (key.scope != "resident" && input.context != key.context)
        {
            return Err(invalid());
        }
        match key.scope {
            "resident"
                if input.resident_service_declaration && self.head_action == "declaration" => {}
            "grant" if input.additional_model_grant_candidate && self.head_action == "grant" => {
                let [mutation] = input.plan.agent_access_grants() else {
                    return Err(invalid());
                };
                let [effect] = operation.grant_effects() else {
                    return Err(invalid());
                };
                if effect
                    .compensation
                    .get("operation_id")
                    .and_then(Value::as_str)
                    != Some(operation.operation_id.as_str())
                {
                    return Err(invalid());
                }
                let actual = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| invalid())?;
                if Some(&actual) != key.grant || actual.owner_scope() != self.workspace.as_str() {
                    return Err(invalid());
                }
            }
            "model" | "collaboration" | "management"
                if action(&input, &key) == Some(self.head_action.as_str()) => {}
            _ => return Err(invalid()),
        }
        Ok(Arc::new(operation))
    }
}

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.agent-locator.lookup")
}

struct Key<'a> {
    scope: &'static str,
    context: &'a str,
    grant: Option<&'a AgentAccessGrantRefV1>,
    hash: String,
}

impl<'a> Key<'a> {
    fn new(value: AgentLocatorScope<'a>) -> PortResult<Self> {
        let (scope, context, grant) = match value {
            AgentLocatorScope::ContextFacet { context_id, facet } => (
                match facet {
                    AgentSettingsFacet::Model => "model",
                    AgentSettingsFacet::Collaboration => "collaboration",
                    AgentSettingsFacet::ManagementSkill => "management",
                },
                context_id,
                None,
            ),
            AgentLocatorScope::GrantProvenance { context_id, grant } => {
                grant.validate().map_err(|_| invalid())?;
                if grant.connection_id() != format!("agent-connection/{context_id}") {
                    return Err(invalid());
                }
                ("grant", context_id, Some(grant))
            }
            AgentLocatorScope::ResidentService => ("resident", "", None),
        };
        if scope != "resident" && (context.is_empty() || context.len() > 256) {
            return Err(invalid());
        }
        let hash = grant
            .map(CanonicalDigest::of)
            .transpose()
            .map_err(|_| invalid())?
            .map(|digest| digest.to_string())
            .unwrap_or_default();
        Ok(Self {
            scope,
            context,
            grant,
            hash,
        })
    }
}

#[derive(Clone)]
struct Member {
    ordinal: i64,
    id: Option<String>,
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

const MEMBER_COLUMNS: &str = "ordinal,operation_id,workspace_id,context_id,generation,accepted_digest,plan_digest,model_action,collaboration_action,management_action,resident_declaration,grant_ref_json,grant_unresolved,classification";

impl Member {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            ordinal: row.get(0)?,
            id: row.get(1)?,
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
    fn affects(&self, workspace: &str, key: &Key<'_>, unknown_grant: bool) -> PortResult<bool> {
        if self
            .workspace
            .as_deref()
            .is_some_and(|candidate| candidate != workspace)
        {
            return Ok(false);
        }
        match self.classification.as_str() {
            "unresolved" => Ok(true),
            "other" => Ok(false),
            "settings" => {
                if self.workspace.as_deref() != Some(workspace) {
                    return Err(invalid());
                }
                if key.scope == "resident" {
                    return Ok(self.resident);
                }
                if self.context.as_deref() != Some(key.context) {
                    return Ok(false);
                }
                match key.scope {
                    "model" => Ok(self.model.is_some()),
                    "collaboration" => Ok(self.collaboration.is_some()),
                    "management" => Ok(self.management.is_some()),
                    "grant" => {
                        if self.grant_unresolved || unknown_grant {
                            return Ok(self.grant_unresolved || self.grant_json.is_some());
                        }
                        self.grant_json
                            .as_ref()
                            .map(|json| {
                                let reference: AgentAccessGrantRefV1 =
                                    serde_json::from_str(json).map_err(|_| invalid())?;
                                reference.validate().map_err(|_| invalid())?;
                                Ok(Some(&reference) == key.grant)
                            })
                            .unwrap_or(Ok(false))
                    }
                    _ => Err(invalid()),
                }
            }
            _ => Err(invalid()),
        }
    }
}

fn row_for_id(db: &Connection, id: &str) -> PortResult<Option<AgentHistoryRow>> {
    db.query_row(
        "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE operation_id=?1",
        [id], AgentHistoryRow::read,
    ).optional().map_err(|_| invalid())
}

fn current_affects(
    db: &Connection,
    row: &AgentHistoryRow,
    workspace: &str,
    key: &Key<'_>,
) -> PortResult<bool> {
    if !agent_locator_input::has_bound_header(row) {
        return Ok(true);
    }
    if row.workspace_id != workspace || row.state != "succeeded" {
        return Ok(false);
    }
    if !["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"]
        .contains(&row.operation_kind.as_str())
    {
        return Ok(false);
    }
    let input = match agent_locator_input::classify(db, row) {
        Ok(Some(input)) => input,
        Ok(None) => return Ok(false),
        Err(error)
            if matches!(
                error.code,
                PortErrorCode::Corrupt | PortErrorCode::InvalidData
            ) =>
        {
            return Ok(true);
        }
        Err(error) => return Err(error),
    };
    if key.scope == "resident" {
        return Ok(input.resident_service_declaration);
    }
    if input.context != key.context {
        return Ok(false);
    }
    Ok(match key.scope {
        "model" => input.model.is_some(),
        "collaboration" => input.collaboration.is_some(),
        "management" => input.management.is_some(),
        "grant" if input.additional_model_grant_candidate => {
            match agent_locator_input::grant_reference(db, row, &input) {
                Ok(reference) => Some(&reference) == key.grant,
                Err(error)
                    if matches!(
                        error.code,
                        PortErrorCode::Corrupt | PortErrorCode::InvalidData
                    ) =>
                {
                    true
                }
                Err(error) => return Err(error),
            }
        }
        "grant" => false,
        _ => return Err(invalid()),
    })
}

fn uncertain_changes(
    db: &Connection,
    workspace: &str,
    key: &Key<'_>,
    head: Option<i64>,
) -> PortResult<bool> {
    let mut query = db.prepare("SELECT operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags FROM agent_locator_dirty ORDER BY event_id").map_err(|_| invalid())?;
    let events = query
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, u32>(5)?,
            ))
        })
        .map_err(|_| invalid())?;
    let mut changes: BTreeMap<String, (u32, Vec<i64>)> = BTreeMap::new();
    for event in events {
        let (id, old, new, old_workspace, new_workspace, flags) = event.map_err(|_| invalid())?;
        let ordinals: Vec<_> = [old, new].into_iter().flatten().collect();
        let workspaces: Vec<_> = [old_workspace.as_deref(), new_workspace.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if !workspaces.is_empty() && !workspaces.contains(&workspace) {
            continue;
        }
        if let Some(head) = head
            && !ordinals.is_empty()
            && ordinals.iter().all(|ordinal| *ordinal <= head)
        {
            continue;
        }
        let Some(id) = id else {
            return Ok(true);
        };
        let entry = changes.entry(id).or_insert_with(|| (0, Vec::new()));
        entry.0 |= flags;
        entry.1.extend(ordinals);
    }
    drop(query);
    for (id, (flags, ordinals)) in changes {
        let sql =
            format!("SELECT {MEMBER_COLUMNS} FROM agent_locator_members WHERE operation_id=?1");
        let mut member = db
            .query_row(&sql, [&id], Member::read)
            .optional()
            .map_err(|_| invalid())?;
        if member.is_none() {
            for ordinal in ordinals {
                let sql =
                    format!("SELECT {MEMBER_COLUMNS} FROM agent_locator_members WHERE ordinal=?1");
                let candidate = db
                    .query_row(&sql, [ordinal], Member::read)
                    .optional()
                    .map_err(|_| invalid())?;
                if candidate
                    .as_ref()
                    .is_some_and(|m| m.classification == "unresolved")
                {
                    member = candidate;
                    break;
                }
            }
        }
        if member.as_ref().is_some_and(|m| {
            m.affects(workspace, key, key.scope == "grant" && flags & 6 != 0)
                .unwrap_or(true)
        }) {
            return Ok(true);
        }
        if (flags & 1 != 0 || member.is_none())
            && let Some(row) = row_for_id(db, &id)?
            && current_affects(db, &row, workspace, key)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn action(input: &agent_locator_input::LocatorInput, key: &Key<'_>) -> Option<&'static str> {
    use agent_locator_input::FacetAction;
    let value = match key.scope {
        "model" => input.model,
        "collaboration" => input.collaboration,
        "management" => input.management,
        _ => None,
    };
    match value {
        Some(FacetAction::Configure) => Some("configure"),
        Some(FacetAction::Restore) => Some("restore"),
        None => None,
    }
}

fn selected_metadata_after_coverage(
    db: &Connection,
    workspace: &WorkspaceId,
    scope: AgentLocatorScope<'_>,
) -> PortResult<Option<(Member, String)>> {
    let key = Key::new(scope)?;
    if key
        .grant
        .is_some_and(|grant| grant.owner_scope() != workspace.as_str())
    {
        return Err(invalid());
    }
    let head: Option<(i64,String)> = db.query_row(
        "SELECT ordinal,action FROM agent_locator_heads WHERE workspace_id=?1 AND scope=?2 AND context_id=?3 AND grant_key=?4",
        params![workspace.as_str(),key.scope,key.context,key.hash], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|_| invalid())?;
    let ordinal = head.as_ref().map(|h| h.0);
    let unresolved: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_locator_members WHERE
         (classification='unresolved' AND (workspace_id IS NULL OR workspace_id=?1)
            OR (?2='grant' AND grant_unresolved=1 AND workspace_id=?1 AND context_id=?3))
         AND (?4 IS NULL OR ordinal>?4))",
            params![workspace.as_str(), key.scope, key.context, ordinal],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if unresolved || uncertain_changes(db, workspace.as_str(), &key, ordinal)? {
        return Err(invalid());
    }
    let Some((ordinal, head_action)) = head else {
        return Ok(None);
    };
    let sql = format!("SELECT {MEMBER_COLUMNS} FROM agent_locator_members WHERE ordinal=?1");
    let member = db
        .query_row(&sql, [ordinal], Member::read)
        .map_err(|_| invalid())?;
    if member.ordinal != ordinal
        || member.classification != "settings"
        || !member.affects(workspace.as_str(), &key, false)?
    {
        return Err(invalid());
    }
    let id = member.id.as_deref().ok_or_else(invalid)?;
    let (actual_ordinal, generation, accepted, actual_workspace, state): (i64,u64,String,String,String) = db
        .query_row(
            "SELECT rowid,generation,accepted_change_digest,workspace_id,state FROM operations WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        )
        .map_err(|_| invalid())?;
    if actual_ordinal != ordinal
        || member.generation != Some(generation)
        || member.accepted.as_deref() != Some(accepted.as_str())
        || actual_workspace != workspace.as_str()
        || state != "succeeded"
    {
        return Err(invalid());
    }
    let expected = match key.scope {
        "model" => member.model.as_deref(),
        "collaboration" => member.collaboration.as_deref(),
        "management" => member.management.as_deref(),
        "resident" => Some("declaration"),
        "grant" => Some("grant"),
        _ => None,
    };
    if expected != Some(head_action.as_str()) {
        return Err(invalid());
    }
    Ok(Some((member, head_action)))
}

fn selected(
    db: &Connection,
    workspace: &WorkspaceId,
    scope: AgentLocatorScope<'_>,
) -> PortResult<Option<AgentLocatorCapture>> {
    let Some((member, head_action)) = selected_metadata_after_coverage(db, workspace, scope)?
    else {
        return Ok(None);
    };
    let row = row_for_id(db, member.id.as_deref().ok_or_else(invalid)?)?.ok_or_else(invalid)?;
    let frozen = FrozenAgentJournal::capture(db, row)?;
    Ok(Some(AgentLocatorCapture {
        frozen,
        member,
        workspace: workspace.clone(),
        scope: scope.into(),
        head_action,
    }))
}

fn summary_after_coverage(
    db: &Connection,
    workspace: &WorkspaceId,
    context_id: &str,
    facet: AgentSettingsFacet,
) -> PortResult<AgentLocatorSummary> {
    let scope = AgentLocatorScope::ContextFacet { context_id, facet };
    if let Some(attention) = super::agent_locator_pending::summary(db, workspace, scope)? {
        return Ok(if attention {
            AgentLocatorSummary::NeedsAttention
        } else {
            AgentLocatorSummary::Pending
        });
    }
    let selected = selected_metadata_after_coverage(db, workspace, scope)?;
    if let Some((member, _)) = &selected {
        let dirty: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_locator_dirty WHERE operation_id=?1)",
                [member.id.as_deref().ok_or_else(invalid)?],
                |r| r.get(0),
            )
            .map_err(|_| invalid())?;
        if dirty {
            return Err(invalid());
        }
    }
    Ok(match selected {
        None => AgentLocatorSummary::Absent,
        Some((_, action)) if action == "configure" => AgentLocatorSummary::RecordedConfigure,
        Some((_, action)) if action == "restore" => AgentLocatorSummary::RecordedRestore,
        _ => return Err(invalid()),
    })
}

impl ControlStore {
    /// The whole Home batch shares one owned snapshot and one coverage check.
    /// Individual scope errors remain explicit, never a fabricated empty result.
    pub fn agent_locator_summaries(
        &self,
        workspace: &WorkspaceId,
        contexts: &[String],
    ) -> PortResult<Vec<[PortResult<AgentLocatorSummary>; 3]>> {
        let db = self.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "control.agent-locator.summary-snapshot",
            ));
        }
        agent_locator::validate_coverage(&db)?;
        Ok(contexts
            .iter()
            .map(|context| {
                [
                    summary_after_coverage(&db, workspace, context, AgentSettingsFacet::Model),
                    summary_after_coverage(
                        &db,
                        workspace,
                        context,
                        AgentSettingsFacet::Collaboration,
                    ),
                    summary_after_coverage(
                        &db,
                        workspace,
                        context,
                        AgentSettingsFacet::ManagementSkill,
                    ),
                ]
            })
            .collect())
    }

    #[cfg(test)]
    pub fn agent_locator_summary(
        &self,
        workspace: &WorkspaceId,
        context_id: &str,
        facet: AgentSettingsFacet,
    ) -> PortResult<AgentLocatorSummary> {
        let db = self.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "control.agent-locator.summary-snapshot",
            ));
        }
        agent_locator::validate_coverage(&db)?;
        summary_after_coverage(&db, workspace, context_id, facet)
    }

    /// A non-authoritative preflight that only selects which physical stores to
    /// stamp. The caller must capture the locator again and reject any mismatch.
    pub fn may_need_agent_model_grant(
        &self,
        workspace: &WorkspaceId,
        context_id: &str,
    ) -> PortResult<bool> {
        let db = self.connection.borrow();
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_locator_heads WHERE workspace_id=?1
                       AND scope='model' AND context_id=?2 AND grant_key='')
               AND NOT EXISTS(SELECT 1 FROM agent_locator_pending p
                       WHERE (p.workspace_id IS NULL OR p.workspace_id=?1)
                         AND (p.classification='unresolved'
                           OR (p.classification='settings' AND p.context_id=?2 AND p.model_action IS NOT NULL)))",
            params![workspace.as_str(), context_id], |r| r.get(0),
        ).map_err(|_| invalid())
    }

    /// Capture inside a caller-owned, bounded Control read transaction. The
    /// snapshot does not grant status authority until verified and fenced.
    pub fn capture_agent_locator(
        &self,
        workspace: &WorkspaceId,
        scope: AgentLocatorScope<'_>,
    ) -> PortResult<CapturedAgentLocator> {
        let db = self.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "control.agent-locator.snapshot",
            ));
        }
        agent_locator::validate_coverage(&db)?;
        if !matches!(scope, AgentLocatorScope::GrantProvenance { .. })
            && let Some(pending) = super::agent_locator_pending::capture(&db, workspace, scope)?
        {
            let witness = AgentLocatorWitness {
                workspace: workspace.clone(),
                scope: scope.into(),
                head: None,
                pending: Some(pending.witness()),
            };
            return Ok(CapturedAgentLocator::Pending(pending, witness));
        }
        Ok(match selected(&db, workspace, scope)? {
            Some(found) => CapturedAgentLocator::Found(found),
            None => CapturedAgentLocator::AbsentWithCompleteCoverage(AgentLocatorWitness {
                workspace: workspace.clone(),
                scope: scope.into(),
                head: None,
                pending: None,
            }),
        })
    }

    /// The returned record has a strict journal proof, but is not a native/current
    /// status proof. History and recovery retain their existing independent readers.
    pub fn find_agent_locator(
        &self,
        workspace: &WorkspaceId,
        scope: AgentLocatorScope<'_>,
    ) -> PortResult<AgentLocatorResult> {
        let found = {
            let db = self.connection.borrow();
            let tx = if db.is_autocommit() {
                Some(db.unchecked_transaction().map_err(|_| invalid())?)
            } else {
                None
            };
            let found = self.capture_agent_locator(workspace, scope)?;
            if let Some(tx) = tx {
                tx.commit().map_err(|_| invalid())?;
            }
            found
        };
        Ok(match found {
            CapturedAgentLocator::Found(found) => AgentLocatorResult::Found(found.verify()?),
            CapturedAgentLocator::Pending(pending, _) => {
                AgentLocatorResult::Pending(pending.verify()?)
            }
            CapturedAgentLocator::AbsentWithCompleteCoverage(_) => {
                AgentLocatorResult::AbsentWithCompleteCoverage
            }
        })
    }
}
