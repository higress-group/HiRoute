//! Codex startup and session overrides come from the same managed configuration.
use super::*;
use hiroute_domain::CanonicalDigest;

pub(super) fn render(
    input: &ProfileInput<'_>,
    tools: &[WorkerToolV1],
    network: WorkerNetworkV1,
    native_session_mode: &str,
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    let catalog_path = input
        .codex_catalog
        .map(|catalog| {
            input
                .session_root
                .prepare_codex_catalog(input.alias, catalog)
        })
        .transpose()?;
    let borrowed = input.native_context.is_borrowed();
    if borrowed && catalog_path.is_none() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let provider = if borrowed {
        let digest = CanonicalDigest::of(&(
            "hiroute-worker-provider-v1",
            path_string(input.session_root.path())?,
        ))
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        format!(
            "hiroute_worker_{}",
            digest.as_str().trim_start_matches("sha256:")
        )
    } else {
        input
            .session_root
            .prepare_codex_provider(input.gateway, catalog_path.as_deref())?;
        "hiroute".into()
    };
    let (approval_policy, sandbox_mode) = match input.permission_policy {
        WorkerPermissionPolicyV1::ApproveAll => ("never", "danger-full-access"),
        WorkerPermissionPolicyV1::ApproveReads | WorkerPermissionPolicyV1::DenyAll => {
            ("on-request", "read-only")
        }
    };
    let mut config = json!({
        "model": input.alias,
        "model_provider": provider,
        "approval_policy": approval_policy,
        "sandbox_mode": sandbox_mode,
        "model_providers": {&provider: {
            "name": "HiRoute managed run",
            "base_url": format!("http://{}/v1", input.gateway),
            "wire_api": "responses", "supports_websockets": false,
            "env_key": "HIROUTE_RUN_TOKEN", "requires_openai_auth": false
        }},
        "features": {"multi_agent": false, "shell_tool": tools.contains(&WorkerToolV1::Shell)},
        "web_search": if network == WorkerNetworkV1::Allowed { "live" } else { "disabled" },
        "sandbox_workspace_write": {"network_access": network == WorkerNetworkV1::Allowed},
    });
    if let Some(effort) = input.native_effort {
        config["model_reasoning_effort"] = json!(effort);
    }
    if let Some(path) = catalog_path {
        config["model_catalog_json"] = json!(path_string(&path)?);
    }
    let mut rendered = RenderedHarnessProfile {
        native_model_id: input.alias.to_owned(),
        ..Default::default()
    };
    let codex_path = if borrowed {
        // codex-acp starts CODEX_PATH with only `app-server`. Session CODEX_CONFIG is
        // applied after account/read, so the launcher must install these flags first.
        if !cfg!(unix) {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        let relative_path = PathBuf::from("codex-worker-launcher");
        rendered.files.push(RunMaterialFile {
            relative_path: relative_path.clone(),
            contents: Zeroizing::new(render_launcher(input.harness_binary, &config)?.into_bytes()),
            executable: true,
        });
        input.private_root.join(relative_path)
    } else {
        input.harness_binary.to_owned()
    };
    for (key, value) in [
        ("CODEX_CONFIG", config.to_string()),
        ("INITIAL_AGENT_MODE", native_session_mode.into()),
        ("MODEL_PROVIDER", provider),
        ("CODEX_PATH", path_string(&codex_path)?),
        (
            "CODEX_HOME",
            path_string(input.native_context.config_root())?,
        ),
        ("HIROUTE_RUN_TOKEN", secret_string(&input.token)?),
    ] {
        rendered.env.insert(key.into(), Zeroizing::new(value));
    }
    Ok(rendered)
}

pub(super) fn render_launcher(binary: &Path, config: &Value) -> Result<String, DelegationErrorV1> {
    let mut overrides = Vec::new();
    collect_overrides("", config, &mut overrides)?;
    let mut script = format!("#!/bin/sh\nexec {}", shell_quote(&path_string(binary)?));
    for value in overrides {
        script.push_str(&format!(" \\\n  -c {}", shell_quote(&value)));
    }
    script.push_str(" \\\n  \"$@\"\n");
    Ok(script)
}

/// The managed object contains only nested tables and scalar strings/bools/numbers.
/// Flatten it into native dotted TOML overrides without reading or rewriting user config.
fn collect_overrides(
    prefix: &str,
    value: &Value,
    output: &mut Vec<String>,
) -> Result<(), DelegationErrorV1> {
    match value {
        Value::Object(table) => {
            for (key, value) in table {
                // Native CLI override keys split on dots; they are not TOML key syntax.
                // All managed keys, including the generated provider name, use this alphabet.
                if key.is_empty()
                    || !key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return Err(DelegationErrorV1::InvalidArguments);
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_overrides(&path, value, output)?;
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Number(_) if !prefix.is_empty() => {
            output.push(format!("{prefix}={value}"));
        }
        _ => return Err(DelegationErrorV1::InvalidArguments),
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
