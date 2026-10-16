//! Restore a complete public Plan read through the existing content Preview/Apply owner.
use super::*;

pub(super) fn prepare(client: &LocalControlClient, plan_id: &str, path: &str) -> Result<Value> {
    let value = crate::request_input::RequestInput::File(std::path::Path::new(path))
        .read(crate::request_input::MAX_REQUEST_BYTES)
        .map_err(error)?;
    let envelope: MachineEnvelopeV2<AgentPlanStatusV2> =
        crate::request_input::deserialize(value).map_err(error)?;
    if envelope.schema_version.major != 2
        || envelope.schema_version.minor != 0
        || envelope.status != MachineStatus::Succeeded
        || envelope.error.is_some()
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    let saved = envelope
        .data
        .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
    if saved.agent_plan_id.as_str() != plan_id
        || saved.head.reference.plan_id != saved.agent_plan_id
        || saved.head.model_alias != saved.model_alias
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    // This is the same projection used by Application reopening: no client defaults,
    // selected-field reconstruction or replacement of frozen resource references.
    let editor = saved
        .desired
        .editor(Some(saved.model_alias.as_str().into()))
        .map_err(|_| error(ErrorCode::InvalidArguments))?;
    let current: AgentPlanStatusV2 = serde_json::from_value(data(
        client,
        "GetAgentPlanStatus",
        json!({"agent_plan_id":plan_id}),
    )?)
    .map_err(|_| error(ErrorCode::Internal))?;
    if current.agent_plan_id != saved.agent_plan_id
        || current.head.reference.workspace_id != saved.head.reference.workspace_id
        || current.model_alias != saved.model_alias
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    Ok(json!(PlanContentPreviewRequestV2 {
        change: PlanContentChangeV2 {
            schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
            target: PlanContentTargetV2::Update {
                plan_id: current.agent_plan_id,
                expected_head_revision: current.head.head_revision,
            },
            editor,
            consumed_draft: None,
        }
    }))
}
