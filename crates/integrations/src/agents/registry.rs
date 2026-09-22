use std::collections::BTreeSet;

use hiroute_domain::{
    AGENT_PROFILE_SCHEMA_V1, AgentIngressProtocolV1, AgentKindV1, AgentProfileV1, ConfigLayerV1,
    FunctionToolV1, ManagedLaunchProfileV1, OwnedConfigFieldV1, SpawnGuidanceProfileV1,
};
use serde_json::json;

#[cfg(test)]
pub const CLAUDE_CODE_VERIFIED_VERSION_V1: &str = "2.1.0";
#[cfg(test)]
pub const CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1: &str = "2.1.231";
pub const CODEX_PROFILE_ID_V1: &str = "codex-responses-v1";
pub const CLAUDE_PROFILE_ID_V1: &str = "claude-messages-v1";
pub const CODEX_INTEGRATION_PROFILE_REF_V1: &str = "builtin/codex-responses/v1";
pub const CLAUDE_INTEGRATION_PROFILE_REF_V1: &str = "builtin/claude-messages/v1";

pub fn builtin_agent_profiles() -> Vec<AgentProfileV1> {
    vec![codex_profile_v1(), claude_code_profile_v1()]
}

pub fn codex_profile_v1() -> AgentProfileV1 {
    let tool = codex_spawn_agent_tool_v1();
    AgentProfileV1 {
        schema: AGENT_PROFILE_SCHEMA_V1.to_owned(),
        profile_id: CODEX_PROFILE_ID_V1.to_owned(),
        integration_profile_ref: CODEX_INTEGRATION_PROFILE_REF_V1.to_owned(),
        kind: AgentKindV1::Codex,
        // Codex versions are diagnostic only. MVP admission never uses a binary-version
        // allowlist; concrete Desktop and CLI surfaces locate and launch their actual engine.
        legacy_exact_versions: BTreeSet::new(),
        ingress_protocol: AgentIngressProtocolV1::Responses,
        config_precedence: AgentKindV1::Codex.config_precedence().to_vec(),
        owned_config_fields: vec![
            field("provider", "model_provider"),
            field("default_model", "model"),
            field("base_endpoint", "model_providers.hiroute.base_url"),
            field("wire_api", "model_providers.hiroute.wire_api"),
            field("static_catalog", "model_catalog_json"),
        ],
        dynamic_catalog: true,
        static_catalog_fallback: true,
        native_subagent_routing: true,
        spawn_guidance: Some(SpawnGuidanceProfileV1 {
            tool_name: "spawn_agent".to_owned(),
            registration_id: tool.registration_id.clone(),
            begin_marker:
                "Available model overrides (optional; inherited parent model is preferred):"
                    .to_owned(),
            end_marker: "Spawn arguments:".to_owned(),
            expected_shape_digest: tool
                .shape_digest()
                .expect("built-in tool shape is serializable"),
        }),
        managed_launch: None,
    }
}

pub fn claude_code_profile_v1() -> AgentProfileV1 {
    AgentProfileV1 {
        schema: AGENT_PROFILE_SCHEMA_V1.to_owned(),
        profile_id: CLAUDE_PROFILE_ID_V1.to_owned(),
        integration_profile_ref: CLAUDE_INTEGRATION_PROFILE_REF_V1.to_owned(),
        kind: AgentKindV1::ClaudeCode,
        legacy_exact_versions: BTreeSet::new(),
        ingress_protocol: AgentIngressProtocolV1::Messages,
        config_precedence: AgentKindV1::ClaudeCode.config_precedence().to_vec(),
        owned_config_fields: vec![
            field("base_endpoint", "env.ANTHROPIC_BASE_URL"),
            field("default_model", "env.ANTHROPIC_MODEL"),
            field("default_opus_model", "env.ANTHROPIC_DEFAULT_OPUS_MODEL"),
            field("default_sonnet_model", "env.ANTHROPIC_DEFAULT_SONNET_MODEL"),
            field("default_haiku_model", "env.ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            field("small_fast_model", "env.ANTHROPIC_SMALL_FAST_MODEL"),
            // These are adapter-semantic paths. The Claude renderer maps them to the native
            // apiKeyHelper field and the complete auth/provider environment family without ever
            // placing the prior helper command or credential bytes in an Operation journal.
            field("auth_helper", "apiKeyHelper"),
            field("auth_environment", "hiroute.auth_environment"),
        ],
        dynamic_catalog: false,
        static_catalog_fallback: false,
        native_subagent_routing: false,
        spawn_guidance: None,
        managed_launch: Some(ManagedLaunchProfileV1::claude_code()),
    }
}

pub fn codex_spawn_agent_tool_v1() -> FunctionToolV1 {
    FunctionToolV1 {
        registration_id: "codex.builtin.spawn-agent.v1".to_owned(),
        name: "spawn_agent".to_owned(),
        kind: "function".to_owned(),
        description: concat!(
            "Delegate bounded work to a native subagent.\n\n",
            "Available model overrides (optional; inherited parent model is preferred):",
            "\nbuilt-in defaults\n\n",
            "Spawn arguments: task, fork_turns, and optional model."
        )
        .to_owned(),
        parameters: json!({
            "additionalProperties": false,
            "properties": {
                "fork_turns": {"type": "string"},
                "model": {"type": "string"},
                "task": {"type": "string"}
            },
            "required": ["task"],
            "type": "object"
        }),
    }
}

fn field(field_id: &str, path: &str) -> OwnedConfigFieldV1 {
    OwnedConfigFieldV1::new(field_id, path, ConfigLayerV1::User)
        .expect("built-in config path is valid")
}
