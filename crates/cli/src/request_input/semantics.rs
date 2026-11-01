//! Presentation of closed input facts returned by the shared domain validators.
use hiroute_application_api::{
    AgentFacetIntent, AgentSettingsSpecV2, ErrorCode, PlanContentChangeV2,
};

pub(crate) fn plan(change: &PlanContentChangeV2) -> Result<(), ErrorCode> {
    let Some(field) = change.editor.invalid_publication_field() else {
        return Ok(());
    };
    let (category, expected) = match field {
        "purpose" => (
            "semantic_requirement",
            "a nonempty Plan purpose when delegation_enabled is true",
        ),
        "work" => (
            "semantic_requirement",
            "work with harness and protocol when delegation_enabled is true",
        ),
        "work.protocol" => (
            "unsupported_protocol",
            "responses or messages for Worker work.protocol",
        ),
        _ => (
            "out_of_range",
            "an integer in 0..1000 (thousandths of a probability)",
        ),
    };
    super::diagnostics::record(category, &format!("$.change.editor.{field}"), expected);
    Err(ErrorCode::InvalidArguments)
}

pub(crate) fn agent(spec: &AgentSettingsSpecV2) -> Result<(), ErrorCode> {
    if let AgentFacetIntent::Configure { settings } = &spec.model
        && settings.unsupported_plan_protocol()
    {
        super::diagnostics::record(
            "unsupported_protocol",
            "$.spec.model.settings.plan_protocols.*",
            "responses or messages for additional model entries; account access and Plan capability are checked separately",
        );
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(())
}
