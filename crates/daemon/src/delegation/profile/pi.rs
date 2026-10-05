//! Pi's native resources are borrowed; SDK transport and exact history stay task-owned.
use super::*;

pub(super) fn render(
    input: &ProfileInput<'_>,
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    hiroute_integrations::agents::pi_cli_installation(input.harness_binary)
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let context = input
        .context_window_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let output = input
        .max_output_tokens
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    if context <= output || context > (1 << 53) - 1 || output == 0 || output > (1 << 53) - 1 {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let provider = "hiroute-worker";
    let mut rendered = RenderedHarnessProfile {
        native_model_id: format!("{provider}/{}", input.alias),
        files: vec![
            RunMaterialFile {
                relative_path: "pi-worker-bridge.mjs".into(),
                contents: Zeroizing::new(include_bytes!("pi_worker_bridge.mjs").to_vec()),
                executable: false,
            },
            RunMaterialFile {
                relative_path: "pi-sdk-contract.mjs".into(),
                contents: Zeroizing::new(hiroute_integrations::PI_SDK_CONTRACT.as_bytes().to_vec()),
                executable: false,
            },
        ],
        ..Default::default()
    };
    let gateway_v1_base = format!("http://{}/v1", input.gateway);
    let (api, endpoint) =
        hiroute_integrations::agents::pi_native_provider_api(input.protocol, &gateway_v1_base)
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let route = json!({"api":api,"minimumNode":hiroute_integrations::PI_NODE_MINIMUM,"provider":provider,"endpoint":endpoint,
        "model":{"id":input.alias,"name":"HiRoute frozen Plan","reasoning":false,"input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"contextWindow":context,"maxTokens":output}});
    for (key, value) in [
        (
            "PI_CODING_AGENT_DIR",
            path_string(input.native_context.config_root())?,
        ),
        (
            "HIROUTE_PI_SESSION_ROOT",
            path_string(input.session_root.path())?,
        ),
        ("HIROUTE_PI_ROUTE", route.to_string()),
        ("HIROUTE_RUN_TOKEN", secret_string(&input.token)?),
        ("PI_OFFLINE", "1".into()),
        ("PI_TELEMETRY", "0".into()),
    ] {
        rendered.env.insert(key.into(), Zeroizing::new(value));
    }
    Ok(rendered)
}
