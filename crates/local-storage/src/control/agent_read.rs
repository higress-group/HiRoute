//! Strict successful Agent journal projection. It has no execution or journal-write capability.
use super::*;
use hiroute_domain::{
    AgentOperationInputView, AgentOperationRead, AgentPlanReadV1, OperationStepKind,
};

#[derive(Clone, Debug)]
pub struct SucceededAgentOperationV1 {
    pub operation_id: OperationId,
    pub workspace_id: WorkspaceId,
    pub idempotency: IdempotencyScopeV1,
    pub accepted_digest: CanonicalDigest,
    pub state: OperationState,
    pub plan: AgentPlanReadV1,
    steps: Vec<OperationStepV1>,
}

impl SucceededAgentOperationV1 {
    pub fn step(&self, kind: OperationStepKind) -> &OperationStepV1 {
        &self.steps[OperationStepKind::ALL
            .iter()
            .position(|k| *k == kind)
            .expect("fixed journal")]
    }
}

impl AgentOperationRead for SucceededAgentOperationV1 {
    fn agent_input(&self) -> AgentOperationInputView<'_> {
        AgentOperationInputView {
            spec: self.plan.spec(),
            control: self.plan.control(),
            external: self.plan.external(),
            grants: self.plan.agent_access_grants(),
        }
    }
    fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }
    fn accepted_digest(&self) -> &CanonicalDigest {
        &self.accepted_digest
    }
    fn state(&self) -> OperationState {
        self.state
    }
}

pub(super) fn decode_succeeded_agent(
    connection: &Connection,
    encoded: &str,
) -> PortResult<SucceededAgentOperationV1> {
    let invalid = || port(PortErrorCode::Corrupt, "control.agent-read.journal");
    let durable: DurableOperation = serde_json::from_str(encoded).map_err(|_| invalid())?;
    if durable.schema_version != hiroute_domain::OPERATION_SCHEMA_VERSION
        || durable.state != OperationState::Succeeded
        || OperationId::derive(
            &durable.workspace_id,
            &durable.idempotency,
            &durable.request_digest,
        ) != durable.operation_id
    {
        return Err(invalid());
    }
    WorkspaceId::parse(durable.workspace_id.as_str()).map_err(|_| invalid())?;
    IdempotencyScopeV1::new(
        durable.idempotency.principal.as_str(),
        durable.idempotency.operation_kind.as_str(),
        durable.idempotency.key.as_str(),
    )
    .map_err(|_| invalid())?;
    for digest in [&durable.request_digest, &durable.accepted_digest] {
        CanonicalDigest::parse(digest.as_str()).map_err(|_| invalid())?;
    }
    let plan = AgentPlanReadV1::from_stored(&durable.plan).map_err(|_| invalid())?;
    let expected_kind = plan.operation_kind().map_err(|_| invalid())?;
    if durable.idempotency.operation_kind != expected_kind
        || durable
            .steps
            .iter()
            .any(|s| s.status != hiroute_domain::OperationStepStatus::Applied)
    {
        return Err(invalid());
    }
    durable
        .plan
        .validate_step_proofs(
            &durable.accepted_digest,
            &durable.expected_revisions,
            plan.agent_access_grants(),
            &durable.steps,
        )
        .map_err(|_| invalid())?;
    if plan
        .external()
        .iter()
        .any(hiroute_domain::is_settings_managed_configuration)
    {
        let receipt = durable.steps[5]
            .terminal_result
            .as_deref()
            .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
            .ok_or_else(invalid)?;
        let proof = CanonicalDigest::of(&(
            "hiroute.settings-service-proof/v2",
            durable.operation_id.as_str(),
            &durable.accepted_digest,
            receipt.publication_revision,
            &receipt.publication_digest,
            durable.plan.digest().map_err(|_| invalid())?,
        ))
        .map_err(|_| invalid())?;
        if proof != receipt.completed_effects_digest
            || !durable.steps[3].effects.iter().any(|e| {
                e.kind == OwnedEffectKind::Publication
                    && e.after_fingerprint.as_ref() == Some(&receipt.publication_digest)
                    && e.compensation["publication_revision"].as_u64()
                        == Some(receipt.publication_revision)
            })
        {
            return Err(invalid());
        }
    }
    let raw: Value = serde_json::from_str(encoded).map_err(|_| invalid())?;
    if serde_json::to_value(&durable.steps).map_err(|_| invalid())? != raw["steps"]
        || durable.steps.iter().flat_map(|s| &s.effects).any(|effect| {
            effect
                .compensation
                .get("operation_id")
                .is_some_and(|id| id != durable.operation_id.as_str())
        })
    {
        return Err(invalid());
    }
    let identity: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE operation_id=?1 AND workspace_id=?2
         AND principal=?3 AND operation_kind=?4 AND idempotency_key=?5 AND request_digest=?6
         AND accepted_change_digest=?7 AND state='succeeded' AND generation=?8)",
            params![
                durable.operation_id.as_str(),
                durable.workspace_id.as_str(),
                durable.idempotency.principal,
                durable.idempotency.operation_kind,
                durable.idempotency.key,
                durable.request_digest.as_str(),
                durable.accepted_digest.as_str(),
                durable.generation
            ],
            |r| r.get(0),
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "control.agent-read.identity"))?;
    if !identity {
        return Err(invalid());
    }
    let mut statement = connection.prepare("SELECT step_no,step_kind,state,step_json FROM operation_steps WHERE operation_id=?1 ORDER BY step_no")
        .map_err(|_| port(PortErrorCode::Unavailable,"control.agent-read.steps"))?;
    let rows = statement
        .query_map([durable.operation_id.as_str()], |r| {
            Ok((
                r.get::<_, usize>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| port(PortErrorCode::Unavailable, "control.agent-read.steps"))?;
    let mut count = 0;
    for row in rows {
        let (number, kind, state, encoded) = row.map_err(|_| invalid())?;
        let step: Value = serde_json::from_str(&encoded).map_err(|_| invalid())?;
        if number != count
            || number >= 6
            || step.as_object().is_none_or(|o| o.len() != 2)
            || step["schema"] != "hiroute.operation-step/v1"
            || step["step"] != raw["steps"][number]
            || step["step"]["kind"] != kind
            || step["step"]["status"] != state
        {
            return Err(invalid());
        }
        count += 1;
    }
    if count != 6 {
        return Err(invalid());
    }
    Ok(SucceededAgentOperationV1 {
        operation_id: durable.operation_id,
        workspace_id: durable.workspace_id,
        idempotency: durable.idempotency,
        accepted_digest: durable.accepted_digest,
        state: durable.state,
        plan,
        steps: durable.steps,
    })
}

impl ControlStore {
    pub fn succeeded_agent_operations_for_kind(
        &self,
        workspace: &WorkspaceId,
        kind: &str,
    ) -> PortResult<Vec<SucceededAgentOperationV1>> {
        self.succeeded_agent_operations_for_kinds(workspace, &[kind])
    }
    pub fn succeeded_agent_operations_for_kinds(
        &self,
        workspace: &WorkspaceId,
        kinds: &[&str],
    ) -> PortResult<Vec<SucceededAgentOperationV1>> {
        if kinds.is_empty()
            || kinds.len() > 2
            || kinds.iter().any(|kind| {
                !matches!(
                    *kind,
                    "ApplyAgentConnectionChange" | "ApplyAgentConnectionRestore"
                )
            })
        {
            return Err(port(PortErrorCode::InvalidData, "control.agent-read.kinds"));
        }
        let placeholders = vec!["?"; kinds.len()].join(",");
        let connection = self.connection.borrow();
        let mut statement = connection.prepare(&format!("SELECT operation_json FROM operations WHERE workspace_id=? AND operation_kind IN ({placeholders}) AND state='succeeded' ORDER BY rowid DESC"))
            .map_err(|_| port(PortErrorCode::Unavailable,"control.agent-read.query"))?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(
                    std::iter::once(workspace.as_str()).chain(kinds.iter().copied()),
                ),
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.agent-read.query"))?;
        rows.map(|row| {
            decode_succeeded_agent(
                &connection,
                &row.map_err(|_| port(PortErrorCode::Corrupt, "control.agent-read.row"))?,
            )
        })
        .collect()
    }
}
