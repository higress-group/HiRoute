use super::*;
use crate::agents::{CodexCatalogMetadataSourceV1, CodexDefaultPolicy};
use serde_json::json;

fn original() -> Value {
    serde_json::from_str(include_str!("codex_bundled_catalog.json")).unwrap()
}

fn validate(value: Value) -> Result<(), CodexCatalogError> {
    CodexCatalogSelection::parse(value)?.validate_schema()
}

#[test]
fn pinned_original_schema_passes_without_value_changes() {
    let value = original();
    let source = CodexCatalogSelection::parse(value.clone()).unwrap();
    source.validate_schema().unwrap();
    assert_eq!(source.original(), &value);
}

macro_rules! missing_required {
    ($($test:ident => $field:literal),+ $(,)?) => {$(
        #[test]
        fn $test() {
            let mut value = original();
            value["models"][0].as_object_mut().unwrap().remove($field);
            assert_eq!(validate(value), Err(CodexCatalogError::InvalidCatalog));
        }
    )+};
}

missing_required! {
    missing_display_name_is_rejected => "display_name",
    missing_effort_options_is_rejected => "supported_reasoning_levels",
    missing_shell_type_is_rejected => "shell_type",
    missing_visibility_is_rejected => "visibility",
    missing_api_support_is_rejected => "supported_in_api",
    missing_priority_is_rejected => "priority",
    missing_verbosity_support_is_rejected => "support_verbosity",
    missing_truncation_policy_is_rejected => "truncation_policy",
    missing_experimental_tools_is_rejected => "experimental_supported_tools",
}

macro_rules! invalid_field {
    ($($test:ident => ($field:literal, $value:expr)),+ $(,)?) => {$(
        #[test]
        fn $test() {
            let mut value = original();
            value["models"][0][$field] = $value;
            assert_eq!(validate(value), Err(CodexCatalogError::InvalidCatalog));
        }
    )+};
}

invalid_field! {
    unknown_shell_is_rejected => ("shell_type", json!("freeform_shell")),
    unknown_patch_tool_is_rejected => ("apply_patch_tool_type", json!("function")),
    unknown_web_tool_is_rejected => ("web_search_tool_type", json!("image")),
    unknown_modality_is_rejected => ("input_modalities", json!(["video"])),
    unknown_summary_is_rejected => ("default_reasoning_summary", json!("full")),
    unknown_verbosity_is_rejected => ("default_verbosity", json!("ultra")),
    empty_default_effort_is_rejected => ("default_reasoning_level", json!("")),
    invalid_effort_preset_is_rejected => ("supported_reasoning_levels", json!([{"effort":"low"}])),
    numeric_effort_is_rejected => ("default_reasoning_level", json!(1)),
    invalid_service_tier_is_rejected => ("service_tiers", json!([{"id":"fast","name":"Fast"}])),
    invalid_upgrade_is_rejected => ("upgrade", json!({"model":"new"})),
    invalid_availability_is_rejected => ("availability_nux", json!({"message":false})),
    null_defaulted_boolean_is_rejected => ("supports_search_tool", Value::Null),
    null_defaulted_array_is_rejected => ("service_tiers", Value::Null),
    floating_context_is_rejected => ("context_window", json!(1.5)),
    overflowing_context_is_rejected => ("context_window", json!(u64::MAX)),
    invalid_truncation_is_rejected => ("truncation_policy", json!({"mode":"bytes","limit":"10000"})),
    numeric_selector_is_rejected => ("tool_mode", json!(7)),
    string_parallel_support_is_rejected => ("supports_parallel_tool_calls", json!("unknown")),
    null_parallel_support_is_rejected => ("supports_parallel_tool_calls", Value::Null),
    numeric_parallel_support_is_rejected => ("supports_parallel_tool_calls", json!(1)),
    malformed_instructions_is_rejected => ("model_messages", json!({"instructions_template":false})),
    malformed_legacy_instructions_is_rejected => ("base_instructions", json!(false)),
    malformed_personality_is_rejected => ("model_messages", json!({"instructions_template":"generic","instructions_variables":{"personality_default":1}})),
    malformed_approval_is_rejected => ("model_messages", json!({"instructions_template":"generic","approvals":{"never":[]}})),
    malformed_collaboration_is_rejected => ("model_messages", json!({"instructions_template":"generic","collaboration_modes":{"plan":1}})),
    malformed_review_is_rejected => ("model_messages", json!({"instructions_template":"generic","auto_review":{"policy":true}})),
    malformed_permissions_is_rejected => ("model_messages", json!({"instructions_template":"generic","permissions":{"read_only":1}})),
    incomplete_token_budget_is_rejected => ("model_messages", json!({"instructions_template":"generic","token_budget":{"reminder_threshold_tokens":1}})),
}

#[test]
fn missing_both_instruction_forms_is_rejected() {
    let mut value = original();
    value["models"][0]
        .as_object_mut()
        .unwrap()
        .remove("base_instructions");
    value["models"][0]["model_messages"] = Value::Null;
    assert_eq!(validate(value), Err(CodexCatalogError::InvalidCatalog));
}

#[test]
fn current_consumer_original_instruction_form_is_not_rewritten() {
    let mut value = original();
    value["models"][0]["model_messages"] = Value::Null;
    value["models"][0]["base_instructions"] = json!("original instructions");
    let source = CodexCatalogSelection::parse(value.clone()).unwrap();
    source.validate_schema().unwrap();
    assert_eq!(source.original(), &value);
}

#[test]
fn custom_effort_and_unknown_string_selectors_follow_current_consumer() {
    let mut value = original();
    value["models"][0]["default_reasoning_level"] = json!("future-effort");
    value["models"][0]["supported_reasoning_levels"] =
        json!([{"effort":"future-effort","description":"Native"}]);
    value["models"][0]["tool_mode"] = json!("future-selector");
    value["models"][0]["multi_agent_version"] = json!("future-selector");
    value["models"][0]["unknown_nested"] = json!({"preserve":[null,1]});
    validate(value).unwrap();
}

#[test]
fn omitted_defaulted_and_nullable_fields_do_not_get_inserted() {
    let mut value = original();
    let model = value["models"][0].as_object_mut().unwrap();
    for field in [
        "description",
        "additional_speed_tiers",
        "service_tiers",
        "input_modalities",
        "default_reasoning_level",
        "context_window",
        "supports_parallel_tool_calls",
        "include_apps_usage_instructions",
        "default_reasoning_summary",
    ] {
        model.remove(field);
    }
    let source = CodexCatalogSelection::parse(value.clone()).unwrap();
    source.validate_schema().unwrap();
    assert_eq!(source.original(), &value);
}

#[test]
fn append_rejects_selection_only_projection_even_without_new_plans() {
    let value = json!({"models":[{"slug":"native","priority":0,"visibility":"list","supported_in_api":true}]});
    let source = CodexCatalogSelection::parse(value).unwrap();
    assert_eq!(
        source.append_plans(
            &[],
            CodexDefaultPolicy {
                explicit_model: None,
                uses_codex_backend: false,
                allow_provider_model_fallback: false,
            },
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::InvalidCatalog)
    );
}
