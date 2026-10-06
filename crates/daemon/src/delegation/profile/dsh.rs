//! DSH's public CLI/configuration and ACP transport; no native SDK or history decoder.
use super::*;

pub(super) fn render(
    input: &ProfileInput<'_>,
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    let context = input
        .context_window_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let output = input
        .max_output_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    if output == 0 || context <= output || context > (1 << 53) - 1 {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let provider = "hiroute-worker";
    let (api, endpoint) = hiroute_integrations::agents::pi_native_provider_api(
        input.protocol,
        &format!("http://{}/v1", input.gateway),
    )
    .map(|(api, endpoint)| (api, endpoint.to_owned()))
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let state = input.session_root.path().join("native-dsh");
    // JSON is a YAML subset. Only public stock-composition IDs are overridden;
    // secret values are referenced by name, never persisted in this overlay.
    let mut patch = vec![
        json!({"id":"llm-pi-ai","config":{"providers":{provider:{
            "apiKeyEnv":"HIROUTE_RUN_TOKEN","api":api,"baseURL":endpoint,
            "models":[{"id":input.alias,"contextWindow":context,"maxTokens":output,"input":["text"]}]
        }}}}),
        json!({"id":"acp","config":{"provider":provider,"model":input.alias}}),
        json!({"id":"agent-default-model","config":{"provider":provider,"model":input.alias}}),
        json!({"id":"session-persistence-jsonl","config":{"root":state.join("sessions"),"compression":"none"}}),
        json!({"id":"attachment-local","config":{"dshHome":state}}),
        // Keep native Skill roots and customSkillDirs from the borrowed composition.
        json!({"id":"tool-bash","config":{"enableRunInBackground":false,"promoteOnTimeout":false}}),
        json!({"id":"tool-terminal","config":{"enableRunInBackground":false}}),
        json!({"id":"session-log-deepseek","config":{"enabled":false}}),
    ];
    for id in [
        "llm-deepseek",
        "llm-deepseek-account",
        "session-telemetry-otel",
        "session-title-llm",
        "tool-subagent",
        "tool-subagent-fork",
        "tool-subagent-control",
        "tool-subagent-list-agents",
        "tool-workflow",
        "tool-goal",
        "tool-ralph",
        "tool-web",
    ] {
        patch.push(json!({"id":id,"disabled":true}));
    }
    let relative = PathBuf::from("dsh-worker.patch.yml");
    let mut rendered = RenderedHarnessProfile {
        native_model_id: json!([provider, input.alias]).to_string(),
        native_args: vec![
            "--profile".into(),
            "acp".into(),
            "--patch".into(),
            input.private_root.join(&relative),
        ],
        files: vec![RunMaterialFile {
            relative_path: relative,
            contents: Zeroizing::new(
                serde_json::to_vec(&patch).map_err(|_| DelegationErrorV1::InvalidArguments)?,
            ),
            executable: false,
        }],
        ..Default::default()
    };
    for (name, value) in [
        ("DSH_HOME", path_string(input.native_context.config_root())?),
        ("DSH_PERMISSION_MODE", "danger-full-access".into()),
        ("DSH_TELEMETRY_DISABLED", "1".into()),
        ("HIROUTE_RUN_TOKEN", secret_string(&input.token)?),
    ] {
        rendered.env.insert(name.into(), Zeroizing::new(value));
    }
    Ok(rendered)
}
