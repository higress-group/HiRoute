//! Native ACP arguments and run materials. Shared Qoder settings live in integrations.
use super::*;
use hiroute_integrations::agents::{QoderTransientRouteInput, render_qoder_transient_route};

pub(super) fn render(
    input: &ProfileInput<'_>,
    private_root: &Path,
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    let context_window_tokens = input
        .context_window_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let max_output_tokens = input
        .max_output_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let task_key = input
        .session_root
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(DelegationErrorV1::InvalidArguments)?;
    let provider_id = format!("hiroute-worker-{task_key}");
    let endpoint = format!("http://{}/v1", input.gateway);
    let route = render_qoder_transient_route(QoderTransientRouteInput {
        protocol: input.protocol,
        provider_id: &provider_id,
        endpoint: &endpoint,
        alias: input.alias,
        credential_env: "HIROUTE_RUN_TOKEN",
        context_window_tokens: Some(context_window_tokens),
        max_output_tokens,
    })
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let settings_file = PathBuf::from("qoder-worker-settings.json");
    let settings =
        serde_json::to_vec(&route.settings).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    let mut rendered = RenderedHarnessProfile {
        native_model_id: route.native_model_id.clone(),
        native_args: vec![
            "--acp".into(),
            "--config-dir".into(),
            input.native_context.config_root().to_owned(),
            "--setting-sources".into(),
            "user,project,local".into(),
            "--settings".into(),
            private_root.join(&settings_file),
            "--model".into(),
            route.native_model_id.clone().into(),
            "--permission-mode".into(),
            "yolo".into(),
            // Nested Agent and other model-using tools have no proven frozen-route contract.
            // Keep the current native tool names explicit instead of granting unknown tools.
            "--tools".into(),
            "Read,Write,Edit,Bash,Grep,Glob,Skill".into(),
            "--strict-mcp-config".into(),
            "--mcp-config".into(),
            r#"{"mcpServers":{}}"#.into(),
        ],
        files: vec![RunMaterialFile {
            relative_path: settings_file,
            contents: Zeroizing::new(settings),
            executable: false,
        }],
        ..Default::default()
    };
    for (key, value) in [
        (
            "QODER_CONFIG_DIR",
            path_string(input.native_context.config_root())?,
        ),
        ("QODER_MODEL", route.native_model_id.clone()),
        ("QODER_SUBAGENT_MODEL", route.native_model_id),
        ("HIROUTE_RUN_TOKEN", secret_string(&input.token)?),
    ] {
        rendered.env.insert(key.into(), Zeroizing::new(value));
    }
    Ok(rendered)
}
