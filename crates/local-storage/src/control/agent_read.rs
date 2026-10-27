//! Strict successful Agent journal projection. It has no execution or journal-write capability.
use super::*;
use hiroute_domain::{
    AgentOperationInputView, AgentOperationRead, AgentPlanReadV1, OperationStepKind,
};

#[cfg(test)]
thread_local! {
    pub(super) static DECODE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Debug)]
pub struct SucceededAgentOperationV1 {
    pub operation_id: OperationId,
    pub workspace_id: WorkspaceId,
    pub idempotency: IdempotencyScopeV1,
    pub accepted_digest: CanonicalDigest,
    pub state: OperationState,
    pub plan: AgentPlanReadV1,
    grant_effects: Vec<OwnedEffectV1>,
    service_publication_revision: Option<u64>,
}

impl SucceededAgentOperationV1 {
    /// Effects needed for the current grant join, after the entire succeeded
    /// journal has been verified. This read projection cannot replay or restore.
    pub fn grant_effects(&self) -> &[OwnedEffectV1] {
        &self.grant_effects
    }

    pub fn service_publication_revision(&self) -> Option<u64> {
        self.service_publication_revision
    }

    pub(super) fn accounted_bytes(&self) -> usize {
        // Large publication rollback bodies were validated but are not retained.
        // Budget the actual immutable projection, with conservative JSON/node
        // overhead; do not charge discarded input or silently raise the limit.
        serde_json::to_vec(&(
            &self.operation_id,
            &self.workspace_id,
            &self.idempotency,
            &self.accepted_digest,
            self.plan.spec(),
            self.plan.control(),
            self.plan.external(),
            self.plan.agent_access_grants(),
            &self.grant_effects,
            self.service_publication_revision,
        ))
        .map_or(usize::MAX, |bytes| {
            bytes.len().saturating_mul(8).saturating_add(64 * 1024)
        })
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
    decode_selected_agent(connection, encoded, None)
}

pub(super) fn decode_selected_agent(
    connection: &Connection,
    encoded: &str,
    selected: Option<&super::agent_history_cache::AgentHistoryRow>,
) -> PortResult<SucceededAgentOperationV1> {
    #[cfg(test)]
    DECODE_COUNT.with(|count| count.set(count.get() + 1));
    let invalid = || port(PortErrorCode::Corrupt, "control.agent-read.journal");
    let expanded = payloads::expand(connection, encoded)?;
    let encoded = expanded.as_str();
    let durable: DurableOperation = serde_json::from_str(encoded).map_err(|_| invalid())?;
    if selected.is_some_and(|row| !row.matches(&durable))
        || durable.schema_version != hiroute_domain::OPERATION_SCHEMA_VERSION
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
        let step =
            payloads::expand_step_record(connection, durable.operation_id.as_str(), &encoded)?;
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
        service_publication_revision: durable.steps[5]
            .terminal_result
            .as_deref()
            .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
            .map(|receipt| receipt.publication_revision),
        operation_id: durable.operation_id,
        workspace_id: durable.workspace_id,
        idempotency: durable.idempotency,
        accepted_digest: durable.accepted_digest,
        state: durable.state,
        plan,
        grant_effects: durable
            .steps
            .into_iter()
            .find(|step| step.kind == OperationStepKind::ApplySecrets)
            .expect("validated fixed journal")
            .effects,
    })
}

impl ControlStore {
    /// Best-effort historical candidates for observation attribution only. The caller must
    /// still prove every current managed publication grant from the returned operations.
    /// This does not relax the strict journal readers used for execution or recovery.
    pub fn agent_attribution_recovery_candidates(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<(Vec<SucceededAgentOperationV1>, u64)> {
        let (facts, rejected) =
            self.read_verified_agent_history(workspace, &["ApplyAgentConnectionChange"], false)?;
        Ok((
            facts.into_iter().map(|fact| (*fact).clone()).collect(),
            rejected,
        ))
    }

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
        self.verified_agent_history_for_kinds(workspace, kinds)
            .map(|facts| facts.into_iter().map(|fact| (*fact).clone()).collect())
    }
}

pub(super) fn validate_kinds(kinds: &[&str]) -> PortResult<()> {
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
    Ok(())
}
