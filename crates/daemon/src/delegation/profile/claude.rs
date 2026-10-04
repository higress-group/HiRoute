//! Claude reuses native settings while the host owns the managed model route.
use super::*;

pub(super) fn render(
    input: &ProfileInput<'_>,
    projected_tools: &[WorkerToolV1],
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    let mut rendered = RenderedHarnessProfile {
        native_model_id: input.alias.to_owned(),
        ..Default::default()
    };
    if let Some(window) = input.context_window_tokens {
        let values = hiroute_domain::claude_context_environment(window)
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        for (key, value) in values {
            rendered.env.insert(key, Zeroizing::new(value));
        }
    }
    for (key, value) in [
        ("CLAUDE_CODE_EXECUTABLE", path_string(input.harness_binary)?),
        (
            "CLAUDE_CONFIG_DIR",
            path_string(input.native_context.config_root())?,
        ),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
        ("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1".into()),
        ("ANTHROPIC_BASE_URL", format!("http://{}", input.gateway)),
        ("ANTHROPIC_AUTH_TOKEN", secret_string(&input.token)?),
        ("ANTHROPIC_MODEL", input.alias.into()),
        ("ANTHROPIC_CUSTOM_MODEL_OPTION", input.alias.into()),
    ] {
        rendered.env.insert(key.into(), Zeroizing::new(value));
    }
    let mut tools = Vec::new();
    for tool in projected_tools {
        match tool {
            WorkerToolV1::Read => tools.extend(["Read", "Glob", "Grep"]),
            WorkerToolV1::Edit => tools.extend(["Edit", "Write"]),
            WorkerToolV1::Shell => tools.push("Bash"),
        }
    }
    if input.permission_policy == WorkerPermissionPolicyV1::ApproveAll {
        tools.extend(["WebSearch", "WebFetch"]);
        if input.native_context.is_borrowed() {
            tools.push("Skill");
        }
    }
    let setting_sources = if input.native_context.is_borrowed() {
        vec!["user", "project", "local"]
    } else {
        vec![]
    };
    let mut options = json!({
        "settingSources": setting_sources, "model": input.alias, "tools": tools,
        "disallowedTools": ["Agent", "Task", "AskUserQuestion"],
        "settings": {"apiKeyHelper": ""},
    });
    if input.native_context.is_borrowed() {
        // The native flag layer must bypass user-configured proxies for this Gateway.
        // Keep other proxy settings available to native tools; never put a token here.
        let gateway_host = input.gateway.ip().to_string();
        for key in ["NO_PROXY", "no_proxy"] {
            rendered
                .env
                .insert(key.into(), Zeroizing::new(gateway_host.clone()));
        }
        options["settings"]["env"] = json!({
            "NO_PROXY": gateway_host,
            "no_proxy": gateway_host,
        });
        let bootstrap = PathBuf::from("claude-adapter-bootstrap.mjs");
        rendered.files.push(RunMaterialFile {
            relative_path: bootstrap.clone(),
            contents: Zeroizing::new(include_bytes!("claude_adapter_bootstrap.mjs").to_vec()),
            executable: false,
        });
        rendered.adapter_bootstrap = Some(bootstrap);
        // Reusing skills does not expand this profile to ambient MCP or native user hooks.
        // Plugin skills remain discoverable; managed-policy hooks are controlled natively.
        options["strictMcpConfig"] = json!(true);
        options["mcpServers"] = json!({});
        options["settings"]["disableAllHooks"] = json!(true);
    } else {
        options["mcpServers"] = json!({});
        options["plugins"] = json!([]);
        options["hooks"] = json!({});
    }
    if let Some(effort) = input.native_effort {
        options["effort"] = json!(effort);
    }
    // Omitted injection options do not disable native MCP, plugins or hooks. Discovery and
    // permission behavior remain separate native capabilities, verified through real runs.
    rendered
        .session_meta
        .insert("claudeCode".into(), json!({"options": options}));
    Ok(rendered)
}
