//! Immutable locator input, never an execution or current-status proof.
//!
//! Classification deliberately reads the operation header and immutable Plan only.
//! Optional historical step compensation is not decoded to decide context/facet.
use super::{agent_history_cache::AgentHistoryRow, *};
use hiroute_domain::{
    AgentFacetIntent, AgentModelSelectionV2, AgentPlanReadV1, AgentSettingsSpecV2,
    StoredOperationInputV1,
};
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FacetAction {
    Configure,
    Restore,
}

/// Facts for building a locator. They do not authenticate the complete journal,
/// grant, publication, current native file, or any right to mutate state.
pub(super) struct LocatorInput {
    pub(super) operation_id: OperationId,
    pub(super) workspace_id: WorkspaceId,
    pub(super) generation: u64,
    pub(super) accepted_digest: CanonicalDigest,
    pub(super) plan_bytes_digest: CanonicalDigest,
    pub(super) context: String,
    pub(super) model: Option<FacetAction>,
    pub(super) collaboration: Option<FacetAction>,
    pub(super) management: Option<FacetAction>,
    pub(super) resident_service_declaration: bool,
    pub(super) additional_model_grant_candidate: bool,
    pub(super) plan: AgentPlanReadV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeFields {
    principal: String,
    operation_kind: String,
    key: String,
}

// Keep irrelevant steps/revisions as raw JSON. Parsing the envelope verifies its
// own field names and duplicate members without expanding optional compensation.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    schema_version: u16,
    operation_id: OperationId,
    workspace_id: WorkspaceId,
    idempotency: ScopeFields,
    request_digest: CanonicalDigest,
    accepted_digest: CanonicalDigest,
    #[serde(rename = "expected_revisions", borrow)]
    _expected_revisions: &'a RawValue,
    #[serde(borrow)]
    plan: &'a RawValue,
    state: OperationState,
    generation: u64,
    #[serde(rename = "steps", borrow)]
    _steps: &'a RawValue,
    #[serde(default, rename = "safe_error_code", borrow)]
    _safe_error_code: Option<&'a RawValue>,
}

fn action<T>(intent: &AgentFacetIntent<T>) -> Option<FacetAction> {
    match intent {
        AgentFacetIntent::Keep => None,
        AgentFacetIntent::Configure { .. } => Some(FacetAction::Configure),
        AgentFacetIntent::Restore { .. } => Some(FacetAction::Restore),
    }
}

fn checked_header(row: &AgentHistoryRow) -> PortResult<(Envelope<'_>, IdempotencyScopeV1)> {
    let invalid = || port(PortErrorCode::Corrupt, "control.agent-locator.input");
    let envelope: Envelope<'_> = serde_json::from_str(&row.encoded).map_err(|_| invalid())?;
    let scope = IdempotencyScopeV1::new(
        &envelope.idempotency.principal,
        &envelope.idempotency.operation_kind,
        &envelope.idempotency.key,
    )
    .map_err(|_| invalid())?;
    WorkspaceId::parse(envelope.workspace_id.as_str()).map_err(|_| invalid())?;
    for digest in [&envelope.request_digest, &envelope.accepted_digest] {
        CanonicalDigest::parse(digest.as_str()).map_err(|_| invalid())?;
    }
    if envelope.schema_version != hiroute_domain::OPERATION_SCHEMA_VERSION
        || envelope.operation_id
            != OperationId::derive(&envelope.workspace_id, &scope, &envelope.request_digest)
        || row.operation_id != envelope.operation_id.as_str()
        || row.workspace_id != envelope.workspace_id.as_str()
        || row.principal != scope.principal
        || row.operation_kind != scope.operation_kind
        || row.idempotency_key != scope.key
        || row.request_digest != envelope.request_digest.as_str()
        || row.accepted_digest != envelope.accepted_digest.as_str()
        || row.state != envelope.state.as_str()
        || row.generation != envelope.generation
    {
        return Err(invalid());
    }
    Ok((envelope, scope))
}

/// Establishes which workspace an otherwise unclassifiable row could affect.
pub(super) fn has_bound_header(row: &AgentHistoryRow) -> bool {
    checked_header(row).is_ok()
}

pub(super) fn classify(
    connection: &impl payloads::PayloadRead,
    row: &AgentHistoryRow,
) -> PortResult<Option<LocatorInput>> {
    let invalid = || port(PortErrorCode::Corrupt, "control.agent-locator.input");
    let (envelope, scope) = checked_header(row)?;
    let expanded =
        payloads::expand_payload(connection, envelope.operation_id.as_str(), envelope.plan)?;
    journal::validate_encoding(expanded.get())?;
    let stored: StoredOperationInputV1 =
        serde_json::from_str(expanded.get()).map_err(|_| invalid())?;
    let plan = AgentPlanReadV1::from_stored(&stored).map_err(|_| invalid())?;
    if plan.operation_kind().map_err(|_| invalid())? != scope.operation_kind {
        return Err(invalid());
    }
    if plan.spec().command_id != "agents.settings.apply" {
        return Ok(None);
    }
    let settings: AgentSettingsSpecV2 =
        serde_json::from_value(plan.spec().desired_state.clone()).map_err(|_| invalid())?;
    if plan.spec().resource_id.as_deref() != Some(settings.context_id.as_str()) {
        return Err(invalid());
    }
    let additional_model_grant_candidate = matches!(
        &settings.model,
        AgentFacetIntent::Configure {
            settings: AgentModelSelectionV2::QoderAdditional { .. }
                | AgentModelSelectionV2::PiAdditional { .. }
                | AgentModelSelectionV2::DshAdditional { .. }
                | AgentModelSelectionV2::OpenCodeAdditional { .. },
        }
    );
    if additional_model_grant_candidate
        && !plan
            .external()
            .iter()
            .any(hiroute_domain::is_settings_additional_model)
    {
        // A malformed candidate cannot silently disappear from grant coverage.
        return Err(invalid());
    }
    let resident_service_declaration = plan
        .external()
        .iter()
        .any(|intent| intent.effect_id() == "agent-connection-login-item");
    Ok(Some(LocatorInput {
        operation_id: envelope.operation_id,
        workspace_id: envelope.workspace_id,
        generation: envelope.generation,
        accepted_digest: envelope.accepted_digest,
        plan_bytes_digest: CanonicalDigest::of_bytes(expanded.get().as_bytes()),
        context: settings.context_id,
        model: action(&settings.model),
        collaboration: action(&settings.collaboration),
        management: settings.management_skill.as_ref().and_then(action),
        resident_service_declaration,
        additional_model_grant_candidate,
        plan,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepRecord<'a> {
    schema: String,
    #[serde(borrow)]
    step: &'a RawValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepShape<'a> {
    sequence: u16,
    kind: hiroute_domain::OperationStepKind,
    deterministic_input_digest: CanonicalDigest,
    status: hiroute_domain::OperationStepStatus,
    attempts: u32,
    #[serde(default, borrow)]
    effects: Vec<&'a RawValue>,
    #[serde(default)]
    terminal_result: Option<String>,
}

/// Read only the operation-owned Ensure receipt needed to distinguish exact
/// grant references. A successful result is a locator hint, never authority.
/// The caller keeps the selected row and this read in one SQLite snapshot.
pub(super) fn grant_reference(
    connection: &Connection,
    row: &AgentHistoryRow,
    input: &LocatorInput,
) -> PortResult<hiroute_domain::AgentAccessGrantRefV1> {
    use hiroute_domain::{AgentAccessGrantRefV1, OperationStepKind, OperationStepStatus};
    let invalid = || port(PortErrorCode::Corrupt, "control.agent-locator.grant");
    if !input.additional_model_grant_candidate
        || input.operation_id.as_str() != row.operation_id
        || input.workspace_id.as_str() != row.workspace_id
        || input.generation != row.generation
        || input.accepted_digest.as_str() != row.accepted_digest
    {
        return Err(invalid());
    }
    let envelope: Envelope<'_> = serde_json::from_str(&row.encoded).map_err(|_| invalid())?;
    let steps: Vec<&RawValue> =
        serde_json::from_str(envelope._steps.get()).map_err(|_| invalid())?;
    if steps.len() != 6 {
        return Err(invalid());
    }
    let (kind, state, encoded): (String, String, String) = connection
        .query_row(
            "SELECT step_kind,state,step_json FROM operation_steps WHERE operation_id=?1 AND step_no=1",
            [row.operation_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| invalid())?;
    journal::validate_encoding(&encoded)?;
    let record: StepRecord<'_> = serde_json::from_str(&encoded).map_err(|_| invalid())?;
    if record.schema != "hiroute.operation-step/v1" {
        return Err(invalid());
    }
    // Valid Agent settings plans contain no other Secret mutations and at most
    // one Ensure. Refuse unexpected extra effects rather than expanding them.
    for fragment in [steps[1], record.step] {
        journal::validate_encoding(fragment.get())?;
        let shape: StepShape<'_> = serde_json::from_str(fragment.get()).map_err(|_| invalid())?;
        if shape.sequence != 1
            || shape.kind != OperationStepKind::ApplySecrets
            || shape.status != OperationStepStatus::Applied
            || shape.effects.len() != 1
        {
            return Err(invalid());
        }
        CanonicalDigest::parse(shape.deterministic_input_digest.as_str()).map_err(|_| invalid())?;
        let _ = (shape.attempts, shape.terminal_result);
    }
    let journal_step = payloads::expand_step(connection, row.operation_id.as_str(), steps[1])?;
    let indexed_step = payloads::expand_step(connection, row.operation_id.as_str(), record.step)?;
    if !journal::journal_fragments_equal(journal_step.get(), indexed_step.get())? {
        return Err(invalid());
    }
    let step: OperationStepV1 = serde_json::from_str(journal_step.get()).map_err(|_| invalid())?;
    let raw: Value = serde_json::from_str(journal_step.get()).map_err(|_| invalid())?;
    if serde_json::to_value(&step).map_err(|_| invalid())? != raw
        || serde_json::to_value(step.kind)
            .map_err(|_| invalid())?
            .as_str()
            != Some(kind.as_str())
        || serde_json::to_value(step.status)
            .map_err(|_| invalid())?
            .as_str()
            != Some(state.as_str())
    {
        return Err(invalid());
    }
    let [mutation] = input.plan.agent_access_grants() else {
        return Err(invalid());
    };
    let [effect] = step.effects.as_slice() else {
        return Err(invalid());
    };
    if effect
        .compensation
        .get("operation_id")
        .and_then(Value::as_str)
        != Some(input.operation_id.as_str())
    {
        return Err(invalid());
    }
    let reference =
        AgentAccessGrantRefV1::from_ensure_effect(effect, mutation).map_err(|_| invalid())?;
    if reference.owner_scope() != input.workspace_id.as_str()
        || reference.connection_id() != format!("agent-connection/{}", input.context)
    {
        return Err(invalid());
    }
    Ok(reference)
}
